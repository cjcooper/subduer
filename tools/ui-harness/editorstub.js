// YAML edit window (index.html?edit=edit-1). Query: change=1 (edit replicas), bad=1 (add a YAML syntax error),
// save=1 (click Save), confirm=1 (click the modal's main button), typed=1 (type the name it asks for),
// prod=1, locked=1, dryfail=1 (the server rejects the dry run), cancel=1, closereq=1 (the window's close is held),
// keep=1 / discard=1 (answer the discard prompt), reload=1.
// The title reports: calls=<commands sent> dirty=<last editor_dirty> modal=<open dialog title> status=<status bar>.
(function () {
  const params = new URLSearchParams(location.search);
  const prod = params.get('prod') === '1';
  const ctx = prod ? 'acme-prd' : 'acme-uat';
  let unlocked = params.get('locked') !== '1';
  const listeners = {};
  const calls = [];
  let dirty = 'unset';
  const yaml = 'apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: acme-api\n  namespace: shop\n  resourceVersion: "4711"\n  labels:\n    app: acme-api\nspec:\n  replicas: 2\n  selector:\n    matchLabels:\n      app: acme-api\n  template:\n    metadata:\n      labels:\n        app: acme-api\n    spec:\n      containers:\n      - name: api\n        image: ghcr.io/acme/api:1.4.0\n        ports:\n        - containerPort: 8080\n';
  const session = () => ({ context: ctx, prod_like: prod, unlocked,
    target: { kind: { group: 'apps', version: 'v1', kind: 'Deployment', plural: 'deployments', namespaced: true }, namespace: 'shop', name: 'acme-api' },
    title: 'Deployment shop/acme-api · ' + ctx + ' · Edit', yaml });
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => {
      if (cmd === 'editor_start') { calls.push('start'); return session(); }
      if (cmd === 'editor_dirty') { dirty = args.dirty; return null; }
      if (cmd === 'editor_close') { calls.push('close'); return null; }
      if (cmd === 'set_unlocked') { calls.push('unlock:' + args.context); if (params.get('hang') === 'unlock') return new Promise(() => {}); unlocked = true; return null; }
      if (cmd === 'editor_save') {
        calls.push(args.dry_run ? 'check' : 'save');
        if (!args.dry_run && params.get('hang') === 'save') return new Promise(() => {});
        if (args.dry_run && params.get('dryfail')) throw 'Deployment.apps "acme-api" is invalid: spec.replicas: Invalid value: -1: must be greater than or equal to 0';
        return args.yaml;
      }
      throw 'no stub for ' + cmd;
    } },
    event: { listen: async (n, cb) => { (listeners[n] = listeners[n] || []).push(cb); return () => {}; } },
  };
  const editor = () => document.querySelector('.editor-host')?.env?.editor;
  const btn = (sel, text) => [...document.querySelectorAll(sel)].find((b) => b.textContent.trim().startsWith(text));
  // Steps are timed from when the editor holds the YAML (about 1 s in), so a slow start doesn't
  // make them run early.
  const ready = new Promise((resolve) => {
    const poll = setInterval(() => { if (editor()?.getValue().includes('kind:')) { clearInterval(poll); resolve(); } }, 50);
  });
  const at = (ms, f) => ready.then(() => setTimeout(f, Math.max(0, ms - 1000)));
  if (params.get('change')) at(1500, () => { const e = editor(); e && e.session.replace(e.find('replicas: 2'), 'replicas: 3'); });
  if (params.get('bad')) at(1500, () => { const e = editor(); e && e.session.insert({ row: 2, column: 0 }, 'labels: [unclosed\n'); });
  if (params.get('save')) at(2600, () => btn('.editor-actions button', 'Save')?.click());
  if (params.get('typed')) at(3000, () => { const i = document.querySelector('.modal .confirm-input'); if (i) { i.value = document.querySelector('.modal h2').textContent.startsWith('Allow') ? ctx : 'acme-api'; i.dispatchEvent(new Event('input', { bubbles: true })); } });
  if (params.get('confirm')) at(3300, () => document.querySelector('.modal-actions .primary, .modal-actions .danger')?.click());
  // A second round for flows that go unlock → confirm.
  if (params.get('typed')) at(3700, () => { const i = document.querySelector('.modal .confirm-input'); if (i) { i.value = 'acme-api'; i.dispatchEvent(new Event('input', { bubbles: true })); } });
  if (params.get('confirm')) at(4000, () => document.querySelector('.modal-actions .primary')?.click());
  if (params.get('cancel')) at(2600, () => btn('.editor-actions button', 'Cancel')?.click());
  if (params.get('reload')) at(2600, () => btn('.editor-actions button', 'Reload')?.click());
  if (params.get('closereq')) at(2600, () => (listeners['edit-close-requested'] || []).forEach((cb) => cb({ payload: null })));
  if (params.get('keep')) at(3000, () => btn('.modal-actions button', 'Keep')?.click());
  if (params.get('discard')) at(3000, () => btn('.modal-actions button', 'Discard')?.click());
  // Reported repeatedly, so the dump shows the latest state whenever it's taken.
  const report = () => {
    const modal = document.querySelector('.modal h2')?.textContent || 'none';
    const status = document.querySelector('.editor-status')?.textContent.trim() || '';
    const highlighted = !!document.querySelector('.ace_editor .ace_meta, .ace_editor .ace_keyword, .ace_editor .ace_variable');
    const errors = (editor()?.session.getAnnotations() || []).filter((x) => x.type === 'error').length;
    const chip = document.querySelector('.editor-title .prod-chip');
    document.title = `prod-chip=${chip ? chip.textContent.replace('Looks like production', '') + (chip.querySelector('.prod-icon') ? '+icon' : '') : 'none'} calls=${calls.join(',')} dirty=${dirty} modal=${modal} highlighted=${highlighted} errors=${errors} status=${status}`;
  };
  setInterval(report, 250);
})();
