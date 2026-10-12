(function () {
  const params = new URLSearchParams(location.search);
  const which = params.get('pod') || 'limited';
  const hoverAt = params.get('hover');
  const prod = params.get('ctx') === 'prod';
  const ctxName = prod ? 'acme-prd' : 'acme-uat';
  let unlocked = params.get('locked') !== '1';
  const listeners = {};
  const kinds = [{ kind: { group: '', version: 'v1', kind: 'Pod', plural: 'pods', namespaced: true }, verbs: ['list', 'watch', 'get', 'delete'], category: 'Workloads' }];
  const usage = (r, l, u, n) => ({ requests: r, limits: l, used: u, node: n });
  const container = (name, cpu, mem) => ({ name, init: false, sidecar: false, image: 'ghcr.io/acme/' + name + ':1.2.3', state: 'Running', detail: 'Running', ready: true, restarts: 0, started: '2026-10-09T10:00:00Z', last_termination: null, ports: ['http 8080/TCP'], cpu, memory: mem, env: [] });
  const NODE_CPU = 2000, NODE_MEM = 3903090688;
  const pods = {
    limited: {
      cpu: usage(350, null, 820, NODE_CPU), memory: usage(335544320, null, 492653184, NODE_MEM),
      containers: [
        container('api', usage(250, 1000, 790, NODE_CPU), usage(268435456, 536870912, 402653184, NODE_MEM)),
        container('cache', usage(100, null, 30, NODE_CPU), usage(67108864, null, 90000000, NODE_MEM)),
      ],
    },
    unlimited: {
      cpu: usage(null, null, 0.74, NODE_CPU), memory: usage(null, null, 19959808, NODE_MEM),
      containers: [container('operator', usage(null, null, 0.74, NODE_CPU), usage(null, null, 19959808, NODE_MEM))],
    },
    // Every kind of container: init, a sidecar, the app, and a capture container added later.
    groups: {
      cpu: usage(350, null, 820, NODE_CPU), memory: usage(335544320, null, 492653184, NODE_MEM),
      containers: [
        { ...container('migrate', usage(100, null, null, NODE_CPU), usage(67108864, null, null, NODE_MEM)), init: true, state: 'Stopped', detail: 'Completed', image: 'ghcr.io/acme/migrate:1' },
        { ...container('proxy', usage(50, 100, 10, NODE_CPU), usage(33554432, 67108864, 20e6, NODE_MEM)), init: true, sidecar: true },
        container('api', usage(250, 1000, 790, NODE_CPU), usage(268435456, 536870912, 402653184, NODE_MEM)),
        { ...container('subduer-capture-1', usage(null, null, null, null), usage(null, null, null, null)), ephemeral: true, image: 'ghcr.io/acme/netshoot:1', ports: [] },
      ],
    },
  };
  const p = pods[which];
  // noports=1: the containers declare no ports.
  if (params.get('noports')) p.containers.forEach((c) => { c.ports = []; });
  const end = Math.floor(Date.now() / 15000) * 15;
  const start = end - 3600;
  const series = (fn, from) => { const out = []; for (let t = from ?? start; t <= end; t += 15) { if (t > start + 1200 && t < start + 1300) continue; out.push([t, fn((t - start) / 3600)]); } return out; };
  const wave = (base, amp, k) => (x) => Math.max(0, base + amp * Math.sin(x * k) + amp * 0.3 * Math.sin(x * 37));
  const hist = {
    limited: {
      source: 'Prometheus',
      containers: [
        { name: 'api', usage: { cpu: series(wave(600, 250, 9)), memory: series((x) => 330e6 + 80e6 * x + 10e6 * Math.sin(x * 20)) } },
        { name: 'cache', usage: { cpu: series(wave(30, 10, 5)), memory: series((x) => 88e6 + 3e6 * Math.sin(x * 8)) } },
      ],
    },
    unlimited: {
      source: 'Local',
      containers: [{ name: 'operator', usage: { cpu: series(wave(0.6, 0.3, 30), end - 900), memory: series((x) => 19.5e6 + 0.8e6 * Math.sin(x * 50), end - 900) } }],
    },
    groups: {
      source: 'Prometheus',
      containers: [{ name: 'api', usage: { cpu: series(wave(600, 250, 9)), memory: series((x) => 330e6 + 80e6 * x) } }],
    },
  }[which];
  const sum = (key) => { const m = new Map(); hist.containers.forEach((c) => c.usage[key].forEach(([t, v]) => m.set(t, (m.get(t) || 0) + v))); return [...m.entries()].sort((a, b) => a[0] - b[0]); };
  const history = { source: hist.source, start, end, step: 15, pod: { cpu: sum('cpu'), memory: sum('memory') }, containers: hist.containers };
  const details = { name: 'acme-' + which, namespace: 'apps', status: 'Running', node: 'ip-10-0-1-5.ec2.internal', pod_ip: '10.0.1.9', host_ip: '10.0.1.5', qos_class: 'Burstable', service_account: 'default', owner: 'ReplicaSet/acme-7d9f', restart_policy: 'Always', priority_class: null, created: '2026-10-01T10:00:00Z', labels: [['app', 'acme']], cpu: p.cpu, memory: p.memory, metrics_available: true, containers: p.containers, events: [] };
  const table = { watch_id: 1, columns: [{ header: 'Name', kind: 'Name' }, { header: 'Namespace', kind: 'Namespace' }], rows: [{ id: 'apps/acme-' + which, name: 'acme-' + which, namespace: 'apps', cells: ['acme-' + which, 'apps'] }], error: null };
  const replies = {
    
    connect: { context: ctxName, kinds, namespaces: ['apps'] }, delete_resource: null,
    pod_details: details, cluster_overview: null, pod_history: history, stop_watch: null, set_unlocked: null,
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args, opts) => {
      if (cmd === params.get('hang')) return new Promise(() => {});
      if (cmd === params.get('slow')) await new Promise((r) => setTimeout(r, 1500));
      if (cmd === 'watch_resources') { setTimeout(() => (listeners['resource-table'] || []).forEach((cb) => cb({ payload: table })), 50); return 1; }
      if (cmd === 'list_contexts') return { contexts: [{ name: ctxName, cluster: ctxName, namespace: null, prod_like: prod, unlocked }], current: ctxName };
      if (cmd === 'set_unlocked') { unlocked = true; return null; }
      if (cmd === 'browse_dir') {
        const fs = {
          '/app': { writable: false, entries: [
            { name: 'config', dir: true, writable: false, readable: true }, { name: 'secrets', dir: true, writable: false, readable: false },
            { name: 'app.log', dir: false, writable: false, readable: true }, { name: 'main', dir: false, writable: false, readable: true },
            { name: 'key.pem', dir: false, writable: false, readable: false }] },
          '/tmp': { writable: true, entries: [{ name: 'cache', dir: true, writable: true, readable: true }, { name: 'app.sock', dir: false, writable: true, readable: true }, { name: 'nginx.conf', dir: false, writable: true, readable: true }] },
          '/': { writable: false, entries: ['app','bin','data','etc','tmp','usr','var'].map((n) => ({ name: n, dir: true, writable: n === 'tmp' || n === 'data', readable: true })).concat([{ name: '.dockerenv', dir: false, writable: false, readable: true }]) },
          '/data': { writable: true, entries: [{ name: 'uploads', dir: true, writable: true, readable: true }, { name: 'README.md', dir: false, writable: false, readable: true }] },
        };
        const p0 = args.path === '.' ? '/app' : args.path; const d = fs[p0] || fs['/'];
        return { path: fs[p0] ? p0 : '/', writable: d.writable, entries: d.entries, truncated: false, suggestions: args.suggest ? ['/tmp', '/data', '/home/app'] : [] };
      }
      if (cmd === 'choose_capture_file') {
        document.title = 'choose_capture_file ' + args.suggested;
        return params.get('capcancel') ? null : '/Users/me/Documents/' + args.suggested;
      }
      if (cmd === 'start_capture') {
        document.title = 'start_capture ' + JSON.stringify(args.request);
        if (params.get('capfail')) throw 'pods "acme-limited" is forbidden: cannot patch resource "pods/ephemeralcontainers"';
        return 'capture-1';
      }
      if (cmd === 'download_file') return { saved_to: '/Users/me/Downloads/' + args.path.split('/').pop(), bytes: 48213 };
      if (cmd === 'upload_file') {
        const t = JSON.parse(decodeURIComponent(opts.headers['x-upload']));
        window.__uploads = (window.__uploads || 0) + 1;
        document.title = 'upload ' + args.length + ' bytes to ' + t.path + ' overwrite=' + t.overwrite;
        return window.__uploads === 1 && params.get('exists') ? { Exists: { path: t.path + t.file_name } } : { Uploaded: { bytes: args.length, path: t.path + t.file_name } };
      }
      if (cmd in replies) return replies[cmd];
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (name, cb) => { (listeners[name] = listeners[name] || []).push(cb); return () => {}; } },
  };
  setTimeout(() => { document.querySelector('.kind-section-title')?.click(); }, 600);
  setTimeout(() => { const t = [...document.querySelectorAll('button.kind')].find((b) => /Pod/.test(b.textContent)); if (t) t.click(); }, 1000);
  setTimeout(() => { const r = document.querySelector('tbody tr'); if (r) r.click(); }, 1500);
  // Capture: cap=1 (Capture… on the pod), capsaved=<image> (remembered from before), capimg=<image> (typed),
  // capfilter=port|custom (the first inbound port, or Custom with capexpr=<text>), caphs=0 (unticks TLS handshakes
  // only), capsecs=<s>, capname=1 (types the context's name), capgo=1 (Save as and start…), capcancel=1 (the Save dialog
  // is cancelled), capfail=1 (the backend refuses).
  try {
    if (params.get('capsaved')) localStorage.setItem('capture.image.' + ctxName, params.get('capsaved'));
    else localStorage.removeItem('capture.image.' + ctxName);
  } catch (e) { /* no storage */ }
  const typeIn = (el, text) => { if (el) { el.value = text; el.dispatchEvent(new Event('input', { bubbles: true })); } };
  if (params.get('cap')) {
    setTimeout(() => [...document.querySelectorAll('.detail-actions button')].find((b) => b.textContent === 'Capture…')?.click(), 2600);
    if (params.get('capimg')) setTimeout(() => typeIn(document.querySelector('.capture-field input'), params.get('capimg')), 2900);
    const f = params.get('capfilter');
    const radio = (match) => [...document.querySelectorAll('.capture-choice')].find((l) => match.test(l.textContent))?.querySelector('input');
    if (f) setTimeout(() => radio(f === 'port' ? /Inbound to port/ : /Custom filter/)?.click(), 3000);
    if (params.get('caphs') === '0') setTimeout(() => document.querySelector('.capture-handshakes input')?.click(), 3100);
    if (params.get('capexpr')) setTimeout(() => typeIn(document.querySelector('.capture-modal .confirm-input'), params.get('capexpr')), 3200);
    if (params.get('capsecs')) setTimeout(() => {
      const s = document.querySelector('.capture-field select');
      if (s) { s.value = params.get('capsecs'); s.dispatchEvent(new Event('change', { bubbles: true })); }
    }, 3200);
    if (params.get('capname')) setTimeout(() => typeIn([...document.querySelectorAll('.capture-modal .confirm-input')].pop(), ctxName), 3300);
    if (params.get('capgo')) setTimeout(() => [...document.querySelectorAll('.capture-modal button')].find((b) => b.textContent === 'Save as and start…')?.click(), 3500);
  }
  // opengroup=init|ephemeral: opens that container group.
  const group = { init: 'Init containers', ephemeral: 'Ephemeral containers' }[params.get('opengroup')];
  if (group) setTimeout(() => [...document.querySelectorAll('.dsection-title')].find((b) => b.textContent.includes(group))?.click(), 2600);
  if (params.get('del')) setTimeout(() => document.querySelector('.danger-outline')?.click(), 2600);
  if (params.get('del') === '2') setTimeout(() => document.querySelector('.modal .danger')?.click(), 3000);
  if (params.get('download')) {
    setTimeout(() => [...document.querySelectorAll('.dsection-actions button')].find((b) => b.textContent === 'Download')?.click(), 2600);
    if (params.get('download') === 'pick') setTimeout(() => [...document.querySelectorAll('.browse-entry')].find((b) => b.textContent.includes('app.log'))?.click(), 3200);
    if (params.get('download') === 'done') {
      setTimeout(() => [...document.querySelectorAll('.browse-entry')].find((b) => b.textContent.includes('app.log'))?.click(), 3200);
      setTimeout(() => [...document.querySelectorAll('.modal button')].find((b) => b.textContent === 'Download')?.click(), 3500);
    }
  }
  if (params.get('upload')) {
    setTimeout(() => [...document.querySelectorAll('.dsection-actions button')].find((b) => b.textContent === 'Upload')?.click(), 2600);
    setTimeout(() => {
      const dt = new DataTransfer(); dt.items.add(new File(['hello '.repeat(2000)], 'nginx.conf', { type: 'text/plain' }));
      document.querySelector('.drop-zone')?.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer: dt }));
    }, 3000);
    if (params.get('browse')) setTimeout(() => [...document.querySelectorAll('.crumb')].find((b) => b.textContent === '/')?.click(), 3300);
    if (params.get('upload') === 'send') setTimeout(() => [...document.querySelectorAll('.modal button')].find((b) => b.textContent === 'Upload')?.click(), 3400);
  }
  // again=<ms>: open the Download (or Upload) dialog again, as after closing it.
  if (params.get('again')) setTimeout(() => [...document.querySelectorAll('.dsection-actions button')].find((b) => b.textContent === (params.get('download') ? 'Download' : 'Upload'))?.click(), Number(params.get('again')));
  if (hoverAt) setTimeout(() => {
    const c = document.querySelector('.lchart:not(.empty)'); if (!c) return;
    const r = c.getBoundingClientRect();
    c.dispatchEvent(new PointerEvent('pointermove', { bubbles: true, clientX: r.left + r.width * Number(hoverAt), clientY: r.top + 30 }));
  }, 2500);
})();
