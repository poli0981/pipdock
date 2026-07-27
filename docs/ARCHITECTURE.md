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

### Engine selection

`Settings.engine ∈ {pip, uv}`. First run: probe `uv --version` on PATH → preselect uv if present, else pip; user can change any time. The status bar always shows the active engine. Per-env override is P2.

## 4. Environment introspection: `probe.py`

Rust cannot cheaply read a foreign env's installed metadata. PipDock embeds a single-file, stdlib-only helper (`probe.py`, ~150 lines, no third-party imports) executed as `<env-python> probe.py --json`, printing one JSON document:

```json
{ "python": "3.12.4", "prefix": "...", "externally_managed": false,
  "dists": [ { "name": "requests", "version": "2.32.3",
               "requires_dist": ["urllib3<3,>=1.21.1", "..."],
               "requires_python": ">=3.8" } ] }
```

From `requires_dist` the core builds the **reverse-dependency graph** used by: held-back attribution, uninstall guard, and pin auto-suggest. The helper is written to a temp file per invocation and never installed into the env. Compatibility floor: Python 3.10 (uses `importlib.metadata` only).

## 5. Index & metadata

- **Name index:** PyPI Simple Index in PEP 691 JSON form → SQLite (`index.db`, table `names(name, normalized)`), refreshed manually or every 7 days. Fuzzy search runs in Rust (nucleo matcher) over the normalized column; target < 50 ms per keystroke.
- **Metadata on demand:** `GET https://pypi.org/pypi/<name>/json` → summary, latest version, requires-python, license, project URLs. Cached in `meta_cache` with 24 h TTL. Strict HTTPS, no redirects off `pypi.org`.
- Offline: search still works over the cached index; metadata panel shows a cached/offline badge.

## 6. Storage (`%LOCALAPPDATA%\PipDock\`)

```text
config.json          # settings: engine, locale, thresholds, consent {docsHash, timestamp}
index.db             # SQLite: names, meta_cache, envs(recent), pins(env_hash, pkg, mode, reason)
snapshots/<envhash>/<iso-ts>.freeze.txt + .meta.json
tools/.venv/         # Code Health tools env (see CODE-HEALTH-SPEC)
logs/pipdock.<date>.log   # tracing rolling files, 14-day retention
```

`env_hash` = SHA-256 of the canonicalized interpreter path. Snapshot `.meta.json` records trigger (which plan), engine, package count, and app version.

## 7. Tauri IPC surface

Commands (all `async`, all returning `Result<T, PdError>` where `PdError` carries a catalog code):
`env_scan`, `env_add_manual`, `env_probe`, `pkg_list`, `pkg_outdated`, `index_search`, `pkg_metadata`, `plan_resolve`, `plan_execute`, `plan_cancel`, `uninstall_guard`, `uninstall_execute`, `pin_list|add|remove`, `snapshot_list|diff|rollback`, `health_run`, `health_fix`, `settings_get|set`, `logs_tail`, `pip_upgrade`, `legal_consent_get|set`.

Events (Tauri event channel): `plan-progress { step, pkg, phase, line }` streams live subprocess output to the in-app console; `scan-progress`; `health-progress`. Long operations are cancellable: `plan_cancel` kills the current child process group and marks remaining steps `Skipped(UserCancelled)` — already-completed steps stay applied (snapshot covers full revert).

## 8. Execution model (two-phase)

Given a confirmed `ResolutionReport` (see DATA-FLOW §3 for the state machine):

1. **Phase A — batch fast path:** one engine invocation installing the full pinned set. Fast (especially uv) and atomic-ish. If it exits 0 → done.
2. **Phase B — isolation pass:** on Phase-A failure, re-run per package (`<engine> install pkg==ver`) sequentially in resolver-report order, collecting individual `StepResult`s. Failures are classified via the error catalog and **do not stop the loop** (owner requirement: skip and continue).
3. Post-run: `engine.check()`; summary aggregates Phase A/B results + check findings.

Uninstalls are always sequential (cheap) with the reverse-dep guard evaluated once up front against the full removal set.

## 9. Frontend architecture

Zustand stores: `useEnvStore`, `usePlanStore`, `useIndexStore`, `useSettingsStore`, `useHealthStore`. All engine data enters via typed IPC wrappers (`ui/src/ipc/*.ts`) mirroring the Rust types (generated with `specta`/`tauri-specta` to prevent drift). No component calls `invoke` directly. i18next with `en`/`vi` JSON catalogs (see I18N.md). Styling via Tailwind 4 tokens defined in UI-SPEC.

## 10. Version manifest (verified 2026-07; re-verify at implementation)

| Component | Version policy |
|---|---|
| Tauri | 2.10.x (latest 2.x line) + `tauri-plugin-updater` |
| Rust | latest stable toolchain, pinned via `rust-toolchain.toml` |
| React / TypeScript | 19.x / latest stable |
| Vite / Tailwind | 8.1.x (Rolldown line) / 4.x |
| Node (dev only) | 24 LTS (Active LTS; EOL 2028-04) |
| pip engine | ≥ 24.0 supported; latest is 26.1.x. `--dry-run --report` requires ≥ 22.2 — PipDock offers to upgrade older pips before first plan. |
| uv engine | latest stable (0.9.x line at writing); minimum pinned after SP-1 |
| Managed Pythons | 3.10 – 3.14 (3.9 is EOL) |
| Windows | 10 (1809+) / 11; WebView2 Evergreen assumed present; `longPathAware` manifest enabled |

Dependabot + `cargo audit` + `npm audit` in CI keep this table honest (RELEASE-CI.md §2).
