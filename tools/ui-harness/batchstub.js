// CronJob/Job Details tab, Run now and Suspend/Resume. Query: job=1 (a Job; default a CronJob), suspended=1, failed=1,
// op=run|suspend|resume, ctx=prod, typed=1, locked=1, go=1 (sent args land in the page title), openjob=1,
// collapse=resources,cronjob,..., scroll=<px>
(function () {
  const params = new URLSearchParams(location.search);
  const prod = params.get('ctx') === 'prod';
  const ctxName = prod ? 'acme-prd' : 'acme-uat';
  const isJob = params.get('job') === '1';
  const suspended = params.get('suspended') === '1';
  const failed = params.get('failed') === '1';
  let kindName = isJob ? 'Job' : 'CronJob';
  let unlocked = params.get('locked') !== '1';
  const listeners = {};
  const kinds = [
    { kind: { group: 'batch', version: 'v1', kind: 'CronJob', plural: 'cronjobs', namespaced: true }, verbs: ['list', 'watch', 'get', 'patch', 'delete'], category: 'Workloads' },
    { kind: { group: 'batch', version: 'v1', kind: 'Job', plural: 'jobs', namespaced: true }, verbs: ['list', 'watch', 'get', 'create', 'patch', 'delete'], category: 'Workloads' },
    { kind: { group: '', version: 'v1', kind: 'Pod', plural: 'pods', namespaced: true }, verbs: ['list', 'watch', 'get', 'delete'], category: 'Workloads' },
  ];
  const ago = (s) => new Date(Date.now() - s * 1000).toISOString();
  const usage = (r, l, u) => ({ requests: r, limits: l, used: u, node: null });
  const ctr = (name, state, detail) => ({ name, state, detail, init: false });
  const runs = [
    { name: 'nightly-manual-k3x9a', status: 'Running', completions: '0/1', manual: true, created: ago(95), started: ago(95), finished: null },
    { name: 'nightly-29860500', status: 'Complete', completions: '1/1', manual: false, created: ago(3 * 3600 + 120), started: ago(3 * 3600 + 120), finished: ago(3 * 3600) },
    { name: 'nightly-29859060', status: 'Failed', completions: '0/1', manual: false, created: ago(27 * 3600), started: ago(27 * 3600), finished: ago(26.5 * 3600) },
    { name: 'nightly-29857620', status: 'Complete', completions: '1/1', manual: false, created: ago(51 * 3600), started: ago(51 * 3600), finished: ago(51 * 3600 - 138) },
  ];
  const cronPods = [
    { name: 'nightly-manual-k3x9a-7tq2w', status: 'Running', containers: [ctr('sync', 'Running', 'Running')], ready: '1/1', restarts: 0, node: 'ip-10-0-1-5.ec2.internal', created: ago(95), updated: true, finished: false, cpu: 410, memory: 220e6 },
    { name: 'nightly-29860500-2c4gx', status: 'Completed', containers: [ctr('sync', 'Stopped', 'Completed')], ready: '0/1', restarts: 0, node: 'ip-10-0-1-6.ec2.internal', created: ago(3 * 3600 + 120), updated: true, finished: true, cpu: null, memory: null },
    { name: 'nightly-29859060-ldzdr', status: 'Error', containers: [ctr('sync', 'Failed', 'Error (exit 1)')], ready: '0/1', restarts: 0, node: 'ip-10-0-1-7.ec2.internal', created: ago(27 * 3600), updated: true, finished: true, cpu: null, memory: null },
  ];
  const jobPods = failed ? [cronPods[2]] : [cronPods[0]];
  const env = [{ name: 'SYNC_MODE', value: { Literal: 'full' }, from: null }, { name: 'API_TOKEN', value: { Secret: { name: 'nightly', key: 'token' } }, from: null }];
  const cronInfo = [
    ['Schedule', '0 3 * * * (America/New_York)'],
    ...(suspended ? [] : [['Next run', new Date(Date.now() + 4 * 3600e3 + 720e3).toISOString()]]),
    ['Last scheduled', ago(3 * 3600 + 120)], ['Last successful', ago(3 * 3600)],
    ['Concurrency', 'Forbid'], ['Starting deadline', '600s'], ['History kept', '3 successful · 1 failed'],
    ['Service account', 'nightly'], ['Restart policy', 'Never'],
  ];
  const jobInfo = failed
    ? [['Completions', '0/1 succeeded'], ['Failed pods', '1 · retries allowed: 0'], ['Started', ago(27 * 3600)], ['Finished', ago(26.5 * 3600)], ['Duration', 'took 30m 0s'], ['Active deadline', '3600s'], ['Service account', 'nightly'], ['Restart policy', 'Never'], ['Controlled by', 'CronJob/nightly']]
    : [['Completions', '0/1 succeeded'], ['Active', '1 pods · 1 ready'], ['Failed pods', '0 · retries allowed: 6'], ['Started', ago(95)], ['Duration', 'running for 1m 35s'], ['Service account', 'nightly'], ['Restart policy', 'Never'], ['Controlled by', 'CronJob/nightly']];
  const event = (type_, reason, message, s, object, source) => ({ type_, reason, message, count: 1, first_seen: ago(s), last_seen: ago(s), source, object });
  const cronEvents = [
    event('Normal', 'SuccessfulCreate', 'Created job nightly-manual-k3x9a', 95, 'Job nightly-manual-k3x9a', 'job-controller'),
    event('Normal', 'SawCompletedJob', 'Saw completed job: nightly-29860500, condition: Complete', 3 * 3600, null, 'cronjob-controller'),
    event('Warning', 'BackoffLimitExceeded', 'Job has reached the specified backoff limit', 26.5 * 3600, 'Job nightly-29859060', 'job-controller'),
  ];
  const jobName = failed ? 'nightly-29859060' : 'nightly-manual-k3x9a';
  const details = isJob ? {
    kind: 'Job', name: jobName, namespace: 'shop', status: failed ? 'Failed' : (suspended ? 'Suspended' : 'Running'), replicas: null, suspended,
    info: jobInfo, created: failed ? ago(27 * 3600) : ago(95), labels: [['app', 'nightly']],
    conditions: failed ? [
      { type_: 'FailureTarget', status: 'True', reason: 'BackoffLimitExceeded', message: 'Job has reached the specified backoff limit', last_transition: ago(26.5 * 3600) },
      { type_: 'Failed', status: 'True', reason: 'BackoffLimitExceeded', message: 'Job has reached the specified backoff limit', last_transition: ago(26.5 * 3600) },
    ] : [],
    cpu: usage(failed ? null : 250, failed ? null : 1000, failed ? 0 : 410), memory: usage(failed ? null : 268435456, failed ? null : 536870912, failed ? 0 : 220e6), metrics_available: true,
    pods: jobPods, jobs: [], events: cronEvents.filter((e) => e.object === 'Job ' + jobName).map((e) => ({ ...e, object: null })),
  } : {
    kind: 'CronJob', name: 'nightly', namespace: 'shop', status: suspended ? 'Suspended' : 'Running', replicas: null, suspended,
    info: cronInfo, created: '2026-09-01T10:00:00Z', labels: [['app', 'nightly'], ['team', 'data']], conditions: [],
    cpu: usage(250, 1000, 410), memory: usage(268435456, 536870912, 220e6), metrics_available: true,
    pods: cronPods, jobs: runs, events: cronEvents,
  };
  details.containers = [{ name: 'sync', init: false, sidecar: false, image: 'ghcr.io/acme/menu-sync:2.3.1', ports: [], cpu: usage(250, 1000, isJob && failed ? 0 : 410), memory: usage(268435456, 536870912, isJob && failed ? 0 : 220e6), env }];
  const end = Math.floor(Date.now() / 15000) * 15, start = end - 3600;
  // Short runs: usage only while a Job ran.
  const series = (fn) => { const out = []; for (let t = start; t <= end; t += 15) { const x = (t - start) / 3600; if ((x > 0.12 && x < 0.16) || x > 0.97) out.push([t, fn(x)]); } return out; };
  const hc = [{ name: 'sync', usage: { cpu: series((x) => 380 + 60 * Math.sin(x * 40)), memory: series((x) => 200e6 + 20e6 * Math.sin(x * 30)) } }];
  const history = { source: 'Prometheus', start, end, step: 15, pod: hc[0].usage, containers: hc };
  const rowsFor = (k) => k === 'CronJob'
    ? [{ id: 'shop/nightly', name: 'nightly', namespace: 'shop', cells: ['nightly', 'shop', '0 3 * * *'] }]
    : runs.map((r) => ({ id: 'shop/' + r.name, name: r.name, namespace: 'shop', cells: [r.name, 'shop', r.completions] }));
  const columns = [{ header: 'Name', kind: 'Name' }, { header: 'Namespace', kind: 'Namespace' }, { header: kindName === 'CronJob' ? 'Schedule' : 'Completions', kind: 'Text' }];
  const replies = {
    connect: { context: ctxName, kinds, namespaces: ['shop'] }, cluster_overview: null, stop_watch: null,
    get_yaml: 'apiVersion: batch/v1\nkind: ' + kindName + '\nmetadata:\n  name: nightly\n  namespace: shop\n',
    workload_details: details, workload_history: history,
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === 'watch_resources') {
        const k = args.kind.kind;
        if (k === 'Job' && !isJob) { kindName = 'Job'; document.title = 'opened jobs list'; }
        setTimeout(() => (listeners['resource-table'] || []).forEach((cb) => cb({ payload: { watch_id: 1, columns, rows: rowsFor(k), error: null } })), 50);
        return 1;
      }
      if (cmd === 'list_contexts') return { contexts: [{ name: ctxName, cluster: ctxName, namespace: null, prod_like: prod, unlocked }], current: ctxName };
      if (cmd === 'set_unlocked') { unlocked = true; return null; }
      if (cmd === 'run_cron_job' || cmd === 'suspend_workload') { document.title = cmd + ' ' + JSON.stringify(args); return cmd === 'run_cron_job' ? 'nightly-manual-zz9q1' : null; }
      if (cmd in replies) return replies[cmd];
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  const btn = (sel, text) => [...document.querySelectorAll(sel)].find((b) => b.textContent.trim().startsWith(text));
  setTimeout(() => document.querySelector('.kind-section-title')?.click(), 600);
  setTimeout(() => btn('button.kind', isJob ? 'Job' : 'CronJob')?.click(), 1000);
  setTimeout(() => (isJob ? [...document.querySelectorAll('tbody tr')].find((r) => r.textContent.includes(jobName)) : document.querySelector('tbody tr'))?.click(), 1500);
  const op = params.get('op');
  if (op) setTimeout(() => btn('.detail-actions button', { run: 'Run now', suspend: 'Suspend', resume: 'Resume' }[op])?.click(), 2200);
  const collapse = params.get('collapse');
  if (collapse) setTimeout(() => collapse.split(',').forEach((c) => [...document.querySelectorAll('.dsection-name')].find((n) => n.textContent.toLowerCase() === c)?.closest('button').click()), 2400);
  const scroll = params.get('scroll');
  if (scroll) setTimeout(() => { const b = document.querySelector('.detail-body'); if (b) b.scrollTop = Number(scroll); }, 2700);
  if (params.get('openjob')) setTimeout(() => document.querySelector('.wpods tbody tr:nth-child(2)')?.click(), 2600);
  if (prod && params.get('typed')) setTimeout(() => {
    const i = [...document.querySelectorAll('.modal .confirm-input')].pop();
    i.value = isJob ? jobName : 'nightly'; i.dispatchEvent(new Event('input', { bubbles: true }));
  }, 3000);
  if (params.get('go')) setTimeout(() => document.querySelector('.modal-actions .primary')?.click(), 3300);
})();
