// The Traces page and a trace's review: recent files, Open trace…, the right-sizing table and a row's charts.
// Query: recent=1 (two recent files saved before start), running=1 (a trace is recording), page=1 (opens the
// Traces page), open=1 (Open trace… picks a file), cancel=1 (the Open dialog is cancelled), file=1 (clicks the
// first recent file), sel=<name> (selects that row), sort=<header text> (clicks that header), flagged=1 (Needs
// attention only), ns=<namespace>, text=<filter>, quick=1h|6h|24h|all, zoom=1 (drags across the CPU chart),
// heat=mem, back=1 (← Traces), bad=1 (the file isn't a trace), offline=1 (connecting fails).
// The title reports the commands sent, the table's rows in order, the selected row and what's shown.
(function () {
  const params = new URLSearchParams(location.search);
  const listeners = {};
  const sent = [];
  const H = 3600, now = Math.floor(Date.now() / 1000 / 60) * 60, start = now - 3 * 24 * H;
  try {
    localStorage.clear();
    if (params.get('recent')) {
      localStorage.setItem('traces.recent', JSON.stringify([
        { path: '/Users/me/Documents/acme-uat-shop-2026-10-08-0900.trace', context: 'acme-uat', what: 'namespace shop', at: Date.now() - 86400000 },
        { path: '/Users/me/Documents/acme-uat-web-2026-10-01-1200.trace', context: 'acme-uat', what: 'Deployment web', at: Date.now() - 9 * 86400000 },
      ]));
    }
  } catch (e) {}
  const MiB = 1048576;
  const key = (name, kind, container, ns = 'shop') => ({ namespace: ns, kind, name, container });
  const row = (k, o) => Object.assign({
    key: k, pods: 2, avg_pods: 2, minutes: 4320, pod_minutes: 8640, rollouts: 0, local_share: 0,
    cpu_request: null, cpu_limit: null, mem_request: null, mem_limit: null,
    cpu_p50: 0, cpu_p95: 0, cpu_p99: 0, cpu_max: 0, mem_p95: 0, mem_max: 0, cpu_spare: null, mem_spare: null, flags: [], suggestion: null,
  }, o);
  const rows = [
    row(key('web', 'Deployment', 'app'), {
      pods: 4, avg_pods: 3, rollouts: 1, cpu_request: 1500, mem_request: 6144 * MiB, mem_limit: 6144 * MiB,
      cpu_p50: 8, cpu_p95: 11.8, cpu_p99: 16, cpu_max: 21, mem_p95: 2400 * MiB, mem_max: 2557 * MiB,
      cpu_spare: (1500 - 11.8) * 3, mem_spare: (6144 - 2557) * MiB * 3, flags: ['RequestsChanged'],
      suggestion: { cpu_request: 15, mem_request: 2944 * MiB, basis: 'p95 CPU and peak memory × 1.15 over 3d (8,640 pod-minutes), 1 rollout, no OOM kills' },
    }),
    row(key('db', 'StatefulSet', 'db'), {
      pods: 1, avg_pods: 1, cpu_limit: 1000, mem_limit: 1000 * MiB, cpu_p50: 420, cpu_p95: 610, cpu_p99: 800, cpu_max: 990,
      mem_p95: 930 * MiB, mem_max: 950 * MiB,
      flags: [{ Throttled: { worst: 0.38 } }, { NearMemoryLimit: { share: 0.95 } }, { OomKilled: { count: 2 } }, 'NoCpuRequest', 'NoMemoryRequest', 'AboveMemoryLimit'],
      suggestion: { cpu_request: 610, mem_request: 1104 * MiB, basis: 'p95 CPU and peak memory × 1.15 over 3d (4,320 pod-minutes), no rollouts, 2 OOM kills' },
    }),
    row(key('worker', 'Deployment', 'worker'), {
      cpu_request: 700, mem_request: 2048 * MiB, cpu_p50: 900, cpu_p95: 1301, cpu_p99: 1340, cpu_max: 1346, mem_p95: 1200 * MiB, mem_max: 1287 * MiB,
      cpu_spare: (700 - 1301) * 2, mem_spare: (2048 - 1287) * MiB * 2, flags: [{ Restarts: { count: 1 } }],
      suggestion: { cpu_request: 1350, mem_request: 1480 * MiB, basis: 'p95 CPU and peak memory × 1.15 over 3d (8,640 pod-minutes), no rollouts, no OOM kills' },
    }),
    row(key('nightly', 'CronJob', 'run', 'ops'), {
      pods: 3, avg_pods: 1, minutes: 120, pod_minutes: 120, cpu_request: 100, mem_request: 128 * MiB, cpu_p95: 80, cpu_max: 95, mem_max: 90 * MiB,
      cpu_spare: 20, mem_spare: 38 * MiB, flags: [{ TooLittleData: { hours: 2 } }],
    }),
  ];
  const summary = {
    path: '', context: 'acme-uat', scope: { Namespaces: ['shop', 'ops'] }, started: start, ended: params.get('running') ? null : now,
    end_reason: params.get('running') ? null : 'Stopped', unfinished: false, data_start: start - 6900, data_end: now,
    prometheus: 'monitoring/prometheus', retention_secs: 7200, pods: 10, containers: 12, prometheus_samples: 30000, local_samples: 300,
    gaps: [{ start: start + 30 * H, end: start + 32 * H, reason: 'Prometheus no longer held this data' }], file_bytes: 4000000,
  };
  // A daily wave with a weekday peak, a rollout halfway (request 3000m → 1500m) and a gap.
  function series(k, s, e, points) {
    const full = [start, now];
    s = s == null ? full[0] : Math.max(full[0], s);
    e = e == null ? full[1] : Math.min(full[1], e);
    const step = Math.max(60, Math.ceil((e - s) / points / 60) * 60);
    const out = { start: s, end: e, step, cpu_avg: [], cpu_max: [], mem_avg: [], mem_max: [], pods: [],
      cpu_request: [], cpu_limit: [], mem_request: [], mem_limit: [], events: [], gaps: [], full_start: full[0], full_end: full[1] };
    const db = k.name === 'db';
    for (let t = Math.floor(s / step) * step; t < e; t += step) {
      if (t > start + 30 * H && t < start + 32 * H) continue;
      const hour = new Date(t * 1000).getHours();
      const wave = 0.5 + 0.5 * Math.sin((hour - 8) / 24 * 2 * Math.PI);
      const cpu = db ? 300 + 400 * wave : 6 + 8 * wave;
      out.cpu_avg.push([t, cpu]);
      out.cpu_max.push([t, cpu * 1.6]);
      const mem = db ? (700 + 230 * wave) * MiB : (1800 + 600 * wave) * MiB;
      out.mem_avg.push([t, mem]);
      out.mem_max.push([t, mem * 1.08]);
      out.pods.push([t, db ? 1 : 3]);
    }
    if (db) {
      out.cpu_limit = [[start, 1000]];
      out.mem_limit = [[start, 1000 * MiB]];
      out.events = [{ t: start + 20 * H, kind: 'restart', reason: 'OOMKilled', count: 1 }, { t: start + 50 * H, kind: 'oom', reason: null, count: 1 }]
        .filter((ev) => ev.t >= s && ev.t < e);
    } else {
      out.cpu_request = [[start, 3000], [start + 36 * H, 1500]];
      out.mem_request = [[start, 6144 * MiB]];
      out.mem_limit = [[start, 6144 * MiB]];
    }
    out.gaps = summary.gaps.filter((g) => g.end > s && g.start < e);
    return out;
  }
  function daily(k) {
    const days = [];
    for (let d = 0; d < 4; d++) {
      const date = new Date((start + d * 86400) * 1000);
      const cpu = [], mem = [];
      for (let h = 0; h < 24; h++) {
        const wave = 0.5 + 0.5 * Math.sin((h - 8) / 24 * 2 * Math.PI);
        const has = !(d === 0 && h < date.getHours()) && !(d === 3 && h > new Date(now * 1000).getHours());
        cpu.push(has ? (k.name === 'db' ? 300 + 400 * wave : 6 + 8 * wave) : null);
        mem.push(has ? (1800 + 600 * wave) * MiB : null);
      }
      days.push({ label: date.toLocaleDateString('en-GB', { weekday: 'short', day: 'numeric', month: 'short' }).replace(',', ''), cpu, mem });
    }
    return { days };
  }
  const kinds = [{ kind: { group: '', version: 'v1', kind: 'Pod', plural: 'pods', namespaced: true }, verbs: ['list', 'watch', 'get'], category: 'Workloads' }];
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (/trace|choose/.test(cmd)) sent.push(cmd + (cmd === 'trace_series' && args.start ? ':zoomed' : ''));
      if (cmd === 'list_contexts') return { contexts: [{ name: 'acme-uat', cluster: 'acme-uat', namespace: null, prod_like: false, unlocked: false }], current: 'acme-uat' };
      if (cmd === 'connect') { if (params.get('offline')) throw 'no answer from the cluster in 10s; check your network or VPN'; return { context: 'acme-uat', kinds, namespaces: ['shop'] }; }
      if (cmd === 'trace_status') return params.get('running') ? { context: 'acme-uat', path: '/Users/me/Documents/acme-uat-shop-now.trace', scope: { Namespaces: ['shop'] },
        started: start, data_start: start, data_end: now, prometheus: true, last_pull: now - 60, last_error: null, gaps: 0, sampled_pods: 0, stopping: false } : null;
      if (cmd === 'choose_trace_to_open') return params.get('cancel') ? null : '/Users/me/Documents/shared-acme-uat.trace';
      if (cmd === 'trace_review') {
        if (params.get('bad')) throw `${args.path} isn't a trace file`;
        return { summary: Object.assign({}, summary, { path: args.path }), rows };
      }
      if (cmd === 'trace_series') return series(args.key, args.start, args.end, args.points);
      if (cmd === 'trace_daily') return daily(args.key);
      if (['set_unsaved', 'stop_watch', 'set_app_theme', 'reveal_trace', 'check_connection'].includes(cmd)) return null;
      if (cmd === 'list_forwards') return [];
      if (cmd === 'cluster_overview') throw 'stub: no overview';
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  const button = (text, scope = document) => [...scope.querySelectorAll('button')].find((b) => b.textContent.trim() === text);
  let t = 900;
  const step = (f, wait = 400) => setTimeout(f, (t += wait));
  const setSelect = (sel, value) => { if (sel) { sel.value = value; sel.dispatchEvent(new Event('change', { bubbles: true })); } };
  if (params.get('page') || params.get('file') || params.get('open')) step(() => [...document.querySelectorAll('button.kind')].find((b) => /Traces/.test(b.textContent))?.click());
  if (params.get('open')) step(() => button('Open trace…')?.click());
  if (params.get('file')) step(() => document.querySelector('.trace-card-open')?.click());
  if (params.get('ns')) step(() => setSelect(document.querySelector('.review-filters select'), params.get('ns')));
  if (params.get('text')) step(() => { const i = document.querySelector('.review-search'); if (i) { i.value = params.get('text'); i.dispatchEvent(new Event('input', { bubbles: true })); } });
  if (params.get('flagged')) step(() => document.querySelector('.review-filters .check input')?.click());
  if (params.get('sort')) step(() => [...document.querySelectorAll('.th-sort')].find((b) => b.textContent.trim().startsWith(params.get('sort')))?.click());
  if (params.get('sel')) step(() => [...document.querySelectorAll('.review-table tbody tr')].find((r) => r.querySelector('.review-name')?.textContent === params.get('sel'))?.click());
  const quick = params.get('quick');
  if (quick) step(() => button({ '1h': 'Last 1h', '6h': 'Last 6h', '24h': 'Last 24h', all: 'All' }[quick])?.click(), 700);
  if (params.get('zoom')) {
    step(() => {
      const c = document.querySelector('.tchart');
      if (!c) return;
      const r = c.getBoundingClientRect();
      const ev = (type, f) => c.dispatchEvent(new PointerEvent(type, { bubbles: true, clientX: r.left + r.width * f, clientY: r.top + 40, pointerId: 1 }));
      ev('pointerdown', 0.5); ev('pointermove', 0.7); ev('pointerup', 0.7);
    }, 700);
  }
  if (params.get('heat') === 'mem') step(() => button('Memory peak', document.querySelector('.review-detail') || document)?.click());
  if (params.get('back')) step(() => button('← Traces')?.click());
  setTimeout(() => {
    const order = [...document.querySelectorAll('.review-table tbody .review-name')].map((n) => n.textContent).join(',');
    const detail = document.querySelector('.review-detail h2')?.textContent || 'none';
    const range = [...document.querySelectorAll('.range-row .toggle.on')].map((b) => b.textContent).join('|') || 'none';
    const heat = document.querySelectorAll('.heat-cell[class*=" l"]').length;
    document.title = `sent=${sent.join(',')} rows=${order || 'none'} detail=${detail} range=${range} heat=${heat}`
      + ` page=${document.querySelector('.traces-page h1')?.textContent || 'none'}`;
  }, t + 1500);
})();
