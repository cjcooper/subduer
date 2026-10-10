// Access Control Details tabs and Check access. Query: kind=sa|role|binding (default sa), check=<resource> (types it and clicks Check;
// `secrets` is denied, anything else allowed; the sent args land in the page title), collapse=…, scroll=<px>
(function () {
  const params = new URLSearchParams(location.search);
  const which = params.get('kind') || 'sa';
  const listeners = {};
  const R = 'rbac.authorization.k8s.io';
  const K = (group, kind, plural, namespaced) => ({ kind: { group, version: 'v1', kind, plural, namespaced }, verbs: ['list', 'watch', 'get', 'patch', 'delete'], category: 'AccessControl' });
  const kinds = [K('', 'ServiceAccount', 'serviceaccounts', true), K(R, 'Role', 'roles', true), K(R, 'RoleBinding', 'rolebindings', true), K(R, 'ClusterRole', 'clusterroles', false), K(R, 'ClusterRoleBinding', 'clusterrolebindings', false),
    { kind: { group: '', version: 'v1', kind: 'Pod', plural: 'pods', namespaced: true }, verbs: ['list', 'watch', 'get'], category: 'Workloads' }];
  const kindName = { sa: 'ServiceAccount', role: 'ClusterRole', binding: 'RoleBinding' }[which];
  const row = (key, value, extra) => ({ key, value, mono: false, time: false, link: null, url: null, ...extra });
  const trow = (cells, extra) => ({ cells, link: null, fill: null, warn: false, ...extra });
  const L = (kind, name, namespace = null) => ({ kind, namespace, name });
  const T = (columns, rows, extra) => ({ columns, numeric: [], mono: [], rows, ...extra });
  const sec = (id, title, summary, table, empty) => ({ id, title, summary, rows: [], table, empty: empty || null });
  const base = (kind, name, ns) => ({ kind, name, namespace: ns, status: null, info: [], created: '2026-09-01T10:00:00Z', labels: [], annotations: [], conditions: [], data: null, sections: [], events: [] });
  const rules = [
    trow(['get, list', 'core', 'nodes, namespaces, endpoints', 'any', '']),
    trow(['get', 'core', 'nodes/metrics, nodes/stats, nodes/proxy', 'any', 'reaches the kubelet'], { warn: true }),
    trow(['get, list, watch', 'core', 'pods, events', 'any', '']),
    trow(['get', 'core', 'secrets', 'acme-api-tls', 'reads secrets'], { warn: true })];
  let d;
  if (which === 'sa') {
    d = base('ServiceAccount', 'acme-api', 'shop');
    d.status = 'Sensitive';
    d.info = [row('AWS IAM role', 'arn:aws:iam::123456789012:role/acme-api', { mono: true }), row('Token in pods', 'mounted automatically (unless a pod opts out)'), row('Acts as', 'system:serviceaccount:shop:acme-api', { mono: true }),
      row('Image pull secret', 'regcred', { link: L('core/Secret', 'regcred', 'shop') })];
    d.sections = [
      sec('sa-bindings', 'Bindings', '2 bindings', T(['Binding', 'Role', 'Where', 'Applies'], [
        trow(['acme-api-monitoring', 'ClusterRole acme-monitoring', 'cluster-wide', 'directly'], { link: L(R + '/ClusterRoleBinding', 'acme-api-monitoring') }),
        trow(['acme-api-tls-reader', 'Role tls-reader', 'in shop', 'directly'], { link: L(R + '/RoleBinding', 'acme-api-tls-reader', 'shop') })])),
      sec('sa-permissions', 'What it can do', '4 rules', T(['Verbs', 'Resources', 'Where', 'From', 'Note'], rules.map((r, i) => trow([r.cells[0], r.cells[2], i < 3 ? 'cluster-wide' : 'in shop', i < 3 ? 'ClusterRole acme-monitoring' : 'Role tls-reader', r.cells[4]], { warn: r.warn })), { mono: [0, 1] })),
      sec('sa-pods', 'Pods running as it', '2 pods', T(['Pod', 'Controlled by'], [trow(['acme-api-7d9f8c6b5-x2k4q', 'ReplicaSet/acme-api-7d9f8c6b5'], { link: L('core/Pod', 'acme-api-7d9f8c6b5-x2k4q', 'shop') }), trow(['acme-api-7d9f8c6b5-m8n3p', 'ReplicaSet/acme-api-7d9f8c6b5'], { link: L('core/Pod', 'acme-api-7d9f8c6b5-m8n3p', 'shop') })]))];
  } else if (which === 'role') {
    d = base('ClusterRole', 'acme-monitoring', null);
    d.status = 'Sensitive';
    d.info = [row('Rules', '4'), row('Adds its rules to', 'view')];
    d.sections = [sec('rules', 'Rules', '4', T(['Verbs', 'API groups', 'Resources', 'Names', 'Note'], rules, { mono: [0, 2] })),
      sec('bound-by', 'Bound by', '2', T(['Binding', 'Where', 'Subjects'], [trow(['acme-api-monitoring', '(cluster-wide)', 'ServiceAccount acme-api'], { link: L(R + '/ClusterRoleBinding', 'acme-api-monitoring') }), trow(['ops-view', 'ops', 'Group acme:ops'], { link: L(R + '/RoleBinding', 'ops-view', 'ops') })]))];
  } else {
    d = base('RoleBinding', 'everyone-view', 'shop');
    d.status = 'Everyone';
    d.info = [row('Grants', 'ClusterRole view', { link: L(R + '/ClusterRole', 'view') }), row('Where', "only in namespace shop (the ClusterRole's rules, limited to here)")];
    d.sections = [sec('granted', 'What it grants', '2', T(['Verbs', 'API groups', 'Resources', 'Names', 'Note'], [trow(['get, list, watch', 'core', 'pods, services, configmaps', 'any', '']), trow(['get, list, watch', 'apps', 'deployments, statefulsets', 'any', ''])], { mono: [0, 2] })),
      sec('subjects', 'Subjects', '2 subjects', T(['Kind', 'Name', 'Namespace'], [trow(['ServiceAccount', 'acme-api', 'shop'], { link: L('core/ServiceAccount', 'acme-api', 'shop') }), trow(['Group', 'system:authenticated', ''], { warn: true })]))];
  }
  const id = d.namespace ? d.namespace + '/' + d.name : d.name;
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === 'watch_resources') { setTimeout(() => (listeners['resource-table'] || []).forEach((cb) => cb({ payload: { watch_id: 1, columns: [{ header: 'Name', kind: 'Name' }, { header: 'Namespace', kind: 'Namespace' }], rows: [{ id, name: d.name, namespace: d.namespace, cells: [d.name, d.namespace || ''] }], error: null } })), 50); return 1; }
      if (cmd === 'list_contexts') return { contexts: [{ name: 'acme-uat', cluster: 'acme-uat', namespace: null, prod_like: false, unlocked: false }], current: 'acme-uat' };
      if (cmd === 'connect') return { context: 'acme-uat', kinds, namespaces: ['shop', 'ops'] };
      if (cmd === 'object_details') return d;
      if (cmd === 'check_access') { document.title = 'check_access ' + JSON.stringify(args); return args.resource === 'secrets' ? { allowed: false, reason: '' } : { allowed: true, reason: 'RBAC: allowed by ClusterRoleBinding "acme-api-monitoring" of ClusterRole "acme-monitoring" to ServiceAccount "acme-api/shop"' }; }
      if (['set_unsaved', 'cluster_overview', 'stop_watch'].includes(cmd)) return null;
      if (cmd === 'get_yaml') return 'kind: ' + kindName + '\n';
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  const type = (el, v) => { if (!el) return; el.value = v; el.dispatchEvent(new Event('input', { bubbles: true })); };
  setTimeout(() => [...document.querySelectorAll('.kind-section-title')].find((b) => /access/i.test(b.textContent))?.click(), 600);
  setTimeout(() => [...document.querySelectorAll('button.kind')].find((b) => b.textContent.trim() === kindName)?.click(), 1000);
  setTimeout(() => document.querySelector('tbody tr')?.click(), 1500);
  const check = params.get('check');
  if (check) setTimeout(() => { type(document.querySelectorAll('.access-field input')[0], check); setTimeout(() => [...document.querySelectorAll('.access-form button')][0]?.click(), 100); }, 2300);
  const collapse = params.get('collapse');
  if (collapse) setTimeout(() => collapse.split(',').forEach((c) => [...document.querySelectorAll('.dsection-name')].find((n) => n.textContent.toLowerCase() === c)?.closest('button').click()), 2200);
  const scroll = params.get('scroll');
  if (scroll) setTimeout(() => { const b = document.querySelector('.detail-body'); if (b) b.scrollTop = Number(scroll); }, 2700);
})();
