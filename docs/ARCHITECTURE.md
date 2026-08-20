# PipDock — Architecture

*Version 0.1 · 2026-07-17*

## 1. Design principles

1. **One core, two heads.** All domain logic lives in `pipdock-core` (Rust). The Tauri GUI and the clap CLI are thin adapters over the same functions; neither contains business logic.
2. **Explain, don't reimplement.** Dependency resolution is always performed by the selected engine (pip or uv) in dry-run mode. PipDock parses and presents; it never computes version resolution itself.
3. **Out-of-band by construction.** PipDock ships as a standalone signed binary. It is never installed into a Python environment, so no operation it performs can break its own runtime (G6).
4. **Everything mutating is previewed and snapshotted.** No code path may call a mutating engine command without (a) a prior dry-run plan and (b) a pre-execution snapshot.
5. **Subprocess, never shell.** Engines are invoked with argv arrays (`tokio::process::Command`), never through a shell. Package names are validated against the PEP 508 name grammar before ever reaching argv.

## 2. Repository layout

```text
pipdock/
├─ crates/
│  ├─ pipdock-core/          # domain logic (this is the product)
│  │  ├─ src/engine/         # Engine trait + pip.rs + uv.rs adapters
│  │  ├─ src/envs/           # discovery (PEP 514, py launcher, uv, venv scan), PEP 668
│  │  ├─ src/index/          # PEP 691 name cache, fuzzy search (nucleo), PyPI JSON metadata
│  │  ├─ src/plan/           # PlanRequest → ResolutionReport → ExecutionReport
│  │  ├─ src/graph/          # reverse-dependency graph from env introspection
│  │  ├─ src/snapshot/       # freeze snapshots, diff, minimal-ops rollback
│  │  ├─ src/pins/           # pin store + auto-suggest scoring
│  │  ├─ src/health/         # tools-venv manager, deptry/vulture/ruff runners
│  │  ├─ src/errors/         # error catalog (codes + stderr classifiers)
│  │  └─ src/probe.py        # embedded env-introspection helper (see §4)
│  └─ pipdock-cli/           # clap binary `pipdock`
├─ src-tauri/                # Tauri 2 app (commands + events only)
├─ ui/                       # React 19 + TS + Tailwind 4 + Vite 8 + Zustand + i18next
├─ legal/                    # EULA, Disclaimer, Privacy, Third-Party (public — legal gate links here)
├─ .github/                  # caller workflows into poli0981/.github + issue templates
└─ docs/
```

Naming conventions: crates `pipdock-*`; React components prefixed `Pd*` (`PdPackageRow`, `PdConflictDialog`); Tauri identifier `com.skullmute.pipdock`; CLI binary `pipdock`.

## 3. The Engine trait

```rust
#[async_trait]
pub trait Engine: Send + Sync {
    fn id(&self) -> EngineId;                       // Pip | Uv
    async fn info(&self, env: &PyEnv) -> EngineInfo;          // version, availability
    async fn list_installed(&self, env: &PyEnv) -> Result<Vec<Dist>>;
    async fn list_outdated(&self, env: &PyEnv) -> Result<Vec<OutdatedDist>>;
    async fn resolve(&self, env: &PyEnv, req: &PlanRequest) -> Result<ResolutionReport>;
    async fn install(&self, env: &PyEnv, specs: &[PinnedSpec], mode: ExecMode,
                     sink: EventSink) -> Result<StepResult>;
    async fn uninstall(&self, env: &PyEnv, names: &[PkgName], sink: EventSink) -> Result<StepResult>;
    async fn check(&self, env: &PyEnv) -> Result<CheckReport>;      // pip check / uv pip check
    async fn upgrade_pip(&self, env: &PyEnv) -> Result<StepResult>; // pip engine only; uv returns Unsupported
}
```

Key model types:

