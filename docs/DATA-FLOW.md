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
[Select env] → probe.py -I → env facts (python ver, PEP 668, hidden_user_site, dist metadata)
     │
     ├─ EXTERNALLY-MANAGED detected → BLOCKED banner (PD-ENV-002)
     │        └─ explicit override in Settings → proceed with persistent warning chip
     ├─ hidden_user_site non-null → Installed screen shows the partial-listing note
     │                              (not a block; see SECURITY §2)
     └─ ok → flow continues; reverse-dep graph built from probe data

Every candidate is then screened against the environment's Python version by
pipdock-core (ARCHITECTURE §3): incompatible ones are dropped from the plan with
PD-PKG-001 before any engine command runs, so the preview is identical under pip
and uv even though the engines themselves disagree.
```

## 3. Update flow (state machine)

```text
Idle
 └─ env selected → Scanning            engine.list_outdated()
     └─ SelectionReady                 rows: name, current → latest, pinned rows locked out of Select-all
         └─ user selects + [Update] → Resolving
             │                         engine.resolve(PlanRequest{upgrades, strategy: Compatible})
             ├─ error → Failed(code)   catalog-classified; retry available
             └─ PreviewReady           ResolutionReport rendered as a diff:
                 │                       · clean upgrades  (A 1.2 → 2.0)
                 │                       · new transitive installs
                 │                       · held-back items (B → 0.1.9, latest 0.5.0, blocked by A: "requests<0.2")
                 │                       · impossible items
                 ├─ conflicts exist → ConflictDecision (per held-back/impossible pkg):
                 │       [Keep compatible]  default; accept resolver's version   (held-back only)
                 │       [Skip]             drop pkg from this plan
                 │       [Force latest]     adds per-pkg override; warning names what breaks
                 │     any Force/Skip chosen → re-Resolve with overrides → PreviewReady (loop, max 3 rounds
                 │     then require manual pruning — prevents decision ping-pong)
                 └─ [Confirm] → Snapshotting   freeze + meta written; failure here ABORTS the plan (PD-SNP-001)
                       └─ Executing
                            Phase A: single batch install of pinned set ── ok ──→ PostCheck
                            Phase B (on A-failure): per-pkg sequential, skip-and-continue,
                                     live stderr → console via plan-progress events
                       └─ PostCheck            engine.check(); findings appended to summary
                            └─ Summary         see §6
```

Cancellation: allowed in Resolving/Executing; kills child process group; completed steps remain; summary marks the rest `Skipped(UserCancelled)`; snapshot offers full revert.

## 4. Install flow

```text
SearchIdle → (keystrokes) → QueryingIndex (local fuzzy, <50 ms) → Results
   └─ row focus → MetadataFetch (PyPI JSON, cached 24 h) → detail panel
   └─ [Add] → Queue ("dock bay", persists across searches; versions default latest, editable to spec)
Queue → [Install n packages] → Resolving (PlanRequest{installs}) → PreviewReady → … same tail as §3
```

Installed-tab presentation (per original concept, kept): up-to-date rows **dimmed**, outdated rows carry an `UPDATE` badge and are mirrored into the Updates tab; already-installed packages appearing in search results show an `INSTALLED ✓` chip (or `UPDATE` if outdated) instead of [Add].

## 5. Uninstall flow

```text
Selection → [Uninstall] → GuardCheck: reverse-dep graph over full removal set
   ├─ breaks nothing → Confirm dialog (lists removals)
   └─ breaks {Y,Z} → Warning dialog: "Removing X breaks Y (requires X>=1), Z"
        options: [Cancel] [Remove dependents too] (adds Y,Z to set, re-guard) [Force remove only X]
