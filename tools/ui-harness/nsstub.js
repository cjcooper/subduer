// Namespace Details tab, New namespace and Filter to it. Query: stuck=1 (a Terminating namespace), new=1 (open New
// namespace), name=<text>, go=1, locked=1, ctx=prod, link=1 (click the Contents "Deployments" row), filter=1.
// The title reports what was sent, or the kind and namespace filter after a link.
(function () {
  const params = new URLSearchParams(location.search);
  const prod = params.get('ctx') === 'prod';
  const ctxName = prod ? 'acme-prd' : 'acme-uat';
  let unlocked = params.get('locked') !== '1';
  const stuck = params.get('stuck') === '1';
  const listeners = {};
  let watchId = 0;
  const K = (group, kind, plural, namespaced, verbs, category) => ({ kind: { group, version: 'v1', kind, plural, namespaced }, verbs, category });
  const kinds = [K('', 'Namespace', 'namespaces', false, ['list', 'watch', 'get', 'create', 'delete'], 'Namespaces'),
    K('apps', 'Deployment', 'deployments', true, ['list', 'watch', 'get'], 'Workloads'), K('', 'Pod', 'pods', true, ['list', 'watch', 'get'], 'Workloads')];
  const ago = (s) => new Date(Date.now() - s * 1000).toISOString();
  const row = (key, value, extra) => ({ key, value, mono: false, time: false, link: null, url: null, ...extra });
  const trow = (cells, extra) => ({ cells, link: null, fill: null, warn: false, ...extra });
  const L = (kind, name = '') => ({ kind, namespace: 'shop', name });
  const name = stuck ? 'old-team' : 'shop';
  const d = {
    kind: 'Namespace', name, namespace: null, status: stuck ? 'Terminating' : 'Active', created: ago(86400 * 120),
    labels: [['team', 'checkout'], ['pod-security.kubernetes.io/enforce', 'baseline']], annotations: [], data: null,
    conditions: stuck ? [{ type_: 'NamespaceContentRemaining', status: 'True', reason: 'SomeResourcesRemain', message: 'Some resources are remaining: widgets.acme.io has 1 resource instances', last_transition: ago(3600 * 5) },
      { type_: 'NamespaceFinalizersRemaining', status: 'True', reason: 'SomeFinalizersRemain', message: 'Some content in the namespace has finalizers remaining: widgets.acme.io/protect in 1 resource instances', last_transition: ago(3600 * 5) }] : [],
    info: stuck ? [row('Pod security', 'not set: the cluster default applies'), row('Deleting', 'waiting: Some resources are remaining: widgets.acme.io has 1 resource instances'), row('Finalizers', 'kubernetes', { mono: true })]
      : [row('Pod security', 'enforce baseline · warn restricted')],
    sections: stuck ? [] : [
      { id: 'ns-resources', title: 'Resources', summary: '8 pods running', rows: [], empty: null, table: { columns: ['', 'Used now', 'Requested', 'Limits', 'Quota on requests'], numeric: [1, 2, 3, 4], mono: [], times: [], rows: [
        trow(['CPU', '2.41 cores', '3.80 cores', '7.50 cores', '8.00 cores'], { fill: 0.475 }), trow(['Memory', '9.8 GiB', '15.0 GiB', 'not set on 2 pods', '16.0 GiB'], { fill: 0.94, warn: true })] } },
      { id: 'ns-contents', title: 'Contents', summary: '64 objects', rows: [], empty: null, table: { columns: ['Kind', 'How many'], numeric: [1], mono: [], times: [], rows: [
        trow(['Pods', '9 (8 running, 1 pending)'], { link: L('core/Pod'), warn: true }), trow(['Deployments', '4'], { link: L('apps/Deployment') }), trow(['Services', '5'], { link: L('core/Service') }),
        trow(['ConfigMaps', '7'], { link: L('core/ConfigMap') }), trow(['Secrets', '31'], { link: L('core/Secret') }), trow(['ServiceAccounts', '6'], { link: L('core/ServiceAccount') })] } },
      { id: 'ns-related', title: 'Quotas, limits and releases', summary: '3', rows: [], empty: null, table: { columns: ['Kind', 'Name'], numeric: [], mono: [], times: [], rows: [
        trow(['ResourceQuota', 'compute'], { link: L('core/ResourceQuota', 'compute') }), trow(['Helm release', 'acme-api'], { link: L('helm.sh/Release', 'acme-api') }), trow(['Helm release', 'acme-web'], { link: L('helm.sh/Release', 'acme-web') })] } }],
    events: stuck ? [] : [
      { type_: 'Warning', reason: 'BackOff', message: 'Back-off restarting failed container api', count: 6, first_seen: ago(1800), last_seen: ago(120), source: 'kubelet', object: 'Pod acme-api-7d9f8c6b5-m8n3p' },
      { type_: 'Warning', reason: 'FailedScheduling', message: '0/6 nodes are available: 6 Insufficient memory.', count: 3, first_seen: ago(900), last_seen: ago(300), source: 'default-scheduler', object: 'Pod acme-worker-2' }],
  };
  const nsRows = [{ id: name, name, namespace: null, cells: [name] }, { id: 'ops', name: 'ops', namespace: null, cells: ['ops'] }];
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === 'watch_resources') { const wid = ++watchId; const k = args.kind.kind; setTimeout(() => (listeners['resource-table'] || []).forEach((cb) => cb({ payload: { watch_id: wid, columns: [{ header: 'Name', kind: 'Name' }], rows: k === 'Namespace' ? nsRows : [{ id: 'shop/acme-api', name: 'acme-api', namespace: 'shop', cells: ['acme-api'] }], error: null } })), 50); if (params.get('link') && k !== 'Namespace') setTimeout(() => { document.title = 'list=' + k + ' ns=' + (args.namespace || 'all'); }, 100); return wid; }
      if (cmd === 'list_contexts') return { contexts: [{ name: ctxName, cluster: ctxName, namespace: null, prod_like: prod, unlocked }], current: ctxName };
      if (cmd === 'connect') return { context: ctxName, kinds, namespaces: ['shop', 'ops'] };
      if (cmd === 'set_unlocked') { unlocked = true; return null; }
      if (cmd === 'object_details') return d;
      if (cmd === 'create_namespace') { document.title = 'create_namespace ' + JSON.stringify(args); return null; }
      if (['set_unsaved', 'cluster_overview', 'stop_watch', 'set_app_theme'].includes(cmd)) return null;
      if (cmd === 'get_yaml') return 'kind: Namespace\n';
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  const btn = (sel, text) => [...document.querySelectorAll(sel)].find((b) => b.textContent.trim().startsWith(text));
  setTimeout(() => [...document.querySelectorAll('.kind-section-title')].find((b) => /namespaces/i.test(b.textContent))?.click(), 600);
  setTimeout(() => [...document.querySelectorAll('button.kind')].find((b) => b.textContent.trim() === 'Namespace')?.click(), 1000);
  if (!params.get('new')) setTimeout(() => document.querySelector('tbody tr')?.click(), 1500);
  if (params.get('new')) setTimeout(() => btn('.resource-header button', 'New namespace')?.click(), 1500);
  if (params.get('locked') && params.get('new')) setTimeout(() => btn('.modal-actions button', 'Allow')?.click(), 1800);
  if (params.get('name')) setTimeout(() => { const i = document.querySelector('.modal .confirm-input'); if (i) { i.value = params.get('name'); i.dispatchEvent(new Event('input', { bubbles: true })); } }, 2200);
  if (params.get('go')) setTimeout(() => document.querySelector('.modal-actions .primary')?.click(), 2500);
  if (params.get('link')) setTimeout(() => [...document.querySelectorAll('.itable tbody tr.clickable')].find((r) => r.textContent.startsWith('Deployments'))?.click(), 2300);
  if (params.get('filter')) setTimeout(() => btn('.detail-actions button', 'Filter')?.click(), 2300);
  if (params.get('filter')) setTimeout(() => { document.title = 'ns-menu=' + document.querySelectorAll('.topbar select')[1]?.value; }, 2800);
})();
