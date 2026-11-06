// The About window: `about_info` with a short license and notices in the
// generated layout. `tab=license|third-party` picks a tab; `filter=<text>`
// types into the filter box and `chip=<label prefix>` clicks a license chip
// (both on the Third-party tab); `open=1` opens the first license;
// `esc=1` presses Esc; `theme=light|dark` is saved before the app starts. The
// title reports which app icon shows (`icon=light|dark`) and the commands sent
// (`sent=open_url:<url>,close_about`).
(function () {
  const params = new URLSearchParams(location.search);
  try { localStorage.clear(); if (params.get('theme')) localStorage.setItem('appearance.theme', params.get('theme')); } catch (e) {}
  const rule = '='.repeat(80);
  const notices = [
    'Third-party notices for Subduer', '',
    'Subduer includes the software listed below. Each license is reproduced', 'with the crates it covers.', '',
    '  2 crate(s) under MIT License', '  1 crate(s) under Apache License 2.0', '',
    rule, 'MIT License', '', 'Used by:', '  serde 1.0.228 (https://github.com/serde-rs/serde)', '  tokio 1.48.0 (https://github.com/tokio-rs/tokio)', '',
    'Permission is hereby granted, free of charge, to any person obtaining a copy', 'of this software.', '',
    rule, 'Apache License 2.0', '', 'Used by:', '  k8s-openapi 0.28.0 (https://github.com/Arnavion/k8s-openapi)', '',
    '                                 Apache License', '                           Version 2.0, January 2004', '',
    rule, 'Vendored JavaScript: xterm', '', 'xterm.js (@xterm/xterm 5.5.0):', 'Copyright (c) 2017-2019, The xterm.js authors', '',
  ].join('\n');
  const sent = [];
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === 'open_url') { sent.push(`open_url:${args.url}`); return null; }
      if (cmd === 'close_about') { sent.push('close_about'); return null; }
      if (cmd === 'about_info') {
        return {
          name: 'Subduer', version: '0.1.0',
          description: 'Desktop app for browsing and controlling Kubernetes clusters',
          repository: 'https://github.com/cjcooper/k8s-subduer', maker: 'it-problems', copyright: 'Copyright © 2026 it-problems',
          license: 'MIT License\n\nCopyright (c) 2026 it-problems\n\nPermission is hereby granted, free of charge, to any person obtaining a copy\nof this software.\n',
          notices,
        };
      }
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async () => () => {} },
  };
  const click = (sel, text) => [...document.querySelectorAll(sel)].find(b => b.textContent.trim().startsWith(text))?.click();
  window.addEventListener('load', () => setTimeout(() => {
    const tab = params.get('tab');
    if (tab) click('.about-head .tab', tab === 'license' ? 'License' : 'Third-party');
    setTimeout(() => {
      const filter = params.get('filter');
      if (filter) {
        const input = document.querySelector('.third-party .search');
        input.value = filter;
        input.dispatchEvent(new Event('input', { bubbles: true }));
      }
      if (params.get('chip')) click('.about-chips .chip-button', params.get('chip'));
      if (params.get('link')) click('.about-main a', params.get('link'));
      if (params.get('open')) document.querySelector('.about-notice').open = true;
      if (params.get('esc')) window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
      setTimeout(() => {
        const shown = [...document.querySelectorAll('.about-icon')].filter(i => getComputedStyle(i).display !== 'none');
        document.title = `icon=${shown.map(i => i.classList.contains('dark') ? 'dark' : 'light').join(',')} sent=${sent.join(',')}`;
      }, 100);
    }, 100);
  }, 400));
})();