→ Snapshot → sequential engine.uninstall per pkg (skip-and-continue) → Summary
```

Rationale: bare `pip uninstall` performs **no** dependency check — this guard is a core value-add.

**The dialog is not the enforcement point.** *Remove dependents too* is the caller running the guard
*again* over the widened set, because pulling Y in can break Z and stopping after one level hands the
user a set that still breaks something. And the removal itself refuses to run unless the guard was
clear or the user explicitly accepted the breakage — `flow::GuardAck`, checked in
`UninstallFlow::execute` the way `SnapshotProof` is, so a caller that never looked at the report
cannot proceed by omission. Attempting it anyway is **PD-RES-004**, raised before the snapshot is
written so a plan that will not run leaves nothing behind.

A removal is blocked on a PEP 668 environment exactly as an update is: §2's preamble is "all mutating
flows", and removing from a system Python is the more dangerous of the two, since no resolver stands
between the user and the damage.

## 6. Summary model (owner requirement)

```json
{ "planId":"…","phase":"B","results":[
   {"pkg":"httpx","from":"0.27.0","to":"0.28.1","status":"ok"},
   {"pkg":"oldlib","to":"2.0.0","status":"failed","code":"PD-BLD-002",
    "reason":"build backend error in pyproject.toml","stderrTail":"…"}],
  "check":{"ok":true,"findings":[]},
  "counts":{"ok":13,"failed":2,"skipped":1} }
```

Rendered as: **"13 successful, 2 failed, 1 skipped"** with an expandable row per failure (catalog code, human reason EN/VI, stderr tail, *Copy log*, *Report bug* deep link — see ERROR-CATALOG §4).

## 7. Engine command mapping

| Operation | pip (`<py> -m pip …`) | uv (`uv pip … --python <py>`) | Adapter notes |
|---|---|---|---|
| List installed | *(not used — see below)* | *(not used — see below)* | `Engine::list_installed` exists and normalizes both shapes to `Dist`, but only `resolve()` calls it, to build SP-1's guard set |
| List outdated | `list --outdated --format=json` | `list --outdated --format=json` | uv output pinned in SP-1 fixtures |
| Dry-run resolve | `install -U --dry-run --quiet --report -` (JSON to stdout) | `uv pip install -U --dry-run` (**text plan; no stable JSON report**) | uv adapter parses text; exact format frozen via SP-1 fixtures + snapshot tests |
| Install pinned | `install pkg==v [--no-deps in Phase A per-report set]` | `uv pip install pkg==v` | Phase A passes the full pinned set in one call |
| Uninstall | `uninstall -y pkg` | `uv pip uninstall pkg` | |
| Env check | `check` | `uv pip check` | normalize findings |
| Freeze (snapshots) | `freeze --all` | `uv pip freeze` | pip's `--all` includes pip/setuptools; record engine in snapshot meta |
| Upgrade pip | `install -U pip` | *(n/a — **always runs through the pip adapter**, whatever engine is configured)* | |

**Pip upkeep does not dispatch on the configured engine** (amended by Phase 3 · P1). Upgrading pip is a pip operation by definition; there is no uv way to do it, and `UvEngine::upgrade_pip` exists only to refuse. Dispatching meant `--engine uv` failed at `PD-ENG-001` for a preference that has nothing to do with the operation, on the one command with no resolver between the user and the result — while the surrounding code was already reaching past the abstraction to read pip's version. Both heads now call `PipEngine::upgrade_pip` directly. Same reasoning as the Code Health tools venv (CODE-HEALTH-SPEC §2 as amended by P2): pip is present wherever Python is, and a feature must not go dark over an unrelated setting.

The refusal it replaces is also why `PipEngine::upgrade_pip` carries its own PEP 668 guard: §2's preamble is "all mutating flows", and replacing pip in site-packages is one. `UpdateFlow` has refused since S1 and `UninstallFlow` since S5; this path had no guard at all while its only caller was a CLI command with no button behind it.

**Neither head lists installed packages through the engine.** `pipdock list` and `pkg_list` both read `probe.py` instead, because the probe carries `Requires-Dist` — which `list --format=json` does not report, and which the reverse-dependency graph, the uninstall guard and blocker attribution are all built from. Reading the engine here would give the Installed screen a different package set than the guard that protects it. The `-I` trade-off this buys is documented in ARCHITECTURE §4 and surfaced as `hidden_user_site`.

Version floors: pip ≥ 22.2 for `--dry-run --report` (offer in-app pip upgrade when older); uv minimum pinned after SP-1.

## 8. Rollback flow

```text
Snapshot selected → Diff = (current freeze) vs (snapshot freeze)
   → plan: uninstall {added}, install {removed ∪ changed @ snapshot versions}
   → dry-run resolve the restore set (a yanked/deleted release cannot be restored → PD-SNP-002,
     listed explicitly; user may proceed partially)
   → snapshot current state first (rollback is itself reversible) → execute two-phase → summary
