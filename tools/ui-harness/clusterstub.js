// File › Manage Clusters…, and its Add Cluster…. Without manage=1, Add Cluster… is clicked straight away.
// Manage Clusters: manage=1 (four contexts: acme-uat, gke_acme_us-central1_web, shop-prod, aks-apps), files=2 (aks-apps in a
// second file), here=<name> (this window's context), up=<name>, down=<name>, remove=<name>, yes=1 (confirms), inuse=1 (the
// backend refuses), drag=<name>&onto=<name>[&after=1], badorder=1 (the reorder fails), closed=1 (never opens).
// known=1: the kubeconfig already has the GKE cluster web (as web, by its entry, and web-admin, by its server) and the AKS one.
// Production and other windows: mark=<name> (clicks its production toggle), elsewhere=<name> (another window is on it),
// later=<name> (another window moves to it after a second).
// Outside changes: gone=1 (acme-uat deleted with kubectl; the backend moves the window to shop-prod), empty=1 (every context
// deleted), startempty=1 (none to begin with), readd=1 (then kubectl adds `lab`).
// The Context menu: menu=1 (opens it), ctx=<name> (picks it), keys=<key,key…> (pressed on it), outside=1 (a click elsewhere). Add Cluster: src=k8s|gke|eks|aks (picks that card; it opens on AWS), name=<text>,
// server=<url>, ca=<text>, insecure=1, auth=token|cert|exec|none, token=<text>, cmd=<text>, args=<lines, \n as |>,
// ns=<text>, go=1 (clicks Add), current=1, noconnect=1, fail=1 (the add is refused), nocli=1 (the CLI isn't installed),
// noclusters=1, pick=<cluster name>, alias=<text>, region=<text>, profile=<name>, noregion=1 (EKS). The title reports the add command sent (with
// its args), the calls made, whether the dialog is open and the context connected afterwards.
(function () {
  const params = new URLSearchParams(location.search);
  const listeners = {};
  const calls = [];
  let sent = '';
  const manage = params.get('manage');
  const files = ['/Users/me/.kube/config'].concat(params.get('files') ? ['/Users/me/.kube/aks'] : []);
  const managed = (name, provider, server, file) => ({ name, cluster: name, user: name, server, provider, file: files[file] });
  let list = [managed('acme-uat', null, 'https://10.0.0.1:6443', 0)].concat(manage ? [
    managed('gke_acme_us-central1_web', 'gke', 'https://34.1.2.3', 0),
    Object.assign(managed('shop-prod', 'eks', 'https://ABC.gr7.eu-west-1.eks.amazonaws.com', 0), { cluster: 'shared', user: 'shared' }),
    Object.assign(managed('aks-apps', 'aks', 'https://aks-1.hcp.westeurope.azmk8s.io:443', params.get('files') ? 1 : 0), { cluster: 'shared' }),
  ] : []).concat(params.get('known') ? [
    Object.assign(managed('web', 'gke', '', 0), { cluster: 'gke_acme-ops_us-central1_web' }),
    Object.assign(managed('web-admin', 'gke', 'https://34.1.2.3', 0), { cluster: 'other' }),
    managed('aks-apps-ctx', 'aks', 'https://aks-apps-dns.hcp.westeurope.azmk8s.io:443', 0),
  ] : []);
  const marks = {};
  const info = (c) => ({ name: c.name, cluster: c.cluster, namespace: null, prod_like: c.name in marks ? marks[c.name] : c.name.includes('prod'), unlocked: false, provider: c.provider });
  if (params.get('startempty')) list = [];
  let contexts = list.map(info);
  let connected = params.get('here') || 'acme-uat';
  let removed = '';
  const kinds = [{ kind: { group: '', version: 'v1', kind: 'Pod', plural: 'pods', namespaced: true }, verbs: ['list', 'watch', 'get'], category: 'Workloads' }];
  const missing = { gke: "The gcloud command isn't installed or isn't on PATH.", eks: "The aws command isn't installed or isn't on PATH.", aks: "The az command isn't installed or isn't on PATH." };
  const accounts = {
    gke: { accounts: [{ id: 'acme-dev', label: 'Acme Dev (acme-dev)', region: null }, { id: 'acme-ops', label: 'acme-ops', region: null }], default: 'acme-ops', region: null },
    eks: { accounts: [{ id: 'work', label: 'work', region: 'eu-west-1' }], default: null, region: params.get('noregion') ? null : 'us-east-2' },
    aks: { accounts: [{ id: '1111', label: 'Dev', region: null }, { id: '2222', label: 'Shared', region: null }], default: '2222', region: null },
  };
  const clusters = {
    gke: [{ name: 'web', location: 'us-central1', group: '', version: '1.33.1-gke.100', status: 'RUNNING', server: '34.1.2.3', entry: 'gke_acme-ops_us-central1_web' },
      { name: 'batch', location: 'us-central1-a', group: '', version: '1.32.9-gke.200', status: 'RUNNING', server: '34.9.9.9', entry: 'gke_acme-ops_us-central1-a_batch' }],
    eks: [{ name: 'api', location: 'eu-west-1', group: '', version: '1.33', status: 'ACTIVE', server: 'https://ABC.gr7.eu-west-1.eks.amazonaws.com', entry: 'arn:aws:eks:eu-west-1:1:cluster/api' }],
    aks: [{ name: 'aks-apps', location: 'westeurope', group: 'rg-apps', version: '1.32.4', status: 'Running', server: 'aks-apps-dns.hcp.westeurope.azmk8s.io', entry: '' }],
  };
  const emit = (name, payload) => (listeners[name] || []).forEach((cb) => cb({ payload }));
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (!['set_app_theme', 'set_unsaved', 'list_forwards', 'stop_watch', 'watch_resources', 'set_transparency', 'system_accent'].includes(cmd)) calls.push(cmd);
      if (cmd === 'list_contexts') return { contexts, current: params.get('here') || 'acme-uat' };
      if (cmd === 'managed_contexts') return { contexts: list, files, current: 'acme-uat' };
      if (cmd === 'contexts_in_use_elsewhere') return params.get('elsewhere') ? [params.get('elsewhere')] : [];
      if (cmd === 'set_production') {
        sent = cmd + ' ' + JSON.stringify(args);
        marks[args.context] = args.production;
        contexts = list.map(info);
        setTimeout(() => emit('contexts-changed', null), 0);
        return null;
      }
      if (cmd === 'remove_context') {
        if (params.get('inuse')) throw 'A window is connected to "' + args.context + '". Switch it to another context first.';
        const c = list.find((x) => x.name === args.context);
        list = list.filter((x) => x !== c);
        contexts = list.map(info);
        removed = args.context;
        setTimeout(() => emit('contexts-changed', null), 0);
        const alone = (f) => !list.some((x) => x[f] === c[f]);
        return { context: c.name, cluster: alone('cluster') ? c.cluster : null, user: alone('user') ? c.user : null };
      }
      if (cmd === 'reorder_contexts') {
        sent = cmd + ' ' + JSON.stringify(args);
        if (params.get('badorder')) throw 'the kubeconfig is being changed by another program';
        list = args.order.map((n) => list.find((x) => x.name === n));
        contexts = list.map(info);
        setTimeout(() => emit('contexts-changed', null), 0);
        return null;
      }
      if (cmd === 'connect') { connected = args.context; return { context: args.context, kinds, namespaces: ['shop'] }; }
      if (cmd === 'watch_resources') return 1;
      if (cmd === 'list_forwards') return [];
      if (['set_unsaved', 'stop_watch', 'set_app_theme'].includes(cmd)) return null;
      if (cmd === 'set_transparency') return false;
      if (cmd === 'cluster_overview') throw 'stub: no overview';
      if (cmd === 'kubeconfig_target') return '/Users/me/.kube/config';
      if (cmd === 'choose_cert_file') return '/Users/me/certs/ca.crt';
      if (cmd === 'cloud_accounts') {
        if (params.get('nocli')) throw missing[args.provider];
        return accounts[args.provider];
      }
      if (cmd === 'cloud_clusters') { calls.push('list:' + args.account + (args.region ? '@' + args.region : '')); return params.get('noclusters') ? [] : clusters[args.provider]; }
      if (cmd === 'add_cluster' || cmd === 'add_cloud_cluster') {
        sent = cmd + ' ' + JSON.stringify(args);
        if (params.get('fail')) throw 'A context named "acme-uat" already exists.';
        const context = cmd === 'add_cluster' ? args.cluster.name : (args.alias || 'gke_' + args.account + '_' + args.cluster.location + '_' + args.cluster.name);
        list = list.concat([managed(context, null, '', 0)]);
        contexts = list.map(info);
        setTimeout(() => emit('contexts-changed', null), 0);
        return { context, output: '' };
      }
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  const type = (sel, value) => {
    const el = typeof sel === 'string' ? document.querySelector(sel) : sel;
    if (!el) return;
    el.value = value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  };
  const field = (label) => [...document.querySelectorAll('.form-field')].find((f) => f.querySelector('.form-label')?.textContent.trim().startsWith(label))?.querySelector('input, textarea, select');
  const at = (ms, f) => setTimeout(f, ms);
  if (!params.get('closed')) at(600, () => emit('open-manage-clusters', null));
  if (params.get('gone')) at(1000, () => {
    list = list.filter((c) => c.name !== 'acme-uat');
    contexts = list.map(info);
    emit('contexts-changed', null);
    emit('context-removed', { removed: 'acme-uat', switch_to: 'shop-prod' });
  });
  if (params.get('empty')) at(1000, () => { list = []; contexts = []; emit('contexts-changed', null); emit('no-contexts', null); });
  if (params.get('readd')) at(1800, () => { list = [managed('lab', null, 'https://10.0.0.9:6443', 0)]; contexts = list.map(info); emit('contexts-changed', null); });
  if (!manage) at(800, () => [...document.querySelectorAll('.manage-clusters .modal-actions button')].find((b) => b.textContent === 'Add Cluster…')?.click());
  const row = (name) => [...document.querySelectorAll('.cluster-row')].find((r) => r.querySelector('.cluster-name .mono')?.textContent === name);
  const button = (r, label) => [...(r?.querySelectorAll('button') || [])].find((b) => b.textContent === label || b.title === label);
  at(1600, () => {
    const button = document.querySelector('.context-button');
    if (params.get('menu') || params.get('ctx')) button?.click();
    if (params.get('ctx')) [...document.querySelectorAll('.context-option')].find((o) => o.querySelector('.context-name')?.textContent === params.get('ctx'))?.click();
    for (const key of (params.get('keys') || '').split(',').filter(Boolean)) button?.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
    if (params.get('outside')) document.querySelector('.sidebar, main, body').dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
  });
  if (manage) {
    at(1000, () => {
      if (params.get('up')) button(row(params.get('up')), 'Move up')?.click();
      if (params.get('down')) button(row(params.get('down')), 'Move down')?.click();
      if (params.get('remove')) button(row(params.get('remove')), 'Remove')?.click();
      if (params.get('drag')) {
        const from = row(params.get('drag')), onto = row(params.get('onto'));
        const dt = new DataTransfer();
        const rect = onto.getBoundingClientRect();
        const y = params.get('after') ? rect.bottom - 2 : rect.top + 2;
        from.querySelector('.cluster-grip').dispatchEvent(new DragEvent('dragstart', { bubbles: true, dataTransfer: dt }));
        onto.dispatchEvent(new DragEvent('dragover', { bubbles: true, cancelable: true, dataTransfer: dt, clientY: y }));
        onto.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer: dt, clientY: y }));
      }
    });
    if (params.get('mark')) at(1000, () => row(params.get('mark'))?.querySelector('.prod-toggle')?.click());
    if (params.get('later')) at(1100, () => emit('contexts-in-use-elsewhere', [params.get('later')]));
    if (params.get('yes')) at(1300, () => document.querySelector('.cluster-confirm .danger')?.click());
  }
  const src = params.get('src');
  const card = { k8s: 'Kubernetes', gke: 'Google Cloud', eks: 'AWS', aks: 'Azure' }[src];
  if (card) at(1000, () => [...document.querySelectorAll('.source-card')].find((b) => b.querySelector('.source-name').textContent === card)?.click());
  at(1200, () => {
    if (params.get('auth')) type(field('Authentication'), params.get('auth'));
  });
  at(1300, () => {
    if (params.get('name')) type(field('Context name'), params.get('name'));
    if (params.get('server')) type(field('API server'), params.get('server'));
    if (params.get('ca')) type(field('CA certificate'), params.get('ca'));
    if (params.get('insecure')) document.querySelector('.add-cluster .check input')?.click();
    if (params.get('token')) type(field('Token'), params.get('token'));
    if (params.get('cmd')) type(field('Command'), params.get('cmd'));
    if (params.get('args')) type(field('Arguments'), params.get('args').split('|').join('\n'));
    if (params.get('ns')) type(field('Default namespace'), params.get('ns'));
    if (params.get('profile')) type(field('Profile'), params.get('profile'));
    if (params.get('region')) type(field('Region'), params.get('region'));
    if (params.get('alias')) type(field('Context name'), params.get('alias'));
    const checks = [...document.querySelectorAll('.add-cluster-checks input')];
    if (params.get('current')) checks[0]?.click();
    if (params.get('noconnect')) checks[1]?.click();
  });
  if (params.get('pick')) at(1500, () => [...document.querySelectorAll('.cloud-row')].find((r) => r.children[1].textContent.replace(/added$/, '').trim() === params.get('pick'))?.click());
  if (params.get('go')) at(1700, () => document.querySelector('.add-cluster .modal-actions .primary')?.click());
  at(2300, () => {
    const add = document.querySelector('.add-cluster .modal-actions .primary');
    const rows = [...document.querySelectorAll('.cluster-row .cluster-name .mono')].map((n) => n.textContent);
    const menu = [...document.querySelectorAll('.context-option .context-name')].map((o) => o.textContent);
    const logo = (sel) => document.querySelector(sel)?.getAttribute('class').replace('provider-logo ', '') || 'none';
    document.title = `${sent || 'nothing sent'} calls=${calls.join(',')} dialog=${document.querySelector('.add-cluster') ? 'open' : 'closed'}` +
      ` add=${add ? (add.disabled ? 'disabled' : 'enabled') : 'none'} connected=${connected} contexts=${contexts.length}` +
      ` manage=${document.querySelector('.manage-clusters') ? 'open' : 'closed'} rows=${rows.join('|')} menu=${menu.join('|')} removed=${removed} menu-logo=${logo('.context-button .provider-logo')} expanded=${document.querySelector('.context-button')?.getAttribute('aria-expanded')}` +
      ` shown=${document.querySelector('.context-button .context-name')?.textContent} prod-icons=${[...document.querySelectorAll('.context-option')].filter((o) => o.querySelector('.prod-icon')).map((o) => o.querySelector('.context-name').textContent).join('|')}` +
      ` active=${document.querySelector('.context-option.active .context-name')?.textContent}` +
      ` prod=${[...document.querySelectorAll('.cluster-row')].filter((r) => r.querySelector('.prod-toggle.on')).map((r) => r.querySelector('.cluster-name .mono').textContent).join('|')}` +
      ` locked-rows=${[...document.querySelectorAll('.cluster-row')].filter((r) => button(r, 'Remove')?.disabled).map((r) => r.querySelector('.cluster-name .mono').textContent).join('|')}` +
      ` added=${[...document.querySelectorAll('.cloud-row')].filter((r) => r.querySelector('.cloud-added')).map((r) => r.children[1].textContent.replace(/added$/, '').trim()).join('|')}` +
      ` notice=${document.querySelector('.notice-bar')?.textContent.replace('×', '') || 'none'}` +
      ` confirm=${document.querySelector('.cluster-confirm .danger') ? (document.querySelector('.cluster-confirm .danger').disabled ? 'disabled' : 'enabled') : 'none'} page-logo=${logo('.resource-header > .provider-logo')}`;
  });
})();
