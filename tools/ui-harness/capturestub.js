// A capture window (?capture=capture-1): the session, then updates as the capture runs.
// Query: capture=capture-1 (required: the window's label), end=done|fail (how it ends, after ~2.5 s; default:
// still capturing), stop=1 (clicks Stop), old=1 (one connection settles on TLS 1.2), slow=1 (stays Starting).
// The title reports the commands sent.
(function () {
  const params = new URLSearchParams(location.search);
  const listeners = {};
  const sent = [];
  const now = Date.now() / 1000;
  const conn = (i, out, name, version, cipher) => ({
    first_seen: now - 20 + i, client: out ? `10.0.1.9:${40000 + i}` : `169.254.175.250:${38000 + i}`, server: out ? '192.0.2.9:443' : '10.0.1.9:9443',
    direction: out ? 'Outbound' : 'Inbound', server_name: name, offered: ['TLS 1.3', 'TLS 1.2'], version, cipher, retried: false,
  });
  const connections = [
    conn(0, false, null, 'TLS 1.3', 'TLS_AES_128_GCM_SHA256'),
    conn(1, true, 'api.example.com', 'TLS 1.3', 'TLS_AES_128_GCM_SHA256'),
    conn(2, true, 'metrics.example.com', params.get('old') ? 'TLS 1.2' : 'TLS 1.3', params.get('old') ? 'TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256' : 'TLS_AES_256_GCM_SHA384'),
    conn(3, true, 'slow.example.com', null, null),
  ];
  const scan = (n) => ({ packets: n * 2 + 1, hellos: n * 2, connections: connections.slice(0, n) });
  const update = (phase, extra) => ({ phase, message: null, image_id: null, started: null, ends: null, bytes: 0, scan: scan(0), ...extra });
  const running = { image_id: 'ecr.example/netshoot@sha256:e47b97ba', started: now - 30, ends: now + 270 };
  const session = {
    context: 'acme-uat', namespace: 'apps', pod: 'acme-limited', container: 'subduer-capture-1791804344', image: 'ecr.example/netshoot:latest',
    filter: { traffic: 'All', handshakes_only: true }, expression: 'tcp and tcp[((tcp[12:1] & 0xf0) >> 2):1] = 0x16', secs: 300,
    path: '/Users/me/Library/Application Support/k8s-subduer/captures/apps-acme-limited-2026-10-12-120000.pcap',
    pod_ips: ['10.0.1.9'], event: 'capture-1', update: update('Starting'),
  };
  const emit = (u) => (listeners[session.event] || []).forEach((cb) => cb({ payload: u }));
  window.__TAURI__ = {
    core: { invoke: async (cmd) => {
      if (cmd === 'system_accent') return null;
      sent.push(cmd);
      if (cmd === 'capture_session') return session;
      if (cmd === 'stop_capture') { emit(update('Stopping', { ...running, bytes: 4096, scan: scan(3) })); return null; }
      if (['set_app_theme'].includes(cmd)) return null;
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  const end = params.get('end');
  // A failed capture never got going.
  if (!params.get('slow') && end !== 'fail') {
    setTimeout(() => emit(update('Capturing', { ...running, bytes: 2048, scan: scan(2) })), 800);
    setTimeout(() => emit(update('Capturing', { ...running, bytes: 61249, scan: scan(4) })), 1600);
  }
  if (end === 'done') setTimeout(() => emit(update('Done', { ...running, bytes: 61249, scan: scan(4), message: 'tcpdump: listening on eth0\n40 packets captured\n0 packets dropped by kernel' })), 2500);
  if (end === 'fail') setTimeout(() => emit(update('Failed', { message: 'ErrImagePull: 403 Forbidden' })), 1200);
  if (params.get('stop')) setTimeout(() => [...document.querySelectorAll('button')].find((b) => b.textContent === 'Stop')?.click(), 2000);
  setTimeout(() => { document.title = `sent=${sent.join(',')}`; }, 4000);
})();
