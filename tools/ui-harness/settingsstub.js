// Settings dialog and Appearance. Query: theme=auto|light|dark (saved before the app starts), pick=auto|light|dark
// (clicked in the dialog), open=1 (opens Settings via the menu event), noauto=1 (pretend the webview can't report
// the system appearance). The title reports data-theme, the page background and the set_app_theme calls.
(function () {
  const params = new URLSearchParams(location.search);
  try { localStorage.clear(); if (params.get('theme')) localStorage.setItem('appearance.theme', params.get('theme')); } catch (e) {}
  if (params.get('noauto')) {
    const real = window.matchMedia.bind(window);
    window.matchMedia = (q) => (q.includes('prefers-color-scheme') ? { media: 'not all', matches: false, addEventListener() {}, removeEventListener() {} } : real(q));
  }
  const listeners = {};
  const themes = [];
  const kinds = [{ kind: { group: '', version: 'v1', kind: 'Pod', plural: 'pods', namespaced: true }, verbs: ['list', 'watch', 'get'], category: 'Workloads' }];
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === 'set_app_theme') { themes.push(args.theme); return null; }
      if (cmd === 'list_contexts') return { contexts: [{ name: 'acme-uat', cluster: 'acme-uat', namespace: null, prod_like: false, unlocked: false }], current: 'acme-uat' };
      if (cmd === 'connect') return { context: 'acme-uat', kinds, namespaces: ['shop'] };
      if (cmd === 'watch_resources') return 1;
      if (['set_unsaved', 'stop_watch', 'list_forwards'].includes(cmd)) return cmd === 'list_forwards' ? [] : null;
      if (cmd === 'cluster_overview') throw 'stub: no overview';
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  if (params.get('open') || params.get('pick')) setTimeout(() => (listeners['open-settings'] || []).forEach((cb) => cb({ payload: null })), 800);
  const pick = params.get('pick');
  if (pick) setTimeout(() => [...document.querySelectorAll('.theme-card')].find((b) => b.textContent.trim().toLowerCase() === pick)?.click(), 1100);
  setTimeout(() => {
    const root = document.documentElement;
    const bg = getComputedStyle(document.body).backgroundColor;
    const buttons = [...document.querySelectorAll('.theme-card')].map((b) => b.textContent.trim() + (b.classList.contains('on') ? '*' : '')).join('|');
    document.title = `data-theme=${root.getAttribute('data-theme') || 'none'} bg=${bg} saved=${localStorage.getItem('appearance.theme')} native=${themes.join(',')} buttons=${buttons}`;
  }, 1600);
})();
