// External Secrets Details tabs, Force sync and Controller logs. Query: kind=es|ces|store|gen (default es), failed=1 (the sync failed),
// once=1 (refreshPolicy CreatedOnce), op=force, ctx=prod, typed=1, locked=1, go=1 (sent args land in the page title),
// ctl=1 (clicks Controller logs; the open_logs target lands in the title), link=1 (follows the Store link), collapse=…, scroll=<px>
(function () {
  const params = new URLSearchParams(location.search);
  const which = params.get('kind') || 'es';
  const prod = params.get('ctx') === 'prod';
  const ctxName = prod ? 'acme-prd' : 'acme-uat';
  const failed = params.get('failed') === '1';
  let unlocked = params.get('locked') !== '1';
  const listeners = {};
  const K = (group, version, kind, plural, namespaced) => ({ kind: { group, version, kind, plural, namespaced }, verbs: ['list', 'watch', 'get', 'patch', 'delete'], category: 'CustomResources' });
  const kinds = [K('external-secrets.io', 'v1', 'ExternalSecret', 'externalsecrets', true), K('external-secrets.io', 'v1', 'ClusterExternalSecret', 'clusterexternalsecrets', false),
    K('external-secrets.io', 'v1', 'SecretStore', 'secretstores', true), K('generators.external-secrets.io', 'v1alpha1', 'Password', 'passwords', true),
    { kind: { group: 'apps', version: 'v1', kind: 'Deployment', plural: 'deployments', namespaced: true }, verbs: ['list', 'watch', 'get'], category: 'Workloads' }];
  const kindName = { es: 'ExternalSecret', ces: 'ClusterExternalSecret', store: 'SecretStore', gen: 'Password' }[which];
  const row = (key, value, extra) => ({ key, value, mono: false, time: false, link: null, url: null, ...extra });
  const trow = (cells, extra) => ({ cells, link: null, fill: null, warn: false, ...extra });
  const L = (kind, name, namespace = null) => ({ kind, namespace, name });
  const ago = (s) => new Date(Date.now() - s * 1000).toISOString().replace(/\.\d+Z$/, 'Z');
  const sec = (id, title, summary, table, rows, empty = null) => ({ id, title, summary, rows: rows || [], table, empty });
  const cond = (status, reason, message) => ({ type_: 'Ready', status, reason, message, last_transition: ago(3600) });
  const logs = (name) => ({ namespace: 'external-secrets', source: { Workload: { kind: { group: 'apps', version: 'v1', kind: 'Deployment', plural: 'deployments', namespaced: true }, name: 'external-secrets' } }, filter: name });
  const controller = row('Controller', 'external-secrets/external-secrets', { link: L('apps/Deployment', 'external-secrets', 'external-secrets') });
  const base = (kind, name, ns) => ({ kind, name, namespace: ns, status: null, info: [], created: '2026-09-01T10:00:00Z', labels: [['app.kubernetes.io/name', 'acme-api']], annotations: [], conditions: [], data: null, sections: [], events: [], logs: logs(name) });
  const esList = (rows) => ({ columns: ['Namespace', 'ExternalSecret', 'Status', 'Last refresh'], numeric: [], mono: [1], times: [3], rows });
  let d;
  if (which === 'es') {
    d = base('ExternalSecret', 'acme-api', 'shop');
    d.status = failed ? 'Sync failed' : 'Synced';
    d.info = [row('Store', 'SecretStore/default-store', { link: L('external-secrets.io/SecretStore', 'default-store', 'shop') }),
      row('Target secret', 'acme-api', { mono: true, link: L('core/Secret', 'acme-api', 'shop') }),
      row('Creation', 'Owner: creates the Secret and owns it (deleted with this)'), row('Deletion', 'Retain: the Secret is kept when the remote secret is gone'),
      ...(params.get('once') ? [row('Refresh policy', 'CreatedOnce: fetched once, never refreshed')] : [row('Refresh interval', '1h')]),
      row('Last refresh', ago(1800), { time: true }), row('Next refresh', new Date(Date.now() + 1800e3).toISOString().replace(/\.\d+Z$/, 'Z'), { time: true }), controller];
    d.conditions = [failed ? cond('False', 'SecretSyncedError', 'could not get secret data from provider: AccessDeniedException') : cond('True', 'SecretSynced', 'secret synced')];
    d.sections = [sec('es-data', 'Data', '2', { columns: ['Secret key', 'Remote key', 'Property'], numeric: [], mono: [0, 1, 2], times: [], rows: [trow(['DB_USER', 'acme/uat/db', 'user']), trow(['DB_PASSWORD', 'acme/uat/db', 'password'])] }),
      sec('es-data-from', 'Data from', '1', { columns: ['How', 'What'], numeric: [], mono: [1], times: [], rows: [trow(['Extract', 'acme/uat/api-key'])] })];
    d.events = [failed
      ? { type_: 'Warning', reason: 'UpdateFailed', message: 'could not get secret data from provider', count: 3, first_seen: ago(900), last_seen: ago(60), source: 'external-secrets', object: null }
      : { type_: 'Normal', reason: 'Updated', message: 'secret updated', count: 1, first_seen: ago(1800), last_seen: ago(1800), source: 'external-secrets', object: null }];
  } else if (which === 'ces') {
    d = base('ClusterExternalSecret', 'shared-registry', null);
    d.status = 'Partly failed';
    d.info = [row('ExternalSecret name', 'shared-registry', { mono: true }), row('Namespaces', 'labelled team=shop', { mono: true }), row('Store', 'ClusterSecretStore/aws', { link: L('external-secrets.io/ClusterSecretStore', 'aws') }),
      row('Target secret', 'registry', { mono: true }), row('Creation', 'Owner: creates the Secret and owns it (deleted with this)'), row('Refresh interval', '1h (each ExternalSecret)'), row('Namespaces checked', 'every 1m'), controller];
    d.sections = [sec('externalsecret-children', 'ExternalSecrets', '3', esList([
      trow(['ops', 'shared-registry', 'Synced', ago(600)], { link: L('external-secrets.io/ExternalSecret', 'shared-registry', 'ops') }),
      trow(['shop', 'shared-registry', 'Synced', ago(700)], { link: L('external-secrets.io/ExternalSecret', 'shared-registry', 'shop') }),
      trow(['sandbox', '', 'Failed: namespace is being terminated', ''], { warn: true })]))];
    d.events = [{ type_: 'Normal', reason: 'Created', message: 'created ExternalSecret in ops', count: 1, first_seen: ago(86400), last_seen: ago(86400), source: 'external-secrets', object: null }];
  } else if (which === 'store') {
    d = base('SecretStore', 'default-store', 'shop');
    d.status = 'Valid';
    d.info = [row('Provider', 'AWS Secrets Manager (us-east-1)'), row('Capabilities', 'ReadWrite: read by ExternalSecrets, written by PushSecrets'), controller];
    d.conditions = [cond('True', 'Valid', 'store validated')];
    d.sections = [sec('store-provider', 'Provider: aws', '3 settings', null, [row('auth.jwt.serviceAccountRef.name', 'default-store', { mono: true, link: L('core/ServiceAccount', 'default-store', 'shop') }), row('region', 'us-east-1', { mono: true }), row('service', 'SecretsManager', { mono: true })]),
      sec('store-users', 'Used by', '2', esList([trow(['shop', 'acme-api', 'Synced', ago(1800)], { link: L('external-secrets.io/ExternalSecret', 'acme-api', 'shop') }),
        trow(['shop', 'acme-worker', 'Sync failed', ago(300)], { link: L('external-secrets.io/ExternalSecret', 'acme-worker', 'shop'), warn: true })]))];
  } else {
    d = base('Password', 'db-password', 'shop');
    d.info = [row('allowRepeat', 'false', { mono: true }), row('digits', '5', { mono: true }), row('length', '32', { mono: true }), row('symbols', '4', { mono: true }), controller];
    d.sections = [sec('generator-users', 'Used by', 'none', esList([]), [], 'No ExternalSecrets use it.')];
  }
  const id = d.namespace ? d.namespace + '/' + d.name : d.name;
  const cluster = () => ({ name: ctxName, cluster: ctxName, namespace: null, prod_like: prod, unlocked });
  const tableFor = (kind) => {
    if (kind && kind.kind === 'ExternalSecret' && which !== 'es') return { id: 'shop/acme-api', name: 'acme-api', namespace: 'shop', cells: ['acme-api', 'shop'] };
    if (kind && kind.kind === 'SecretStore' && which !== 'store') return { id: 'shop/default-store', name: 'default-store', namespace: 'shop', cells: ['default-store', 'shop'] };
    return { id, name: d.name, namespace: d.namespace, cells: [d.name, d.namespace || ''] };
  };
  let watch = 0;
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === 'watch_resources') {
        const n = ++watch;
        const r = tableFor(args.kind);
        setTimeout(() => (listeners['resource-table'] || []).forEach((cb) => cb({ payload: { watch_id: n, columns: [{ header: 'Name', kind: 'Name' }, { header: 'Namespace', kind: 'Namespace' }], rows: [r], error: null } })), 50);
        return n;
      }
      if (cmd === 'list_contexts') return { contexts: [cluster()], current: ctxName };
      if (cmd === 'connect') return { context: ctxName, kinds, namespaces: ['shop', 'ops'] };
      if (cmd === 'set_unlocked') { unlocked = args.unlocked; return null; }
      if (cmd === 'object_details') { if (args.kind.kind !== d.kind) document.title = 'details ' + args.kind.kind + ' ' + args.name; return args.kind.kind === d.kind ? d : { ...base(args.kind.kind, args.name, args.namespace), status: 'Valid' }; }
      if (cmd === 'force_sync') { document.title = cmd + ' ' + JSON.stringify(args); return which === 'ces' ? 2 : 1; }
      if (cmd === 'open_logs') { document.title = cmd + ' ' + JSON.stringify(args); return null; }
      if (['set_unsaved', 'cluster_overview', 'stop_watch', 'check_connection'].includes(cmd)) return null;
      if (cmd === 'get_yaml') return 'kind: ' + kindName + '\n';
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  const btn = (sel, text) => [...document.querySelectorAll(sel)].find((b) => b.textContent.trim().startsWith(text));
  const type = (el, v) => { if (!el) return; el.value = v; el.dispatchEvent(new Event('input', { bubbles: true })); };
  setTimeout(() => [...document.querySelectorAll('.kind-section-title')].find((b) => b.textContent.includes('Custom'))?.click(), 600);
  setTimeout(() => [...document.querySelectorAll('button.kind')].find((b) => b.textContent.trim() === kindName)?.click(), 1000);
  setTimeout(() => document.querySelector('tbody tr')?.click(), 1500);
  if (params.get('op') === 'force') setTimeout(() => btn('.detail-actions button', 'Force sync')?.click(), 2300);
  if (params.get('ctl')) setTimeout(() => btn('.detail-actions button', 'Controller logs')?.click(), 2300);
  if (params.get('link')) setTimeout(() => [...document.querySelectorAll('.detail-body a, .detail-body button.link')].find((a) => a.textContent.includes('default-store'))?.click(), 2300);
  // A locked context asks to allow changes first.
  if (params.get('locked')) setTimeout(() => { const i = document.querySelector('.modal .confirm-input'); if (i) type(i, ctxName); setTimeout(() => btn('.modal-actions button', 'Allow')?.click(), 100); }, 2600);
  if (prod && params.get('typed')) setTimeout(() => type([...document.querySelectorAll('.modal .confirm-input')].pop(), d.name), 3000);
  if (params.get('go')) setTimeout(() => document.querySelector('.modal-actions .primary')?.click(), 3300);
  const collapse = params.get('collapse');
  if (collapse) setTimeout(() => collapse.split(',').forEach((c) => [...document.querySelectorAll('.dsection-name')].find((n) => n.textContent.toLowerCase() === c)?.closest('button').click()), 2300);
  const scroll = params.get('scroll');
  if (scroll) setTimeout(() => { const b = document.querySelector('.detail-body'); if (b) b.scrollTop = Number(scroll); }, 2600);
})();
