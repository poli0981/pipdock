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
[Select env] → probe.py → env facts (python ver, PEP 668, dist metadata)
     │
     ├─ EXTERNALLY-MANAGED detected → BLOCKED banner (PD-ENV-002)
     │        └─ explicit override in Settings → proceed with persistent warning chip
     └─ ok → flow continues; reverse-dep graph built from probe data
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
| List installed | `list --format=json` | `list --format=json` | shapes differ slightly; normalize to `Dist` |
| List outdated | `list --outdated --format=json` | `list --outdated --format=json` | uv output pinned in SP-1 fixtures |
| Dry-run resolve | `install -U --dry-run --quiet --report -` (JSON to stdout) | `uv pip install -U --dry-run` (**text plan; no stable JSON report**) | uv adapter parses text; exact format frozen via SP-1 fixtures + snapshot tests |
| Install pinned | `install pkg==v [--no-deps in Phase A per-report set]` | `uv pip install pkg==v` | Phase A passes the full pinned set in one call |
| Uninstall | `uninstall -y pkg` | `uv pip uninstall pkg` | |
| Env check | `check` | `uv pip check` | normalize findings |
| Freeze (snapshots) | `freeze --all` | `uv pip freeze` | pip's `--all` includes pip/setuptools; record engine in snapshot meta |
| Upgrade pip | `install -U pip` | *(n/a — surface "pip upkeep" only when pip engine active or pip present in env)* | |

Version floors: pip ≥ 22.2 for `--dry-run --report` (offer in-app pip upgrade when older); uv minimum pinned after SP-1.

## 8. Rollback flow

```text
Snapshot selected → Diff = (current freeze) vs (snapshot freeze)
   → plan: uninstall {added}, install {removed ∪ changed @ snapshot versions}
   → dry-run resolve the restore set (a yanked/deleted release cannot be restored → PD-SNP-002,
     listed explicitly; user may proceed partially)
   → snapshot current state first (rollback is itself reversible) → execute two-phase → summary
```

## 9. Data-flow invariants (enforced in `pipdock-core`, tested)

1. No mutating engine call without a `ResolutionReport` accepted in this session.
2. No mutating engine call without a successful snapshot write.
3. `plan_execute` refuses a report older than 10 minutes or if the env's probe hash changed (env drifted → re-resolve).
4. Every failure surfaced to UI/CLI carries a catalog code.
5. Pinned packages never appear in a `PlanRequest.upgrades` unless the user explicitly unpinned them this session.
