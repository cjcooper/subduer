// Node Details tab: resource charts (node_history), the pods on it (each opening its pod) and events. Query: cordoned=1, notready=1, local=1 (history recorded by the app), fargate=1 (one pod per node: no pod count),
// link=1 (click the Pods table's "api-2" row; the title then reports the pod list opened and the pod_details call),
// collapse=…, scroll=<px>.
(function () {
  const params = new URLSearchParams(location.search);
  const listeners = {};
  let watchId = 0;
  const K = (group, kind, plural, namespaced, category) => ({ kind: { group, version: 'v1', kind, plural, namespaced }, verbs: ['list', 'watch', 'get'], category });
  const kinds = [K('', 'Node', 'nodes', false, 'TopLevel'), K('', 'Pod', 'pods', true, 'Workloads')];
  const ago = (s) => new Date(Date.now() - s * 1000).toISOString();
  const row = (key, value, extra) => ({ key, value, mono: false, time: false, link: null, url: null, ...extra });
  const trow = (cells, extra) => ({ cells, link: null, fill: null, warn: false, ...extra });
  const P = (namespace, name) => ({ kind: 'core/Pod', namespace, name });
  const cordoned = params.get('cordoned') === '1';
  const notReady = params.get('notready') === '1';
  const status = notReady ? 'NotReady' : cordoned ? 'Cordoned' : 'Ready';
  const cond = (type_, s, reason) => ({ type_, status: s, reason, message: '', last_transition: ago(86400 * 3) });
  const d = {
    kind: 'Node', name: 'node-a', namespace: null, status, created: ago(86400 * 40),
    labels: [['node-role.kubernetes.io/worker', ''], ['node.kubernetes.io/instance-type', 'm5.large'], ['topology.kubernetes.io/zone', 'us-east-1a']],
    annotations: [], data: null,
    conditions: [cond('MemoryPressure', notReady ? 'True' : 'False', 'KubeletHasSufficientMemory'), cond('DiskPressure', 'False', 'KubeletHasNoDiskPressure'),
      cond('PIDPressure', 'False', 'KubeletHasSufficientPID'), cond('Ready', notReady ? 'Unknown' : 'True', notReady ? 'NodeStatusUnknown' : 'KubeletReady')],
    info: [row('Roles', 'worker'), row('Instance type', 'm5.large'), row('Zone', 'us-east-1a · us-east-1'),
      ...(cordoned ? [row('Scheduling', 'cordoned: no new pods are placed here')] : []),
      row('Internal IP', '10.0.1.5', { mono: true }), row('Hostname', 'ip-10-0-1-5', { mono: true }),
      row('Taints', 'dedicated=batch:NoSchedule', { mono: true }), row('Kubelet', 'v1.33.1'), row('OS', 'Amazon Linux 2023'),
      row('Kernel', '6.1.0'), row('Container runtime', 'containerd://1.7.2'), row('Architecture', 'amd64'),
      row('Pod CIDR', '10.0.1.0/24', { mono: true }), row('Provider ID', 'aws:///us-east-1a/i-0abc', { mono: true })],
    sections: [
      { id: 'node-pods', title: 'Pods', summary: '3 pods · 1 not ready', rows: [], empty: 'No pods are running on it.', table: {
        columns: ['Pod', 'Namespace', 'Status', 'Restarts', 'CPU used / req.', 'Memory used / req.', 'Age'],
        numeric: [3, 4, 5], mono: [], times: [6], rows: [
          trow(['proxy-1', 'kube-system', 'Running', '0', '3m / 0m', '1.0 MiB / 0 B', ago(86400 * 40)], { link: P('kube-system', 'proxy-1') }),
          trow(['api-1', 'shop', 'Running', '0', '120m / 1.50 cores', '256.0 MiB / 1.0 GiB', ago(3600 * 5)], { link: P('shop', 'api-1') }),
          trow(['api-2', 'shop', 'CrashLoopBackOff', '4', '— / 250m', '— / 256.0 MiB', ago(1800)], { link: P('shop', 'api-2'), warn: true })] } }],
    events: [{ type_: 'Warning', reason: 'NodeNotReady', message: 'Node node-a status is now: NodeNotReady', count: 1, first_seen: ago(7200), last_seen: ago(7200), source: 'node-controller', object: null },
      { type_: 'Normal', reason: 'NodeReady', message: 'Node node-a status is now: NodeReady', count: 1, first_seen: ago(7000), last_seen: ago(7000), source: 'kubelet', object: null }],
    node: { cpu: { capacity: 2000, allocatable: 1930, requests: 1750, limits: 0, used: 123 },
      memory: { capacity: 8 * 2 ** 30, allocatable: 7 * 2 ** 30, requests: 1.25 * 2 ** 30, limits: 1.25 * 2 ** 30, used: 257 * 2 ** 20 },
      ...(params.get('fargate') ? { pods: 1, pod_capacity: 1, one_pod_hosting: 'AWS Fargate' } : { pods: 3, pod_capacity: 29 }) },
  };
  // The last hour, every 15 s: CPU wanders around 120m with a spike; memory creeps up.
  const end = Math.floor(Date.now() / 15000) * 15;
  const times = Array.from({ length: 241 }, (_, i) => end - 3600 + i * 15);
  const history = { source: params.get('local') ? 'Local' : 'Prometheus', start: end - 3600, end, step: 15,
    pod: { cpu: times.map((t, i) => [t, 110 + 25 * Math.sin(i / 9) + (i > 150 && i < 165 ? 300 : 0)]),
      memory: times.map((t, i) => [t, (230 + i * 0.11) * 2 ** 20]) }, containers: [] };
  const totals = (cap, alloc, req, used) => ({ capacity: cap, allocatable: alloc, requests: req, limits: 0, used });
  const node = (name, extra) => ({ name, ready: !notReady, schedulable: !cordoned, roles: ['worker'], instance_type: 'm5.large', kubelet_version: 'v1.33.1',
    created: ago(86400 * 40), pods: 3, pod_capacity: 29, cpu: totals(2000, 1930, 1750, 123), memory: totals(8 * 2 ** 30, 7 * 2 ** 30, 1.3 * 2 ** 30, 257 * 2 ** 20), ...extra });
  const overview = { server_version: 'v1.33.1', nodes: [node('node-a'), node('node-b', { ready: true, schedulable: true })], pods: { running: 5, pending: 0, succeeded: 0, failed: 0, unknown: 0 },
    cpu: totals(4000, 3860, 3500, 246), memory: totals(16 * 2 ** 30, 14 * 2 ** 30, 2.6 * 2 ** 30, 514 * 2 ** 20), namespaces: [], warnings_last_hour: 1,
    metrics_available: true, usage_source: 'Kubelet', history: null };
  const report = (s) => { document.title = (document.title ? document.title + ' | ' : '') + s; };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === 'watch_resources') {
        const wid = ++watchId;
        const k = args.kind.kind;
        const rows = k === 'Pod' ? [{ id: 'shop/api-2', name: 'api-2', namespace: 'shop', cells: ['api-2'] }] : [];
        setTimeout(() => (listeners['resource-table'] || []).forEach((cb) => cb({ payload: { watch_id: wid, columns: [{ header: 'Name', kind: 'Name' }], rows, error: null } })), 50);
        if (k === 'Pod') report('list=Pod ns=' + (args.namespace || 'all'));
        return wid;
      }
      if (cmd === 'list_contexts') return { contexts: [{ name: 'acme-uat', cluster: 'acme-uat', namespace: null, prod_like: false, unlocked: true }], current: 'acme-uat' };
      if (cmd === 'connect') return { context: 'acme-uat', kinds, namespaces: ['shop', 'kube-system'] };
      if (cmd === 'cluster_overview') return overview;
      if (cmd === 'object_details') { report('object_details ' + args.kind.kind + ' ' + args.name); return d; }
      if (cmd === 'node_history') { report('node_history ' + args.name + (args.namespace === undefined ? '' : ' with namespace')); return history; }
      if (cmd === 'pod_details') { report('pod_details ' + args.namespace + '/' + args.name); throw 'stub: no pod details'; }
      if (['set_unsaved', 'stop_watch', 'set_app_theme', 'check_connection'].includes(cmd)) return null;
      if (cmd === 'get_yaml') return 'kind: Node\n';
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  setTimeout(() => [...document.querySelectorAll('button.kind')].find((b) => b.textContent.trim() === 'Node')?.click(), 800);
  setTimeout(() => document.querySelector('tbody tr.clickable')?.click(), 1500);
  if (params.get('link')) setTimeout(() => [...document.querySelectorAll('tr.clickable')].find((r) => r.textContent.includes('api-2'))?.click(), 2600);
  const collapse = params.get('collapse');
  if (collapse) setTimeout(() => collapse.split(',').forEach((c) => [...document.querySelectorAll('.dsection-name')].find((n) => n.textContent.toLowerCase() === c)?.closest('button').click()), 2300);
  const scroll = params.get('scroll');
  if (scroll) setTimeout(() => { const b = document.querySelector('.detail-body'); if (b) b.scrollTop = Number(scroll); }, 2600);
})();
