# PipDock — Third-Party Notices

*Version 1.0 · 2026-07-17 · This inventory is regenerated at each release (`cargo about` + `license-checker`); the lists below name the principal components. All bundled licenses are GPL-3.0-compatible; full license texts ship in the release under `licenses/`.*

## A. Bundled / linked components (distributed with PipDock)

| Component | License | Role |
|---|---|---|
| Tauri 2 (+ plugin: dialog) | MIT OR Apache-2.0 | application framework |
| React 19, react-dom | MIT | UI |
| TypeScript | Apache-2.0 | build-time |
| Vite 8, Rolldown toolchain | MIT | build-time |
| Tailwind CSS 4 | MIT | styling |
| Zustand | MIT | state |
| i18next, react-i18next | MIT | localization |
| tokio, serde, clap, tracing, async-trait | MIT OR Apache-2.0 | Rust core |
| nucleo (fuzzy matcher) | MPL-2.0 | search (file-level copyleft; compatible) |
| rusqlite / SQLite | MIT / Public Domain | local cache |
| specta / tauri-specta | MIT | type generation |
| Inter typeface | SIL OFL 1.1 | UI font — `ui/src/assets/fonts/InterVariable.woff2` |
| JetBrains Mono typeface | SIL OFL 1.1 | mono font — `ui/src/assets/fonts/JetBrainsMono-Regular.woff2` |
| Microsoft Edge WebView2 Runtime | proprietary, system component | rendering (not distributed by PipDock; Evergreen runtime assumed present per Microsoft's terms) |

Both typefaces are committed to the repository rather than fetched at build time. The verbatim OFL
text for each, the release they came from and the SHA-256 of each file are in `legal/fonts/`.

## B. External tools invoked as separate processes (NOT distributed with PipDock)

These run as independent programs from your system or from PipDock's isolated tools environment; they are separate works under their own licenses, obtained by you from PyPI/their distributors:

| Tool | License | Invoked for |
|---|---|---|
| pip | MIT | engine |
| uv | MIT OR Apache-2.0 | engine |
| deptry | MIT | Code Health — dependency issues |
| vulture | MIT | Code Health — dead code |
| ruff | MIT | Code Health — lint/format |
| pip-audit | Apache-2.0 | Security tab (when enabled) |
| CPython | PSF License | the environments themselves |

## C. Services

PyPI (operated by the Python Software Foundation) supplies the package name index and metadata under its Terms of Use. GitHub supplies release hosting for PipDock's own updates.

## D. Trademarks

"Python", "pip", "PyPI" — Python Software Foundation. "uv", "ruff" — Astral. "Windows", "Edge WebView2" — Microsoft. Used for identification only; no affiliation or endorsement implied.

---

*Cảm ơn các dự án mã nguồn mở trên — PipDock đứng trên vai họ. / PipDock stands on the shoulders of the projects above — thank you.*
