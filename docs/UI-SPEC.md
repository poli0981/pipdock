# PipDock — UI Specification

*Version 0.1 · 2026-07-17 · Direction: **dark terminal-tech** (owner decision 2026-07)*

## 1. Design intent

PipDock should feel like a **precision instrument docked over a terminal** — the calm confidence of a good TUI with the affordances of a native app. Not a neon cyberpunk cliché, not a generic admin dashboard: restrained phosphor-green on near-black, monospace where data lives, one accent doing real work. Signature moves: a terminal-style **status line** pinned to the bottom, a live **execution console** that streams real engine output, and a **Ctrl+K command palette** (shipped 1.1.0).

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
| `--color-danger` | `#F26A63` | impossible, failures, force actions. `#F0574F` until 1.4.0, which read at 3.92:1 on its own badge tint over a selected row |
| `--color-info` | `#5BC0DE` | pinned, informational chips |

Type: **Inter** (UI), **JetBrains Mono** (package names, versions, console, status line — both OFL-licensed, bundled). Base 14 px, mono data 13 px. Radius 6 px; borders 1 px; shadows minimal. Motion: 120–160 ms ease-out; live-activity pulse on the status-line engine badge during execution (**not built**: the status line says nothing of a running plan, only `scanning` or `idle` — audit backlog U52); `prefers-reduced-motion` respected.

**A badge is checked as what it is drawn as, not as its token** (1.4.0). A badge is its text on a tint
of its own colour — `badgeTones.ts`: 20 % for warn, info and accent, 15 % for danger — and the tint
lowers the contrast the plain token has. Danger read **3.92:1** on its own 20 % tint over a selected
row (`surface-2`), under AA, while the token test passed, because it checked tokens on plain surfaces
only. `tokens.test.ts` now composites every tone's tint over every surface and asserts AA on the
result. Both changes were needed: the new `#F26A63` at the old 20 % tint is still 4.29:1 there, and at
15 % it is 4.67:1.

**`--container-content` (87.5rem)** is the content column's widest. It is what stops a table
stretching across a maximized 4K window, now that the window itself is the user's to size (§5).

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
with its own toggle rather than teaching the status line to choose. The Security tab does the same,
since 1.2.0 and for the same reason: an audit holds a slot of its own as well.

