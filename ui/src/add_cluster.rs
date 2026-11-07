//! Add Cluster…, opened from File › Manage Clusters… (the menu is the only
//! way in): adds a context to the kubeconfig kubectl uses, typed in (server,
//! CA, credentials) or fetched from Google Cloud, AWS or Azure by their own
//! CLIs. Every window's context list then shows it, and this window can
//! connect to it straight away.
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use subduer_types::{AddedCluster, CloudAccounts, CloudCluster, CloudProvider, ManagedContext, ManagedContexts, NewCluster};

use crate::app::{Store, guard};
use crate::icons::ProviderLogo;
use crate::logic::add_cluster::{self as logic, AUTHS, AWS_REGIONS, DEFAULT_AUTH, DEFAULT_SOURCE, ManualInput, SOURCES, Source};
use crate::modal::{Generation, Modal};
use crate::tauri::{NoArgs, invoke};

/// The typed-in form.
#[derive(Clone, Copy)]
struct Manual {
    name: RwSignal<String>,
    server: RwSignal<String>,
    ca: RwSignal<String>,
    insecure: RwSignal<bool>,
    proxy: RwSignal<String>,
    /// `none`, `token`, `cert` or `exec`.
    auth: RwSignal<&'static str>,
    token: RwSignal<String>,
    cert: RwSignal<String>,
    key: RwSignal<String>,
    command: RwSignal<String>,
    args: RwSignal<String>,
    namespace: RwSignal<String>,
}

impl Manual {
    fn new() -> Self {
        Self {
            name: RwSignal::new(String::new()),
            server: RwSignal::new(String::new()),
            ca: RwSignal::new(String::new()),
            insecure: RwSignal::new(false),
            proxy: RwSignal::new(String::new()),
            auth: RwSignal::new(DEFAULT_AUTH),
            token: RwSignal::new(String::new()),
            cert: RwSignal::new(String::new()),
            key: RwSignal::new(String::new()),
            command: RwSignal::new(String::new()),
            args: RwSignal::new(String::new()),
            namespace: RwSignal::new(String::new()),
        }
    }

    fn clear(self) {
        for s in [self.name, self.server, self.ca, self.proxy, self.token, self.cert, self.key, self.command, self.args, self.namespace] {
            s.set(String::new());
        }
        self.insecure.set(false);
        self.auth.set(DEFAULT_AUTH);
    }

    /// Something was typed: only then is what's missing pointed out.
    fn touched(self) -> bool {
        logic::touched(&self.name.get(), &self.server.get())
    }

    fn cluster(self, make_current: bool) -> NewCluster {
        let input = ManualInput {
            name: self.name.get(),
            server: self.server.get(),
            ca: self.ca.get(),
            insecure: self.insecure.get(),
            proxy: self.proxy.get(),
            auth: self.auth.get(),
            token: self.token.get(),
            cert: self.cert.get(),
            key: self.key.get(),
            command: self.command.get(),
            args: self.args.get(),
            namespace: self.namespace.get(),
        };
        input.cluster(make_current)
    }
}

/// The cloud form: the account, the clusters it lists and the one picked.
#[derive(Clone, Copy)]
struct Cloud {
    accounts: RwSignal<Option<Result<CloudAccounts, String>>>,
    account: RwSignal<String>,
    region: RwSignal<String>,
    clusters: RwSignal<Option<Result<Vec<CloudCluster>, String>>>,
    picked: RwSignal<Option<CloudCluster>>,
    alias: RwSignal<String>,
    /// Drops answers meant for another provider, account or opening.
    asked: Generation,
}

impl Cloud {
    fn new() -> Self {
        Self {
            accounts: RwSignal::new(None),
            account: RwSignal::new(String::new()),
            region: RwSignal::new(String::new()),
            clusters: RwSignal::new(None),
            picked: RwSignal::new(None),
            alias: RwSignal::new(String::new()),
            asked: Generation::new(),
        }
    }

    fn clear(self) {
        self.asked.next();
        self.accounts.set(None);
        self.account.set(String::new());
        self.region.set(String::new());
        self.clusters.set(None);
        self.picked.set(None);
        self.alias.set(String::new());
    }

    /// Asks the CLI for accounts, then lists the default one's clusters.
    fn load_accounts(self, provider: CloudProvider) {
        #[derive(Serialize)]
        struct Args {
            provider: CloudProvider,
        }
        self.clear();
        let asked = self.asked.now();
        spawn_local(async move {
            let result = invoke::<_, CloudAccounts>("cloud_accounts", &Args { provider }).await;
            if !self.asked.still(asked) {
                return;
            }
            if let Ok(found) = &result {
                self.choose_account(provider, found, logic::default_account(provider, found));
            }
            self.accounts.set(Some(result));
        });
    }