- `PlanRequest` — the user's intent: `{ upgrades: Vec<PkgName>, installs: Vec<Spec>, strategy: Compatible | ForceLatest(per-pkg overrides) }`.
- `ResolutionReport` — normalized output of the dry-run: `{ changes: Vec<Change>, held_back: Vec<HeldBack{ pkg, resolved, latest, blockers: Vec<Blocker{by, constraint}> }>, impossible: Option<ImpossibleDetail>, raw: String }`. **Both adapters must emit this same shape** — this is the whole point of the trait. The pip adapter fills it from `--dry-run --report` JSON; the uv adapter from its dry-run plan output (format pinned during spike SP-1; fixtures under `crates/pipdock-core/tests/fixtures/`).
- `Blocker` computation: the engine's report says *what* was held back; *who* is responsible comes from cross-referencing the reverse-dependency graph (§4) with each blocker's `Requires-Dist` constraints. If attribution is ambiguous, show constraints without a culprit rather than guessing.

### Requires-Python is enforced by PipDock, not the engine

**Owner decision 2026-07-27 (spike SP-2).** The two engines disagree: `scipy==1.7.3` declares `Requires-Python >=3.7,<3.11`, and on Python 3.12 pip refuses it while uv plans to install it. Since G5 promises one behavior across heads, the preview must not change shape because the user flipped the engine radio — so `pipdock-core` evaluates `Requires-Python` itself in `src/compat.rs`, before any candidate reaches an engine command, and reports rejects as `PD-PKG-001` with the required range against the environment's version.

This does not violate "explain, don't reimplement" (§1.2): resolution is still entirely the engine's. `compat.rs` only filters candidates the engine should never have been offered, using a narrow slice of PEP 440 — the comparison, wildcard and compatible-release operators as they appear in `Requires-Python`. Specifiers it cannot parse are treated as **compatible**, because PipDock failing to read metadata must never be the reason an installable package is refused.

### Engine selection

`Settings.engine ∈ {pip, uv}`. First run: probe `uv --version` on PATH → preselect uv if present, else pip; user can change any time. The status bar always shows the active engine. Per-env override is P2.

## 4. Environment introspection: `probe.py`

Rust cannot cheaply read a foreign env's installed metadata. PipDock embeds a single-file, stdlib-only helper (`probe.py`, ~150 lines, no third-party imports) executed as `<env-python> probe.py --json`, printing one JSON document:

```json
{ "python": "3.12.4", "prefix": "...", "externally_managed": false,
  "is_venv": true, "hidden_user_site": null,
  "dists": [ { "name": "requests", "version": "2.32.3",
               "requires_dist": ["urllib3<3,>=1.21.1", "..."],
               "requires_python": ">=3.8" } ] }
```

`hidden_user_site` is non-null only when `-I` is actually hiding packages from the listing — see SECURITY §2 for the decision and the UI note it drives.

From `requires_dist` the core builds the **reverse-dependency graph** used by: held-back attribution, uninstall guard, pin auto-suggest, and the dependency view (P1-6). The helper is written to a temp file per invocation and never installed into the env. Compatibility floor: Python 3.10 (uses `importlib.metadata` only).

`ReverseDeps` holds **both directions** as of 1.3.0. The first three consumers each look exactly one edge out and only ever asked "who depends on this", so the forward map was never needed; the dependency view asks the other question. Both are built in the same pass from the same parse and filtered by the same `Requirement::applies_in`, and `edges_to` is a projection of `breaking_dependents` rather than a second walk — so the dependents column and the uninstall guard are the same set **by construction**. Two features applying two edge rules is the failure this module exists to prevent, and a second traversal is how it would happen.

**The graph crosses the bridge whole, once per environment** (`deps_graph` → `DepsGraph`), not a package at a time. The view re-centres on a click, so a per-package command would pay the probe again for each one; measured in `--release` on a 352-package environment, the probe is 605 ms while building the graph is 1.8 ms and computing every node's transitive counts is 43 ms, for a 249 KB payload. Nothing about which edges are in force is recomputed in the frontend — marker evaluation and extra-gating live in `graph/markers.rs`, and a JavaScript reimplementation would drift from the guard the user is warned by.

