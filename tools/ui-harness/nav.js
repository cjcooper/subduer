(function () {
  const params = new URLSearchParams(location.search);
  const K = (group, kind, category, namespaced = true, verbs = ['list', 'watch', 'get', 'delete']) =>
    ({ kind: { group, version: 'v1', kind, plural: kind.toLowerCase() + 's', namespaced }, verbs, category });
  const kinds = [
    K('', 'Node', 'TopLevel', false), K('', 'Event', 'TopLevel'),
    K('', 'Pod', 'Workloads'), K('apps', 'Deployment', 'Workloads'), K('apps', 'StatefulSet', 'Workloads'), K('apps', 'DaemonSet', 'Workloads'),
    K('apps', 'ReplicaSet', 'Workloads'), K('batch', 'Job', 'Workloads'), K('batch', 'CronJob', 'Workloads'),
    K('', 'ConfigMap', 'Config'), K('', 'Secret', 'Config'), K('autoscaling', 'HorizontalPodAutoscaler', 'Config'),
    K('policy', 'PodDisruptionBudget', 'Config'), K('', 'ResourceQuota', 'Config'), K('', 'LimitRange', 'Config'),
    K('scheduling.k8s.io', 'PriorityClass', 'Config', false), K('coordination.k8s.io', 'Lease', 'Config'),
    K('admissionregistration.k8s.io', 'MutatingWebhookConfiguration', 'Config', false),
    K('', 'Service', 'Network'), K('networking.k8s.io', 'Ingress', 'Network'), K('networking.k8s.io', 'IngressClass', 'Network', false),
    K('networking.k8s.io', 'NetworkPolicy', 'Network'), K('discovery.k8s.io', 'EndpointSlice', 'Network'),
    K('', 'PersistentVolumeClaim', 'Storage'), K('', 'PersistentVolume', 'Storage', false), K('storage.k8s.io', 'StorageClass', 'Storage', false),
    K('', 'Namespace', 'Namespaces', false),
    K('', 'ServiceAccount', 'AccessControl'), K('rbac.authorization.k8s.io', 'Role', 'AccessControl'), K('rbac.authorization.k8s.io', 'RoleBinding', 'AccessControl'),
    K('datadoghq.com', 'DatadogDashboard', 'CustomResources'),
    K('datadoghq.com', 'DatadogGenericResource', 'CustomResources'),
    K('datadoghq.com', 'DatadogMonitor', 'CustomResources'),
    K('datadoghq.com', 'DatadogSLO', 'CustomResources'),
    K('elbv2.k8s.aws', 'ALBTargetControlConfig', 'Aws'),
    K('elbv2.k8s.aws', 'IngressClassParams', 'Aws'),
    K('elbv2.k8s.aws', 'TargetGroupBinding', 'Aws'),
    K('eventing.keda.sh', 'CloudEventSource', 'CustomResources'),
    K('eventing.keda.sh', 'ClusterCloudEventSource', 'CustomResources'),
    K('external-secrets.io', 'ClusterExternalSecret', 'CustomResources'),
    K('external-secrets.io', 'ClusterPushSecret', 'CustomResources'),
    K('external-secrets.io', 'ClusterSecretStore', 'CustomResources'),
    K('external-secrets.io', 'ExternalSecret', 'CustomResources'),
    K('external-secrets.io', 'PushSecret', 'CustomResources'),
    K('external-secrets.io', 'SecretStore', 'CustomResources'),
    K('generators.external-secrets.io', 'ECRAuthorizationToken', 'CustomResources'),
    K('generators.external-secrets.io', 'Password', 'CustomResources'),
    K('generators.external-secrets.io', 'SSHKey', 'CustomResources'),
    K('keda.sh', 'ClusterTriggerAuthentication', 'CustomResources'),
    K('keda.sh', 'ScaledJob', 'CustomResources'),
    K('keda.sh', 'ScaledObject', 'CustomResources'),
    K('keda.sh', 'TriggerAuthentication', 'CustomResources'),
    K('kyverno.io', 'ClusterCleanupPolicy', 'CustomResources'),
    K('kyverno.io', 'ClusterPolicy', 'CustomResources'),
    K('kyverno.io', 'Policy', 'CustomResources'),
    K('kyverno.io', 'PolicyException', 'CustomResources'),
    K('kyverno.io', 'UpdateRequest', 'CustomResources'),
    K('networking.k8s.aws', 'ClusterNetworkPolicy', 'Aws'),
    K('networking.k8s.aws', 'PolicyEndpoint', 'Aws'),
    K('vpcresources.k8s.aws', 'CNINode', 'Aws'),
    K('vpcresources.k8s.aws', 'SecurityGroupPolicy', 'Aws'),
    K('wgpolicyk8s.io', 'ClusterPolicyReport', 'CustomResources'),
    K('wgpolicyk8s.io', 'PolicyReport', 'CustomResources'),
    K('monitoring.coreos.com', 'Alertmanager', 'CustomResources'),
    K('monitoring.coreos.com', 'AlertmanagerConfig', 'CustomResources'),
    K('monitoring.coreos.com', 'PodMonitor', 'CustomResources'),
    K('monitoring.coreos.com', 'Probe', 'CustomResources'),
    K('monitoring.coreos.com', 'Prometheus', 'CustomResources'),
    K('monitoring.coreos.com', 'PrometheusAgent', 'CustomResources'),
    K('monitoring.coreos.com', 'PrometheusRule', 'CustomResources'),
    K('monitoring.coreos.com', 'ScrapeConfig', 'CustomResources'),
    K('monitoring.coreos.com', 'ServiceMonitor', 'CustomResources'),
    K('monitoring.coreos.com', 'ThanosRuler', 'CustomResources'),
    K('networking.gke.io', 'ManagedCertificate', 'GoogleCloud'), K('cloud.google.com', 'BackendConfig', 'GoogleCloud'), K('monitoring.googleapis.com', 'PodMonitoring', 'GoogleCloud'),
    K('resources.azure.com', 'ResourceGroup', 'Azure'), K('containerservice.azure.com', 'ManagedCluster', 'Azure'),
    K('certificates.k8s.io', 'CertificateSigningRequest', 'Other', false),
    { kind: { group: 'helm.sh', version: 'v3', kind: 'Release', plural: 'releases', namespaced: true }, verbs: ['list', 'watch', 'get'], category: 'Helm' },
  ];
  const listeners = {};
  const ago = (h) => new Date(Date.now() - h * 3600e3).toISOString().replace(/\.\d+Z$/, 'Z');
  const releases = [
    ['datadog', 'datadog', 'datadog-3.170.1', '7', '8', 'deployed', ago(400)],
    ['keda', 'keda', 'keda-2.19.0', '2.19.0', '1', 'deployed', ago(900)],
    ['corporate-wallet', 'shop', 'corporate-wallet-1.20261009.3', '1.20261009.3', '1469', 'deployed', ago(1)],
    ['migrate', 'shop', 'migrate-0.4.1', '0.4.1', '12', 'failed', ago(3)],
    ['canary', 'shop', 'canary-1.2.0', '1.2.0', '54', 'pending-upgrade', ago(0.1)],
  ];
  const table = {
    watch_id: 1,
    columns: [['Name', 'Name'], ['Namespace', 'Namespace'], ['Chart', 'Text'], ['App Version', 'Text'], ['Revision', 'Number'], ['Status', 'Badge'], ['Updated', 'Age']].map(([header, kind]) => ({ header, kind })),
    rows: releases.map((c) => ({ id: c[1] + '/' + c[0], name: c[0], namespace: c[1], cells: c })), error: null,
  };
  const yaml = "# Helm release summary. Values and manifest are not shown.\nrelease: keda\nnamespace: keda\nrevision: 1\nstatus: deployed\nchart: keda\nchartVersion: 2.19.0\nappVersion: 2.19.0\ndescription: Install complete\nhistory:\n- revision: 1\n  status: deployed\n  updated: 2026-02-18T23:31:47Z\n";
  const replies = {
    list_contexts: { contexts: [{ name: 'acme-uat', cluster: 'acme-uat', namespace: null, prod_like: false, unlocked: false }], current: 'acme-uat' },
    connect: { context: 'acme-uat', kinds, namespaces: ['shop'] }, cluster_overview: null, stop_watch: null, get_yaml: yaml,
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd) => {
      if (cmd === 'watch_resources') { setTimeout(() => (listeners['resource-table'] || []).forEach((cb) => cb({ payload: table })), 50); return 1; }
      if (cmd in replies) return replies[cmd];
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (name, cb) => { (listeners[name] = listeners[name] || []).push(cb); return () => {}; } },
  };
  const open = params.get('open') || 'all';
  setTimeout(() => {
    document.querySelectorAll('.kind-section-title').forEach((b, i) => { if (open === 'all' || (open === 'cr' && /Custom/.test(b.textContent)) || (open === 'some' && i < 2)) b.click(); });
  }, 700);
  if (open !== 'cr') setTimeout(() => { [...document.querySelectorAll('button.kind')].find((b) => b.textContent.includes('Release'))?.click(); }, 1100);
  if (!params.get('sort')) setTimeout(() => { document.querySelectorAll('tbody tr')[1]?.click(); }, 1700);
  const SORT_SEL = 'table.resources', SORT_AT = 1700;
  // sort=<heading>&clicks=<n>: clicks a table heading n times (default 1); the rows' first cells land in the title.
  const sortBy = params.get('sort');
  if (sortBy) {
    const n = Number(params.get('clicks') || 1);
    for (let i = 0; i < n; i++) setTimeout(() => [...document.querySelectorAll(SORT_SEL + ' th .th-sort')].find((b) => b.textContent.replace(/[↑↓]/g, '').trim() === sortBy)?.click(), SORT_AT + i * 200);
    setTimeout(() => { document.title = 'rows ' + [...document.querySelectorAll(SORT_SEL + ' tbody tr')].map((tr) => tr.querySelector('td').textContent.trim()).join(','); }, SORT_AT + n * 200 + 200);
  }

})();
