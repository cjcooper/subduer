// Deployment/StatefulSet Details tab, Restart and Scale. Query: op=restart|scale, sts=1, ctx=prod, locked=1, to=<n>, go=1,
// rolling=1 (mid-rollout), local=1 (no Prometheus), scroll=<px> (detail body), collapse=resources,deployment,...
(function () {
  const params = new URLSearchParams(location.search);
  const prod = params.get('ctx') === 'prod';
  const ctxName = prod ? 'acme-prd' : 'acme-uat';
  const sts = params.get('sts') === '1';
  const kindName = sts ? 'StatefulSet' : 'Deployment';
  let unlocked = params.get('locked') !== '1';
  const listeners = {};
  const kinds = [
    { kind: { group: 'apps', version: 'v1', kind: kindName, plural: kindName.toLowerCase() + 's', namespaced: true }, verbs: ['list', 'watch', 'get', 'patch', 'delete'], category: 'Workloads' },
    { kind: { group: '', version: 'v1', kind: 'Pod', plural: 'pods', namespaced: true }, verbs: ['list', 'watch', 'get', 'delete'], category: 'Workloads' },
  ];
  const rolling = params.get('rolling') === '1';
  const usage = (r, l, u) => ({ requests: r, limits: l, used: u, node: null });
  const ctr = (name, state, init) => ({ name, state, detail: { Running: 'Running', Stopped: 'Completed', Starting: 'ContainerCreating' }[state], init });
  const podNames = sts ? ['acme-db-0', 'acme-db-1', 'acme-db-2'] : ['acme-api-7d9f8c6b5-x2k4q', 'acme-api-7d9f8c6b5-m8n3p', 'acme-api-5c4b7f9d8-q7w2e'];
  const pods = podNames.map((n, i) => ({
    name: n, status: rolling && i === 2 ? 'ContainerCreating' : 'Running',
    containers: [ctr('migrate', 'Stopped', true), ctr('api', rolling && i === 2 ? 'Starting' : 'Running', false), ctr('cache', 'Running', false)],
    ready: rolling && i === 2 ? '0/2' : '2/2', restarts: i === 1 ? 3 : 0, node: 'ip-10-0-1-' + (5 + i) + '.ec2.internal',
    created: new Date(Date.now() - (i === 2 ? 40e3 : 86400e3 * 3)).toISOString(), updated: rolling ? i === 2 : true,
    cpu: rolling && i === 2 ? null : 240 + i * 60, memory: rolling && i === 2 ? null : 300e6 + i * 20e6,
  }));
  const info = sts
    ? [['Strategy', 'RollingUpdate · partition 0'], ['Pod management', 'OrderedReady'], ['Service', 'acme-db-headless'], ['Volume claims', 'data (20Gi, gp3)'], ['Revision', 'acme-db-6f7c9d8b4'], ['Selector', 'app=acme-db'], ['Service account', 'acme-db']]
    : [['Strategy', 'RollingUpdate · max surge 25% · max unavailable 25%'], ['Revision', rolling ? '15' : '14'], ['Last restart', '2026-10-08T14:02:11Z'], ['Selector', 'app=acme-api,tier=web'], ['Service account', 'acme-api']];
  const events = [
    { type_: 'Normal', reason: 'ScalingReplicaSet', message: 'Scaled up replica set acme-api-5c4b7f9d8 to 1', count: 1, first_seen: new Date(Date.now() - 45e3).toISOString(), last_seen: new Date(Date.now() - 45e3).toISOString(), source: 'deployment-controller', object: null },
    { type_: 'Warning', reason: 'BackOff', message: 'Back-off restarting failed container api in pod ' + podNames[1], count: 4, first_seen: new Date(Date.now() - 1800e3).toISOString(), last_seen: new Date(Date.now() - 600e3).toISOString(), source: 'kubelet', object: 'Pod ' + podNames[1] },
    { type_: 'Normal', reason: 'Pulled', message: 'Container image "ghcr.io/acme/api:1.4.0" already present on machine', count: 1, first_seen: new Date(Date.now() - 40e3).toISOString(), last_seen: new Date(Date.now() - 40e3).toISOString(), source: 'kubelet', object: 'Pod ' + podNames[2] },
  ];
  const env = [{ name: 'LOG_LEVEL', value: { Literal: 'info' }, from: null }, { name: 'DB_PASSWORD', value: { Secret: { name: 'acme-db', key: 'password' } }, from: null }];
  const details = {
    kind: kindName, name: sts ? 'acme-db' : 'acme-api', namespace: 'shop', status: rolling ? 'Rolling out' : (sts ? 'Ready' : 'Available'),
    replicas: rolling ? { desired: 3, current: 3, ready: 2, updated: 1, available: 2 } : { desired: 3, current: 3, ready: 3, updated: 3, available: 3 },
    info, created: '2026-09-01T10:00:00Z', labels: [['app', 'acme-api'], ['team', 'checkout']],
    conditions: [
      { type_: 'Available', status: rolling ? 'True' : 'True', reason: 'MinimumReplicasAvailable', message: 'Deployment has minimum availability.', last_transition: '2026-10-08T14:03:00Z' },
      { type_: 'Progressing', status: 'True', reason: rolling ? 'ReplicaSetUpdated' : 'NewReplicaSetAvailable', message: rolling ? 'ReplicaSet "acme-api-5c4b7f9d8" is progressing.' : 'ReplicaSet "acme-api-7d9f8c6b5" has successfully progressed.', last_transition: '2026-10-08T14:02:11Z' },
    ],
    cpu: usage(750, 3000, 900), memory: usage(1207959552, 2415919104, 980e6), metrics_available: true, pods,
    containers: [
      { name: 'migrate', init: true, sidecar: false, image: 'ghcr.io/acme/api:1.4.0', ports: [], cpu: usage(100, null, 0), memory: usage(67108864, null, 0), env: [] },
      { name: 'api', init: false, sidecar: false, image: 'ghcr.io/acme/api:1.4.0', ports: ['http 8080/TCP', 'metrics 9090/TCP'], cpu: usage(200, 1000, 820), memory: usage(268435456, 536870912, 800e6), env },
      { name: 'cache', init: false, sidecar: false, image: 'docker.io/library/redis:7.4', ports: ['6379/TCP'], cpu: usage(50, null, 80), memory: usage(134217728, 268435456, 180e6), env: [] },
    ],
    events,
  };
  const end = Math.floor(Date.now() / 15000) * 15, start = end - 3600;
  const series = (fn, from) => { const out = []; for (let t = from ?? start; t <= end; t += 15) out.push([t, fn((t - start) / 3600)]); return out; };
  const local = params.get('local') === '1';
  const from = local ? end - 600 : start;
  const hc = [
    { name: 'api', usage: { cpu: series((x) => 700 + 150 * Math.sin(x * 9), from), memory: series((x) => 760e6 + 60e6 * x, from) } },
    { name: 'cache', usage: { cpu: series((x) => 70 + 15 * Math.sin(x * 5), from), memory: series(() => 180e6, from) } },
  ];
  const sum = (k) => hc[0].usage[k].map(([t, v], i) => [t, v + hc[1].usage[k][i][1]]);
  const history = { source: local ? 'Local' : 'Prometheus', start, end, step: 15, pod: { cpu: sum('cpu'), memory: sum('memory') }, containers: hc };
  const name = sts ? 'acme-db' : 'acme-api';
  const table = { watch_id: 1, columns: [{ header: 'Name', kind: 'Name' }, { header: 'Namespace', kind: 'Namespace' }, { header: 'Ready', kind: 'Text' }], rows: [{ id: 'shop/' + name, name, namespace: 'shop', cells: [name, 'shop', '3/3'] }], error: null };
  const replies = {
    connect: { context: ctxName, kinds, namespaces: ['shop'] }, cluster_overview: null, stop_watch: null,
    get_yaml: 'apiVersion: apps/v1\nkind: ' + kindName + '\nmetadata:\n  name: ' + name + '\n  namespace: shop\nspec:\n  replicas: 3\n',
    get_replicas: { desired: 3, current: 3 }, workload_details: details, workload_history: history,
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === 'watch_resources' && args.kind.kind === 'Pod') { setTimeout(() => (listeners['resource-table'] || []).forEach((cb) => cb({ payload: { watch_id: 2, columns: table.columns, rows: podNames.map((n) => ({ id: 'shop/' + n, name: n, namespace: 'shop', cells: [n, 'shop', '2/2'] })), error: null } })), 50); return 2; }
      if (cmd === 'pod_details') throw 'stub: pod details not faked here';
      if (cmd === 'pod_history') throw 'stub';
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
  const collapse = params.get('collapse');
  if (collapse) setTimeout(() => collapse.split(',').forEach((c) => [...document.querySelectorAll('.dsection-name')].find((n) => n.textContent.toLowerCase() === c)?.closest('button').click()), 2400);
  const scroll = params.get('scroll');
  if (scroll) setTimeout(() => { const b = document.querySelector('.detail-body'); if (b) b.scrollTop = Number(scroll); }, 2700);
  if (params.get('openpod')) setTimeout(() => document.querySelector('.wpods tbody tr:nth-child(2)')?.click(), 2600);
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