    fn choose_account(self, provider: CloudProvider, found: &CloudAccounts, account: String) {
        if let Some(region) = logic::account_region(provider, found, &account) {
            self.region.set(region);
        }
        self.account.set(account);
        self.load_clusters(provider);
    }

    fn load_clusters(self, provider: CloudProvider) {
        #[derive(Serialize)]
        struct Args {
            provider: CloudProvider,
            account: String,
            region: String,
        }
        self.asked.next();
        self.picked.set(None);
        let account = self.account.get_untracked();
        let region = self.region.get_untracked();
        if !logic::ready_to_list(provider, &account, &region) {
            self.clusters.set(Some(Ok(vec![])));
            return;
        }
        self.clusters.set(None);
        let asked = self.asked.now();
        spawn_local(async move {
            let result = invoke::<_, Vec<CloudCluster>>("cloud_clusters", &Args { provider, account, region }).await;
            if self.asked.still(asked) {
                if let Some(only) = logic::only_cluster(&result) {
                    self.picked.set(Some(only));
                }
                self.clusters.set(Some(result));
            }
        });
    }
}

#[component]
pub fn AddClusterDialog(store: Store) -> impl IntoView {
    let source = RwSignal::new(DEFAULT_SOURCE);
    let manual = Manual::new();
    let cloud = Cloud::new();
    let make_current = RwSignal::new(false);
    let connect = RwSignal::new(true);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let target = RwSignal::new(None::<String>);
    // The kubeconfig's contexts, to say when a cluster is in it already.
    let existing = RwSignal::new(Vec::<ManagedContext>::new());
    let opened = Generation::new();

    Effect::new(move |_| {
        if store.add_cluster_open.get() {
            opened.next();
            let at = opened.now();
            spawn_local(async move {
                let path = invoke::<_, String>("kubeconfig_target", &NoArgs {}).await.ok();
                if opened.still(at) {
                    target.set(path);
                }
                let list = invoke::<_, ManagedContexts>("managed_contexts", &NoArgs {}).await;
                if opened.still(at) {
                    existing.set(list.map(|l| l.contexts).unwrap_or_default());
                }
            });
            if let Some(provider) = source.get_untracked() {
                untrack(|| cloud.load_accounts(provider));
            }
        }
    });

    let close = move || {
        opened.next();
        manual.clear();
        cloud.clear();
        source.set(DEFAULT_SOURCE);
        make_current.set(false);
        connect.set(true);
        busy.set(false);
        error.set(None);
        store.add_cluster_open.set(false);
    };
    let pick_source = move |s: Source| {
        if logic::same_source(source.get_untracked(), s, cloud.accounts.with_untracked(Option::is_some)) {
            return;
        }
        error.set(None);
        source.set(s);
        match s {
            Some(provider) => cloud.load_accounts(provider),
            None => cloud.clear(),
        }
    };
    let problem = move || logic::problem(source.get(), &|| manual.cluster(false).problem(), cloud.picked.with(Option::is_some));
    let ok = move || problem().is_none() && !busy.get();
    // Done: every window lists it (the backend tells them); this one connects.
    let added = move |added: AddedCluster| {
        let go = connect.get_untracked();
        close();
        if go {
            let ctx = added.context;
            spawn_local(async move {
                store.refresh_contexts().await;
                crate::trace::before_switch(store, ctx.clone(), move || guard(store, move || spawn_local(store.connect(ctx))));
            });
        }
    };
    let add = move || {
        if !ok() {
            return;
        }
        busy.set(true);
        error.set(None);
        let at = opened.now();
        let current = make_current.get_untracked();
        match source.get_untracked() {
            None => {
                #[derive(Serialize)]
                struct Args {
                    cluster: NewCluster,
                }
                let cluster = manual.cluster(current);
                spawn_local(async move {
                    let result = invoke::<_, AddedCluster>("add_cluster", &Args { cluster }).await;
                    if opened.still(at) {
                        match result {
                            Ok(a) => added(a),
                            Err(e) => {
                                busy.set(false);
                                error.set(Some(e));
                            }
                        }
                    }
                });
            }
            Some(provider) => {
                #[derive(Serialize)]
                struct Args {
                    provider: CloudProvider,
                    account: String,
                    cluster: CloudCluster,
                    alias: String,
                    make_current: bool,
                }
                let Some(cluster) = cloud.picked.get_untracked() else { return };
                let args = Args { provider, account: cloud.account.get_untracked(), cluster, alias: cloud.alias.get_untracked(), make_current: current };
                spawn_local(async move {
                    let result = invoke::<_, AddedCluster>("add_cloud_cluster", &args).await;
                    if opened.still(at) {
                        match result {
                            Ok(a) => added(a),
                            Err(e) => {
                                busy.set(false);
                                error.set(Some(e));
                            }
                        }
                    }
                });
            }
        }
    };

    move || {
        store.add_cluster_open.get().then(|| {
            let cards = SOURCES
                .into_iter()
                .map(|(s, name, sub)| {
                    view! {
                        <button
                            class="source-card"
                            class:on=move || source.get() == s
                            aria-pressed=move || (source.get() == s).to_string()
                            prop:disabled=move || busy.get()
                            on:click=move |_| pick_source(s)
                        >
                            <ProviderLogo provider=s />
                            <span class="source-name">{name}</span>
                            <span class="source-sub muted small">{sub}</span>
                        </button>
                    }
                })
                .collect_view();
            view! {
                <Modal on_cancel=move |()| close() busy=busy wide=true class="add-cluster">
                    <h2>"Add Cluster"</h2>
                    <div class="source-cards" role="group" aria-label="Where the cluster is">{cards}</div>
                    {move || match source.get() {
                        None => view! { <ManualForm form=manual existing=existing busy=busy /> }.into_any(),
                        Some(provider) => view! { <CloudForm provider=provider cloud=cloud existing=existing busy=busy /> }.into_any(),
                    }}
                    <div class="add-cluster-checks">
                        <label class="check">
                            <input type="checkbox" prop:checked=move || make_current.get() prop:disabled=move || busy.get()
                                on:change=move |ev| make_current.set(event_target_checked(&ev)) />
                            "Make it kubectl's current context"
                        </label>
                        <label class="check">
                            <input type="checkbox" prop:checked=move || connect.get() prop:disabled=move || busy.get()
                                on:change=move |ev| connect.set(event_target_checked(&ev)) />
                            "Connect to it now"
                        </label>
                    </div>
                    <p class="muted small add-cluster-target">
                        {move || match target.get() {
                            Some(path) => view! { "Saved to "<span class="mono">{path}</span>", shared with kubectl and other tools." }.into_any(),
                            None => view! { "Saved to your kubeconfig, shared with kubectl and other tools." }.into_any(),
                        }}
                    </p>
                    // Only the typed-in form says what's missing, once something's typed.
                    {move || {
                        let show = logic::shows_problem(source.get(), manual.touched(), busy.get());
                        show.then(problem).flatten().map(|h| view! { <p class="warn-text">{h}</p> })
                    }}
                    {move || error.get().map(|e| view! { <pre class="warn add-cluster-error">{e}</pre> })}
                    <div class="modal-actions">
                        <button prop:disabled=move || busy.get() on:click=move |_| close()>"Cancel"</button>
                        <button class="primary" prop:disabled=move || !ok() on:click=move |_| add()>
                            {move || logic::add_label(busy.get(), source.get())}
                        </button>
                    </div>
                </Modal>
            }
        })
    }
}

