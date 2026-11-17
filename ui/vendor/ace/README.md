# Vendored Ace

Code editor for the YAML edit windows (syntax highlighting, YAML error
marking in a worker, auto-indent, folding, search, autocompletion). Bundled so
the app works offline; loaded only by edit windows (`ui/src/editor.rs`).

| Files | Package | Integrity (npm `dist.integrity`, verified on download) |
|---|---|---|
| `ace.js`, `mode-yaml.js`, `worker-yaml.js`, `theme-cloud_editor.js`, `theme-cloud_editor_dark.js`, `ext-language_tools.js`, `ext-searchbox.js` | `ace-builds@1.45.2` (`src-min-noconflict/`) | `sha512-a0IADaETGBnBW5CIkniUJn7F+NyTuHWKqlv1A8FXaMmFflJsMGnU3TzkiyslInqIZ3odfcVKOdXBGIKBefzFDA==` |

Only change: trailing `//# sourceMappingURL=` comments removed (maps aren't bundled).
BSD-3-Clause licensed; see `LICENSE`. To update, download the new tarball, check its
integrity against the registry (`https://registry.npmjs.org/ace-builds/<version>`), and
replace these files.
