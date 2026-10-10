// Deployment/StatefulSet Restart and Scale. Query: op=restart|scale, sts=1, ctx=prod, locked=1, to=<n>, go=1
(function () {
  const params = new URLSearchParams(location.search);
  const prod = params.get('ctx') === 'prod';
  const ctxName = prod ? 'acme-prd' : 'acme-uat';
  const sts = params.get('sts') === '1';
  const kindName = sts ? 'StatefulSet' : 'Deployment';
  let unlocked = params.get('locked') !== '1';
  const listeners = {};
  const kinds = [{ kind: { group: 'apps', version: 'v1', kind: kindName, plural: kindName.toLowerCase() + 's', namespaced: true }, verbs: ['list', 'watch', 'get', 'patch', 'delete'], category: 'Workloads' }];
  const name = sts ? 'acme-db' : 'acme-api';
  const table = { watch_id: 1, columns: [{ header: 'Name', kind: 'Name' }, { header: 'Namespace', kind: 'Namespace' }, { header: 'Ready', kind: 'Text' }], rows: [{ id: 'shop/' + name, name, namespace: 'shop', cells: [name, 'shop', '3/3'] }], error: null };
  const replies = {
    connect: { context: ctxName, kinds, namespaces: ['shop'] }, cluster_overview: null, stop_watch: null,
    get_yaml: 'apiVersion: apps/v1\nkind: ' + kindName + '\nmetadata:\n  name: ' + name + '\n  namespace: shop\nspec:\n  replicas: 3\n',
    get_replicas: { desired: 3, current: 3 },
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === 'watch_resources') { setTimeout(() => (listeners['resource-table'] || []).forEach((cb) => cb({ payload: table })), 50); return 1; }
      if (cmd === 'list_contexts') return { contexts: [{ name: ctxName, cluster: ctxName, namespace: null, prod_like: prod, unlocked }], current: ctxName };
      if (cmd === 'set_unlocked') { unlocked = true; return null; }
      if (cmd === 'restart_workload' || cmd === 'scale_workload') { document.title = cmd + ' ' + JSON.stringify(args); return null; }
      if (cmd in replies) return replies[cmd];
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  const btn = (sel, text) => [...document.querySelectorAll(sel)].find((b) => b.textContent.trim().startsWith(text));
  setTimeout(() => document.querySelector('.kind-section-title')?.click(), 600);
  setTimeout(() => btn('button.kind', kindName)?.click(), 1000);
  setTimeout(() => document.querySelector('tbody tr')?.click(), 1500);
  const op = params.get('op');
  if (op) setTimeout(() => btn('.detail-actions button', op === 'scale' ? 'Scale' : 'Restart')?.click(), 2200);
  const to = params.get('to');
  if (to) setTimeout(() => {
    const i = document.querySelector('.replicas-input'); if (!i) return;
    i.value = to; i.dispatchEvent(new Event('input', { bubbles: true }));
  }, 2800);
  if (prod && params.get('typed')) setTimeout(() => {
    const i = [...document.querySelectorAll('.modal .confirm-input')].pop();
    i.value = name; i.dispatchEvent(new Event('input', { bubbles: true }));
  }, 3000);
  if (params.get('go')) setTimeout(() => document.querySelector('.modal-actions .primary')?.click(), 3300);
})();