/// A labelled text field.
#[component]
fn Field(
    label: &'static str,
    value: RwSignal<String>,
    #[prop(optional)] placeholder: &'static str,
    #[prop(optional)] password: bool,
    #[prop(optional)] mono: bool,
    #[prop(into)] busy: Signal<bool>,
) -> impl IntoView {
    view! {
        <label class="form-field">
            <span class="form-label">{label}</span>
            <input
                class="confirm-input"
                class:mono=mono
                type=if password { "password" } else { "text" }
                placeholder=placeholder
                autocomplete="off"
                autocapitalize="off"
                prop:value=move || value.get()
                prop:disabled=move || busy.get()
                on:input=move |ev| value.set(event_target_value(&ev))
            />
        </label>
    }
}

/// PEM text, or a file chosen with Choose file… (the kubeconfig points to it).
#[component]
fn PemField(label: &'static str, value: RwSignal<String>, hint: &'static str, #[prop(into)] busy: Signal<bool>) -> impl IntoView {
    let choose = move |_| {
        #[derive(Serialize)]
        struct Args {
            title: String,
        }
        spawn_local(async move {
            if let Ok(Some(path)) = invoke::<_, Option<String>>("choose_cert_file", &Args { title: format!("Choose {label}") }).await {
                value.set(path);
            }
        });
    };
    view! {
        <div class="form-field">
            <span class="form-label">
                {label}
                <button class="btn small" prop:disabled=move || busy.get() on:click=choose>"Choose file…"</button>
            </span>
            <textarea
                class="confirm-input mono pem"
                rows="2"
                placeholder=hint
                spellcheck="false"
                prop:value=move || value.get()
                prop:disabled=move || busy.get()
                on:input=move |ev| value.set(event_target_value(&ev))
            ></textarea>
        </div>
    }
}