## 5. Index & metadata

- **Name index:** PyPI Simple Index in PEP 691 JSON form → SQLite (`index.db`, table `names(name, normalized)`), refreshed manually or every 7 days. Fuzzy search runs in Rust (nucleo matcher) over the normalized column; target < 50 ms per keystroke.
- **Metadata on demand:** `GET https://pypi.org/pypi/<name>/json` → summary, latest version, requires-python, license, project URLs. Cached in `meta_cache` with 24 h TTL. Strict HTTPS, no redirects off `pypi.org`.
- Offline: search still works over the cached index; metadata panel shows a cached/offline badge.

## 6. Storage (`%LOCALAPPDATA%\PipDock\data\`)

```text
config.json          # settings: engine, locale, thresholds, consent {docsHash, timestamp}
index.db             # SQLite: names, meta_cache, envs(recent), pins(env_hash, pkg, mode, reason)
snapshots/<envhash>/<iso-ts>.freeze.txt + .meta.json
tools/.venv/         # Code Health tools env (see CODE-HEALTH-SPEC)
logs/pipdock.<date>.log   # tracing rolling files, 14-day retention
```

`env_hash` = SHA-256 of the canonicalized interpreter path. Snapshot `.meta.json` records trigger (which plan), engine, package count, and app version.

## 7. Tauri IPC surface

Commands return `Result<T, PdError>` where `PdError` carries a catalog code, and are `async` except `app_info`, which is a compile-time constant. Each is a **thin** wrapper over a `pipdock_core` function — a wrapper that starts making decisions is logic the CLI will not inherit, and G5 promises the two heads never diverge.

This table is the surface. A command that is not listed here does not exist; adding one means amending this section in the same commit. It was written as a prose list of 26 with `a|b|c` groupings, which made the count ambiguous and let two Stage 1 additions land undocumented — hence the table.

