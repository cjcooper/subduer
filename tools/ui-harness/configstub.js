// Config-section Details tabs, data editing and Edit range. Query: kind=cm|secret|hpa|quota|webhook|pdb|lease (default cm),
// show=1 (Show a secret value), edit=1 (start editing), change=1 (edit a value, rename nothing), add=1 (add a key),
// badkey=1 (add an invalid key), remove=1 (remove the first key), save=1 (open the Save dialog), range=1 (open Edit range),
// ctx=prod, typed=1, locked=1, go=1 (sent args land in the page title), collapse=…, scroll=<px>,
// leave=row|close|yaml|kind|refresh|link|ns|context|cancel|window|quit (try to leave an edit), then discard=1 or keep=1.
// The title then reads `unsaved=<last set_unsaved> selected=<row> …`.
(function () {
  const params = new URLSearchParams(location.search);
  const prod = params.get('ctx') === 'prod';
  const ctxName = prod ? 'acme-prd' : 'acme-uat';
  const which = params.get('kind') || 'cm';
  let unlocked = params.get('locked') !== '1';
  const listeners = {};
  const K = (group, version, kind, plural, namespaced, verbs) => ({ kind: { group, version, kind, plural, namespaced }, verbs: verbs || ['list', 'watch', 'get', 'patch', 'update', 'delete'], category: 'Config' });
  const kinds = [
    K('', 'v1', 'ConfigMap', 'configmaps', true), K('', 'v1', 'Secret', 'secrets', true),
    K('autoscaling', 'v2', 'HorizontalPodAutoscaler', 'horizontalpodautoscalers', true),
    K('policy', 'v1', 'PodDisruptionBudget', 'poddisruptionbudgets', true), K('', 'v1', 'ResourceQuota', 'resourcequotas', true),
    K('coordination.k8s.io', 'v1', 'Lease', 'leases', true),
    K('admissionregistration.k8s.io', 'v1', 'ValidatingWebhookConfiguration', 'validatingwebhookconfigurations', false),
    { kind: { group: '', version: 'v1', kind: 'Pod', plural: 'pods', namespaced: true }, verbs: ['list', 'watch', 'get'], category: 'Workloads' },
    { kind: { group: 'apps', version: 'v1', kind: 'Deployment', plural: 'deployments', namespaced: true }, verbs: ['list', 'watch', 'get'], category: 'Workloads' },
  ];
  const kindName = { cm: 'ConfigMap', secret: 'Secret', hpa: 'HorizontalPodAutoscaler', quota: 'ResourceQuota', webhook: 'ValidatingWebhookConfiguration', pdb: 'PodDisruptionBudget', lease: 'Lease' }[which];
  const ago = (s) => new Date(Date.now() - s * 1000).toISOString();
  const row = (key, value, extra) => ({ key, value, mono: false, time: false, link: null, ...extra });
  const trow = (cells, extra) => ({ cells, link: null, fill: null, warn: false, ...extra });
  const podLink = (n) => ({ kind: 'core/Pod', namespace: 'shop', name: n });
  const usedBy = (how) => ({ id: 'used-by', title: 'Used by', summary: '2 pods', rows: [], empty: 'No pods in this namespace use it.',
    table: { columns: ['Pod', 'Controlled by', 'As'], numeric: [], mono: [], rows: [
      trow(['acme-api-7d9f8c6b5-x2k4q', 'ReplicaSet/acme-api-7d9f8c6b5', how], { link: podLink('acme-api-7d9f8c6b5-x2k4q') }),
      trow(['acme-worker-0', 'StatefulSet/acme-worker', 'volume'], { link: podLink('acme-worker-0') })] } });
  const base = (kind, name, ns) => ({ kind, name, namespace: ns, status: null, info: [], created: '2026-09-01T10:00:00Z', labels: [['app', 'acme-api']], annotations: [], conditions: [], data: null, sections: [], events: [] });
  const secretValues = [['DB_PASSWORD', 'hunter2-but-longer'], ['API_TOKEN', 'tok_8f2a9c1e77'], ['tls.p12', null]];
  let details;
  if (which === 'cm') {
    details = base('ConfigMap', 'acme-api-config', 'shop');
    details.info = [row('Keys', '3'), row('Size', '612 B')];
    details.annotations = [['meta.helm.sh/release-name', 'acme-api'], ['meta.helm.sh/release-namespace', 'shop']];
    details.data = { secret: false, resource_version: '4711', read_only: null, warning: 'Managed by Helm (release acme-api): the next helm upgrade puts the chart\'s values back.', entries: [
      { key: 'LOG_LEVEL', value: 'info', size: 4, binary: false },
      { key: 'app.yaml', value: 'server:\n  port: 8080\n  timeout: 30s\nfeatures:\n  checkout: true\n  wishlist: false\n', size: 78, binary: false },
      { key: 'logo.png', value: null, size: 530, binary: true }] };
    details.sections = [usedBy('env, volume')];
  } else if (which === 'secret') {
    details = base('Secret', 'acme-api-secrets', 'shop');
    details.info = [row('Type', 'Opaque', { mono: true }), row('Keys', '3'), row('Size', '2.1 KiB')];
    details.data = { secret: true, resource_version: '815', read_only: null, warning: null, entries: [
      { key: 'API_TOKEN', value: null, size: 14, binary: false }, { key: 'DB_PASSWORD', value: null, size: 18, binary: false },
      { key: 'tls.p12', value: null, size: 2048, binary: true }] };
    details.sections = [usedBy('env')];
  } else if (which === 'hpa') {
    details = base('HorizontalPodAutoscaler', 'acme-api', 'shop');
    details.status = 'At limit';
    details.info = [row('Scales', 'Deployment acme-api', { link: { kind: 'apps/Deployment', namespace: 'shop', name: 'acme-api' } }), row('Replicas', '10 now · 10 wanted'), row('Min replicas', '2'), row('Max replicas', '10'), row('Last scaled', ago(1500), { time: true })];
    details.conditions = [
      { type_: 'AbleToScale', status: 'True', reason: 'ReadyForNewScale', message: 'recommended size matches current size', last_transition: ago(1500) },
      { type_: 'ScalingActive', status: 'True', reason: 'ValidMetricFound', message: 'the HPA was able to successfully calculate a replica count from cpu resource utilization', last_transition: ago(86400) },
      { type_: 'ScalingLimited', status: 'True', reason: 'TooManyReplicas', message: 'the desired replica count is more than the maximum replica count', last_transition: ago(1500) }];
    details.sections = [
      { id: 'metrics', title: 'Metrics', summary: '2', rows: [], empty: null, table: { columns: ['Metric', 'Now', 'Target'], numeric: [1, 2], mono: [], rows: [trow(['cpu', '93% (465m)', '70% of request']), trow(['rps (per pod)', '140 average', '100 average'])] } },
      { id: 'behavior', title: 'Behavior', summary: '', rows: [row('Scale down', 'waits 300s · 50% per 60s (Max)')], table: null, empty: null }];
  } else if (which === 'quota') {
    details = base('ResourceQuota', 'compute', 'shop');
    details.status = 'At limit';
    details.labels = [];
    const q = (r, u, h, f) => trow([r, u, h], { fill: f, warn: f >= 0.9 });
    details.sections = [{ id: 'quota', title: 'Usage', summary: '5 resources · 1 at limit', rows: [], empty: null, table: { columns: ['Resource', 'Used', 'Hard'], numeric: [1, 2], mono: [0], rows: [
      q('limits.cpu', '7500m', '16', 0.47), q('limits.memory', '28Gi', '32Gi', 0.875), q('pods', '40', '40', 1), q('requests.cpu', '3800m', '8', 0.475), q('requests.memory', '15Gi', '16Gi', 0.94)] } }];
  } else if (which === 'pdb') {
    details = base('PodDisruptionBudget', 'acme-api', 'shop');
    details.status = 'Blocking';
    details.info = [row('Min available', '2'), row('Healthy', '2 of 2 pods · 2 needed'), row('Evictions allowed', '0'), row('Selector', 'app=acme-api', { mono: true })];
    details.conditions = [{ type_: 'DisruptionAllowed', status: 'False', reason: 'InsufficientPods', message: '', last_transition: ago(3600) }];
    details.sections = [{ id: 'pdb-pods', title: 'Pods', summary: '2', rows: [], empty: null, table: { columns: ['Pod', 'Status', 'Node'], numeric: [], mono: [], rows: [
      trow(['acme-api-7d9f8c6b5-x2k4q', 'Running', 'ip-10-0-1-5.ec2.internal'], { link: podLink('acme-api-7d9f8c6b5-x2k4q') }),
      trow(['acme-api-7d9f8c6b5-m8n3p', 'Running', 'ip-10-0-1-6.ec2.internal'], { link: podLink('acme-api-7d9f8c6b5-m8n3p') })] } }];
  } else if (which === 'lease') {
    details = base('Lease', 'acme-controller-leader', 'shop');
    details.status = 'Held';
    details.info = [row('Holder', 'acme-controller-5d8b7_1f2e', { mono: true }), row('Duration', '15s'), row('Acquired', ago(86400 * 2), { time: true }), row('Renewed', ago(3), { time: true }), row('Changed hands', '61 times')];
  } else {
    details = base('ValidatingWebhookConfiguration', 'acme-admission', null);
    details.info = [row('Webhooks', '1')];
    details.sections = [{ id: 'webhook:check.acme.io', title: 'check.acme.io', summary: 'Fail · 1 rule', empty: null, rows: [
      row('Calls', 'service acme/acme-admission:443/validate', { mono: true, link: { kind: 'core/Service', namespace: 'acme', name: 'acme-admission' } }),
      row('If it fails', 'Fail: requests are rejected'), row('Timeout', '10s'), row('Side effects', 'None'), row('Match policy', 'Equivalent'), row('Namespaces', 'acme.io/checked=true', { mono: true }), row('Objects', 'all', { mono: true })],
      table: { columns: ['Operations', 'Groups', 'Versions', 'Resources', 'Scope'], numeric: [], mono: [3], rows: [trow(['CREATE, UPDATE', 'apps', 'v1', 'deployments, statefulsets', 'Namespaced'])] } }];
  }
  if (details.namespace) details.events = [{ type_: 'Normal', reason: which === 'hpa' ? 'SuccessfulRescale' : 'Updated', message: which === 'hpa' ? 'New size: 10; reason: cpu resource utilization (percentage of request) above target' : 'Updated', count: 1, first_seen: ago(1500), last_seen: ago(1500), source: 'horizontal-pod-autoscaler', object: null }];
  const ns = details.namespace;
  const id = ns ? ns + '/' + details.name : details.name;
  const table = { watch_id: 1, columns: [{ header: 'Name', kind: 'Name' }, { header: 'Namespace', kind: 'Namespace' }], rows: [{ id, name: details.name, namespace: ns, cells: [details.name, ns || ''] }, { id: (ns ? ns + '/' : '') + 'other', name: 'other', namespace: ns, cells: ['other', ns || ''] }], error: null };
  let unsaved = null, closed = null;
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === 'watch_resources') { setTimeout(() => (listeners['resource-table'] || []).forEach((cb) => cb({ payload: table })), 50); return 1; }
      if (cmd === 'list_contexts') return { contexts: [{ name: ctxName, cluster: ctxName, namespace: null, prod_like: prod, unlocked }], current: ctxName };
      if (cmd === 'connect') return { context: ctxName, kinds, namespaces: ['shop', 'acme'] };
      if (cmd === 'set_unlocked') { unlocked = true; return null; }
      if (cmd === 'object_details') return args.name === details.name ? details : { ...details, name: args.name, data: details.data && { ...details.data, entries: [] } };
      if (cmd === 'secret_value') return secretValues.find(([k]) => k === args.key)?.[1] ?? '<binary>';
      if (cmd === 'secret_values') return secretValues;
      if (cmd === 'set_unsaved') { unsaved = args.unsaved; return null; }
      if (cmd === 'close_discarding') { closed = args.quit ? 'quit' : 'window'; return null; }
      if (cmd === 'open_editor') { document.title = 'open_editor ' + JSON.stringify(args); return null; }
      if (cmd === 'save_config_data' || cmd === 'set_hpa_range') { document.title = cmd + ' ' + JSON.stringify(args); return null; }
      if (cmd === 'get_yaml') return 'kind: ' + kindName + '\n';
      if (cmd === 'cluster_overview' || cmd === 'stop_watch') return null;
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  const btn = (sel, text) => [...document.querySelectorAll(sel)].find((b) => b.textContent.trim().startsWith(text));
  const type = (el, v) => { if (!el) return; el.value = v; el.dispatchEvent(new Event('input', { bubbles: true })); };
  setTimeout(() => [...document.querySelectorAll('.kind-section-title')].find((b) => b.textContent.includes('Config'))?.click(), 600);
  setTimeout(() => btn('button.kind', kindName)?.click(), 1000);
  setTimeout(() => document.querySelector('tbody tr')?.click(), 1500);
  if (params.get('show')) setTimeout(() => btn('.data-secret button', 'Show')?.click(), 2100);
  if (params.get('edit')) setTimeout(() => btn('.dsection-actions button', 'Edit')?.click(), 2100);
  if (params.get('change')) setTimeout(() => type([...document.querySelectorAll('.data-value-input')][0], which === 'secret' ? 'tok_rotated_2026' : 'debug'), 2400);
  if (params.get('add')) setTimeout(() => { btn('.data-edit-foot button', 'Add key')?.click(); setTimeout(() => { const keys = document.querySelectorAll('.data-key-input'); type(keys[keys.length - 1], 'FEATURE_FLAGS'); const vals = document.querySelectorAll('.data-value-input'); type(vals[vals.length - 1], 'wishlist,giftcards'); }, 100); }, 2500);
  if (params.get('badkey')) setTimeout(() => { btn('.data-edit-foot button', 'Add key')?.click(); setTimeout(() => { const keys = document.querySelectorAll('.data-key-input'); type(keys[keys.length - 1], 'bad key/'); }, 100); }, 2500);
  if (params.get('remove')) setTimeout(() => btn('.data-entry.editing button', 'Remove')?.click(), 2700);
  if (params.get('save')) setTimeout(() => btn('.dsection-actions button', 'Save')?.click(), 2900);
  if (params.get('range')) setTimeout(() => btn('.detail-actions button', 'Edit range')?.click(), 2200);
  if (params.get('range')) setTimeout(() => { const i = document.querySelectorAll('.modal .replicas-input'); type(i[1], '14'); }, 2500);
  const collapse = params.get('collapse');
  if (collapse) setTimeout(() => collapse.split(',').forEach((c) => [...document.querySelectorAll('.dsection-name')].find((n) => n.textContent.toLowerCase() === c)?.closest('button').click()), 2300);
  if (prod && params.get('typed')) setTimeout(() => type([...document.querySelectorAll('.modal .confirm-input')].pop(), details.name), 3100);
  const scroll = params.get('scroll');
  if (scroll) setTimeout(() => { const b = document.querySelector('.detail-body'); if (b) b.scrollTop = Number(scroll); }, 3000);
  if (params.get('go')) setTimeout(() => document.querySelector('.modal-actions .primary')?.click(), 3400);
  if (params.get('yamledit')) {
    setTimeout(() => btn('.tabs button', 'YAML')?.click(), 2100);
    setTimeout(() => btn('.yaml-tools button', 'Edit')?.click(), 2600);
  }
  const leave = params.get('leave');
  const fire = (n, payload) => (listeners[n] || []).forEach((cb) => cb({ payload }));
  const pick = (v) => { const el = document.querySelector('.topbar select'); if (!el) return; el.value = v; el.dispatchEvent(new Event('change', { bubbles: true })); };
  const pickContext = (v) => { document.querySelector('.context-button')?.click(); [...document.querySelectorAll('.context-option')].find((o) => o.querySelector('.context-name')?.textContent === v)?.click(); };
  const leaves = {
    row: () => [...document.querySelectorAll('tbody tr')].find((r) => r.textContent.includes('other'))?.click(),
    close: () => document.querySelector('.detail .close')?.click(),
    yaml: () => btn('.tabs button', 'YAML')?.click(),
    kind: () => btn('button.kind', 'Lease')?.click(),
    refresh: () => btn('.tabs button', 'Refresh')?.click(),
    link: () => document.querySelector('.itable tbody tr.clickable')?.click(),
    ns: () => pick('acme'),
    context: () => pickContext(ctxName),
    cancel: () => btn('.dsection-actions button', 'Cancel')?.click(),
    window: () => fire('unsaved-close', false),
    quit: () => fire('unsaved-close', true),
  };
  if (leave) setTimeout(() => leaves[leave](), 3000);
  if (params.get('discard')) setTimeout(() => btn('.modal-actions button', 'Discard')?.click(), 3300);
  if (params.get('keep')) setTimeout(() => btn('.modal-actions button', 'Keep')?.click(), 3300);
  if (leave) setTimeout(() => {
    const sel = document.querySelector('tbody tr.selected')?.textContent || 'none';
    const prompt = !!document.querySelector('.modal h2')?.textContent.includes('Discard');
    const editing = !!document.querySelector('.data-value-input');
    const tab = document.querySelector('.tabs .tab.active')?.textContent || '-';
    const ns = document.querySelector('.topbar select')?.value;
    document.title = `unsaved=${unsaved} prompt=${prompt} editing=${editing} selected=${sel} tab=${tab} ns=${ns || 'all'} closed=${closed}`;
  }, 3800);
})();
