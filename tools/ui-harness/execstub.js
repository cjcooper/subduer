(function () {
  const params = new URLSearchParams(location.search);
  const label = params.get('exec');
  const listeners = {};
  const b64 = (s) => btoa(unescape(encodeURIComponent(s)));
  const emit = (payload) => (listeners[label] || []).forEach((cb) => cb({ payload }));
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === 'exec_start') {
        setTimeout(() => emit({ Data: b64('\x1b[1;32mroot@web-7d9f-abcde\x1b[0m:\x1b[1;34m/app\x1b[0m# ls\r\nDockerfile  config  main  static  ✓ utf-8\r\n\x1b[1;32mroot@web-7d9f-abcde\x1b[0m:\x1b[1;34m/app\x1b[0m# ') }), 200);
        if (params.get('exit')) setTimeout(() => emit({ Exited: 'Shell exited.' }), 600);
        return { context: 'acme-uat', title: 'app · shop/web-7d9f-abcde · acme-uat · Shell', event: label, target: { namespace: 'shop', pod: 'web-7d9f-abcde', container: 'app' } };
      }
      if (cmd === 'exec_input') { emit({ Data: b64(args.data) }); return null; }
      if (cmd === 'exec_resize') return null;
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (name, cb) => { (listeners[name] = listeners[name] || []).push(cb); return () => {}; } },
  };
})();