| Command | Returns | Purpose |
|---|---|---|
| `app_info` | `AppInfo` | PipDock's version, and the hash of the legal documents this build ships against. |
| `env_scan` | `EnvRow[]` | Discover environments, streaming `scan-progress`. A probe failure is reported on its own row, never fatal to the scan. Carries `pipVersion` and `healthProject`, both read before the probe loop so the store guard is never held across a probe. |
| `env_add_manual` | `EnvRow` | Persist an interpreter chosen through *Browse…*. |
| `env_probe` | `EnvRow` | Probe one interpreter without persisting it. Takes an optional `source`: absent means *Browse…*, and a **refresh must pass the row's own**, or the row comes back relabelled `manual`. Fills every field `env_scan` does, because `upgradePip` replaces the row wholesale. |
| `env_export` | `string` | Write the environment's `Engine::freeze` document to a user-chosen path and return it (PRD P1-3). Byte for byte, with no formatter: a freeze *is* a requirements file, and a constraints file is the same body under a different name. Rust-side for `health_save_report`'s reason — `dialog:allow-save` asks for a path and grants no `fs`. |
| `requirements_read` | `ParsedRequirements` | Read a `requirements.txt` into install specs **and whatever it could not use**. The skipped lines are the point: an include or an editable install means the file asks for something PipDock will not do, and the user has to see that before a preview claims to represent their file. |
| `pkg_list` | `Dist[]` | Installed distributions, read from `probe.py` rather than the engine — see §4 and DATA-FLOW §7. |
| `pkg_outdated` | `OutdatedDist[]` | Outdated distributions, via the configured engine. |
| `index_search` | `Hit[]` | Fuzzy search over the in-memory name index. |
| `index_refresh` | `RefreshReport` | Re-download the PEP 691 name index. |
| `pkg_metadata` | `PackageMeta` | Cached PyPI metadata for the details panel, with its freshness. |
| `plan_resolve` | `FlowStep` | Begin an update or install flow: dry-run resolve, then derive held-back items. |
| `plan_decide` | `FlowStep` | Apply the user's 3-way conflict decisions and re-resolve. |
| `plan_execute` | `ExecutionOutcome` | Take the snapshot, then run the two-phase execution (§8). Returns the summary **and** the snapshot meta: the CLI prints the id before execution and the summary sheet needs it afterwards to offer the rollback, so one envelope beats a second command to go and look it up. |
| `plan_cancel` | `bool` | Stop the session, and say whether there was one. A *parked* flow counts and is discarded — it has no process to kill, and leaving it refuses the next plan on behalf of a preview nobody is looking at. |
| `uninstall_guard` | `GuardReport` | Reverse-dependency check over a removal set, parking the flow that would run it. Called **again** with `withDependents` for DATA-FLOW §5's *Remove dependents too*, so a dependent of a dependent surfaces on the next pass. |
| `uninstall_execute` | `ExecutionOutcome` | Snapshot, then remove. Takes `force` — §5's *Force remove only X*; without it a removal the guard objected to is refused with `PD-RES-004` before the snapshot is written. |
| `pin_list` | `Pin[]` | Pins for an environment. |
| `pin_add` | `()` | Add or replace a pin. |
| `pin_remove` | `bool` | Remove a pin; reports whether one existed. |
| `deps_graph` | `DepsGraph` | The in-force dependency graph of one environment (PRD P1-6): every installed package, its dependents and dependencies with the specifier on each edge, and its transitive `impact` and `reach`. **One call per environment, not one per package** — the view re-centres on a click, and a per-package command would pay a 605 ms probe for each one. 249 KB and 45 ms on the 352-package fixture. Uncapped, because the view's "+ N more" count has to come from the full set. |
| `pin_suggestions` | `PinSuggestion[]` | Packages worth pinning, by in-force reverse-dependency count (PRD P1-2). **One probe per call**, which is why UI-SPEC §4 puts this on the Pins screen rather than a sidebar badge — the cost is paid only by someone who opened the tab. |
| `snapshot_list` | `SnapshotMeta[]` | Snapshots for an environment, newest first. Takes the `env_hash`, not a `PyEnv`: snapshots outlive the interpreter that made them, and an environment whose Python is gone still has a history worth showing. |
| `snapshot_create` | `SnapshotMeta` | Take a snapshot on demand, outside any plan. |
| `snapshot_diff` | `Diff` | The environment against a snapshot. Claims no session — browsing a timeline must not start a flow. |
| `snapshot_rollback_preview` | `RollbackPreview` | What restoring a snapshot would do, parking the flow that would do it. Split from the execute for the reason `plan_resolve` is: what the user confirms must be the plan they were shown. |
| `snapshot_rollback` | `ExecutionOutcome` | Restore the parked snapshot — itself snapshotted first, per DATA-FLOW §8, which is why the outcome always carries one. |
| `health_run` | `HealthReport` | Run the Code Health tools against a project folder. **Not `CheckReport`** — that name is taken by `engine.check()` and is already a published `pipdock schema` contract, so two unrelated shapes would have shared one name on the wire. |
| `health_fix` | `FixReport` | Apply the gated `ruff` fix (P5). **Not `ExecutionSummary`** — there is no plan, no phase and no per-package counts in a fix, and inventing them would be four lies for one operation. Same correction §7 needed for `pip_upgrade`. |
| `health_dirty` | `number \| null` | Uncommitted entries in the parked report's project, so the fix confirm can name one. `null` means no repository, no git, or git failed — a courtesy, not a precondition. Asked again inside `health_fix`: this decides what to render, that decides what to allow. |
| `health_save_report` | `string[]` | Write a finished report as Markdown **and** JSON beside a user-chosen path, returning both. Rust-side rather than a filesystem capability: `dialog:allow-save` lets the webview ask for a path and nothing more. |
| `audit_run` | `AuditReport` | pip-audit over an environment (PRD P1-1). Takes the freeze itself, because pip is invoked with `--all` and uv is not, so the configured engine changes what is audited. Syncs its **own** venv on first use — a `msgpack` wheel gap fails this and leaves Code Health working. pip-audit failing is not an error: it lands in `problems` and the report still returns. |
| `audit_cancel` | `bool` | Stop a running audit, reporting whether there was one. Exists where `health_run` has no equivalent because an audit is **18-68 s** rather than 1.3 s, dominated by a one-off advisory-database fetch. A cancel is a *state* on the report, never `PD-INT-001`. |
| `audit_save_report` | `string[]` | Write a finished audit as Markdown **and** JSON beside a user-chosen path, returning both. `health_save_report`'s shape, for its reasons: `dialog:allow-save` asks for a path and grants no `fs`, and the report comes from the frontend because what the user asked to save is what is on their screen. |
| `engine_info` | `EngineInfo[]` | Detected version and availability per engine. Settings shows both, so this returns both. |
| `pip_upgrade` | `StepResult` | Upgrade pip itself in the selected environment. **Not `ExecutionSummary`** — there is no plan, no phase and no per-package counts, and inventing them would be four lies for one step. Carries no versions either (`from`/`to` are always absent): the caller re-probes, which it must do anyway to refresh the row. |
| `settings_get` | `Settings` | Read stored settings. |
| `settings_set` | `Settings` | Persist settings, returning what was actually stored rather than what was sent. |
| `cache_usage` | `Usage` | What PipDock has written to its own data directory (PRD P1-4). Never fails as a whole; an unreadable path reports zero bytes. |
| `cache_clear` | `number` | Delete one cache target, resolving to the bytes freed. Takes a `Target` **enum, never a path** — `cache::clear` is the only thing that turns one into a directory. `index.db` is deliberately not a target: it holds settings, pins and the consent record as well as the package index, and *clear the cache* must never take a user's pins. |
| `legal_consent_get` | `ConsentState` | Whether the legal gate can be skipped for this build's documents. |
| `legal_consent_set` | `Consent` | Record acceptance. |
| `logs_tail` | `string[]` | Tail of the in-memory log ring buffer, for the console drawer. |
| `report_bug_url` | `string` | Prefilled GitHub issue URL (ERROR-CATALOG §4). Built in Rust so it cannot drift from `pipdock self report-bug`. **Nothing is ever sent automatically.** |

