// Traces: the top-bar button, the start dialog, the running indicator and its panel, Stop, and the prompts on
// quit and context switch. Query:
//   open=1 (clicks Trace…), level=cluster|ns|workloads|pod, ns=<namespace> (the picker), pick=<name> (ticks a
//   namespace, workload or pod), go=1 (Save as and start), cancel=1 (the Save dialog is cancelled),
//   running=1 (a trace is already running), gaps=<n>, err=1 (the last pull failed), noprom=1, panel=1 (opens it),
//   stop=1 (Stop trace), quit=1|close=1 (the backend held quitting / closing),
//   switch=1 (picks the other context), confirm=1 (the prompt's stop button), keep=1 (Keep tracing),
//   this=1 (selects a pod and clicks Trace… in its panel), menu=1 (the Trace… menu item),
//   x=1 (the dialog's × button), esc=1 (presses Escape).
// The title reports the trace commands sent (with args) and which dialogs are open.
(function () {
  const params = new URLSearchParams(location.search);
  const listeners = {};
  const sent = [];
  let watches = 0;
  const now = Date.now() / 1000;
  const status = (extra) => ({
    context: 'acme-uat', path: '/Users/me/Documents/acme-uat-shop-2026-10-11-1405.trace',
    scope: { Namespaces: ['shop'] }, started: now - 2 * 3600 - 14 * 60, data_start: now - 4 * 3600, data_end: now - 120,
    prometheus: !params.get('noprom'), last_pull: params.get('noprom') ? null : now - 180,
    last_error: params.get('err') ? "Prometheus didn't answer" : null, gaps: Number(params.get('gaps') || 0),
    sampled_pods: 1, stopping: false, ...extra,
  });
  let running = params.get('running') ? status({}) : null;
  const kinds = [{ kind: { group: '', version: 'v1', kind: 'Pod', plural: 'pods', namespaced: true }, verbs: ['list', 'watch', 'get'], category: 'Workloads' }];
  const emit = (name, payload) => (listeners[name] || []).forEach((cb) => cb({ payload }));
  const contexts = [
    { name: 'acme-uat', cluster: 'acme-uat', namespace: null, prod_like: false, unlocked: false },
    { name: 'acme-prd', cluster: 'acme-prd', namespace: null, prod_like: true, unlocked: false },
  ];
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd.includes('trace') || cmd === 'close_discarding' || cmd === 'connect') sent.push(`${cmd} ${JSON.stringify(args || {})}`);
      if (cmd === 'list_contexts') return { contexts, current: 'acme-uat' };
      if (cmd === 'connect') return { context: args.context, kinds, namespaces: ['ops', 'shop'] };
      if (cmd === 'watch_resources') {
        watches += 1;
        const table = { watch_id: watches, columns: [{ header: 'Name', kind: 'Name' }, { header: 'Namespace', kind: 'Namespace' }],
          rows: [{ id: 'shop/web-7d9f-abcde', name: 'web-7d9f-abcde', namespace: 'shop', cells: ['web-7d9f-abcde', 'shop'] }], error: null, stopped: false };
        setTimeout(() => emit('resource-table', table), 50);
        return watches;
      }
      if (cmd === 'trace_status') return running;
      if (cmd === 'trace_targets') {
        const kind = (k, g, p) => ({ group: g, version: 'v1', kind: k, plural: p, namespaced: true });
        return {
          workloads: [
            { kind: kind('Deployment', 'apps', 'deployments'), namespace: args.namespace, name: 'web' },
            { kind: kind('StatefulSet', 'apps', 'statefulsets'), namespace: args.namespace, name: 'db' },
            { kind: kind('CronJob', 'batch', 'cronjobs'), namespace: args.namespace, name: 'nightly' },
          ],
          pods: ['db-0', 'web-7d9f-abcde'],
        };
      }
      if (cmd === 'choose_trace_file') return params.get('cancel') ? null : '/Users/me/Documents/' + args.suggested;
      if (cmd === 'start_trace') {
        running = status({ scope: args.scope, path: args.path, started: now, data_start: now - 6900, data_end: now });
        return running;
      }
      if (cmd === 'stop_trace' || cmd === 'close_discarding') {
        const s = running;
        running = null;
        return cmd === 'close_discarding' ? null : {
          path: s.path, context: s.context, scope: s.scope, started: s.started, ended: now, end_reason: args.reason,
          unfinished: false, data_start: s.data_start, data_end: now, prometheus: 'monitoring/prometheus', retention_secs: 7200,
          pods: 21, containers: 29, prometheus_samples: 6000, local_samples: 40, gaps: [], file_bytes: 150000,
        };
      }
      if (['set_unsaved', 'stop_watch', 'set_app_theme', 'reveal_trace'].includes(cmd)) return null;
      if (cmd === 'list_forwards') return [];
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  const button = (text, scope = document) => [...scope.querySelectorAll('button')].find((b) => b.textContent.trim() === text);
  const modalButton = (text) => button(text, document.querySelector('.modal') || document);
  const at = (ms, f) => setTimeout(f, ms);
  let t = 800;
  const step = (f) => at((t += 400), f);
  if (params.get('this')) {
    step(() => document.querySelector('.kind-section-title')?.click());
    step(() => [...document.querySelectorAll('button.kind')].find((b) => /Pod/.test(b.textContent))?.click());
    step(() => document.querySelector('tbody tr')?.click());
    step(() => button('Trace…', document.querySelector('.detail-actions'))?.click());
  }
  if (params.get('menu')) step(() => emit('open-trace', null));
  if (params.get('open')) step(() => button('Trace…', document.querySelector('.topbar'))?.click());
  const level = params.get('level');
  if (level) {
    const label = { cluster: 'Whole cluster', ns: 'Namespaces', workloads: 'Workloads', pod: 'Pod' }[level];
    step(() => [...document.querySelectorAll('.trace-level')].find((l) => l.textContent.includes(label))?.querySelector('input')?.click());
  }
  const ns = params.get('ns');
  if (ns) {
    step(() => {
      const select = document.querySelector('.trace-ns select');
      if (select) { select.value = ns; select.dispatchEvent(new Event('change', { bubbles: true })); }
    });
  }
  const pick = params.get('pick');
  if (pick) step(() => [...document.querySelectorAll('.trace-option')].find((o) => o.textContent.includes(pick))?.querySelector('input')?.click());
  if (params.get('go')) step(() => modalButton('Save as and start…')?.click());
  if (params.get('panel') || params.get('stop')) step(() => document.querySelector('.trace-indicator')?.click());
  if (params.get('stop')) step(() => modalButton('Stop trace')?.click());
  if (params.get('quit') || params.get('close')) step(() => emit('unsaved-close', !!params.get('quit')));
  if (params.get('switch')) {
    step(() => {
      const select = document.querySelector('.topbar select');
      if (select) { select.value = 'acme-prd'; select.dispatchEvent(new Event('change', { bubbles: true })); }
    });
  }
  if (params.get('confirm')) step(() => document.querySelector('.modal .danger')?.click());
  if (params.get('keep')) step(() => modalButton('Keep tracing')?.click());
  if (params.get('x')) step(() => document.querySelector('.modal-head .close')?.click());
  if (params.get('esc')) step(() => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' })));
  at(t + 1200, () => {
    const dialogs = [...document.querySelectorAll('.modal h2')].map((h) => h.textContent.trim()).join('|');
    const ind = document.querySelector('.trace-indicator');
    const indicator = ind ? ind.textContent.trim() + (ind.querySelector('.trace-warn') ? ' (warn)' : '') : 'none';
    document.title = `sent=${sent.filter((s) => !s.startsWith('connect')).join(' ; ')} dialogs=${dialogs || 'none'} indicator=${indicator}`
      + ` connects=${sent.filter((s) => s.startsWith('connect')).length}`;
  });
})();
