# PipDock — UI Specification

*Version 0.1 · 2026-07-17 · Direction: **dark terminal-tech** (owner decision 2026-07)*

## 1. Design intent

PipDock should feel like a **precision instrument docked over a terminal** — the calm confidence of a good TUI with the affordances of a native app. Not a neon cyberpunk cliché, not a generic admin dashboard: restrained phosphor-green on near-black, monospace where data lives, one accent doing real work. Signature moves: a terminal-style **status line** pinned to the bottom, a live **execution console** that streams real engine output, and (P1) a Ctrl+K command palette.

Anti-goals: gradients-everywhere SaaS look, rounded-blob cards, decorative glow. Any glow is functional (focus rings, live-activity pulse) and subtle.

## 2. Design tokens (Tailwind 4 `@theme`)

| Token | Value | Use |
|---|---|---|
| `--color-bg` | `#0A0E0C` | app background |
| `--color-surface` | `#101612` | panels, rows |
| `--color-surface-2` | `#161E18` | hover, elevated |
| `--color-border` | `#1F2B23` | hairlines |
| `--color-accent` | `#4ADE80` | primary actions, focus, live pulse |
| `--color-accent-dim` | `#22935B` | secondary accents, links |
| `--color-text` | `#E7F0EA` | primary text |
| `--color-text-dim` | `#8AA394` | secondary text, **dimmed up-to-date rows** |
| `--color-warn` | `#F5B94A` | held-back, warnings |
| `--color-danger` | `#F0574F` | impossible, failures, force actions |
| `--color-info` | `#5BC0DE` | pinned, informational chips |

Type: **Inter** (UI), **JetBrains Mono** (package names, versions, console, status line — both OFL-licensed, bundled). Base 14 px, mono data 13 px. Radius 6 px; borders 1 px; shadows minimal. Motion: 120–160 ms ease-out; live-activity pulse on the status-line engine badge during execution; `prefers-reduced-motion` respected.

## 3. Layout

```text
┌──────────────────────────────────────────────────────────────┐
│ ⌂ PipDock      [env: .venv (Py 3.12) ▾]        [engine: uv]  │  top bar
├───────────┬──────────────────────────────────────────────────┤
│ Environments        │                                        │
│ Installed           │                                        │
│ Updates (7)         │              content area              │
│ Search              │                                        │
│ Pins                │                                        │
│ Health              │                                        │
│ Security (P1)       │                                        │
│ Settings            │                                        │
├───────────┴──────────────────────────────────────────────────┤
│ ▸ .venv · Py 3.12.4 · uv 0.9.x · idle            log ⌄  ⚠ 0  │  status line
└──────────────────────────────────────────────────────────────┘
```

Sidebar is icon+label, collapsible to icons. Status line (mono) always shows env · python · engine · state, plus a log-drawer toggle. The **console drawer** slides up over the status line during execution, streaming `plan-progress` lines with per-package section markers; collapsible, never modal.

## 4. Screens

**Environments.** Auto-discovered list (source chip: registry / py launcher / uv / manual) with python version and package count; PEP 668 envs carry a red `MANAGED` chip and a disabled state explaining PD-ENV-002. Actions: *Browse…*, *Rescan*, star-as-default.

**Installed.** Virtualized table: name (mono) · version · latest · size · chips. When the env's `hidden_user_site` is non-null (non-venv system Pythons only — see SECURITY §2), an inline `--color-info` note above the table says user-account packages are not listed and names the path; it is informational, never a block, and never appears for a venv. Up-to-date rows use `--color-text-dim` (owner's "dimmed" requirement); outdated rows show an `UPDATE` badge; pinned rows a 🔒 `--color-info` chip. Row actions: pin/unpin, uninstall, details. Multi-select via checkboxes; bulk action bar appears on selection.

