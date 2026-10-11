(function () {
  const params = new URLSearchParams(location.search);
  const listeners = {};
  const K = (group, kind, category) => ({ kind: { group, version: 'v1', kind, plural: kind.toLowerCase() + 's', namespaced: true }, verbs: ['list', 'watch', 'get'], category });
  const kinds = [K('', 'Pod', 'Workloads'), K('', 'Service', 'Network')];
  const forwards = [
    { id: 1, context: 'acme-uat', namespace: 'monitoring', source: { Service: { service: 'prometheus' } }, remote_port: 9090, local_port: 9090, backend: 'prometheus-0:9090', active: 2, total: 37, last_error: null },
    { id: 2, context: 'acme-uat', namespace: 'shop', source: { Pod: { pod: 'web-7d9f-abcde' } }, remote_port: 8080, local_port: 54112, backend: 'web-7d9f-abcde:8080', active: 0, total: 3, last_error: 'error forwarding port 8080 to pod web-7d9f-abcde: connection refused' },
  ];
  const table = { watch_id: 1, columns: [{ header: 'Name', kind: 'Name' }, { header: 'Namespace', kind: 'Namespace' }], rows: [{ id: 'monitoring/prometheus', name: 'prometheus', namespace: 'monitoring', cells: ['prometheus', 'monitoring'] }], error: null };
  if (params.get('late')) table.rows.push({ id: 'monitoring/grafana', name: 'grafana', namespace: 'monitoring', cells: ['grafana', 'monitoring'] });
  const replies = {
    list_contexts: { contexts: [{ name: 'acme-uat', cluster: 'acme-uat', namespace: null, prod_like: false, unlocked: false }], current: 'acme-uat' },
    connect: { context: 'acme-uat', kinds, namespaces: ['shop', 'monitoring'] }, cluster_overview: null, stop_watch: null, get_yaml: 'kind: Service\n',
    list_forwards: forwards,
    forward_ports: [{ port: 9090, name: 'http-web', detail: '→ 9090' }, { port: 8080, name: 'reloader-web', detail: '→ reloader-web' }],
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === params.get('hang')) return new Promise(() => {});
      // late=1: prometheus's ports arrive after 1.5 s, grafana's at once.
      if (cmd === 'forward_ports' && params.get('late')) {
        if (args.source.Service?.service === 'grafana') return [{ port: 3000, name: 'grafana-web', detail: '→ 3000' }];
        await new Promise((r) => setTimeout(r, 1500));
      }
      if (cmd === 'watch_resources') { setTimeout(() => (listeners['resource-table'] || []).forEach((cb) => cb({ payload: table })), 50); return 1; }
      if (cmd in replies) return replies[cmd];
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (name, cb) => { (listeners[name] = listeners[name] || []).push(cb); return () => {}; } },
  };
  const click = (sel, text, at) => setTimeout(() => { const el = [...document.querySelectorAll(sel)].find((b) => !text || b.textContent.includes(text)); el?.click(); }, at);
  if (params.get('page') === 'forwards') click('button.kind', 'Port Forwards', 900);
  if (params.get('page') === 'dialog') {
    click('.kind-section-title', 'Network', 700);
    click('button.kind', 'Service', 900);
    click('tbody tr', null, 1400);
    click('.detail-actions button', 'Forward', 1900);
    // late=1: Esc at 2200 (esc.js), then open grafana's dialog before prometheus's ports land.
    if (params.get('late')) {
      click('tbody tr', 'grafana', 2400);
      click('.detail-actions button', 'Forward', 2700);
    }
  }
})();
