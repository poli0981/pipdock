# PipDock — Data Flow

*Version 0.1 · 2026-07-17. This document is the corrected, authoritative version of the original concept flows.*

## 1. Correction log vs. original concept

| Original concept | Corrected design | Why |
|---|---|---|
| "Check Conflict → If True → update" | **No conflict → update.** Conflict → explain → 3-way choice | Original condition was inverted; binary Yes/No hid the safe middle option |
| Per-package conflict check | **Group dry-run resolve** of the whole selection | Resolving A+B+D together often succeeds where one-by-one is stuck; matches how pip's resolver actually works |
| "Show the full list of all Python packages" | **Search-first** over a locally cached PyPI name index | PyPI hosts ~600 k projects; a browsable list is unusable and unfetchable |
| (absent) | **Environment selection + PEP 668 guard** as step zero | Without it, users mutate system Python by accident — the worst possible outcome |
| (absent) | **Snapshot before every batch** | Makes every flow reversible; prerequisite for user trust |

## 2. Shared preamble (all mutating flows)

```text
[Select env] → probe.py -X utf8 -I → env facts (python ver, PEP 508 markers, PEP 668,
     │                                hidden_user_site, dist metadata)
     │    probed afresh by every mutating flow's start (flow::preamble); the PEP 668
     │    answer comes from this probe, never from the PyEnv the caller sent
     ├─ EXTERNALLY-MANAGED detected → BLOCKED banner (PD-ENV-002), before any engine command
     │        └─ explicit override in Settings → proceed with persistent warning chip;
     │           SystemPackages::OverrideAllowed → --break-system-packages on every
     │           mutating engine call, and on uv's dry run (pip's dry run never);
     │           pip 23.0 → still PD-ENV-002 (no flag exists); pip < 23.0 → no flag
     ├─ hidden_user_site non-null → Installed screen shows the partial-listing note
     │                              (not a block; see SECURITY §2)
     └─ ok → flow continues; reverse-dep graph built from probe data, its markers
             evaluated against the probe's marker environment
```

**The gate is decided on a fresh probe** (1.4.0). It read `env.externally_managed` — whatever the caller last cached about the environment — before anything had been probed, and the rollback flow had no gate at all (H12); the Settings override saved and nothing read it, so turning it on changed nothing (H11). `flow::preamble` now probes, gates on that answer, and — only with the override on, the marker present **and** an engine that has the flag — hands the flow `SystemPackages::OverrideAllowed`, which is the one way `--break-system-packages` reaches argv (hard invariant 5). A venv never carries the marker, so the override never applies inside one. The CLI reads the override from the store both heads share, has no flag of its own, and says on stderr every time it applies; the GUI shows a chip wherever it applies. Pip upkeep is not a flow and keeps its own guard (§7).

What a caller decides before a flow starts is `flow::FlowOptions { cancel, clock, allow_externally_managed, releases }`: the cancellation token — the caller's, because the first resolve runs inside `start` and a token made there could only be handed out after the resolve it was needed to stop; a clock, so a test can age a preview without waiting ten minutes; the Settings override as the caller read it; and where a release's `Requires-Python` comes from when the engine does not say (`release::PyPiReleases` in both heads; `None` checks only what the engine itself reported).

**Requires-Python is screened by `pipdock-core`, not the engine** (ARCHITECTURE §3), so the preview is identical under pip and uv even though the engines themselves disagree. Exact pins the user typed and forced pins are checked before the engine sees them; every change the engine planned is checked after the dry run — pip's from its own report, uv's against PyPI's per-release JSON — and a violating release is ruled out as `name!=version` and re-resolved. What is left out is listed in `report.incompatible` (PD-PKG-001's preview row); a request that is nothing but such pins is `PD-PKG-001` outright. A release whose range nobody can tell is never blocked. This applies to the update and install flows; a rollback's restore set is not screened (audit backlog N4).

## 3. Update flow (state machine)