**Updates.** Same table filtered to outdated, badge count in sidebar. Header: *Select all* (excludes pinned; shows "3 pinned excluded"), *Update selected*. After resolve → **Preview panel** replaces the table: three grouped sections — *Will upgrade* (A `1.2 → 2.0`), *New dependencies*, *Needs decision* (held-back in `--color-warn` with one-line blocker sentence; impossible in `--color-danger`). Each needs-decision row hosts a segmented control: **Keep compatible · Skip · Force latest** (Force opens an inline confirm naming what breaks). Footer: `Confirm (n changes)` / `Back`.

**Search.** Search field autofocused; results stream under 50 ms per keystroke from the local index. Result row: name · summary (from cached metadata) · latest · `INSTALLED ✓`/`UPDATE` chip when applicable · **[Add]**. Right detail panel: description, requires-python, license, links. Queue ("**dock bay**") docks along the right edge as a slim column of added packages with editable version fields and `Install (n)`.

**Pins.** Pin list with reason field; (P1) auto-suggest section: "`urllib3` — 12 packages depend on it. Pin?".

**Snapshots** (surfaced under Environments → env detail): timeline of snapshots with trigger label, diff viewer (added/removed/changed in mono), `Rollback…` with its own preview per DATA-FLOW §8.

**Health.** Project-folder picker (persisted per env) → run panel with three result tabs (deptry / vulture / ruff) per CODE-HEALTH-SPEC; `Fix with ruff` gated behind explicit confirm listing file count.

**Settings.** Engine (pip/uv radio + detected versions), locale (EN/VI), thresholds (pin auto-suggest count, snapshot retention), index refresh, PEP 668 override (off by default, scary copy), *Open logs folder*, *Legal & About* (links to GitHub legal docs + re-open legal gate).

**Legal gate (first run).** Modal listing the five documents with GitHub links and a summarized disclaimer sentence; single checkbox "I have read and accept" → `Continue`. Decline exits the app. Consent stored with docs-version hash; hash bump re-triggers the gate.

## 5. Click budgets (owner ceiling: 5)

| Flow | Clicks | Path |
|---|---|---|
| Update everything | **4** | Updates → Select all → Update → Confirm (app auto-scans on env open) |
| Update everything, 1 conflict kept compatible | **4** | default choice needs no extra click |
| Install one package | **4** + typing | Search (autofocus) → result [Add] → Install → Confirm |
| Uninstall one package | **3** | Installed row ⋮ → Uninstall → Confirm |
| Rollback last snapshot | **4** | Env detail → snapshot → Rollback → Confirm |
| Switch engine | **3** | Settings → engine radio → (auto-saved) back |
| Run Code Health | **3** | Health → Run (folder persisted) → view |

## 6. Component inventory (prefix `Pd`)

`PdSidebar`, `PdStatusLine`, `PdEnvSwitcher`, `PdPackageTable` (virtualized), `PdPackageRow`, `PdBadge`, `PdPreviewDiff`, `PdConflictRow` (segmented 3-way), `PdDockBay` (queue), `PdConsoleDrawer`, `PdSummarySheet`, `PdSnapshotTimeline`, `PdHealthReport`, `PdLegalGate`, `PdEmptyState`, `PdOfflineBanner`.

## 7. States & feedback

- Loading: skeleton rows in surface-2, never spinners longer than 300 ms without text.
- Empty states: one mono glyph + one sentence + one action (e.g. Updates empty: `▸ all packages up to date`).
- Offline: banner chip in top bar; search works (local index), metadata panel shows cached-at timestamp.
- Errors: inline with catalog code, human message (localized), and *Details* expanding stderr tail. Toasts only for background completions.
- Destructive confirms (`Force latest`, uninstall with breakage, PEP 668 override) use `--color-danger` primary buttons and require the dialog's default focus to be **Cancel**.

## 8. Keyboard & accessibility

Full keyboard traversal: `Ctrl+1..8` sidebar tabs, `/` focuses search, `Space` toggles row selection, `Ctrl+A` select-all-visible, `Enter` primary action, `Esc` closes drawers. Contrast: all text tokens meet WCAG AA on their surfaces (verified in the design-token test); focus ring 2 px accent, never removed. Screen-reader labels on all icon buttons; live region announces execution progress summaries ("13 of 15 complete"). Windows high-contrast mode: tokens fall back to system colors.
