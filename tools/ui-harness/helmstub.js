// Helm release Details tab, Rollback and Delete. Query: op=rollback|delete, keep=1 (keep history), to=<revision>, nowait=1, ctx=prod, typed=1, locked=1,
// go=1 (sent args land in the page title), fail=1 (helm answers with an error), collapse=…, scroll=<px>
(function () {
  const params = new URLSearchParams(location.search);
  const prod = params.get('ctx') === 'prod';
  const ctxName = prod ? 'acme-prd' : 'acme-uat';
  let unlocked = params.get('locked') !== '1';
  const listeners = {};
  let watchId = 0;
  const kinds = [
    { kind: { group: 'helm.sh', version: 'v3', kind: 'Release', plural: 'releases', namespaced: true }, verbs: ['list', 'watch', 'get'], category: 'Helm' },
    { kind: { group: 'apps', version: 'v1', kind: 'Deployment', plural: 'deployments', namespaced: true }, verbs: ['list', 'watch', 'get'], category: 'Workloads' },
    { kind: { group: 'rbac.authorization.k8s.io', version: 'v1', kind: 'ClusterRole', plural: 'clusterroles', namespaced: false }, verbs: ['list', 'watch', 'get'], category: 'AccessControl' },
  ];
  const ago = (s) => new Date(Date.now() - s * 1000).toISOString();
  const row = (key, value, extra) => ({ key, value, mono: false, time: false, link: null, url: null, ...extra });
  const trow = (cells, extra) => ({ cells, link: null, fill: null, warn: false, ...extra });
  const hist = [[42, 'deployed', '1.8.2', '3.4.1', 3600, 'Upgrade complete'], [41, 'failed', '1.8.1', '3.4.0', 7200, 'Upgrade "acme-api" failed: context deadline exceeded'],
    [40, 'superseded', '1.8.0', '3.3.9', 86400 * 2, 'Upgrade complete'], [39, 'superseded', '1.7.5', '3.3.8', 86400 * 6, 'Rollback to 37'], [38, 'superseded', '1.7.6', '3.3.9', 86400 * 7, 'Upgrade complete']];
  const d = {
    kind: 'Release', name: 'acme-api', namespace: 'shop', status: 'deployed', created: ago(86400 * 60), labels: [], annotations: [], conditions: [], data: null, events: [],
    info: [row('Chart', 'acme-api-1.8.2', { mono: true }), row('App version', '3.4.1'), row('Revision', '42'), row('Last change', 'Upgrade complete'),
      row('First deployed', ago(86400 * 60), { time: true }), row('Last deployed', ago(3600), { time: true }), row('About the chart', 'The acme storefront API'),
      row('Chart home', 'https://charts.acme.io/acme-api', { url: 'https://charts.acme.io/acme-api' }), row('Values', 'not shown: they can hold secrets')],
    sections: [
      { id: 'helm-history', title: 'History', summary: '5 revisions', rows: [], empty: null, table: { columns: ['Revision', 'Status', 'Chart', 'App version', 'Updated', 'Description'], numeric: [0], mono: [], times: [4],
        rows: hist.map(([r, s, c, a, t, desc]) => trow([String(r), s, 'acme-api-' + c, a, ago(t), desc], { warn: s === 'failed' })) } },
      { id: 'helm-resources', title: 'Resources', summary: '4 objects', rows: [], empty: null, table: { columns: ['Kind', 'Name', 'Namespace'], numeric: [], mono: [], rows: [
        trow(['Deployment', 'acme-api', ''], { link: { kind: 'apps/Deployment', namespace: 'shop', name: 'acme-api' } }), trow(['Service', 'acme-api', ''], { link: { kind: 'core/Service', namespace: 'shop', name: 'acme-api' } }),
        trow(['Secret', 'acme-api-env', ''], { link: { kind: 'core/Secret', namespace: 'shop', name: 'acme-api-env' } }), trow(['ClusterRole', 'acme-api-reader', ''], { link: { kind: 'rbac.authorization.k8s.io/ClusterRole', namespace: 'shop', name: 'acme-api-reader' } })] } },
      { id: 'helm-notes', title: 'Notes', summary: '', rows: [row('', 'acme-api is running.\n\n  Get the URL:\n    kubectl -n shop get ingress acme-api\n\nDocs: https://docs.acme.io/api', { mono: true })], table: null, empty: null }],
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === 'watch_resources') { const k = args.kind.kind; const wid = ++watchId; setTimeout(() => (listeners['resource-table'] || []).forEach((cb) => cb({ payload: { watch_id: wid, columns: [{ header: 'Name', kind: 'Name' }, { header: 'Namespace', kind: 'Namespace' }], rows: k === 'Release' ? [{ id: 'shop/acme-api', name: 'acme-api', namespace: 'shop', cells: ['acme-api', 'shop'] }] : [{ id: k === 'ClusterRole' ? 'acme-api-reader' : 'shop/acme-api', name: k === 'ClusterRole' ? 'acme-api-reader' : 'acme-api', namespace: k === 'ClusterRole' ? null : 'shop', cells: ['x', ''] }], error: null } })), 50); return wid; }
      if (cmd === 'list_contexts') return { contexts: [{ name: ctxName, cluster: ctxName, namespace: null, prod_like: prod, unlocked }], current: ctxName };
      if (cmd === 'connect') return { context: ctxName, kinds, namespaces: ['shop'] };
      if (cmd === 'set_unlocked') { unlocked = true; return null; }
      if (cmd === 'object_details') return args.kind.kind === 'Release' ? d : { ...d, kind: args.kind.kind, name: args.name, sections: [], info: [] };
      if (cmd === 'helm_uninstall') { document.title = 'helm_uninstall ' + JSON.stringify(args); if (params.get('fail')) throw 'Error: uninstallation completed with 1 error(s): context deadline exceeded'; return 'release "acme-api" uninstalled'; }
      if (cmd === 'helm_rollback') { document.title = 'helm_rollback ' + JSON.stringify(args); if (params.get('fail')) throw 'Error: release acme-api failed: timed out waiting for the condition'; return 'Rollback was a success! Happy Helming!'; }
      if (['set_unsaved', 'cluster_overview', 'stop_watch', 'workload_history'].includes(cmd)) return null;
      if (cmd === 'get_yaml') return '# Helm release summary\n';
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  const btn = (sel, text) => [...document.querySelectorAll(sel)].find((b) => b.textContent.trim().startsWith(text));
  setTimeout(() => [...document.querySelectorAll('.kind-section-title')].find((b) => /helm/i.test(b.textContent))?.click(), 600);
  setTimeout(() => [...document.querySelectorAll('button.kind')].find((b) => /release/i.test(b.textContent))?.click(), 1000);
  setTimeout(() => document.querySelector('tbody tr')?.click(), 1500);
  if (params.get('op')) setTimeout(() => btn('.detail-actions button', params.get('op') === 'delete' ? 'Delete' : 'Rollback')?.click(), 2300);
  if (params.get('keep')) setTimeout(() => { const c = document.querySelector('.check-row input'); if (c) { c.checked = true; c.dispatchEvent(new Event('change', { bubbles: true })); } }, 2650);
  if (params.get('to')) setTimeout(() => { const s = document.querySelector('.rollback-select'); if (s) { s.value = params.get('to'); s.dispatchEvent(new Event('change', { bubbles: true })); } }, 2600);
  if (params.get('nowait')) setTimeout(() => { const c = document.querySelector('.check-row input'); if (c) { c.checked = false; c.dispatchEvent(new Event('change', { bubbles: true })); } }, 2650);
  if (params.get('typed')) setTimeout(() => { const i = [...document.querySelectorAll('.modal .confirm-input')].pop(); if (i) { i.value = 'acme-api'; i.dispatchEvent(new Event('input', { bubbles: true })); } }, 2800);
  if (params.get('go')) setTimeout(() => document.querySelector('.modal-actions .primary, .modal-actions .danger')?.click(), 3100);
  if (params.get('link')) setTimeout(() => [...document.querySelectorAll('.itable tbody tr.clickable')].find((r) => r.textContent.includes('ClusterRole'))?.click(), 2400);
  if (params.get('link')) setTimeout(() => { document.title = 'selected=' + (document.querySelector('.detail-title')?.textContent || 'none'); }, 3400);
  const collapse = params.get('collapse');
  if (collapse) setTimeout(() => collapse.split(',').forEach((c) => [...document.querySelectorAll('.dsection-name')].find((n) => n.textContent.toLowerCase() === c)?.closest('button').click()), 2200);
  const scroll = params.get('scroll');
  if (scroll) setTimeout(() => { const b = document.querySelector('.detail-body'); if (b) b.scrollTop = Number(scroll); }, 2700);
})();