```text
Idle
 └─ env selected → Scanning            engine.list_outdated()
     └─ SelectionReady                 rows: name, current → latest, pinned rows locked out of Select-all
         └─ user selects + [Update] → Resolving
             │                         probe + PEP 668 gate (§2), then
             │                         engine.resolve(PlanRequest{upgrades, strategy: Compatible}, ResolveCtx)
             │                         Hold pins restated at their version; Requires-Python screened (§2)
             ├─ error → Failed(code)   catalog-classified; retry available
             ├─ Stop → Nothing(cancelled)   nothing planned, nothing ran
             └─ PreviewReady           ResolutionReport rendered as a diff:
                 │                       · clean upgrades  (A 1.2 → 2.0)
                 │                       · new transitive installs
                 │                       · held-back items (B → 0.1.9, latest 0.5.0, blocked by A: "requests<0.2")
                 │                       · impossible items
                 │                       · left out for Requires-Python (incompatible), yanked releases,
                 │                         and what each forced package will break (forced)
                 ├─ conflicts exist → ConflictDecision (per held-back/impossible pkg):
                 │       [Keep compatible]  default; accept resolver's version   (held-back only)
                 │       [Skip]             drop pkg from this plan
                 │       [Force latest]     pin it to latest and release its blockers from the guard set;
                 │                          the preview names what breaks before Confirm
                 │     any Force/Skip chosen → re-Resolve with overrides → PreviewReady (loop, max 3 rounds,
                 │     then RoundsExhausted: still confirmable, never re-decided — prevents decision ping-pong)
                 │     a re-Resolve that fails leaves the previous PreviewReady exactly as it was
                 └─ [Confirm] → Snapshotting   re-probe first: preview > 10 min old or env drifted → PD-RES-002,
                       │                       nothing written; then freeze + meta written; failure here
                       │                       ABORTS the plan (PD-SNP-001)
                       └─ Executing
                            Phase A: single batch install of pinned set, the rest held by -c ── ok ──→ PostCheck
                                     Stop or an environment-wide failure → a row for every package, no Phase B
                            Phase B (on any other A-failure): per-pkg sequential, skip-and-continue,
                                     live stderr → console via plan-progress events
                       └─ PostCheck            engine.check(); findings appended to summary
                                               (a check that could not run: ok false, CheckReport.error)
                            └─ Summary         see §6
```

**Force latest forces** (1.4.0). The decision was recorded in the request and nothing read it, so the "forced" resolve was the held-back one again (C6). The flow now turns it into the resolve's `GuardAdjust`: the forced package pinned to its latest, and the packages whose constraints held it back released from the guard set. Forcing still failed in the canonical case, because the guard set pins every installed package the plan does not move — the forced package's own dependencies included (httpcore 1.0.9 needs `h11>=0.16`; the guard had `h11==0.12.0`). Which pins collide is resolution, and PipDock does not resolve (invariant 3), but the engine names them in its impossibility: `plan::forced_collisions` releases exactly those, installed and still pinned, and asks again, at most `FORCE_RELEASE_ROUNDS` (3) times. Only a forced pin releases anything — a `Hold` pin's impossibility is shown as it stands. `report.forced` carries each forced package with its blockers, which is what the preview's "Will break" rows and the CLI print.

Cancellation: allowed in Resolving and Executing, through the token the caller hands the flow before its first resolve (`FlowOptions.cancel`). In Resolving it kills the dry run and the flow ends `Nothing(cancelled)` — before 1.4.0 a resolve could not be stopped at all (H13). In Executing it kills the child's whole process tree; completed steps remain, the rest are `Skipped` and `ExecutionSummary.cancelled` is set — `StepStatus` carries no payload, so the `Skipped(UserCancelled)` this line used to name does not exist on the wire. A Stop during Phase A is that summary too, every row skipped; it used to be `PD-INT-001` with no summary and no rollback offer (C5). The snapshot offers full revert.

## 4. Install flow

