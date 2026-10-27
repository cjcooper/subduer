// KEDA Details tabs, Controller logs, Pause/Resume and Edit range. Query: kind=so|sj|auth|cauth (default so), failed=1 (a
// ScaledObject whose trigger is failing and isn't ready), paused=1 (paused, no HPA), ctl=1 (clicks Controller logs; the
// open_logs target lands in the title), link=1 (follows the HPA link), op=pause|resume|range, hold=<n> (pause at n replicas),
// min=<n>, max=<n> (typed into Edit range), helm=1 (Helm-managed), ctx=prod, typed=1, locked=1, go=1 (sent args land in the title), collapse=…, scroll=<px>
(function () {
  const params = new URLSearchParams(location.search);
  const which = params.get('kind') || 'so';
  const prod = params.get('ctx') === 'prod';
  const ctxName = prod ? 'acme-prd' : 'acme-uat';
  let unlocked = params.get('locked') !== '1';
  const failed = params.get('failed') === '1';
  const paused = params.get('paused') === '1';
  const listeners = {};
  const K = (group, version, kind, plural, namespaced, category = 'CustomResources') => ({ kind: { group, version, kind, plural, namespaced }, verbs: ['list', 'watch', 'get', 'patch', 'delete'], category });
  const kinds = [K('keda.sh', 'v1alpha1', 'ScaledObject', 'scaledobjects', true), K('keda.sh', 'v1alpha1', 'ScaledJob', 'scaledjobs', true),
    K('keda.sh', 'v1alpha1', 'TriggerAuthentication', 'triggerauthentications', true), K('keda.sh', 'v1alpha1', 'ClusterTriggerAuthentication', 'clustertriggerauthentications', false),
    K('autoscaling', 'v2', 'HorizontalPodAutoscaler', 'horizontalpodautoscalers', true, 'Config'),
    K('apps', 'v1', 'Deployment', 'deployments', true, 'Workloads'), K('batch', 'v1', 'Job', 'jobs', true, 'Workloads')];
  const kindName = { so: 'ScaledObject', sj: 'ScaledJob', auth: 'TriggerAuthentication', cauth: 'ClusterTriggerAuthentication' }[which];
  const row = (key, value, extra) => ({ key, value, mono: false, time: false, link: null, url: null, ...extra });
  const trow = (cells, extra) => ({ cells, link: null, fill: null, warn: false, ...extra });
  const L = (kind, name, namespace = null) => ({ kind, namespace, name });
  const ago = (s) => new Date(Date.now() - s * 1000).toISOString().replace(/\.\d+Z$/, 'Z');
  const sec = (id, title, summary, table, rows, empty = null) => ({ id, title, summary, rows: rows || [], table, empty });
  const cond = (type_, status, reason, message) => ({ type_, status, reason, message, last_transition: ago(3600) });
  const logs = (name) => ({ namespace: 'keda', source: { Workload: { kind: { group: 'apps', version: 'v1', kind: 'Deployment', plural: 'deployments', namespaced: true }, name: 'keda-operator' } }, filter: name });
  const controller = row('Controller', 'keda/keda-operator', { link: L('apps/Deployment', 'keda-operator', 'keda') });
  const base = (kind, name, ns) => ({ kind, name, namespace: ns, status: null, info: [], created: '2026-09-01T10:00:00Z', labels: [['app.kubernetes.io/name', 'acme-api']], annotations: [], conditions: [], data: null, sections: [], events: [], logs: logs(name) });
  const authLink = L('keda.sh/TriggerAuthentication', 'prom-auth', 'shop');
  let d;
  const helm = params.get('helm') === '1';
  if (which === 'so') {
    d = base('ScaledObject', 'acme-api', 'shop');
    d.status = paused ? 'Paused' : failed ? 'Not ready' : 'Active';
    d.info = [row('Scales', 'Deployment acme-api', { link: L('apps/Deployment', 'acme-api', 'shop') }),
      ...(paused ? [row('HPA', 'keda-hpa-acme-api (none while paused)', { mono: true })]
        : [row('Replicas', '3 now · 4 wanted'), row('HPA', 'keda-hpa-acme-api', { mono: true, link: L('autoscaling/HorizontalPodAutoscaler', 'keda-hpa-acme-api', 'shop') })]),
      row('Range', '0 to 10 replicas'), row('When idle', '0 replicas after 300s without activity'), row('Polling', 'every 30s'),
      ...(paused ? [row('Paused', 'yes: scaling stopped where it is (autoscaling.keda.sh/paused)')] : []),
      row('Fallback', '2 replicas after 3 failed reads in a row'), row('When deleted', 'leaves the replicas as they are'), row('Last active', ago(60), { time: true }), controller];
    d.conditions = [failed ? cond('Ready', 'False', 'ScaledObjectCheckFailed', 'failed to ensure HPA is correctly created for ScaledObject') : cond('Ready', 'True', 'ScaledObjectReady', 'ScaledObject is defined correctly and is ready for scaling'),
      cond('Active', 'True', 'ScalerActive', 'Scaling is performed because triggers are active')];
    const columns = paused ? ['Trigger', 'Name', 'Health', 'Authentication'] : ['Trigger', 'Name', 'Now', 'Target', 'Health', 'Authentication'];
    const nowTarget = (n, t) => (paused ? [] : [n, t]);
    const m = (key, value) => row(key, value, { mono: true });
    d.sections = [sec('keda-triggers', 'Triggers', '2', { columns, numeric: [], mono: [], times: [], rows: [
      trow(['prometheus', 'rps', ...nowTarget('61 average', '50 average'), failed ? 'failing (4 in a row)' : 'OK', 'TriggerAuthentication/prom-auth'], { link: authLink, warn: failed }),
      trow(['cpu', '', ...nowTarget('55% (110m)', '70% of request'), '', ''])] }),
      sec('keda-trigger-0', 'Trigger: prometheus (rps)', '4 settings', null, [row('password', '(not shown)'),
        m('query', 'sum(rate(http_requests_total{namespace="shop",service="acme-api",code=~"2.."}[1m]))/sum(kube_deployment_status_replicas_available{deployment="acme-api"})'),
        m('serverAddress', 'http://prometheus.monitoring:9090'), m('threshold', '50')]),
      sec('keda-trigger-1', 'Trigger: cpu', '2 settings', null, [m('value', '70'), m('metricType', 'Utilization')])];
    d.events = [failed
      ? { type_: 'Warning', reason: 'KEDAScalerFailed', message: 'error inspecting prometheus metrics: connection refused', count: 4, first_seen: ago(900), last_seen: ago(30), source: 'keda-operator', object: null }
      : { type_: 'Normal', reason: 'KEDAScalersStarted', message: 'Started scalers watch', count: 1, first_seen: ago(86400), last_seen: ago(86400), source: 'keda-operator', object: null }];
  } else if (which === 'sj') {
    d = base('ScaledJob', 'acme-worker', 'shop');
    d.status = paused ? 'Paused' : 'Idle';
    d.info = [row('Runs', 'worker (acme/worker:1)', { mono: true }), row('Each job', 'parallelism 1 · retries 2 · deadline 600s'), row('Jobs at once', '0 to 20'), row('Polling', 'every 30s'),
      row('Strategy', 'default: starts as many jobs as there are waiting items, less those running'), row('On change', 'gradual: running jobs finish; new ones get the change'),
      row('Keeps', '5 succeeded · 100 failed jobs'), ...(paused ? [row('Paused', 'yes: scaling stopped where it is (autoscaling.keda.sh/paused)')] : []),
      row('Last active', ago(7200), { time: true }), controller];
    d.conditions = [cond('Ready', 'True', 'ScaledJobReady', 'ScaledJob is defined correctly and is ready to scaling'), cond('Active', 'False', 'ScalerNotActive', 'Scaling is not performed because triggers are not active')];
    d.sections = [sec('keda-triggers', 'Triggers', '1', { columns: ['Trigger'], numeric: [], mono: [], times: [], rows: [trow(['rabbitmq'])] }),
      sec('keda-trigger-0', 'Trigger: rabbitmq', '4 settings', null, [['host', 'amqp://acme:(not shown)@rabbitmq:5672/'], ['mode', 'QueueLength'], ['queueName', 'jobs'], ['value', '5']].map(([k, v]) => row(k, v, { mono: true }))),
      sec('scaledjob-jobs', 'Jobs', '3 · 1 running', { columns: ['Job', 'Status', 'Started'], numeric: [], mono: [0], times: [2], rows: [
        trow(['acme-worker-x7k2p', 'Running', ago(120)], { link: L('batch/Job', 'acme-worker-x7k2p', 'shop') }),
        trow(['acme-worker-q9d4m', 'Failed', ago(3600)], { link: L('batch/Job', 'acme-worker-q9d4m', 'shop'), warn: true }),
        trow(['acme-worker-b3n8s', 'Succeeded', ago(7200)], { link: L('batch/Job', 'acme-worker-b3n8s', 'shop') })] })];
  } else {
    const cluster = which === 'cauth';
    const ns = cluster ? 'keda' : 'shop';
    d = base(kindName, 'prom-auth', cluster ? null : 'shop');
    d.info = [row('Pod identity', 'AWS (EKS Pod Identity or IRSA) · the scaled workload\'s'), row('Role ARN', 'arn:aws:iam::123456789012:role/keda-prom', { mono: true }), controller,
      ...(cluster ? [row('Secrets from', 'keda (KEDA\'s namespace)')] : [])];
    d.sections = [sec('keda-auth-params', 'Parameters', '2', { columns: ['Parameter', 'From', 'Name', 'Key'], numeric: [], mono: [0, 2, 3], times: [], rows: [
      trow(['password', 'Secret', 'prom-creds', 'password'], { link: L('core/Secret', 'prom-creds', ns) }),
      trow(['user', 'Env var', 'PROM_USER', 'container app'])] }),
      sec('keda-auth-users', 'Used by', '2', { columns: ['Namespace', 'Kind', 'Name', 'Status'], numeric: [], mono: [2], times: [], rows: [
        trow(['shop', 'ScaledObject', 'acme-api', 'Active'], { link: L('keda.sh/ScaledObject', 'acme-api', 'shop') }),
        trow(['shop', 'ScaledObject', 'acme-search', 'Not ready'], { link: L('keda.sh/ScaledObject', 'acme-search', 'shop'), warn: true })] })];
  }
  if (helm) { d.labels.push(['app.kubernetes.io/managed-by', 'Helm']); d.annotations.push(['meta.helm.sh/release-name', 'acme-api']); }
  const id = d.namespace ? d.namespace + '/' + d.name : d.name;
  const tableFor = (kind) => {
    if (kind && kind.kind === 'HorizontalPodAutoscaler') return { id: 'shop/keda-hpa-acme-api', name: 'keda-hpa-acme-api', namespace: 'shop', cells: ['keda-hpa-acme-api', 'shop'] };
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
      if (cmd === 'list_contexts') return { contexts: [{ name: ctxName, cluster: ctxName, namespace: null, prod_like: prod, unlocked }], current: ctxName };
      if (cmd === 'set_unlocked') { unlocked = args.unlocked; return null; }
      if (cmd === 'pause_scaling' || cmd === 'set_scaling_range') { document.title = cmd + ' ' + JSON.stringify(args); return null; }
      if (cmd === 'connect') return { context: ctxName, kinds, namespaces: ['shop', 'keda'] };
      if (cmd === 'object_details') { if (args.kind.kind !== d.kind) document.title = 'details ' + args.kind.kind + ' ' + args.name; return args.kind.kind === d.kind ? d : base(args.kind.kind, args.name, args.namespace); }
      if (cmd === 'open_logs') { document.title = cmd + ' ' + JSON.stringify(args); return null; }
      if (['set_unsaved', 'cluster_overview', 'stop_watch', 'check_connection'].includes(cmd)) return null;
      if (cmd === 'get_yaml') return 'kind: ' + kindName + '\n';
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  const btn = (sel, text) => [...document.querySelectorAll(sel)].find((b) => b.textContent.trim().startsWith(text));
  setTimeout(() => [...document.querySelectorAll('.kind-section-title')].find((b) => b.textContent.includes('Custom'))?.click(), 600);
  setTimeout(() => [...document.querySelectorAll('button.kind')].find((b) => b.textContent.trim() === kindName)?.click(), 1000);
  setTimeout(() => document.querySelector('tbody tr')?.click(), 1500);
  const type = (el, v) => { if (!el) return; el.value = v; el.dispatchEvent(new Event('input', { bubbles: true })); };
  const op = { pause: 'Pause', resume: 'Resume', range: 'Edit range' }[params.get('op')];
  if (op) setTimeout(() => btn('.detail-actions button', op)?.click(), 2300);
  // A locked context asks to allow changes first.
  if (params.get('locked')) setTimeout(() => { const i = document.querySelector('.modal .confirm-input'); if (i) type(i, ctxName); setTimeout(() => btn('.modal-actions button', 'Allow')?.click(), 100); }, 2600);
  setTimeout(() => {
    const inputs = [...document.querySelectorAll('.modal .replicas-input')];
    if (params.get('min')) type(inputs[0], params.get('min'));
    if (params.get('max')) type(inputs[1], params.get('max'));
    if (params.has('hold')) {
      const box = document.querySelector('.modal .check-row input');
      if (box) { box.checked = true; box.dispatchEvent(new Event('change', { bubbles: true })); }
      setTimeout(() => type(document.querySelector('.modal .replicas-input'), params.get('hold')), 100);
    }
  }, 2900);
  if (prod && params.get('typed')) setTimeout(() => type([...document.querySelectorAll('.modal .confirm-input:not(.replicas-input)')].pop(), d.name), 3100);
  if (params.get('go')) setTimeout(() => document.querySelector('.modal-actions .primary')?.click(), 3400);
  if (params.get('ctl')) setTimeout(() => btn('.detail-actions button', 'Controller logs')?.click(), 2300);
  if (params.get('link')) setTimeout(() => [...document.querySelectorAll('.detail-body a, .detail-body button.link')].find((a) => a.textContent.includes('keda-hpa-'))?.click(), 2300);
  const collapse = params.get('collapse');
  if (collapse) setTimeout(() => collapse.split(',').forEach((c) => [...document.querySelectorAll('.dsection-name')].find((n) => n.textContent.toLowerCase() === c)?.closest('button').click()), 2300);
  const scroll = params.get('scroll');
  if (scroll) setTimeout(() => { const b = document.querySelector('.detail-body'); if (b) b.scrollTop = Number(scroll); }, 2600);
})();