#[component]
fn ManualForm(form: Manual, existing: RwSignal<Vec<ManagedContext>>, #[prop(into)] busy: Signal<bool>) -> impl IntoView {
    let same_server = move || {
        let server = form.server.get();
        existing.with(|e| logic::same_server(e, &server))
    };
    view! {
        <div class="form-grid">
            <Field label="Context name" value=form.name placeholder="e.g. lab" busy=busy />
            <Field label="API server" value=form.server placeholder="https://10.0.0.1:6443" mono=true busy=busy />
        </div>
        {move || logic::server_already_text(&same_server()).map(|text| view! { <p class="already-in small">{text}</p> })}
        <PemField label="CA certificate" value=form.ca busy=busy
            hint="Paste the PEM (-----BEGIN CERTIFICATE-----…) or choose a file. Empty: the system's CAs." />
        <label class="check">
            <input type="checkbox" prop:checked=move || form.insecure.get() prop:disabled=move || busy.get()
                on:change=move |ev| form.insecure.set(event_target_checked(&ev)) />
            "Skip TLS verification (insecure)"
        </label>
        <div class="form-field">
            <span class="form-label">"Authentication"</span>
            <select class="rollback-select" prop:disabled=move || busy.get()
                on:change=move |ev| {
                    form.auth.set(logic::auth_choice(&event_target_value(&ev)));
                }>
                {AUTHS.into_iter().map(|(k, label)| view! { <option value=k selected=move || form.auth.get() == k>{label}</option> }).collect_view()}
            </select>
        </div>
        {move || match form.auth.get() {
            "token" => view! { <Field label="Token" value=form.token password=true mono=true busy=busy /> }.into_any(),
            "cert" => view! {
                <PemField label="Client certificate" value=form.cert busy=busy hint="Paste the PEM or choose a file." />
                <PemField label="Client key" value=form.key busy=busy hint="Paste the PEM or choose a file." />
            }.into_any(),
            "exec" => view! {
                <div class="form-grid">
                    <Field label="Command" value=form.command placeholder="e.g. kubelogin" mono=true busy=busy />
                    <div class="form-field">
                        <span class="form-label">"Arguments, one per line"</span>
                        <textarea class="confirm-input mono" rows="2" spellcheck="false" placeholder="get-token"
                            prop:value=move || form.args.get() prop:disabled=move || busy.get()
                            on:input=move |ev| form.args.set(event_target_value(&ev))></textarea>
                    </div>
                </div>
            }.into_any(),
            _ => view! { <p class="muted small">"Requests are sent without credentials."</p> }.into_any(),
        }}
        <div class="form-grid">
            <Field label="Default namespace (optional)" value=form.namespace placeholder="default" busy=busy />
            <Field label="Proxy URL (optional)" value=form.proxy placeholder="socks5://localhost:1080" mono=true busy=busy />
        </div>
    }
}