```text
SearchIdle → (keystrokes) → QueryingIndex (local fuzzy, <50 ms) → Results
   │            index still loading → ready: false → the screen asks again
   └─ row focus → MetadataFetch (PyPI JSON, cached 24 h) → detail panel
   └─ [Add] → Queue ("dock bay", persists across searches; versions default latest, editable to spec)
Queue → [Install n packages] → Resolving (PlanRequest{installs}) → PreviewReady → … same tail as §3
                                 queue kept until the resolve answers
```

A queued version is a PEP 440 specifier set — `==1.0`, `>=5.0,<6`, `~=1.4`, `===legacy`, or a bare version read as `==` — checked by `Spec::parse` on every way in (1.4.0). It was a plain string: `2.0 ; python_version<'0'` reached argv as one token pip read as a requirement whose marker is false, a silent no-op install, and an imported `django>=5.0,<6` failed because the flow split on `==` alone. An install never runs `list --outdated` — a sweep of the index for every installed package that nothing in an install reads (M28) — and its exact pins are screened for Requires-Python before any engine sees them (§2).

Installed-tab presentation (per original concept, kept): up-to-date rows **dimmed**, outdated rows carry an `UPDATE` badge and are mirrored into the Updates tab; already-installed packages appearing in search results show an `INSTALLED ✓` chip (or `UPDATE` if outdated) instead of [Add].

## 5. Uninstall flow

```text
Selection → [Uninstall] → probe + PEP 668 gate (§2)
   → GuardCheck: reverse-dep graph over full removal set (markers evaluated, §2)
   ├─ breaks nothing → Confirm dialog (lists removals)
   └─ breaks {Y,Z} → Warning dialog: "Removing X breaks Y (requires X>=1), Z"
        options: [Cancel] [Remove dependents too] (adds Y,Z to set, re-guard) [Force remove only X]
→ re-probe: env changed since the guard, or guard > 10 min old → PD-RES-002, nothing written
→ Snapshot → sequential engine.uninstall per pkg (skip-and-continue) → Summary
```

Rationale: bare `pip uninstall` performs **no** dependency check — this guard is a core value-add.

**The dialog is not the enforcement point.** *Remove dependents too* is the caller running the guard
*again* over the widened set, because pulling Y in can break Z and stopping after one level hands the
user a set that still breaks something. And the removal itself refuses to run unless the guard was
clear or the user explicitly accepted the breakage — `flow::GuardAck`, checked in
`UninstallFlow::execute` the way `SnapshotProof` is, so a caller that never looked at the report
cannot proceed by omission. Attempting it anyway is **PD-RES-004**, raised before the snapshot is
written so a plan that will not run leaves nothing behind — and the GUI keeps the flow parked, so the
dialog's *Force remove* is still a choice rather than a button that answers "no plan".

**The guard is a decision made against a listing**, so it goes stale like a preview (§9.3). Since
1.4.0 `UninstallFlow::take_snapshot` probes again before anything is written: a package installed
after the guard ran may depend on what is about to go (M44).

A removal is blocked on a PEP 668 environment exactly as an update is: §2's preamble is "all mutating
flows", and removing from a system Python is the more dangerous of the two, since no resolver stands
between the user and the damage. With the override on, each `uninstall` is given
`--break-system-packages`, without which pip and uv both refuse to remove from a managed environment.

## 6. Summary model (owner requirement)

```json
{ "planId":"…","phase":"isolated","results":[
   {"pkg":"httpx","from":"0.27.0","to":"0.28.1","status":"ok"},
   {"pkg":"oldlib","to":"2.0.0","status":"failed","code":"PD-BLD-002","stderrTail":"…"}],
  "check":{"ok":true,"findings":[]},
  "counts":{"ok":13,"failed":2,"skipped":1},
  "cancelled":false }
```

Rendered as: **"13 successful, 2 failed, 1 skipped"** with an expandable row per failure (catalog code, human reason EN/VI, stderr tail, *Copy log*, *Report bug* deep link — see ERROR-CATALOG §4).

