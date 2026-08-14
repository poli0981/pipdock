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
│ About               │                                        │
├───────────┴──────────────────────────────────────────────────┤
│ ▸ .venv · Py 3.12.4 · uv 0.9.x · idle            log ⌄  ⚠ 0  │  status line
└──────────────────────────────────────────────────────────────┘
```

Sidebar is icon+label, collapsible to icons. Status line (mono) always shows env · python · engine · state, plus a log-drawer toggle.

**`⚠ n` is the number of error rows currently on screen** — defined in S7, having been drawn in this
diagram and specified nowhere since. It is deliberately *not* a session tally: that would read
`⚠ 47` after a batch where 47 packages failed, stay there, and tell the user nothing the summary in
front of them does not. Counting live rows means it returns to zero when the problems do, so the
number and the screen always agree. `PdSummarySheet`'s per-package rows opt out — one failed run is
one problem, not forty-seven. **This definition is an invention**; nothing specified it.

**Code Health's failed tools count** (P4). A `HealthReport` carries at most three `problems`, one
per tool, and each renders as a `PdErrorRow` above the tabs — so they are error rows currently on
screen and the rule applies unchanged. This is not the `PdSummarySheet` case: three separable
tools are three problems, where forty-seven failed packages are one run. They render above the
tabs rather than inside them so the explanation is never reachable only from the tab it explains.

**Health owns its own console drawer**, and that is a deviation worth naming. The drawer above is
specified against `plan-progress`, and the status line's toggle is disabled unless a plan is
running. Health claims a *separate* session slot by design — a health run and an install can be in
flight at once — so one toggle cannot address two drawers. `PdHealth` renders its own instance
with its own toggle rather than teaching the status line to choose.

The log toggle opens the console drawer, and is disabled when no run is in progress. Until M3's
logging subsystem exists the drawer is the only log surface there is, and a toggle that opened an
empty panel would be the same empty gesture the plain-text version was. The **console drawer** slides up over the status line during execution, streaming `plan-progress` lines with per-package section markers; collapsible, never modal.

## 4. Screens

**Environments.** Auto-discovered list (source chip: registry / py launcher / uv / manual) with python version and package count; PEP 668 envs carry a red `MANAGED` chip and a disabled state explaining PD-ENV-002. Actions: *Browse…*, *Rescan*, star-as-default.

**Installed.** Virtualized table: name (mono) · version · latest · size · chips. When the env's `hidden_user_site` is non-null (non-venv system Pythons only — see SECURITY §2), an inline `--color-info` note above the table says user-account packages are not listed and names the path; it is informational, never a block, and never appears for a venv. Up-to-date rows use `--color-text-dim` (owner's "dimmed" requirement); outdated rows show an `UPDATE` badge; pinned rows a 🔒 `--color-info` chip. Row actions: pin/unpin and uninstall, as two inline buttons rather than a `⋮` menu. A menu would
spend one click opening and one choosing, leaving one of §5's three for a confirm that has to name
what breaks; "details" has no panel to open yet, and when it does all three fold into a menu with the
budget intact. Neither button is a tab stop — the row is (see §8). Multi-select via checkboxes; bulk action bar appears on selection.

**Updates.** Same table filtered to outdated, badge count in sidebar. Header: *Select all* (excludes pinned; shows "3 pinned excluded"), *Update selected*. After resolve → **Preview panel** replaces the table: grouped sections — *Will upgrade* (A `1.2 → 2.0`), *Will downgrade*, *Will install*, *New dependencies*, *Needs decision* (held-back in `--color-warn` with one-line blocker sentence; impossible in `--color-danger`). Each needs-decision row hosts a segmented control: **Keep compatible · Skip · Force latest** (Force opens an inline confirm naming what breaks). Footer: `Confirm (n changes)` / `Back`.

*Will downgrade* was added in S3. `ChangeKind` has always had four variants and this section named three, leaving `Downgrade` with nowhere to go — and it is not a rare case: a *compatible* resolve routinely moves a package **down** to satisfy something else's constraint. Folding it in with the upgrades would have been the worst outcome, since a row reading `2.0 → 1.9` under a heading that says "Will upgrade" is actively misleading about the one change most likely to surprise. It carries `--color-warn` for the same reason.

**On impossible rows, *Keep compatible* is disabled.** UI-SPEC once said every needs-decision row hosts the full 3-way control, but `default_decision(is_impossible = true, …)` returns `Skip`, because an impossible package has no compatible version to keep — there is nothing for the control to select. The UI mirrors the core rather than offering a choice that cannot be honoured.

**The round counter is visible.** `MAX_CONFLICT_ROUNDS` is 3; when a decision round is applied the preview says how many remain, and the exhausted state keeps the preview on screen with the controls disabled rather than dead-ending. Without it a user hits the cap unwarned.

**Search.** Search field autofocused; results stream under 50 ms per keystroke from the local index. Result row: name · summary (from cached metadata) · latest · `INSTALLED ✓`/`UPDATE` chip when applicable · **[Add]**. Right detail panel: description, requires-python, license, links. Queue ("**dock bay**") docks along the right edge as a slim column of added packages with editable version fields and `Install (n)`.

**Pins.** Pin list with reason field, committed on blur; unpin per row. **Exclude pins only**: a
`Hold` pin is rendered faithfully if one exists, but creating one is not offered, because
`pins::hold_requirements` has no caller and `engine::plan_requirements` restates every package at its
*installed* version — a hold at any other version is a promise nothing keeps, and the CLI cannot
create one either. With no environment selected the screen says so rather than "no pins": pins are
keyed by `env_hash`, so that is not an empty list.

**The auto-suggest section** — "`urllib3` — 12 packages depend on it. Pin?" — landed post-1.0 as
P1-A. It sits *above* the pin list, because it is a prompt to act and the list is a record of
having acted, and it is absent entirely when there is nothing to suggest. Accepting writes an
ordinary `Exclude` pin with the count as its reason (PRD P1-2's "suggest pin with reason"), which
the reason field below can then replace.

**It shows the top 8 and says how many more qualify.** Measured against the 352-package fixture,
the default threshold of 5 qualifies 94 — a quarter of the environment. Raising the default would
be the wrong fix, because a deep dependency tree is exactly where a bulk update is most dangerous;
the count in the "N more" line comes from the full list, so a capped view never misreports a total.
Same rule as `RUFF_ROWS_SHOWN` in the Health report.

It costs one `probe.py` run, and that is why it is here rather than on a sidebar badge: only
someone who opened this tab pays for it. It is also **silent on failure** — this screen's job is
listing pins and it does that either way, so an error row about an advisory would be the loudest
thing on it. Counts come from `graph::dependent_count`, which counts in-force edges only: the same
set the uninstall guard warns about, because a user pinning on one rule and being warned on another
is the failure worth designing against.

**Snapshots** (surfaced under Environments → env detail): timeline of snapshots with trigger label, diff viewer (added/removed/changed in mono), `Rollback…` with its own preview per DATA-FLOW §8.

The detail view is a **mode of the Environments tab**, not a sidebar entry of its own. The reason is
adjacency, not arity: §8's shortcuts are *positional* over `NAV_KEYS`, so **appending is free and
inserting is not** — Phase 4 put About on the end and it became `Ctrl+9` without moving a single
existing binding, while a Snapshots entry would have to sit *beside* Environments to read as
related, renumbering everything after it. Which
environment is open lives in `useEnvStore`, because the plan panel replaces the whole content area
while a rollback runs and component state would be unmounted with it.

The **trigger label is load-bearing**, not decoration. A restore snapshots the current state before
restoring, so one rollback moves `latest` twice and two entries a minute apart are otherwise
indistinguishable — while one is the state the user wants back and the other is the state they are
leaving. Nothing in the UI ever names `latest`; the timeline hands ids to every command.

An environment whose interpreter is gone still reaches the detail view: snapshots are keyed by
`env_hash` and outlive the Python that made them, so the timeline lists while the diff and the
rollback are disabled with a line saying why. The diff's copy names the *direction* — "here but not
in the snapshot — restoring removes it" — because "added" is ambiguous the moment you read it from
the snapshot's side.

**Health.** Project-folder picker (persisted per env) → run panel with three result tabs (deptry / vulture / ruff) per CODE-HEALTH-SPEC; `Fix with ruff` gated behind explicit confirm listing file count.

**Settings.** Engine (pip/uv radio + detected versions), locale (EN/VI), **pin auto-suggest count** (P1-A; zero turns suggestions off), snapshot retention, index refresh, PEP 668 override (off by default, scary copy), *Open logs folder*.

The threshold is the app's **first numeric input**, and it sets the rule for the next one: reject at the boundary, never store junk. `<input type="number">` reports `''` for anything it cannot parse, so coercing the raw value writes `0` — which here is a meaningful setting, "off", that the user did not ask for. Its label avoids the word *threshold* on purpose; see `PdPins.test.tsx`'s Hold assertion, which matches `/hold/i`.

**Legal & About left Settings** in Phase 4 and became the About tab below. Neither is a setting: they are read-only surfaces, and folding them into a screen of controls is what kept them unbuilt for four milestones while every control around them shipped.

**Legal gate (first run).** Modal listing the five documents with GitHub links and a summarized disclaimer sentence; single checkbox "I have read and accept" → `Continue`. Decline exits the app. Consent stored with docs-version hash; hash bump re-triggers the gate.

It has a second, **presentational** mode since Phase 4: About re-opens it so the documents can be read again, and in that mode the checkbox and both buttons are replaced by a single *Close*. Consent is neither read nor written. This is a flag of its own (`useLegalStore.review`) rather than setting `accepted = false`, which would have been one line and would have made a review indistinguishable from a revocation — offering Accept and a Decline that closes the application to someone who only wanted to re-read their privacy policy.

**About.** The ninth tab (`Ctrl+9`): what PipDock is; this build — version from `app_info`, the SPDX identifier `GPL-3.0-only`, and the legal-documents hash consent is recorded against; a button that puts the gate back on screen; and contact — two addresses and the author's links page. The addresses are **copied to the clipboard, not opened**: `mailto:` would widen the opener capability by a whole URL scheme, and SECURITY §4 asks for a reason per widening. They are also rendered from a TypeScript constant rather than the catalogs, because an address is data (I18N §2) and a catalog invites a translator to change it. Only `https://poli0981.dev/*` was added to the allowlist, for the links page.