Events (Tauri event channel): `plan-progress` streams live subprocess output to the in-app console; `scan-progress`; `health-progress`.

`plan-progress` carries a **tagged lifecycle**, not a bare line — amended in S3, where the console drawer and the live region were first built:

| `kind` | Payload | Meaning |
|---|---|---|
| `stepStarted` | `step, total, pkg?, phase` | Opens a section in the console drawer |
| `line` | `step, pkg?, phase, stream, line` | One line of engine output, verbatim and never localized |
| `stepFinished` | `step, total, pkg?, phase, status` | Closes the section; advances the "13 of 15 complete" live region |

It was originally specified as `{ step, pkg, phase, line }`, which made both of those features unimplementable: there was no event meaning "a step began" to group a section under, and none meaning "one finished" to count. Neither is recoverable from the text — engine output does not reliably name the package it concerns, and counting lines is not counting steps. Every step now emits exactly one `stepStarted`, any number of `line`s, and exactly one `stepFinished`, whichever way it ended. `stream` distinguishes stdout from stderr, which matters for uv in particular: uv writes its **plan** to stderr (SP-1), so stderr does not mean failure.

**One mutation session at a time, across all of them.** Update, install, uninstall and rollback share a single slot, so a second one is refused with `PD-RES-003` rather than allowed to interleave engine commands against the same environment. The commands that resume a session (`plan_decide`, `plan_execute`, `uninstall_execute`, `snapshot_rollback`) name the flow they expect and get `PD-INT-001` if a different one is parked — a frontend sequencing bug, reported as one instead of running the wrong plan.

Long operations are cancellable: `plan_cancel` trips the plan's token, which kills the child **and its whole process tree** (a Windows Job Object — `python -m pip` spawns build backends), and remaining steps are reported `Skipped` with `ExecutionSummary.cancelled` set. Already-completed steps stay applied; the snapshot covers full revert.