Until 1.4.0 this sample showed two things the wire has never carried. **A row has no `reason`**: the human reason is looked up from `code` in the catalogs, in either language (hard invariant 4), and a Rust-assembled English sentence on the row is exactly what that rule forbids. **`phase` is `batch` or `isolated`**, never `A`/`B`. The rest is as shipped: `from` is absent for a fresh install, and on an update it is taken from the plan's changes — the engines cannot know it, and until 1.4.0 every row carried none, so every upgrade read as a fresh install (M29); `check.error` carries the code when the post-run check could not run, with `ok` false (it said `ok: true`, M30); `cancelled` is set when the user stopped the run; `planId` is the flow's for an update or install, unique per plan and the same id its snapshot records, and the pre-execution snapshot's id for a removal or a rollback.

**A flow that ends before anything runs returns no summary.** Its last step is `{ "step": "nothing", "reason": … }`, and `reason` is one of three:

| `reason` | When | CLI exit |
|---|---|---|
| `nothing-to-do` | the request was empty once pins and `--except` were applied | 0 |
| `everything-skipped` | every candidate was skipped in the decision rounds | 2 |
| `cancelled` | the user stopped the resolve — nothing was planned and nothing ran (1.4.0, when a resolve became stoppable) | 130 |

A request that is nothing but exact pins this interpreter cannot install is not one of these: it is `PD-PKG-001`, with the range and the interpreter (§2).

## 7. Engine command mapping

| Operation | pip (`<py> -m pip …`) | uv (`uv pip … --python <py>`) | Adapter notes |
|---|---|---|---|
| List installed | *(not used — see below)* | *(not used — see below)* | `Engine::list_installed` exists and normalizes both shapes to `Dist`, but only `resolve()` calls it, to build SP-1's guard set |
| List outdated | `list --outdated --format=json` | `list --outdated --format=json` | uv output pinned in SP-1 fixtures. An update asks; an install does not (M28) |
| Dry-run resolve | `install -U --dry-run --quiet --report - -r <req>` (JSON to stdout) | `uv pip install -U --dry-run -r <req>` (**text plan; no stable JSON report**), plus `--break-system-packages` under the override | uv adapter parses text; exact format frozen via SP-1 fixtures + snapshot tests. Both read the exit status first: 0 is a plan, an impossibility is `report.impossible`, anything else is classified (C3) |
| Install pinned | `install -r <pins> -c <held>` | `uv pip install -r <pins> -c <held>` | `--break-system-packages` only under the override. Phase A: the whole pinned set, everything the plan does not touch held at its current version. Phase B: one pin, the rest of the plan at its planned versions held too. Rollback: one pin, the rest of the restore held |
| Uninstall | `uninstall -y pkg` | `uv pip uninstall pkg` | One package per invocation; `--break-system-packages` only under the override |
| Env check | `check` | `uv pip check` | normalize findings: pip's `requires X, which is not installed` and `has requirement X, but you have Y`, and uv's ``The package `x` requires …`` — only the first was read, and every uv finding was filed under a package called "the" (H18) |
| Freeze (snapshots) | `freeze --all` | `uv pip freeze` | pip's `--all` includes pip/setuptools; record engine in snapshot meta, which a rollback reads (§8) |
| Upgrade pip | `install -U pip`, falling back to `-m ensurepip --upgrade` when pip cannot start | *(n/a — **always runs through the pip adapter**, whatever engine is configured)* | |

`<req>` and `<pins>` are `-r` temp files and `<held>` a `-c` one (ARCHITECTURE §8): every plan restates the installed set (SP-1), and as argv that passed Windows' command-line limit at about 1 300 packages. The `<req>` of a resolve holds three groups — the packages to move as bare names, the installs with their specifiers, and the guard set at `==` (`===` for an installed version that is not PEP 440; one that could be read as more than one token is left out, and so left alone) — plus the flow's `GuardAdjust`: forced and `Hold` pins, and `name!=version` exclusions. Every command runs under the watchdog for its kind of work (ARCHITECTURE §8), and a fired one is `PD-ENG-004`.

