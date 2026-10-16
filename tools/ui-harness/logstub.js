(function () {
  const params = new URLSearchParams(location.search);
  const merged = params.get('merged') === '1' || params.get('target') === '1';
  const label = params.get('logs');
  const listeners = {};
  const now = Date.now();
  const iso = (ms) => new Date(ms).toISOString().replace('Z', '123456Z');
  const msgs = ['GET /healthz 200 1.2ms', 'Reconciling ScaledObject shop/web', '{"level":"info","msg":"cache synced","controller":"scaledobject"}',
    'WARN slow query took 1534ms: SELECT * FROM orders WHERE customer_id = $1 AND created_at > now() - interval \'30 days\' ORDER BY created_at DESC LIMIT 100',
    'POST /api/orders 201 23.4ms', 'ERROR connection reset by peer (upstream payments:8080)'];
  const lines = [];
  for (let i = 0; i < 80; i++) {
    lines.push({ source: merged ? ['web-7d9f-abcde/app', 'web-7d9f-fghij/app', 'web-7d9f-abcde/envoy'][i % 3] : null, time: iso(now - (80 - i) * 1500), text: msgs[i % msgs.length] });
  }
  const session = merged
    ? { context: 'acme-uat', title: 'Deployment shop/web · acme-uat · Logs', event: label, target: { namespace: 'shop', source: { Workload: { kind: { group: 'apps', version: 'v1', kind: 'Deployment', plural: 'deployments', namespaced: true }, name: 'web' } } } }
    : { context: 'acme-uat', title: 'app · shop/web-7d9f-abcde · acme-uat · Logs', event: label, target: { namespace: 'shop', source: { Container: { pod: 'web-7d9f-abcde', container: 'app' } } } };
  // target=1: the target names a starting filter, like an operator's controller logs for one object.
  if (params.get('target')) session.target.filter = 'Reconciling';
  window.__TAURI__ = {
    core: { invoke: async (cmd) => {
      if (cmd === 'logs_start') {
        setTimeout(() => (listeners[label] || []).forEach((cb) => cb({ payload: { lines, notices: merged ? ['Following 3 containers in 2 pods (pods started later aren\'t added).'] : [] } })), 100);
        return session;
      }
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (name, cb) => { (listeners[name] = listeners[name] || []).push(cb); return () => {}; } },
  };
  if (params.get('filter')) setTimeout(() => { const i = document.querySelector('.logs-tools .search'); i.value = params.get('filter'); i.dispatchEvent(new Event('input', { bubbles: true })); }, 900);
})();
