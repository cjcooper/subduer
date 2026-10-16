// Storage-section Details tabs, Expand and Reclaim policy. Query: kind=pvc|pv|sc|va (default pvc), pending=1 (an unbound claim),
// op=expand|reclaim, size=<new size>, ctx=prod, typed=1, go=1 (sent args land in the page title), collapse=…, scroll=<px>
(function () {
  const params = new URLSearchParams(location.search);
  const which = params.get('kind') || 'pvc';
  const prod = params.get('ctx') === 'prod';
  const ctxName = prod ? 'acme-prd' : 'acme-uat';
  const pending = params.get('pending') === '1';
  const listeners = {};
  const K = (group, kind, plural, namespaced) => ({ kind: { group, version: 'v1', kind, plural, namespaced }, verbs: ['list', 'watch', 'get', 'patch', 'delete'], category: 'Storage' });
  const kinds = [K('', 'PersistentVolumeClaim', 'persistentvolumeclaims', true), K('', 'PersistentVolume', 'persistentvolumes', false), K('storage.k8s.io', 'StorageClass', 'storageclasses', false), K('storage.k8s.io', 'VolumeAttachment', 'volumeattachments', false),
    { kind: { group: '', version: 'v1', kind: 'Pod', plural: 'pods', namespaced: true }, verbs: ['list', 'watch', 'get'], category: 'Workloads' }];
  const kindName = { pvc: 'PersistentVolumeClaim', pv: 'PersistentVolume', sc: 'StorageClass', va: 'VolumeAttachment' }[which];
  const row = (key, value, extra) => ({ key, value, mono: false, time: false, link: null, url: null, ...extra });
  const trow = (cells, extra) => ({ cells, link: null, fill: null, warn: false, ...extra });
  const L = (kind, name, namespace = null) => ({ kind, namespace, name });
  const ago = (s) => new Date(Date.now() - s * 1000).toISOString();
  const sec = (id, title, summary, table, rows) => ({ id, title, summary, rows: rows || [], table, empty: null });
  const base = (kind, name, ns) => ({ kind, name, namespace: ns, status: null, info: [], created: '2026-09-01T10:00:00Z', labels: [['app', 'acme-db']], annotations: [], conditions: [], data: null, sections: [], events: [] });
  let d;
  if (which === 'pvc') {
    d = base('PersistentVolumeClaim', 'data-acme-db-0', 'shop');
    if (pending) {
      d.status = 'Pending';
      d.info = [row('Requested', '20Gi'), row('Access', 'ReadWriteOnce (one node)'), row('Storage class', 'gp3', { link: L('storage.k8s.io/StorageClass', 'gp3') }), row('Can expand', 'yes'), row('Why pending', 'its class creates the volume when a pod first uses the claim')];
      d.sections = [sec('mounted-by', 'Mounted by', 'none', { columns: ['Pod', 'Status', 'Node'], numeric: [], mono: [], rows: [] })];
      d.sections[0].empty = 'No pods use it right now.';
      d.events = [{ type_: 'Normal', reason: 'WaitForFirstConsumer', message: 'waiting for first consumer to be created before binding', count: 4, first_seen: ago(600), last_seen: ago(30), source: 'persistentvolume-controller', object: null }];
    } else {
      d.status = 'Bound';
      d.info = [row('Requested', '20Gi'), row('Capacity', '20Gi'), row('Access', 'ReadWriteOnce (one node)'), row('Storage class', 'gp3', { link: L('storage.k8s.io/StorageClass', 'gp3') }), row('Volume', 'pvc-3f1c9a2e-77b0-4c1e-9d0f-1a2b3c4d5e6f', { link: L('core/PersistentVolume', 'pvc-3f1c9a2e-77b0-4c1e-9d0f-1a2b3c4d5e6f') }), row('Can expand', 'yes'), row('Controlled by', 'StatefulSet/acme-db')];
      d.sections = [
        sec('disk', 'Disk usage', '87% used', { columns: ['', 'Used', 'Free', 'Size'], numeric: [1, 2, 3], mono: [], rows: [trow(['Space', '16.9 GiB', '2.6 GiB', '19.5 GiB'], { fill: 0.87, warn: true }), trow(['Files (inodes)', '48211', '1261869', '1310720'], { fill: 0.037 })] }),
        sec('mounted-by', 'Mounted by', '1 pod', { columns: ['Pod', 'Status', 'Node'], numeric: [], mono: [], rows: [trow(['acme-db-0', 'Running', 'ip-10-0-1-5.ec2.internal'], { link: L('core/Pod', 'acme-db-0', 'shop') })] })];
    }
  } else if (which === 'pv') {
    d = base('PersistentVolume', 'pvc-3f1c9a2e-77b0-4c1e-9d0f-1a2b3c4d5e6f', null);
    d.status = 'Bound';
    d.info = [row('Capacity', '20Gi'), row('Access', 'ReadWriteOnce (one node)'), row('Reclaim policy', 'Delete: deleting the claim deletes the volume and its data'), row('Storage class', 'gp3', { link: L('storage.k8s.io/StorageClass', 'gp3') }),
      row('Claim', 'shop/data-acme-db-0', { link: L('core/PersistentVolumeClaim', 'data-acme-db-0', 'shop') }), row('Source', 'CSI ebs.csi.aws.com'), row('Volume ID', 'vol-0a1b2c3d4e5f60718', { mono: true }), row('Nodes', 'topology.kubernetes.io/zone in (us-east-1a)', { mono: true }), row('Phase since', ago(86400 * 30), { time: true })];
  } else if (which === 'sc') {
    d = base('StorageClass', 'gp3', null);
    d.status = 'Default';
    d.info = [row('Provisioner', 'ebs.csi.aws.com', { mono: true }), row('Default class', 'yes: claims without a class use it'), row('Reclaim policy', 'Delete: deleting the claim deletes the volume and its data'), row('Binding', 'WaitForFirstConsumer: created when a pod first uses the claim, in that pod\'s zone'), row('Expansion', 'allowed')];
    d.sections = [sec('parameters', 'Parameters', '2', { columns: ['Parameter', 'Value'], numeric: [], mono: [0, 1], rows: [trow(['type', 'gp3']), trow(['encrypted', 'true'])] }),
      sec('class-claims', 'Claims', '3', { columns: ['Namespace', 'Claim', 'Status', 'Size'], numeric: [3], mono: [], rows: [
        trow(['shop', 'data-acme-db-0', 'Bound', '20Gi'], { link: L('core/PersistentVolumeClaim', 'data-acme-db-0', 'shop') }), trow(['shop', 'data-acme-db-1', 'Bound', '20Gi'], { link: L('core/PersistentVolumeClaim', 'data-acme-db-1', 'shop') }), trow(['ops', 'scratch', 'Pending', ''], { link: L('core/PersistentVolumeClaim', 'scratch', 'ops'), warn: true })] })];
  } else {
    d = base('VolumeAttachment', 'csi-5b3e9f0c1d2a', null);
    d.status = 'Error';
    d.info = [row('Attacher', 'ebs.csi.aws.com', { mono: true }), row('Node', 'ip-10-0-1-6.ec2.internal', { link: L('core/Node', 'ip-10-0-1-6.ec2.internal') }), row('Volume', 'pvc-3f1c9a2e-77b0-4c1e-9d0f-1a2b3c4d5e6f', { link: L('core/PersistentVolume', 'pvc-3f1c9a2e-77b0-4c1e-9d0f-1a2b3c4d5e6f') }), row('Attached', 'no'),
      row('Attach error', 'rpc error: code = Internal desc = Could not attach volume "vol-0a1b2c3d4e5f60718" to node: VolumeInUse'), row('Attach error at', ago(240), { time: true })];
  }
  const id = d.namespace ? d.namespace + '/' + d.name : d.name;
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === 'watch_resources') { setTimeout(() => (listeners['resource-table'] || []).forEach((cb) => cb({ payload: { watch_id: 1, columns: [{ header: 'Name', kind: 'Name' }, { header: 'Namespace', kind: 'Namespace' }], rows: [{ id, name: d.name, namespace: d.namespace, cells: [d.name, d.namespace || ''] }], error: null } })), 50); return 1; }
      if (cmd === 'list_contexts') return { contexts: [{ name: ctxName, cluster: ctxName, namespace: null, prod_like: prod, unlocked: true }], current: ctxName };
      if (cmd === 'connect') return { context: ctxName, kinds, namespaces: ['shop', 'ops'] };
      if (cmd === 'object_details') return d;
      if (cmd === 'expand_claim' || cmd === 'set_reclaim_policy') { document.title = cmd + ' ' + JSON.stringify(args); return null; }
      if (['set_unsaved', 'cluster_overview', 'stop_watch'].includes(cmd)) return null;
      if (cmd === 'get_yaml') return 'kind: ' + kindName + '\n';
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  const btn = (sel, text) => [...document.querySelectorAll(sel)].find((b) => b.textContent.trim().startsWith(text));
  const type = (el, v) => { if (!el) return; el.value = v; el.dispatchEvent(new Event('input', { bubbles: true })); };
  setTimeout(() => [...document.querySelectorAll('.kind-section-title')].find((b) => b.textContent.includes('Storage'))?.click(), 600);
  setTimeout(() => [...document.querySelectorAll('button.kind')].find((b) => b.textContent.trim() === kindName)?.click(), 1000);
  setTimeout(() => document.querySelector('tbody tr')?.click(), 1500);
  const op = params.get('op');
  if (op) setTimeout(() => btn('.detail-actions button', op === 'expand' ? 'Expand' : 'Reclaim')?.click(), 2300);
  if (params.get('size')) setTimeout(() => type(document.querySelector('.modal .size-input'), params.get('size')), 2600);
  if (prod && params.get('typed')) setTimeout(() => type([...document.querySelectorAll('.modal .confirm-input')].pop(), d.name), 2800);
  if (params.get('go')) setTimeout(() => document.querySelector('.modal-actions .primary')?.click(), 3100);
  const SORT_SEL = '.itable', SORT_AT = 2300;
  // sort=<heading>&clicks=<n>: clicks a table heading n times (default 1); the rows' first cells land in the title.
  const sortBy = params.get('sort');
  if (sortBy) {
    const n = Number(params.get('clicks') || 1);
    for (let i = 0; i < n; i++) setTimeout(() => [...document.querySelectorAll(SORT_SEL + ' th .th-sort')].find((b) => b.textContent.replace(/[↑↓]/g, '').trim() === sortBy)?.click(), SORT_AT + i * 200);
    setTimeout(() => { document.title = 'rows ' + [...document.querySelectorAll(SORT_SEL + ' tbody tr')].map((tr) => tr.querySelector('td').textContent.trim()).join(','); }, SORT_AT + n * 200 + 200);
  }
  const collapse = params.get('collapse');
  if (collapse) setTimeout(() => collapse.split(',').forEach((c) => [...document.querySelectorAll('.dsection-name')].find((n) => n.textContent.toLowerCase() === c)?.closest('button').click()), 2300);
  const scroll = params.get('scroll');
  if (scroll) setTimeout(() => { const b = document.querySelector('.detail-body'); if (b) b.scrollTop = Number(scroll); }, 2600);
})();