**Pip upkeep does not dispatch on the configured engine** (amended by Phase 3 · P1). Upgrading pip is a pip operation by definition; there is no uv way to do it, and `UvEngine::upgrade_pip` exists only to refuse. Dispatching meant `--engine uv` failed at `PD-ENG-001` for a preference that has nothing to do with the operation, on the one command with no resolver between the user and the result — while the surrounding code was already reaching past the abstraction to read pip's version. Both heads now call `PipEngine::upgrade_pip` directly. Same reasoning as the Code Health tools venv (CODE-HEALTH-SPEC §2 as amended by P2): pip is present wherever Python is, and a feature must not go dark over an unrelated setting.

The refusal it replaces is also why `PipEngine::upgrade_pip` carries its own PEP 668 guard: §2's preamble is "all mutating flows", and replacing pip in site-packages is one. `UpdateFlow` has refused since S1 and `UninstallFlow` since S5; this path had no guard at all while its only caller was a CLI command with no button behind it. **It is not the preamble**, and 1.4.0 left it so: it reads `externally_managed` from the `PyEnv` it is handed — probed for that command by the CLI, the Environments row's in the GUI — rather than probing again, and the Settings override never reaches it, so pip upkeep on a managed Python is refused whatever the setting.

**Neither head lists installed packages through the engine.** `pipdock list` and `pkg_list` both read `probe.py` instead, because the probe carries `Requires-Dist` — which `list --format=json` does not report, and which the reverse-dependency graph, the uninstall guard and blocker attribution are all built from. Reading the engine here would give the Installed screen a different package set than the guard that protects it. The `-I` trade-off this buys is documented in ARCHITECTURE §4 and surfaced as `hidden_user_site`.

Version floors: pip ≥ 22.2 for `--dry-run --report` (offer in-app pip upgrade when older); uv minimum 0.10, pinned by SP-1 (`UV_MIN_VERSION`). Both are declared constants that nothing enforces yet (audit backlog). What a pip below 22.2 actually says is `no such option: --dry-run` — optparse stops at the first option it does not know, and in the planner's argv that is `--dry-run`, not `--report` — and since 1.4.0 that classifies as `PD-ENG-002` too.

## 8. Rollback flow

```text
Snapshot selected → probe + PEP 668 gate (§2)
   → Diff = (current freeze) vs (snapshot freeze); across engines pip, setuptools and wheel
     are left out of it, and pip is never in {added}
   → plan: uninstall {added}, install {removed ∪ changed @ snapshot versions}
   → dry-run resolve the restore set (a yanked/deleted release cannot be restored → PD-SNP-002,
     listed explicitly; user may proceed partially) — not implemented, see below
   → freeze again: changed since the preview, or preview > 10 min old → PD-RES-002, nothing written
   → snapshot current state first (rollback is itself reversible)
   → remove {added} one at a time, then restore each pin in isolation, the rest of the restore
     held by -c → summary
```

**Across engines a rollback keeps pip** (1.4.0, C7). uv's freeze has no `--all`, so a uv snapshot never lists pip, setuptools or wheel; compared with pip's freeze, which does, every one of them looked added since the snapshot, and the restore uninstalled pip from the environment. `Meta::engine` had been recorded for exactly this and was read by nothing. `snapshot::restorable_diff` leaves those three out of the comparison when the snapshot's engine and the restoring engine differ, and — whichever engine — never plans pip's removal: a snapshot taken before pip arrived would otherwise take away the tool the next plan needs.

**The restore plan is a diff against one freeze**, and applied to anything else it removes and reinstalls the wrong things, so `RollbackFlow::take_snapshot` compares the freeze it is about to write with the one `start` diffed (M44). The restores are not two-phase, as this diagram used to say: `plan::execute_rollback` removes, then installs each pin on its own in `Isolated` mode, because exactness is the whole job and a batch resolve is free to pick another version; a Stop reports the step it interrupted and every one after it as skipped. It no longer runs a `pip check` between the removals and the restores.

**The dry-run resolve step is implemented in neither head**, and S6 deliberately did not add it.
`RollbackFlow::start` diffs, plans, and reports `unrestorable_lines` — the entries no index can
supply, which is what the preview lists as `PD-SNP-002`. What it does *not* do is ask the engine
whether each restore spec is still fetchable, so a release yanked since the snapshot was taken
surfaces as a per-package failure at execute time rather than in the preview.

