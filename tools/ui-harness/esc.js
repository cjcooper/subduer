// Presses Esc, for every stub (run_e2e.py loads it after the stub).
// esc=dialog: once a dialog is open (plus 500 ms); esc=busy: once a dialog's
// Cancel (its first action) is disabled, as while its action runs (plus 300 ms);
// esc=<ms>: at that time.
// eschold=1: a repeat follows, as when the key is held.
// escfrom=field: from the dialog's text field, or the page's first one when
// no dialog is open; escfrom=select: from the page's first dropdown;
// otherwise from the page body.
// Writes "esc-report before: … after: …" (top dialog's heading, whether the
// detail panel shows) into the page text 500 ms after the key.
(() => {
  const params = new URLSearchParams(location.search);
  const when = params.get('esc');
  if (!when) return;
  const state = () => {
    const dialogs = document.querySelectorAll('.modal-backdrop');
    const top = dialogs[dialogs.length - 1]?.querySelector('h2')?.textContent || 'none';
    return `dialog=${top} detail=${document.querySelector('aside.detail') ? 'yes' : 'no'}`;
  };
  const press = () => {
    const before = state();
    const dialogs = document.querySelectorAll('.modal-backdrop');
    const scope = dialogs[dialogs.length - 1] || document;
    const from = params.get('escfrom');
    const field = from === 'select' ? document.querySelector('select') : from === 'field' && scope.querySelector('input:not([type=checkbox]):not([type=radio]):not([type=file]):not([type=button]), textarea');
    const target = field || document.body;
    if (field) field.focus();
    target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    // A held key: the repeats that follow the first press.
    if (params.get('eschold')) setTimeout(() => document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', repeat: true, bubbles: true, cancelable: true })), 50);
    setTimeout(() => {
      const out = document.createElement('div');
      out.id = 'esc-report';
      out.textContent = `esc-report from=${field ? from : 'body'} before: ${before} after: ${state()}`;
      document.body.append(out);
    }, 500);
  };
  const waitFor = { dialog: ['.modal-backdrop', 500], busy: ['.modal-backdrop .modal-actions button:first-child:disabled', 300] }[when];
  if (waitFor) {
    const poll = setInterval(() => {
      if (document.querySelector(waitFor[0])) { clearInterval(poll); setTimeout(press, waitFor[1]); }
    }, 100);
  } else {
    setTimeout(press, Number(when));
  }
})();