## 8. Execution model (two-phase)

Given a confirmed `ResolutionReport` (see DATA-FLOW §3 for the state machine):

1. **Phase A — batch fast path:** one engine invocation installing the full pinned set. Fast (especially uv) and atomic-ish. If it exits 0 → done.
2. **Phase B — isolation pass:** on Phase-A failure, re-run per package (`<engine> install pkg==ver`) sequentially in resolver-report order, collecting individual `StepResult`s. Failures are classified via the error catalog and **do not stop the loop** (owner requirement: skip and continue).
3. Post-run: `engine.check()`; summary aggregates Phase A/B results + check findings.

Uninstalls are always sequential (cheap) with the reverse-dep guard evaluated once up front against the full removal set.

## 9. Frontend architecture

Zustand stores: `useEnvStore`, `usePlanStore`, `useIndexStore`, `useSettingsStore`, `useHealthStore`. All engine data enters via typed IPC wrappers (`ui/src/ipc/*.ts`) mirroring the Rust types. **This section originally specified `specta`/`tauri-specta` and that is not what shipped:** `ui/src/ipc/generated.ts` comes from a first-party `xtask` over `schemars`, which makes drift a *test* failure rather than a compile error. `crates/pipdock-core/src/bindings.rs` records why — specta 2 is still `2.0.0-rc`, and a second derive stack over ~40 types was not worth it — and names the condition for revisiting. No component calls `invoke` directly. i18next with `en`/`vi` JSON catalogs (see I18N.md). Styling via Tailwind 4 tokens defined in UI-SPEC.

## 10. Version manifest (verified 2026-07; re-verify at implementation)

| Component | Version policy |
|---|---|
| Tauri | 2.10.x (latest 2.x line). **No `tauri-plugin-updater`** — PipDock does not update itself (SECURITY §5) |
| Rust | latest stable toolchain, pinned via `rust-toolchain.toml` |
| React / TypeScript | 19.x / latest stable |
| Vite / Tailwind | 8.1.x (Rolldown line) / 4.x |
| Node (dev only) | 24 LTS (Active LTS; EOL 2028-04) |
| pip engine | ≥ 24.0 supported; latest is 26.1.x. `--dry-run --report` requires ≥ 22.2 — PipDock offers to upgrade older pips before first plan. |
| uv engine | latest stable (0.11.x line at writing); **minimum 0.10.0**, pinned by SP-1 (`UV_MIN_VERSION`, `engine/mod.rs`) |
| Managed Pythons | 3.10 – 3.14 (3.9 is EOL) |
| Windows | 10 (1809+) / 11; WebView2 Evergreen assumed present; `longPathAware` manifest enabled |
| rusqlite | **held at 0.37** (`libsqlite3-sys` 0.35). 0.40 pulls `libsqlite3-sys` 0.38.1, whose build script uses the unstable `cfg_select!` and therefore does not compile on stable Rust 1.94.1. Revisit when the toolchain advances. |
| TypeScript | **held at 6.0.x.** 7.x is latest, but `typescript-eslint` peers on `>=4.8.4 <6.1.0`; taking 7 breaks `npm run lint`. Revisit when typescript-eslint supports TS 7. |
| ESLint | **10.x, with `eslint-plugin-react` deliberately absent.** That plugin caps at ESLint 9, which drags in a `minimatch`/`brace-expansion` chain carrying six high-severity advisories and would fail `npm audit` in CI. The one rule it was wanted for — no hardcoded JSX strings (I18N §1) — is a local rule in `eslint.config.js`, which also implements the documented allowlist precisely. |

**Dependabot will keep proposing the held versions.** Each hold above is a compile or lint failure, not a preference, so those PRs are closed rather than merged until the stated condition changes. `cargo audit` informational advisories are handled separately in `.cargo/audit.toml` (SECURITY §7).

Dependabot + `cargo audit` + `npm audit` in CI keep this table honest (RELEASE-CI.md §2).