Adding it is a network round trip on an interaction budgeted as instant, and it would resolve
against the *pre-removal* environment — reporting conflicts the execution never hits, because the
removals happen first. It needs its own design, and it belongs with the engine work in
M3 rather than bolted onto a preview. Until then the preview's promise is precise: it lists what
*cannot* be restored, not everything that might fail.

## 9. Data-flow invariants (enforced in `pipdock-core`, tested)

1. No mutating engine call without an **accepted proof** for this session: a `ResolutionReport` for
   a resolve-shaped plan (update, install), a `GuardReport` plus a `GuardAck` for a removal (§5), a
   `RollbackPlan` for a restore (§8), and **nothing for pip upkeep, which is not resolver-planned
   and cannot be** (§7; added by P1). Originally written as "a `ResolutionReport`", which the
   uninstall path cannot produce — there is nothing to resolve — leaving the one flow with no
   preview also the one flow the invariant did not describe. The fourth shape is the same lesson a
   second time: pip below 22.2 cannot produce `--dry-run --report` output *at all*, which is
   precisely the case the feature exists for, so demanding a report would refuse the only
   environments that need it. `pip_upgrade` is one IPC message parking nothing, so it needs no
   waiver type either — those exist for a decision made in one message and consumed in another.
   **Where:** by type. `plan::execute` takes an `AcceptedPlan`, which only `AcceptedPlan::accept`
   builds; `UninstallFlow::execute` demands a `GuardAck` and checks it against the `GuardReport`
   it holds (`PD-RES-004`); `RollbackFlow` executes only the `RollbackPlan` its own `start`
   produced.
2. No mutating engine call without a successful snapshot write, **except pip upkeep** (added by P1).
   A snapshot's only restore path is `pip install pip==X` executed *by pip*, so one taken to protect
   against a broken pip has no consumer that could use it. The exemption is **made visible rather
   than silent**: the confirm dialog states that no snapshot is taken. That is what keeps it an
   exemption rather than an erosion — every other mutating path still writes one, and
   `flow::proof_from` still refuses `NotTaken` with `PD-SNP-001`. **Where:** `plan::execute`,
   `plan::execute_uninstall` and `plan::execute_rollback` each take a `SnapshotProof` — a written
   snapshot, or the CLI's named `--no-snapshot` waiver for disposable environments — and the three
   flows build it in `proof_from`.
3. **A decision made against a listing of the environment is refused when it is older than 10
   minutes (`PLAN_MAX_AGE`) or the environment no longer matches the listing** — `PD-RES-002`,
   before any snapshot is written, so a refusal costs nothing. **Unenforced until 1.4.0 while every
   test passed** (C1): the flow accepted the plan inside `execute()` with the current time, so its
   age was always zero; it passed the resolve-time listing as the "current" one, so the fingerprint
   always matched; and `plan::execute` compared the plan's environment hash with itself. **Where:**
   `AcceptedPlan::accept` now takes the basis the report was resolved over — the probe's listing
   and the resolve time — and `UpdateFlow::take_snapshot` probes the environment again after
   Confirm and verifies against it; `plan::execute` verifies once more, against the hash of the
   interpreter it is about to change, and acts on the re-probed listing. The rule reaches the two
   flows that have no report (M44): `UninstallFlow::take_snapshot` re-probes and checks the guard's
   basis (`plan::verify_basis`), since a package installed after the guard ran may depend on what
   is about to go, and `RollbackFlow::take_snapshot` compares the freeze it is about to write with
   the one the restore plan was diffed against (`plan::verify_age` for the age). `health_fix`
   applies the drift half to a source tree (§9.1).
4. Every failure surfaced to UI/CLI carries a catalog code. **Where:** `PdError.code` is not
   optional, and the classifiers fall back to `PD-ENG-999`. A *stopped* run is not a failure and is
   never shown as one: a stopped step is `Skipped`, a stopped resolve is `nothing`/`cancelled` (§6),
   and the typed `PdError.stop` that tells them apart never reaches the wire. A post-run check that
   could not run carries its code in `CheckReport.error` rather than reading as a pass.
