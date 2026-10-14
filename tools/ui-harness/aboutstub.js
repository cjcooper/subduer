// The About window: `about_info` with a short license and notices in the
// generated layout. `filter=<text>` types into the filter box; `open=1` opens
// the first license.
(function () {
  const params = new URLSearchParams(location.search);
  const rule = '='.repeat(80);
  const notices = [
    'Third-party notices for k8s-subduer', '',
    'k8s-subduer includes the software listed below. Each license is reproduced', 'with the crates it covers.', '',
    '  2 crate(s) under MIT License', '  1 crate(s) under Apache License 2.0', '',
    rule, 'MIT License', '', 'Used by:', '  serde 1.0.228 (https://github.com/serde-rs/serde)', '  tokio 1.48.0 (https://github.com/tokio-rs/tokio)', '',
    'Permission is hereby granted, free of charge, to any person obtaining a copy', 'of this software.', '',
    rule, 'Apache License 2.0', '', 'Used by:', '  k8s-openapi 0.28.0 (https://github.com/Arnavion/k8s-openapi)', '',
    '                                 Apache License', '                           Version 2.0, January 2004', '',
    rule, 'Vendored JavaScript: xterm', '', 'xterm.js (@xterm/xterm 5.5.0):', 'Copyright (c) 2017-2019, The xterm.js authors', '',
  ].join('\n');
  window.__TAURI__ = {
    core: { invoke: async (cmd) => {
      if (cmd === 'about_info') {
        return {
          name: 'k8s-subduer', version: '0.1.0', maker: 'it-problems', copyright: 'Copyright © 2026 it-problems',
          license: 'MIT License\n\nCopyright (c) 2026 it-problems\n\nPermission is hereby granted, free of charge, to any person obtaining a copy\nof this software.\n',
          notices,
        };
      }
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async () => () => {} },
  };
  window.addEventListener('load', () => setTimeout(() => {
    const filter = params.get('filter');
    if (filter) {
      const input = document.querySelector('.about-section .search');
      input.value = filter;
      input.dispatchEvent(new Event('input', { bubbles: true }));
    }
    if (params.get('open')) document.querySelector('.about-notice').open = true;
  }, 400));
})();