The log toggle opens the console drawer, and is disabled when no run is in progress: a toggle that
opened an empty panel would be the same empty gesture the plain-text version was. The drawer was once
the only log surface there was. **Since 1.4.0 PipDock also writes its own log** — one file per UTC
day under `data\logs\`, kept 14 days — and Settings' Logs row (§4) says where it is and copies its
recent lines. The toggle still opens the drawer, which streams the run in progress; the file is the
record of what PipDock ran, which outlives the window. The **console drawer** slides up over the status line during execution, streaming `plan-progress` lines with per-package section markers; collapsible, never modal.

## 4. Screens

**Environments.** Auto-discovered list (source chip: registry / py launcher / uv / venv / manual) with Python version, package count and pip version; PEP 668 envs carry a red `MANAGED` chip and a line pointing at a virtual environment instead. Actions: *Use*, *Open* (the detail view below), *Rescan*, and *Upgrade pip* when pip is below the planner's floor (§5).

**This line used to promise three things the screen does not have.** A managed environment is not
*disabled*: it can be selected and read, and a change to it is refused with PD-ENV-002 by the flow —
decided on a fresh probe since 1.4.0, not on whatever the row last cached — unless the Settings
override is on. *Browse…* is still owed: `env_add_manual` is the one command left in `NOT_YET`, so an
environment is found only by discovery — the registry, the `py` launcher, `uv python list`, and a
`.venv`, `venv`, `env` or `.env` in the working directory. And there is no star-as-default: the
selection lasts for the session, and a rescan keeps it or falls back to the first usable row. The
CLI's `env use` stores a default, honoured since 1.4.0; the GUI does not read it.

**Installed.** Virtualized table: name (mono) · version · latest · size · chips. When the env's `hidden_user_site` is non-null (non-venv system Pythons only — see SECURITY §2), an inline `--color-info` note above the table says user-account packages are not listed and names the path; it is informational, never a block, and never appears for a venv. Up-to-date rows use `--color-text-dim` (owner's "dimmed" requirement); outdated rows show an `UPDATE` badge; pinned rows a 🔒 `--color-info` chip. Row actions: pin/unpin, uninstall, and **details**, as three inline buttons rather than a `⋮`
menu. A menu would spend one click opening and one choosing, leaving one of §5's three for a confirm
that has to name what breaks.

**This paragraph used to say the opposite**, and the correction is worth keeping rather than
overwriting. It said "details" had no panel to open yet, "and when it does all three fold into a
menu with the budget intact". P1-6 built the panel and the fold was wrong: it makes uninstall
`Installed → ⋮ → ✕ → Remove`, four clicks where §5's table records **three**, hand-counted, for the
second most destructive action in the application. Four is under the ceiling of 5, which is exactly
why the budget alone would have waved it through — a ceiling is not a ratchet, and a table of
measured counts is. The third button costs nothing instead: the row is the only tab stop (§8), so
its controls are a roving sequence rather than tab stops, and a third is one more arrow step, not
200 more stops. Ordered non-destructive first — details, pin, remove.

No button is a tab stop — the row is (see §8). Multi-select via checkboxes; bulk action bar appears on selection.

**The header and every row share one set of column widths** (`packageColumns.ts`, 1.4.0). The header
was a hand-written mirror of the row and had drifted: one 2rem slot stood above three action buttons,
so every fixed column's heading sat about 76 px right of its cells. And the fixed widths summed to more
than the table has at the 900 px minimum window, with the name column absorbing the whole deficit — so
at that width it showed no names at all. **A narrow table drops Size first** and tightens the rest, by
container query against the table's own width rather than the window's (Size returns at 48rem): the
plan panel or a split view changes the one without the other. Header and scroller reserve the same
scrollbar gutter, so a scrollbar appearing does not push the cells out from under their headings.

**Updates.** Same table filtered to outdated, badge count in sidebar. Header: *Select all* (excludes pinned; shows "3 pinned excluded"), *Update selected*. After resolve → **Preview panel** replaces the table: grouped sections — *Will upgrade* (A `1.2 → 2.0`), *Will downgrade*, *Will install*, *New dependencies*, then *Will break*, *Left out — needs a different Python* and *Withdrawn releases* (1.4.0, below), and last *Needs decision* (held-back in `--color-warn` with one-line blocker sentence; impossible in `--color-danger`). Each needs-decision row hosts a segmented control: **Keep compatible · Skip · Force latest** (Force opens an inline confirm naming what breaks). Footer: `Back`, *Apply choices* while decisions are pending, and `Confirm (n changes)` — `Confirm (n changes, breaks m)`, with the danger treatment, when the plan forces something. The panel's heading names the interpreter it changes, with the PEP 668 chip beside it where that applies: the panel replaces the screen it was started from, so a plan started from Search's dock bay gave no sign of which environment it was for.

*Will downgrade* was added in S3. `ChangeKind` has always had four variants and this section named three, leaving `Downgrade` with nowhere to go — and it is not a rare case: a *compatible* resolve routinely moves a package **down** to satisfy something else's constraint. Folding it in with the upgrades would have been the worst outcome, since a row reading `2.0 → 1.9` under a heading that says "Will upgrade" is actively misleading about the one change most likely to surprise. It carries `--color-warn` for the same reason.

**On impossible rows, *Keep compatible* is disabled.** UI-SPEC once said every needs-decision row hosts the full 3-way control, but `default_decision(is_impossible = true, …)` returns `Skip`, because an impossible package has no compatible version to keep — there is nothing for the control to select. The UI mirrors the core rather than offering a choice that cannot be honoured.

**The round counter is visible.** `MAX_CONFLICT_ROUNDS` is 3; when a decision round is applied the preview says how many remain, and the exhausted state keeps the preview on screen with the controls disabled rather than dead-ending. Without it a user hits the cap unwarned.

**Three sections arrived in 1.4.0, each rendering a field `ResolutionReport` had and nothing showed.**

- ***Will break (n)***, in `--color-danger`: each package forced past what held it back, as
  `name → latest`, and under it every dependent named with the constraint the forced version
  violates. *n* counts broken **dependents**, not forced packages (the heading reads *Forced (n)*
  when no blocker names one), and the same count is on Confirm — DISCLAIMER §2 requires what forcing
  breaks to be said before it runs, and a section the user has not scrolled to has not said it. There
  was nothing to show before 1.4.0: *Force latest* was recorded in the request and never read, so a
  "forced" resolve was the held-back one again.
- ***Left out — needs a different Python (n)***, in `--color-warn`: each release left out because its
  `Requires-Python` excludes this interpreter, with the range set against the interpreter's version
  (hard invariant 10). It is one of three things: an exact pin the user asked for, the latest of a
  package they chose to force, or a release the engine planned — uv ignores an upper bound — which is
  ruled out by name and the engine asked again. A request that is nothing but such pins is refused
  with PD-PKG-001; one with anything else in it plans the rest, and a plan whose only content is a
  left-out row is not "nothing to change", because the user asked for something and is owed the
  reason it is not happening.
- ***Withdrawn releases (n)***, in `--color-warn`: PD-PKG-003 for each yanked release in the plan,
  with the maintainers' reason when the engine printed one. A warning, not a refusal — both engines
  install a yanked release (SP-2). Both parsers had always extracted these; the adapters dropped them.

**Back from a preview is not a finished run** (1.4.0). The selection is kept and nothing is
refetched, where it used to be cleared as if something had changed. Leaving work the backend still
holds — Back during a resolve, or out of a parked preview — stops it there (`plan_cancel`), and a late
answer no longer re-opens the panel the user closed. A decision round that fails leaves the preview
usable, because the flow stays parked; its Confirm used to answer "no plan".

**After a run the panel shows the summary** (`PdSummarySheet`): the counts, a row per package, the
snapshot with *Roll back to this*, and what the post-run check found. Since 1.4.0 each row says what
it moved *from* — the engines cannot know it, so every upgrade used to read as a fresh install — and
a check that could not run **says so** ("could not check the environment afterwards", with its code)
instead of rendering nothing, which read as a pass in exactly the case nothing was verified: a run
that broke pip itself.

**Search.** Search field autofocused; results stream under 50 ms per keystroke from the local index. Result row: name · summary (from cached metadata) · latest · `INSTALLED ✓`/`UPDATE` chip when applicable · **[Add]**. Right detail panel: description, requires-python, license, links. Queue ("**dock bay**") docks along the right edge as a slim column of added packages with editable version fields and `Install (n)`.

*Install (n)* keeps the queue until the resolve has answered (1.4.0): a plan already in flight, or an
index that could not be reached, used to cost the user the list they had just built. Once there is a
preview, the preview is the list. A search typed while the index is still warming is asked again when
it is ready, rather than waiting for another keystroke. The field's focus on arrival is the shell's
job — see §8's `data-autofocus`.

**Pins.** Pin list with reason field, committed on blur; unpin per row. An `Exclude` pin keeps its
package out of **every** update, a named one included — DATA-FLOW §9.5, enforced by
`pins::filter_upgrades` — not only out of *Select all*. **Exclude pins only**: a `Hold` pin is
rendered faithfully if one exists, but creating one is not offered, here or by the CLI, whose
`pin add` writes `Exclude` and nothing else. The reason this paragraph gave until 1.4.0 has gone:
`pins::hold_requirements` had no caller, so a hold was a promise nothing kept, and since 1.4.0 every
resolve restates a held package at its version. A hold is now honoured if one exists; nothing makes
one. With no environment selected the screen says so rather than "no pins": pins are keyed by
`env_hash`, so that is not an empty list.

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

**Dependencies** (surfaced under Installed → row → details): the focused package between two
columns — what requires it, and what it requires — each row carrying the specifier written on that
edge, and each clickable to re-centre. Below them, the two transitive counts: how many packages a
removal would leave with a missing dependency, and how many the package pulls in itself. A
requirement that nothing installed satisfies is called out; nothing else in PipDock reports one.
Platform markers are read since 1.4.0 — probe schema 3 reports the marker environment — so a
requirement gated to another platform, such as ipython's `pexpect; sys_platform != "win32"`, is no
longer reported missing from every Windows environment.

**Drawn at depth 1, and depth is navigated rather than drawn.** Measured on the 352-package fixture
before the screen was designed: a depth-1 neighbourhood is a median of **7** nodes and a p90 of 25,
while depth 2 is a median of **172** and exceeds 60 nodes for **212 of 352 packages**. No layout
algorithm makes 172 nodes legible, so the node-link diagram PRD P1-6's wording suggests would have
cost an eleventh runtime dependency, a `THIRD-PARTY-NOTICES.md` edit that re-shows the legal gate,
and a CSP argument — `tauri.conf.json` sets no `script-src`, so anything reaching for `eval` is
refused — to buy a picture nobody could read. Paths are not offered at all: `setuptools` has more
than **2000** distinct paths to a root in that environment, so "why is this here" has no
path-shaped answer at this scale, and the honest answer is a count.

Like Snapshots, it is a **mode** rather than a tab, for the reason below: `NAV_KEYS` is already
nine, and a Dependencies entry would have to sit beside Installed to read as related — an insert.

Columns are capped at `DEPS_ROWS_SHOWN` with the overflow counted from the **full** set, the
`SUGGESTIONS_SHOWN` rule for its reason: `setuptools` has 150 dependents, and a column of 150 is a
second package list. Nothing is carried by colour — §8's `forced-colors` block collapses `warn`,
`danger` and `info` to `CanvasText`, so every specifier is present as text.

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

**Health.** Project-folder picker (persisted per env — on disk, through `EnvRow.healthProject`; on screen the folder is taken from the row only while none is set, so after a switch it stays the previous environment's for the rest of the session, audit backlog U16) → run panel with three result tabs (deptry / vulture / ruff) per CODE-HEALTH-SPEC; `Fix with ruff` gated behind explicit confirm listing file count.

**Security.** *Run audit* → advisories grouped by package, each with its id, the versions that fix
it (or **no fix**, stated rather than left blank), its aliases — which is where the CVE lives, since
pip-audit's primary id is a `PYSEC-*` — the advisory prose verbatim, and a link to its OSV entry.
**No severity column**: pip-audit publishes none under either vulnerability service, so the order is
package → fixable → id, and inventing a severity would put a number PipDock made up beside real
ones (PRD P1-1 is amended to say so). A *Cancel* appears while a run is going, which Code Health has
no equivalent of: an audit is 18-68 s against Health's 1.3 s. **The body says nothing while a run is
in flight** — not "no audit has run", which is what it said until someone clicked Run against a slow
bridge and read it.

**It scrolls** (1.4.0). The screen's root was a bare `<div>` inside a `<main>` that clips, so a report
taller than the window could not be reached at all; it is now one scroll container with its console
drawer pinned beneath, as Health's is. *Cancel* is honoured from the start as well: the cancellation
token used to be registered after the environment's freeze, so a Cancel during it did nothing, and a
run stopped that early now skips the audit venv's sync and reports itself cancelled.

**Settings.** In screen order: engine (pip/uv radio, each with its version for the selected environment), language (en/vi), **pin auto-suggest count** (P1-A; zero turns suggestions off), PEP 668 override (off by default, scary copy), Disk usage, Logs. Every control saves on change — the override, once its confirm is answered — which is what keeps *Switch engine* at three clicks (§5).

**This line used to list two settings that do not exist.** There is no snapshot retention — snapshots
are kept until cleared under Disk usage — and no index-refresh setting: the index is downloaded from
Search when it is missing, or by `pipdock index refresh`, and its seven-day staleness is reported,
never acted on. *Open logs folder* became the Logs row below.

**Each engine's version is beside its radio** (1.4.0), for the selected environment — pip's comes from
`<python> -m pip --version`, so there is no answer without one — and an engine that is not installed
reads *not found*. `engine_info` had a typed wrapper and no caller, so nothing said uv was missing
until a plan failed for want of it. The answer is keyed by interpreter, so switching environments
never shows one's versions under another. Nothing preselects uv yet: a fresh install drives pip, and
the first-run detection ARCHITECTURE §3 specified is owed (PRD P0-9). `settings.rs` puts it here, as a
choice made where it can be seen, rather than in a read of an empty store.

**The language is stored** (1.4.0) as the user's *choice*, `null` meaning "follow the OS", and applied
at start. `save()` sent `locale: null` every time, so choosing Vietnamese lasted until the window
closed. Settings now load when the app starts, not only when this screen mounts, and before the
legal gate — so the gate is in the chosen language too, and the status line's engine and the PEP 668
chip are right before anyone has opened Settings.

**Turning the PEP 668 override on asks first** (1.4.0, SECURITY §3): a `PdDialog` with Cancel focused,
saying that `--break-system-packages` will be passed for every environment marked
`EXTERNALLY-MANAGED` and that rolling back to a snapshot may not repair what that breaks. Turning it
off saves at once — restoring protection never needs a second thought. Until 1.4.0 the override was
one click on a checkbox that saved and **nothing read**; every mutating flow now probes first, decides
on what the probe says, and carries the override to the engine call only when it is on (hard
invariant 5).

**While the override applies, a chip says so** — `PdOverrideChip`, SECURITY §3's persistent warning —
on Installed and Updates, Search's dock bay, the plan panel and an environment's detail. *Applies*
means the setting is on **and** the environment carries the `EXTERNALLY-MANAGED` marker. A venv never
does, and a chip there would claim the flag is being sent when it is not.

**Disk usage** (P1-C) shows what PipDock has written — snapshots, the Code Health tools venv and, since 1.2.0, the Security tab's audit venv, each with a *Clear*, and `index.db` reported without one because it holds settings, pins and the consent record in the same file. Since 1.4.0 a *Clear* is refused with PD-RES-003 while the feature that owns its target is running — Health for the tools venv, an audit for the audit venv, a plan for the snapshots — rather than deleting the venv a run is executing from; a preview that is merely parked is put back afterwards. **Logs are not a row.** This paragraph once said there were no log files and that a "logs — 0 B" row would invent one. Since 1.4.0 there are, and they look after themselves, so a *Clear* would only take away the record a bug report needs.

**Logs** (1.4.0) shows the log folder as a path — data, never translated (I18N §2) — and *Copy recent
log*, which re-reads the tail at the moment of copying, so the clipboard holds the run that just went
wrong rather than what the log said when the screen opened. The folder is shown rather than opened:
the opener is scoped to four `https` hosts and nothing local, and the copy uses the clipboard
permission *Copy full log* already had, so no capability was widened for the row.

The threshold is the app's **first numeric input**, and it sets the rule for the next one: reject at the boundary, never store junk. `<input type="number">` reports `''` for anything it cannot parse, so coercing the raw value writes `0` — which here is a meaningful setting, "off", that the user did not ask for. Its label avoids the word *threshold* on purpose; see `PdPins.test.tsx`'s Hold assertion, which matches `/hold/i`.

**Legal & About left Settings** in Phase 4 and became the About tab below. Neither is a setting: they are read-only surfaces, and folding them into a screen of controls is what kept them unbuilt for four milestones while every control around them shipped.

**Legal gate (first run).** Modal listing the five documents with GitHub links and a summarized disclaimer sentence; single checkbox "I have read and accept" → `Continue`. Decline exits the app. Consent stored with docs-version hash; hash bump re-triggers the gate.

**Decline has exited the app only since 1.4.0.** The webview was granted `core:default` — some sixty
core commands nothing called — and not `core:window:allow-close`, so in the installed app Decline did
nothing, while its component test, which mocks `close()`, passed. `capabilities/default.json` now
holds exactly what the frontend calls, and a test in `lib.rs` pins the union of every capability
file. **A consent that cannot be read or recorded is shown**, as an error row on the gate; the failure
used to be stored and never rendered, so *Continue* looked like a button that did nothing. And the
hash ignores line endings: a build from a CRLF checkout used to carry a different hash from an LF
build of the same commit, so two builds of one commit could disagree about whether the documents had
changed.

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
| Run a security audit | **2** | Security → *Run audit*. Counted in the running app against a stubbed bridge. The first run also builds the audit environment, which is a wait rather than a click. |
| See what depends on a package | **2** | Installed → row `⇄` (Installed is not the landing screen, so its tab click counts). Re-centring on a neighbour is one more click and no fetch — the graph is one value per environment. |
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

**The window maximizes and snaps; the content column does not stretch.** Until 1.4.0 the window
was bounded at 1600×1100 and could not be maximized, because the layout — a fixed sidebar plus one
scrolling column — stretched across a 4K panel is a column of text beside a metre of empty surface.
That answered the right worry with the wrong control: the column now stops at `max-w-content`
(87.5rem) and centres in what is left, and the window is the user's to size. Fullscreen stays off.

**The window opens at a size that fits.** 1180×760 is taller than the work area of a 1920×1080 panel
at 150 % and of a 1366×768 laptop, so the status line and the plan panel's Confirm opened under the
taskbar. The window is created hidden and `src-tauri/src/window.rs` shrinks it to the monitor's work
area — never below `minWidth`/`minHeight`, 900×560 — centres it there, then shows it.

## 6. Component inventory (prefix `Pd`)

`PdSidebar`, `PdStatusLine`, `PdEnvSwitcher`, `PdPackageTable` (virtualized), `PdPackageRow`, `PdBadge`, `PdPreviewDiff`, `PdConflictRow` (segmented 3-way), `PdDockBay` (queue), `PdConsoleDrawer`, `PdSummarySheet`, `PdSnapshotTimeline`, `PdHealthReport`, `PdLegalGate`, `PdEmptyState`, `PdOfflineBanner`.

**15 of those 16 exist** as of Phase 3 · P4, `PdHealthReport` having arrived with the Health screen. Absent:
`PdEnvSwitcher` alone, which nothing needs while the header shows the selected interpreter and the
Environments tab switches it. (ROADMAP and
CLAUDE.md both claimed 16 of 16 after S4; the count had quietly folded in components that are not on
this list.) **Six** components not on this list have
turned out to be load-bearing and are listed here so the inventory stays honest — the number said
*four* while naming five for two milestones, which is the counting trap this section exists to
avoid: `PdErrorRow` (ERROR-CATALOG §3's row, used by every error surface), `PdDialog` (the shared
modal §7's destructive confirms need), `PdUninstallDialog` (§5's three options), `PdPinChip` (the
🔒 chip, shared by the table and the Pins screen), `PdRollbackPreview` (DATA-FLOW §8's preview,
including its `PD-SNP-002` list) and `PdAuditReport` (P1-1's advisory list). Three more shipped in
1.1.0 and are named nowhere in this section: `PdCacheUsage`, `PdPalette` and `PdRequirements`.
`PdDepsFocus` (1.3.0, PRD P1-6's focus view) makes **ten** off-list components; it is written here
in the same breath as the count so the two cannot drift apart again. 1.4.0 adds `PdOverrideChip`
(SECURITY §3's chip, §4) and `PdLogs` (Settings' Logs row), which makes **twelve** — and 15 + 12 is
the 27 `Pd*` components in `ui/src/components/`, which is the check to run rather than trusting
either number. Screens (`ui/src/screens/`, `PdPlanPanel` among them) are not counted.

## 7. States & feedback

- Loading: skeleton rows in surface-2, never spinners longer than 300 ms without text.
- Empty states: one mono glyph + one sentence + one action (e.g. Updates empty: `▸ all packages up to date`).
- Offline: banner chip in top bar; search works (local index), metadata panel shows cached-at timestamp.
- Errors: inline with catalog code, human message (localized), and *Details* expanding stderr tail. Toasts only for background completions. *Report bug* opens a prefilled GitHub issue that nothing sends until the user submits it; since 1.4.0 the link is rebuilt at the click with the selected environment, so the issue carries its Python version and the configured engine's version — one subprocess per report, rather than one per error row mounted, which a summary of many failures would multiply.
- Destructive confirms (`Force latest`, uninstall with breakage, PEP 668 override, and — since P5 — `Fix with ruff`, the only one that writes outside an environment) use `--color-danger` primary buttons and require the dialog's default focus to be **Cancel**. The PEP 668 override was on this list while it was a bare checkbox with no confirm at all; it has been a `PdDialog` since 1.4.0. A `data-action` in `styles.css`'s forced-colors list gives a destructive control a 2 px border, because high contrast drops every tint and a destructive primary that looks like Cancel is worse than no colour at all. **The list is `force` (the guard's *Remove anyway*), `uninstall`, `rollback`, `upgrade-pip` and `fix`, and it is not complete:** the uninstall dialog's plain *Remove* (`remove`), the *Force latest* inline confirm's button, the override's *Allow* (`allow-override`), Disk usage's *Clear* confirm (`confirm-clear-cache`) and the plan panel's danger-treated Confirm (a rollback, or since 1.4.0 a forced plan) are not in it, so in high contrast each is told from the control beside it by its label alone. The active sidebar tab has the same problem by another route: it carries `aria-current="page"`, and the outline rule matches `'true'`. All of it is audit backlog U45, deferred to a High Contrast pass done by hand.

## 8. Keyboard & accessibility

Full keyboard traversal: `Ctrl+1..9` sidebar tabs (by physical key), **`Ctrl+K` opens the command palette**, `/` focuses the Search field, `Space` toggles row selection, `Ctrl+A` select-all-visible, `Enter` primary action, `Esc` closes drawers, and **`←`/`→` move within a focused row** (P6).

**Four rules from 1.4.0:**

- **`Ctrl+1..9` match the physical key** — `e.code`, `Digit1..9` or `Numpad1..9` — not the character
  the layout types. On AZERTY, Ctrl+1 reports `&`, so the tab shortcuts did nothing on every French
  keyboard. `Ctrl+K` still reads `e.key`, and that is an open question rather than an oversight: for
  a letter the physical key is the wrong answer on Dvorak (audit backlog U49).
- **`/` focuses the Search field, from Search itself too.** There `nav` does not change, so the focus
  effect did not run and `/` did nothing on the one screen it names. It is ignored while typing in a
  text field, so a version specifier can still contain a slash.
- **A screen can name its first focus stop with `data-autofocus`**, and that wins over the heading the
  shell otherwise focuses on every tab change. §5's four-click install depends on Search's field
  having focus on arrival, and it lost that focus every time to the shell's effect, which runs after
  its children's.
- **A release build swallows the browser's own shortcuts** (`browserKeys.ts`): F5 and Ctrl+R
  (reload), Ctrl+P (print), Ctrl+F, F3 and Ctrl+G (find), Ctrl+U (source), Ctrl+S (save), Alt+←/→, and
  the hardware browser keys for back, forward and refresh. WebView2 is a browser and kept them: a
  reload dropped a preview mid-decision along with everything the stores had not persisted, and find
  opened over a virtualized table whose off-screen rows are not in the DOM. The listener is in the
  capture phase and above the legal gate, so no screen or modal lets one through. Development keeps
  them, because a reload is how a developer recovers from a bad hot update. **Right-click is not
  covered:** WebView2's context menu still offers Reload, Back and Print in a release build, and
  removing it would take Copy and Paste out of every text field with it — a decision, not yet made.

A dialog — `PdDialog`, and the palette — returns focus to whatever opened it when it closes; it used
to fall to `<body>`, and a keyboard user lost their place in a 200-row table. The package grid itself
is focusable but not a tab stop: the row is.

**The arrow keys are not decoration, and the gap they close was real.** Making the row the only tab stop is what keeps 200 rows from becoming 600 — but every control inside one is `tabIndex={-1}`, and that left *Pin* and *Remove* reachable by mouse only: a WCAG 2.1.1 failure, found by tabbing to a row in the running app and discovering there was nowhere further to go. `←`/`→` are the ARIA grid pattern's roving tabindex, `Esc` returns focus to the row, and no tab stops are added.

**`Enter` on a row is the *non-destructive* primary.** On a package row that is pin, never uninstall: a row has one reversible annotation and one irreversible removal, and a destructive operation one keypress from a focused row is an accident waiting rather than a primary action. Everywhere else in the app `Enter` needs no binding at all, because every other primary action is a real `<button>`.

**"Select-all-visible" means the current filtered set, not the rendered window.** Resolved in S2: the package table is virtualized, so only ~25 of a 200-package environment are in the DOM at any moment and "visible" would otherwise mean whatever the scroll position happens to be. `Ctrl+A` selects every *selectable* row in the set the screen is showing — so on Updates that is the outdated rows, and pinned rows are excluded there as they are from the *Select all* button. The row, not its checkbox, is the tab stop and the target of `Space`; tabbing through two controls per row is not traversal at 200 rows. Contrast: all text tokens meet WCAG AA on their surfaces, and since 1.4.0 so does every badge's text on its own tint over every surface (§2; both verified in the design-token test); focus ring 2 px accent, never removed. Screen-reader labels on all icon buttons; live region announces execution progress summaries ("13 of 15 complete"). Windows high-contrast mode: tokens fall back to system colors.