```

**The dry-run resolve step is implemented in neither head**, and S6 deliberately did not add it.
`RollbackFlow::start` diffs, plans, and reports `unrestorable_lines` — the entries no index can
supply, which is what the preview lists as `PD-SNP-002`. What it does *not* do is ask the engine
whether each restore spec is still fetchable, so a release yanked since the snapshot was taken
surfaces as a per-package failure at execute time rather than in the preview.

Adding it is a network round trip on an interaction budgeted as instant, and it would resolve
against the *pre-removal* environment — reporting conflicts the two-phase execution never hits,
because the removals happen first. It needs its own design, and it belongs with the engine work in
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
2. No mutating engine call without a successful snapshot write, **except pip upkeep** (added by P1).
   A snapshot's only restore path is `pip install pip==X` executed *by pip*, so one taken to protect
   against a broken pip has no consumer that could use it. The exemption is **made visible rather
   than silent**: the confirm dialog states that no snapshot is taken. That is what keeps it an
   exemption rather than an erosion — every other mutating path still writes one, and
   `flow::proof_from` still refuses `NotTaken` with `PD-SNP-001`.
3. `plan_execute` refuses a report older than 10 minutes or if the env's probe hash changed (env drifted → re-resolve).
4. Every failure surfaced to UI/CLI carries a catalog code.
5. Pinned packages never appear in a `PlanRequest.upgrades` unless the user explicitly unpinned them this session.
6. **A write *outside* an environment needs a `FixConsent` accepted this session and a re-verified count, and takes no snapshot** (added by P5). Invariants 1 and 2 are scoped to a mutating **engine** call and do not reach `ruff check --fix`, which rewrites the user's *source tree*. No snapshot in this application describes one — a snapshot is a freeze, and §8's rollback is *uninstall the added, install the removed at snapshot versions* — so one taken here would have no consumer that could use it. That is invariant 2's own argument for the pip-upkeep exemption, a second time, and like that one it is **made visible rather than silent**: the confirm states that PipDock cannot undo the change and that only the user's own version control can. The safety net is therefore git, which is why `git status --porcelain` is asked twice — once to decide what the dialog *renders*, once inside the command to decide what it *allows* — and why the dirty finding lives inside `FixConsent` rather than beside it.

### 9.1 The fix flow

```
ReportReady ──Fix──► Confirm(clean | dirty) ──confirm──► Verifying ──► Fixing ──► FixSummary
                            │                                │
                            └──cancel──► ReportReady          └── PD-RES-002 / PD-PRM-003 ──► ReportReady
```

`Verifying` is not decoration and it is not one check. It re-reads the project read-only and refuses with **`PD-RES-002`** when the safely-fixable counts no longer match what the user confirmed — the source-tree analogue of §9's rule 3, for the one thing no snapshot describes — and it refuses with **`PD-PRM-003`** when any target cannot be written. Both happen **before the first byte**: a partial rewrite of a source tree is the worst outcome this flow can produce, because nothing here can undo half of one. ruff can fail to write a file and still exit 1, which the findings-exit rule accepts as a clean run, so the writability check cannot be a classification of the failure afterwards.

`Fixing` is a single `ruff check --fix --output-format json`, which both applies the safe fixes and reports what remains — so `FixReport.remaining` cannot describe a different moment than the fix, and the tab refreshes without a second run. Only **safe** fixes are ever applied; `--unsafe-fixes` is never passed, and `ruff format` is not wired at all (CODE-HEALTH-SPEC §7's non-goals).
