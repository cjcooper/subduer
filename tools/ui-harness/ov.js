(function () {
  const req = new XMLHttpRequest(); req.open('GET', 'ov.json', false); req.send();
  const ov = JSON.parse(req.responseText);
  const replies = {
    list_contexts: { contexts: [{ name: 'acme-uat', cluster: 'acme-uat', namespace: null, prod_like: false, unlocked: false }], current: 'acme-uat' },
    connect: { context: 'acme-uat', kinds: [], namespaces: ['shop'] }, cluster_overview: ov, stop_watch: null,
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd) => { if (cmd === 'watch_resources') return 1; if (cmd in replies) return replies[cmd]; throw 'no stub for ' + cmd; } },
    event: { listen: async () => () => {} },
  };
})();