5. Pinned packages never appear in a `PlanRequest.upgrades` unless the user explicitly unpinned them
   this session. **Where:** `pins::filter_upgrades`, inside `flow::build_request`, for a package the
   update *names* as much as for one *Select all* sweeps up — the "N pinned excluded" note on screen
   is presentation, and the flow's `excluded_pins()` is what a preview shows. `PinMode::Exclude`'s
   doc used to promise that a pinned package could still be updated deliberately, one at a time;
   nothing implemented that, and this rule was right — 1.4.0 corrected the doc, and unpinning is the
   deliberate step. A `Hold` pin is also **restated at its version in every resolve**
   (`GuardAdjust.pins`), so it stays put as anyone's dependency too: `pins::hold_requirements`
   existed and had no caller, so a held package was only left out of the upgrade list and free to
   move under it (H17).
6. **A write *outside* an environment needs a `FixConsent` accepted this session and a re-verified count, and takes no snapshot** (added by P5). Invariants 1 and 2 are scoped to a mutating **engine** call and do not reach `ruff check --fix`, which rewrites the user's *source tree*. No snapshot in this application describes one — a snapshot is a freeze, and §8's rollback is *uninstall the added, install the removed at snapshot versions* — so one taken here would have no consumer that could use it. That is invariant 2's own argument for the pip-upkeep exemption, a second time, and like that one it is **made visible rather than silent**: the confirm states that PipDock cannot undo the change and that only the user's own version control can. The safety net is therefore git, which is why `git status --porcelain` is asked twice — once to decide what the dialog *renders*, once inside the command to decide what it *allows* — and why the dirty finding lives inside `FixConsent` rather than beside it. Both ask with `-c core.fsmonitor=false` (1.4.0): a project folder that carries its own `.git/config` can name a program for `git status` to run, and asking "is it safe to write here" would have run it (M40).

### 9.1 The fix flow

```
ReportReady ──Fix──► Confirm(clean | dirty) ──confirm──► Verifying ──► Fixing ──► FixSummary
                            │                                │
                            └──cancel──► ReportReady          └── PD-RES-002 / PD-PRM-003 ──► ReportReady
```

`Verifying` is not decoration and it is not one check. It re-reads the project read-only and refuses with **`PD-RES-002`** when the safely-fixable counts no longer match what the user confirmed — the source-tree analogue of §9's rule 3, for the one thing no snapshot describes — and it refuses with **`PD-PRM-003`** when any target cannot be written. Both happen **before the first byte**: a partial rewrite of a source tree is the worst outcome this flow can produce, because nothing here can undo half of one. ruff can fail to write a file and still exit 1, which the findings-exit rule accepts as a clean run, so the writability check cannot be a classification of the failure afterwards.

`Fixing` is a single `ruff check --fix --output-format json`, which both applies the safe fixes and reports what remains — so `FixReport.remaining` cannot describe a different moment than the fix, and the tab refreshes without a second run. Only **safe** fixes are ever applied; `--unsafe-fixes` is never passed, and `ruff format` is not wired at all (CODE-HEALTH-SPEC §7's non-goals).

**The fix's argv is the read-only run's plus `--fix`** (`health::fix::fix_argv` over `run::ruff_argv`), so it excludes exactly what the preview excluded — notebooks and the default directories (`.venv`, `venv`, `node_modules`, `build`, `dist`, `.git`) — **by construction**. Until 1.4.0 it ran `check . --fix` with none of them (C4): run against real ruff 0.17.1 it rewrote every `.ipynb` in the tree, and in a project whose `pyproject.toml` sets its own `exclude` (which replaces ruff's defaults) the files under `.venv` as well — files nobody had been shown. The fix's failures speak Code Health's codes, and a watchdog that stops it says that some files may already have been rewritten, because ruff writes as it goes (M39).