## 5. Click budgets (owner ceiling: 5)

Every click counts, including the one that opens the tab — except on Environments, which is where the
app lands. Counted by hand in the running app, never inferred from the markup.

| Flow | Clicks | Path |
|---|---|---|
| Update everything | **4** | Updates → Select all → Update → Confirm (app auto-scans on env open) |
| Update everything, 1 conflict kept compatible | **4** | default choice needs no extra click |
| Install one package | **4** + typing | Search (autofocus) → result [Add] → Install → Confirm |
| Uninstall one package | **3** | Installed → row ✕ → *Remove* (4 when the guard trips: → *Remove the dependents too* → *Remove*) |
| Rollback last snapshot | **4** | Environments → *Open* → timeline entry → *Rollback…* → *Roll back* (2 from a run's summary, via *Roll back to this*) |
| Switch engine | **3** | Settings → engine radio → (auto-saved) back |
| Run Code Health | **3** | Health → Run (folder persisted) → view (**4** the first time in a project: Health → *Choose folder…* → the OS dialog, which is not PipDock's click → Run → view. The folder is remembered per environment from then on.) |
| Fix ruff findings | **3** | ruff tab → *Fix N…* → *Fix* (from a report already on screen; **5** from the landing screen with a folder remembered, which is the owner's ceiling). Cancel holds default focus, so `Enter` without `Tab` cancels. |
| Upgrade pip | **2** | *Upgrade pip* → *Upgrade* (Environments is the landing screen, so no tab click). The button appears only when pip is below the 22.2 planner floor — the case the ordinary Updates path cannot fix, because the planner it needs will not run. |

### Shell rules established by the first installed build (2026-08-13)

**One window, always.** `tauri-plugin-single-instance` is registered before every other plugin; a
second launch unminimizes and focuses the window that exists. Five launches used to give five
windows, which is five `AppState`s over one machine — five `Sessions` slots each certain it owned
the mutation in flight, and five `PD-RES-003` guards that cannot see each other. Verified: five
launches, one process, one window.

**The sidebar is disabled while `PANEL_PHASES` owns the content area.** Navigating away left the
plan parked in Rust with nothing driving it, and the next Update answered `PD-RES-003` about a plan
the user could no longer see. §8's `Ctrl+1..9` is the same rule for the keyboard.

**And it did not, until Phase 4.** This paragraph said the shortcuts "already refuse" for two
milestones while they did not: `App.tsx` returned early on the guard dialog alone, and
`PANEL_PHASES` *excludes* `guard` on purpose, so the two sets are disjoint and guarding on one
guarded neither. Nothing looked wrong — the sidebar was disabled and the panel kept rendering —
until the plan finished and dropped the user on a tab they never chose. The shortcuts now also
refuse while the legal gate is on screen, which matters once About can re-open it.

**The window is bounded at 1600×1100 and cannot be maximized or go fullscreen.** The layout is a
fixed sidebar plus one scrolling column; stretched across a 4K panel it is a column of text beside
a metre of empty surface. Still resizable, and `minWidth`/`minHeight` are unchanged.

## 6. Component inventory (prefix `Pd`)

`PdSidebar`, `PdStatusLine`, `PdEnvSwitcher`, `PdPackageTable` (virtualized), `PdPackageRow`, `PdBadge`, `PdPreviewDiff`, `PdConflictRow` (segmented 3-way), `PdDockBay` (queue), `PdConsoleDrawer`, `PdSummarySheet`, `PdSnapshotTimeline`, `PdHealthReport`, `PdLegalGate`, `PdEmptyState`, `PdOfflineBanner`.

**15 of those 16 exist** as of Phase 3 · P4, `PdHealthReport` having arrived with the Health screen. Absent:
`PdEnvSwitcher` alone, which nothing needs while the header shows the selected interpreter and the
Environments tab switches it. (ROADMAP and
CLAUDE.md both claimed 16 of 16 after S4; the count had quietly folded in components that are not on
this list.) Four components not on this list have
turned out to be load-bearing and are listed here so the inventory stays honest: `PdErrorRow`
(ERROR-CATALOG §3's row, used by every error surface), `PdDialog` (the shared modal §7's destructive
confirms need), `PdUninstallDialog` (§5's three options), `PdPinChip` (the 🔒 chip, shared by the
table and the Pins screen) and `PdRollbackPreview` (DATA-FLOW §8's preview, including its
`PD-SNP-002` list).

## 7. States & feedback

- Loading: skeleton rows in surface-2, never spinners longer than 300 ms without text.
- Empty states: one mono glyph + one sentence + one action (e.g. Updates empty: `▸ all packages up to date`).
- Offline: banner chip in top bar; search works (local index), metadata panel shows cached-at timestamp.
- Errors: inline with catalog code, human message (localized), and *Details* expanding stderr tail. Toasts only for background completions.
- Destructive confirms (`Force latest`, uninstall with breakage, PEP 668 override, and — since P5 — `Fix with ruff`, the only one that writes outside an environment) use `--color-danger` primary buttons and require the dialog's default focus to be **Cancel**. Each also carries a `data-action` in `styles.css`'s forced-colors list, because high contrast drops every tint and a destructive primary that looks like Cancel is worse than no colour at all.

## 8. Keyboard & accessibility

Full keyboard traversal: `Ctrl+1..9` sidebar tabs, `/` focuses search, `Space` toggles row selection, `Ctrl+A` select-all-visible, `Enter` primary action, `Esc` closes drawers, and **`←`/`→` move within a focused row** (P6).

**The arrow keys are not decoration, and the gap they close was real.** Making the row the only tab stop is what keeps 200 rows from becoming 600 — but every control inside one is `tabIndex={-1}`, and that left *Pin* and *Remove* reachable by mouse only: a WCAG 2.1.1 failure, found by tabbing to a row in the running app and discovering there was nowhere further to go. `←`/`→` are the ARIA grid pattern's roving tabindex, `Esc` returns focus to the row, and no tab stops are added.

**`Enter` on a row is the *non-destructive* primary.** On a package row that is pin, never uninstall: a row has one reversible annotation and one irreversible removal, and a destructive operation one keypress from a focused row is an accident waiting rather than a primary action. Everywhere else in the app `Enter` needs no binding at all, because every other primary action is a real `<button>`.

**"Select-all-visible" means the current filtered set, not the rendered window.** Resolved in S2: the package table is virtualized, so only ~25 of a 200-package environment are in the DOM at any moment and "visible" would otherwise mean whatever the scroll position happens to be. `Ctrl+A` selects every *selectable* row in the set the screen is showing — so on Updates that is the outdated rows, and pinned rows are excluded there as they are from the *Select all* button. The row, not its checkbox, is the tab stop and the target of `Space`; tabbing through two controls per row is not traversal at 200 rows. Contrast: all text tokens meet WCAG AA on their surfaces (verified in the design-token test); focus ring 2 px accent, never removed. Screen-reader labels on all icon buttons; live region announces execution progress summaries ("13 of 15 complete"). Windows high-contrast mode: tokens fall back to system colors.
