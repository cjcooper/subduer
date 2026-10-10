// Network-section Details tabs. Query: kind=svc|ingress|netpol|slice (default svc), open=1 (click the first Open button;
// the sent URL lands in the page title), link=1 (click the first table row link), collapse=…, scroll=<px>
(function () {
  const params = new URLSearchParams(location.search);
  const which = params.get('kind') || 'svc';
  const listeners = {};
  const K = (group, kind, plural, namespaced) => ({ kind: { group, version: 'v1', kind, plural, namespaced }, verbs: ['list', 'watch', 'get', 'patch', 'delete'], category: 'Network' });
  const kinds = [
    K('', 'Service', 'services', true), K('networking.k8s.io', 'Ingress', 'ingresses', true), K('networking.k8s.io', 'IngressClass', 'ingressclasses', false),
    K('networking.k8s.io', 'NetworkPolicy', 'networkpolicies', true), K('discovery.k8s.io', 'EndpointSlice', 'endpointslices', true), K('', 'Endpoints', 'endpoints', true),
    { kind: { group: '', version: 'v1', kind: 'Pod', plural: 'pods', namespaced: true }, verbs: ['list', 'watch', 'get'], category: 'Workloads' },
  ];
  const kindName = { svc: 'Service', ingress: 'Ingress', netpol: 'NetworkPolicy', slice: 'EndpointSlice' }[which];
  const row = (key, value, extra) => ({ key, value, mono: false, time: false, link: null, url: null, ...extra });
  const trow = (cells, extra) => ({ cells, link: null, fill: null, warn: false, ...extra });
  const L = (kind, name, namespace = 'shop') => ({ kind, namespace, name });
  const table = (columns, rows, extra) => ({ columns, numeric: [], mono: [], rows, ...extra });
  const sec = (id, title, summary, t, rows) => ({ id, title, summary, rows: rows || [], table: t, empty: null });
  const endpoints = [
    trow(['10.1.0.4', 'ready', 'Pod acme-api-7d9f8c6b5-x2k4q', 'ip-10-0-1-5.ec2.internal', 'us-east-1a'], { link: L('core/Pod', 'acme-api-7d9f8c6b5-x2k4q') }),
    trow(['10.1.0.9', 'ready', 'Pod acme-api-7d9f8c6b5-m8n3p', 'ip-10-0-1-6.ec2.internal', 'us-east-1b'], { link: L('core/Pod', 'acme-api-7d9f8c6b5-m8n3p') }),
    trow(['10.1.0.12', 'not ready', 'Pod acme-api-5c4b7f9d8-q7w2e', 'ip-10-0-1-7.ec2.internal', 'us-east-1c'], { link: L('core/Pod', 'acme-api-5c4b7f9d8-q7w2e'), warn: true }),
  ];
  const epTable = table(['Address', 'State', 'Target', 'Node', 'Zone'], endpoints, { mono: [0] });
  const base = (kind, name) => ({ kind, name, namespace: 'shop', status: null, info: [], created: '2026-09-01T10:00:00Z', labels: [['app', 'acme-api']], annotations: [], conditions: [], data: null, sections: [], events: [] });
  let d;
  if (which === 'svc') {
    d = base('Service', 'acme-api');
    d.status = 'Partly ready';
    d.info = [row('Type', 'LoadBalancer'), row('Cluster IP', '172.20.19.62', { mono: true }), row('Load balancer', 'k8s-shop-acmeapi-1a2b3c.elb.us-east-1.amazonaws.com', { mono: true }),
      row('Selector', 'app=acme-api', { mono: true }), row('External traffic', 'Local: only nodes running its pods, client IP kept')];
    d.sections = [
      sec('ports', 'Ports', '2', table(['Name', 'Port', 'Target', 'Node port', 'Protocol', 'App protocol'], [trow(['http', '80', 'http', '31080', 'TCP', '']), trow(['grpc', '9090', '9090', '31090', 'TCP', 'kubernetes.io/h2c'])], { numeric: [1, 3], mono: [2] })),
      sec('endpoints', 'Endpoints', '2 of 3 ready', epTable),
      sec('routed-from', 'Routed from', '2', table(['Ingress', 'Host', 'Path'], [trow(['web', 'shop.acme.io', '/api'], { link: L('networking.k8s.io/Ingress', 'web') }), trow(['web', 'admin.acme.io', '/'], { link: L('networking.k8s.io/Ingress', 'web') })], { mono: [2] }))];
  } else if (which === 'ingress') {
    d = base('Ingress', 'web');
    d.status = 'Backend missing';
    d.info = [row('Class', 'alb', { link: L('networking.k8s.io/IngressClass', 'alb', null) }), row('Address', 'k8s-shop-web-9f8e7d.us-east-1.elb.amazonaws.com', { mono: true })];
    d.annotations = [['alb.ingress.kubernetes.io/scheme', 'internet-facing'], ['alb.ingress.kubernetes.io/target-type', 'ip']];
    d.sections = [
      sec('rules', 'Rules', '3 paths', table(['Host', 'Path', 'Match', 'Backend', 'Ready'], [
        trow(['shop.acme.io', '/', 'Prefix', 'acme-web:80', '3/3'], { link: L('core/Service', 'acme-web') }),
        trow(['shop.acme.io', '/api', 'Prefix', 'acme-api:http', '2/3'], { link: L('core/Service', 'acme-api') }),
        trow(['admin.acme.io', '/', 'Prefix', 'acme-admin:80', 'no such service'], { link: L('core/Service', 'acme-admin'), warn: true })], { numeric: [4], mono: [1, 3] })),
      sec('urls', 'URLs', '', null, [row('URL', 'https://shop.acme.io/', { mono: true, url: 'https://shop.acme.io/' }), row('URL', 'https://shop.acme.io/api', { mono: true, url: 'https://shop.acme.io/api' }), row('URL', 'http://admin.acme.io/', { mono: true, url: 'http://admin.acme.io/' })]),
      sec('tls', 'TLS', '1', table(['Hosts', 'Certificate secret'], [trow(['shop.acme.io', 'shop-tls'], { link: L('core/Secret', 'shop-tls') })]))];
    d.events = [{ type_: 'Warning', reason: 'FailedDeployModel', message: 'Failed deploy model due to InvalidParameter: service acme-admin not found', count: 3, first_seen: new Date(Date.now() - 900e3).toISOString(), last_seen: new Date(Date.now() - 120e3).toISOString(), source: 'ingress', object: null }];
  } else if (which === 'netpol') {
    d = base('NetworkPolicy', 'acme-api');
    d.status = 'Denies outgoing';
    d.info = [row('Applies to', 'pods app=acme-api'), row('Incoming', 'only what the 2 rules below allow'), row('Outgoing', 'all blocked (unless another policy allows it)')];
    d.sections = [
      sec('ingress', 'Incoming', '2', table(['From', 'Ports'], [trow(['pods app=acme-web in this namespace; all pods in namespaces team=ops', 'TCP 8080']), trow(['addresses 10.0.0.0/8 except 10.1.0.0/16', 'TCP 9090, UDP 5000–5010'])])),
      sec('policy-pods', 'Pods it applies to', '2 pods', table(['Pod', 'Controlled by'], endpoints.slice(0, 2).map((r) => trow([r.link.name, 'ReplicaSet/acme-api-7d9f8c6b5'], { link: r.link }))))];
  } else {
    d = base('EndpointSlice', 'acme-api-x7k2p');
    d.status = 'Partly ready';
    d.info = [row('Service', 'acme-api', { link: L('core/Service', 'acme-api') }), row('Address type', 'IPv4'), row('Managed by', 'endpointslice-controller.k8s.io'), row('Ports', 'http 8080/TCP, grpc 9090/TCP'), row('Controlled by', 'Service/acme-api')];
    d.sections = [sec('slice-endpoints', 'Endpoints', '2 of 3 ready', epTable)];
  }
  const id = 'shop/' + d.name;
  const rows = [{ id, name: d.name, namespace: 'shop', cells: [d.name, 'shop'] }];
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === 'watch_resources') { setTimeout(() => (listeners['resource-table'] || []).forEach((cb) => cb({ payload: { watch_id: 1, columns: [{ header: 'Name', kind: 'Name' }, { header: 'Namespace', kind: 'Namespace' }], rows, error: null } })), 50); return 1; }
      if (cmd === 'list_contexts') return { contexts: [{ name: 'acme-uat', cluster: 'acme-uat', namespace: null, prod_like: false, unlocked: false }], current: 'acme-uat' };
      if (cmd === 'connect') return { context: 'acme-uat', kinds, namespaces: ['shop'] };
      if (cmd === 'object_details') return d;
      if (cmd === 'open_url') { document.title = 'open_url ' + JSON.stringify(args); return null; }
      if (cmd === 'forward_ports') return [];
      if (cmd === 'set_unsaved' || cmd === 'cluster_overview' || cmd === 'stop_watch' || cmd === 'list_forwards') return cmd === 'list_forwards' ? [] : null;
      if (cmd === 'get_yaml') return 'kind: ' + kindName + '\n';
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  const btn = (sel, text) => [...document.querySelectorAll(sel)].find((b) => b.textContent.trim().startsWith(text));
  setTimeout(() => [...document.querySelectorAll('.kind-section-title')].find((b) => b.textContent.includes('Network'))?.click(), 600);
  setTimeout(() => btn('button.kind', kindName)?.click(), 1000);
  setTimeout(() => document.querySelector('tbody tr')?.click(), 1500);
  if (params.get('open')) setTimeout(() => btn('.url-row button', 'Open')?.click(), 2400);
  if (params.get('link')) setTimeout(() => { document.querySelector('.itable tbody tr.clickable')?.click(); setTimeout(() => { document.title = 'kind=' + (document.querySelector('.kinds .kind.active')?.textContent || '?'); }, 300); }, 2400);
  const collapse = params.get('collapse');
  if (collapse) setTimeout(() => collapse.split(',').forEach((c) => [...document.querySelectorAll('.dsection-name')].find((n) => n.textContent.toLowerCase() === c)?.closest('button').click()), 2300);
  const scroll = params.get('scroll');
  if (scroll) setTimeout(() => { const b = document.querySelector('.detail-body'); if (b) b.scrollTop = Number(scroll); }, 2600);
})();
