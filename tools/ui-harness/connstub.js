// Giving up on a failing connection. Query: start=fail (the first connect fails, as with an expired token),
// watch=stop (the Pods watch gives up), check=fail|ok|late (late: fails once three refreshes have) (the connection check after a watch or refreshes give up),
// overview=fail (every Cluster page refresh fails), overview=hang (the first refresh never answers),
// vpn=drop (the background check fails until Connect is clicked; nothing else errors),
// click=connect|retry|refresh (clicks that button), at=<ms> (when to click; the title follows 1s later).
// The title reports the backend calls by name and what the page shows.
(function () {
  const params = new URLSearchParams(location.search);
  const listeners = {};
  const calls = [];
  let connects = 0;
  const kinds = [{ kind: { group: '', version: 'v1', kind: 'Pod', plural: 'pods', namespaced: true }, verbs: ['list', 'watch', 'get'], category: 'Workloads' }];
  const expired = 'exec plugin: token has expired; run your sign-in command again';
  const table = (stopped) => ({
    watch_id: calls.filter((c) => c === 'watch_resources').length,
    columns: [{ header: 'Name', kind: 'Name' }, { header: 'Namespace', kind: 'Namespace' }],
    rows: [{ id: 'shop/api-1', name: 'api-1', namespace: 'shop', cells: ['api-1', 'shop'] }],
    error: stopped ? 'Unauthorized' : null, stopped,
  });
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      calls.push(cmd);
      if (calls.length === 1) start();
      if (cmd === 'list_contexts') return { contexts: [{ name: 'acme-uat', cluster: 'acme-uat', namespace: null, prod_like: false, unlocked: false }], current: 'acme-uat' };
      if (cmd === 'connect') {
        connects += 1;
        if (params.get('start') === 'fail' && connects === 1) throw expired;
        return { context: 'acme-uat', kinds, namespaces: ['shop'] };
      }
      if (cmd === 'watch_resources') {
        // The first watch gives up when asked to; a Retry or a new connection works.
        const stop = params.get('watch') === 'stop' && calls.filter((c) => c === 'watch_resources').length === 1;
        setTimeout(() => (listeners['resource-table'] || []).forEach((cb) => cb({ payload: table(stop) })), 50);
        return calls.filter((c) => c === 'watch_resources').length;
      }
      if (cmd === 'check_connection') {
        if (params.get('check') === 'fail') throw expired;
        if (params.get('check') === 'late' && calls.filter((c) => c === 'cluster_overview').length >= 3) throw expired;
        if (params.get('vpn') === 'drop' && connects < 2) throw 'no answer from the cluster in 15s; check your network or VPN';
        return null;
      }
      if (cmd === 'cluster_overview') {
        if (params.get('overview') === 'fail') throw 'Unauthorized';
        if (params.get('overview') === 'hang' && calls.filter((c) => c === 'cluster_overview').length === 1) return new Promise(() => {});
        throw 'stub: no overview';
      }
      if (['set_unsaved', 'stop_watch', 'set_app_theme'].includes(cmd)) return null;
      if (cmd === 'list_forwards') return [];
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  // Timers count from the app's first call: under virtual time, a long budget can otherwise run out
  // the clock while the WebAssembly is still loading.
  function start() {
    const button = (text) => [...document.querySelectorAll('button')].find((b) => b.textContent.trim() === text);
    const pods = !params.get('overview');
    if (pods) {
      setTimeout(() => document.querySelector('.kind-section-title')?.click(), 600);
      setTimeout(() => [...document.querySelectorAll('button.kind')].find((b) => /Pod/.test(b.textContent))?.click(), 1000);
    }
    const click = params.get('click');
    // Refreshes fail at 0s, 30s and 60s, so the overview scenarios run long in virtual time.
    const at = Number(params.get('at')) || (params.get('overview') === 'fail' ? 95000 : 2000);
    if (click) setTimeout(() => button({ connect: 'Connect', retry: 'Retry', refresh: 'Refresh' }[click])?.click(), at);
    setTimeout(() => {
      const count = (c) => calls.filter((x) => x === c).length;
      document.title = `connect=${count('connect')} watch=${count('watch_resources')} check=${count('check_connection')} overview=${count('cluster_overview')}`
        + ` connect-button=${!!button('Connect')} retry-button=${!!button('Retry')}`;
    }, at + 1000);
  }
})();
