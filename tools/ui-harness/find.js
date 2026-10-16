// Loaded with every stub, like esc.js; does nothing without find=. Drives the detail panel's find bar:
// find=<text> (typed after opening with ⌘F, or the Find button with findbtn=1), findat=<ms> (when, default 2900),
// findtab=yaml (switch to the YAML tab first), next=<n> (Enter n more times), prev=1 (then Shift+Enter once),
// findesc=1 (Esc in the field afterwards). The title reports: the count text, the current match's text and
// section, how many closed sections there are, whether the bar is open and whether the panel is still open.
(function () {
  const params = new URLSearchParams(location.search);
  const text = params.get('find');
  if (text === null) return;
  const at = Number(params.get('findat') || 2900);
  const key = (k, extra = {}) => new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...extra });
  const field = () => document.querySelector('.find-bar .find-input');
  if (params.get('findtab') === 'yaml') setTimeout(() => [...document.querySelectorAll('.tabs .tab')].find((t) => t.textContent.trim() === 'YAML')?.click(), at - 400);
  setTimeout(() => {
    if (params.get('findbtn')) document.querySelector('.find-open')?.click();
    else window.dispatchEvent(key('f', { metaKey: true }));
  }, at);
  setTimeout(() => { const i = field(); if (!i) return; i.value = text; i.dispatchEvent(new Event('input', { bubbles: true })); }, at + 200);
  const next = Number(params.get('next') || 0);
  for (let n = 0; n < next; n++) setTimeout(() => field()?.dispatchEvent(key('Enter')), at + 500 + n * 150);
  let t = at + 500 + next * 150;
  if (params.get('prev')) { setTimeout(() => field()?.dispatchEvent(key('Enter', { shiftKey: true })), t); t += 150; }
  if (params.get('findesc')) { setTimeout(() => field()?.dispatchEvent(key('Escape')), t); t += 150; }
  setTimeout(() => {
    const count = document.querySelector('.find-count')?.textContent.trim() ?? '';
    let current = '', section = '';
    const h = window.CSS && CSS.highlights && CSS.highlights.get('subduer-find-current');
    if (h) for (const r of h) {
      current = r.toString();
      section = r.startContainer.parentElement?.closest('.dsection')?.querySelector('.dsection-name')?.textContent ?? '';
    }
    const all = window.CSS && CSS.highlights && CSS.highlights.get('subduer-find');
    const closed = document.querySelectorAll('.dsection-title[aria-expanded="false"]').length;
    document.title = `find count=${count} | current=${current} | section=${section} | painted=${all ? all.size : 0} | closed=${closed} | bar=${!!document.querySelector('.find-bar')} | panel=${!!document.querySelector('aside.detail')}`;
  }, t + 400);
})();