#[component]
fn CloudForm(provider: CloudProvider, cloud: Cloud, existing: RwSignal<Vec<ManagedContext>>, #[prop(into)] busy: Signal<bool>) -> impl IntoView {
    let on_account = move |ev: leptos::ev::Event| {
        let id = event_target_value(&ev);
        if let Some(Ok(found)) = cloud.accounts.get_untracked() {
            cloud.choose_account(provider, &found, id);
        }
    };
    let on_region = move |ev: leptos::ev::Event| {
        cloud.region.set(event_target_value(&ev).trim().to_owned());
        cloud.load_clusters(provider);
    };
    let accounts = move || match cloud.accounts.get() {
        None => view! { <p class="muted">{logic::asking_text(provider)}</p> }.into_any(),
        Some(Err(e)) => view! {
            <pre class="warn add-cluster-error">{e}</pre>
            <button class="btn small" on:click=move |_| cloud.load_accounts(provider)>"Try again"</button>
        }
        .into_any(),
        Some(Ok(found)) => {
            let options = found
                .accounts
                .iter()
                .map(|a| {
                    let id = a.id.clone();
                    let selected = move || cloud.account.get() == id;
                    view! { <option value=a.id.clone() selected=selected>{a.label.clone()}</option> }
                })
                .collect_view();
            let none = match provider {
                CloudProvider::Eks => Some(view! { <option value="" selected=move || cloud.account.get().is_empty()>"Default credentials"</option> }),
                _ => None,
            };
            let empty = logic::no_accounts(provider, &found);
            view! {
                <div class="form-grid">
                    <div class="form-field">
                        <span class="form-label">{logic::account_label(provider)}</span>
                        <select class="rollback-select" prop:disabled=move || busy.get() || empty on:change=on_account>
                            {none}
                            {options}
                        </select>
                    </div>
                    {(provider == CloudProvider::Eks).then(|| view! {
                        <label class="form-field">
                            <span class="form-label">"Region"</span>
                            <input class="confirm-input mono" list="aws-regions" placeholder="e.g. us-east-1" autocomplete="off"
                                prop:value=move || cloud.region.get() prop:disabled=move || busy.get() on:change=on_region />
                            <datalist id="aws-regions">
                                {AWS_REGIONS.into_iter().map(|r| view! { <option value=r></option> }).collect_view()}
                            </datalist>
                        </label>
                    })}
                </div>
                {empty.then(|| view! { <p class="warn-text">{logic::no_accounts_text(provider)}</p> })}
                <ClusterList provider=provider cloud=cloud existing=existing busy=busy />
            }
            .into_any()
        }
    };
    let placeholder = move || logic::alias_placeholder(provider, &cloud.account.get(), cloud.picked.get().as_ref());
    view! {
        <p class="muted small">
            {logic::sign_in_text(provider)}
            <span class="mono">{logic::credential_command(provider)}</span>
            " for credentials, in kubectl too."
        </p>
        {accounts}
        <label class="form-field">
            <span class="form-label">"Context name (optional)"</span>
            <input class="confirm-input" autocomplete="off" placeholder=placeholder
                prop:value=move || cloud.alias.get() prop:disabled=move || busy.get()
                on:input=move |ev| cloud.alias.set(event_target_value(&ev)) />
        </label>
    }
}

#[component]
fn ClusterList(provider: CloudProvider, cloud: Cloud, existing: RwSignal<Vec<ManagedContext>>, #[prop(into)] busy: Signal<bool>) -> impl IntoView {
    let refresh = move |_| cloud.load_clusters(provider);
    let where_label = logic::where_label(provider);
    move || match cloud.clusters.get() {
        None => view! { <p class="muted">"Listing clusters…"</p> }.into_any(),
        Some(Err(e)) => view! {
            <pre class="warn add-cluster-error">{e}</pre>
            <button class="btn small" on:click=refresh>"Try again"</button>
        }
        .into_any(),
        Some(Ok(list)) if list.is_empty() => {
            let why = logic::no_clusters_text(provider, &cloud.region.get());
            view! { <p class="muted cloud-empty">{why}</p> }.into_any()
        }
        Some(Ok(list)) => {
            let rows = list
                .into_iter()
                .map(|c| {
                    let row = c.clone();
                    let mine = c.clone();
                    let on = Memo::new(move |_| cloud.picked.with(|p| p.as_ref() == Some(&mine)));
                    let place = logic::place(&c);
                    let (server, entry) = (c.server.clone(), c.entry.clone());
                    let added = move || existing.with(|e| !logic::already_in(e, &server, &entry).is_empty());
                    view! {
                        <tr class="cloud-row" class:selected=on on:click=move |_| if !busy.get_untracked() { cloud.picked.set(Some(row.clone())) }>
                            <td><input type="radio" name="cloud-cluster" prop:checked=on prop:disabled=move || busy.get() /></td>
                            <td class="mono">
                                {c.name.clone()}
                                {move || added().then(|| view! { <span class="badge cloud-added" title="Already in your kubeconfig">"added"</span> })}
                            </td>
                            <td>{place}</td>
                            <td>{c.version.clone()}</td>
                            <td>{c.status.clone()}</td>
                        </tr>
                    }
                })
                .collect_view();
            view! {
                <div class="cloud-clusters">
                    <table class="itable">
                        <thead><tr><th></th><th>"Cluster"</th><th>{where_label}</th><th>"Version"</th><th>"Status"</th></tr></thead>
                        <tbody>{rows}</tbody>
                    </table>
                </div>
                // The picked cluster's contexts, if it's in the kubeconfig already.
                {move || {
                    let names = cloud.picked.with(|p| p.as_ref().map(|c| existing.with(|e| logic::already_in(e, &c.server, &c.entry))).unwrap_or_default());
                    logic::cluster_already_text(&names).map(|text| view! {
                        <p class="already-in small">
                            {text}
                            "You only need to add it again if its credentials have changed, or to give it another context name."
                        </p>
                    })
                }}
                <button class="btn small" prop:disabled=move || busy.get() on:click=refresh>"Refresh"</button>
            }
            .into_any()
        }
    }
}
