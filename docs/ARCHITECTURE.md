# PipDock — Architecture

*Version 0.1 · 2026-07-17*

## 1. Design principles

1. **One core, two heads.** All domain logic lives in `pipdock-core` (Rust). The Tauri GUI and the clap CLI are thin adapters over the same functions; neither contains business logic.
2. **Explain, don't reimplement.** Dependency resolution is always performed by the selected engine (pip or uv) in dry-run mode. PipDock parses and presents; it never computes version resolution itself.
3. **Out-of-band by construction.** PipDock ships as a standalone binary. It is never installed into a Python environment, so no operation it performs can break its own runtime (G6). The binary is **not** code-signed — this line said "signed" until 1.4.0 — so what a user verifies is `SHA256SUMS.txt` and, since 1.4.0, the build-provenance attestation each release carries (SECURITY §5).
4. **Everything mutating is previewed and snapshotted.** No code path may call a mutating engine command without (a) a prior dry-run plan and (b) a pre-execution snapshot. The two exceptions — pip upkeep, and the gated `ruff --fix`, which writes outside any environment — are named rather than silent; DATA-FLOW §9 says what each mutation must present instead.
5. **Subprocess, never shell.** Engines are invoked with argv arrays (`tokio::process::Command`), never through a shell. Package names are validated against the PEP 508 name grammar, and versions and specifier sets against PEP 440 (`pep440.rs` — until 1.4.0 a version was any string), before either can reach an engine. The requirement lines of a resolve or an install travel as `-r`/`-c` temp files holding exactly those validated tokens, so the command line no longer grows with the environment (§8).

## 2. Repository layout

```text
pipdock/
├─ crates/
│  ├─ pipdock-core/          # domain logic (this is the product)
│  │  ├─ src/engine/         # Engine trait + pip.rs + uv.rs adapters
│  │  ├─ src/envs/           # discovery (PEP 514, py launcher, uv, venv scan), PEP 668
│  │  ├─ src/index/          # PEP 691 name cache, fuzzy search (nucleo), PyPI JSON metadata
│  │  ├─ src/plan/           # PlanRequest → ResolutionReport → ExecutionSummary; two-phase execution
│  │  ├─ src/flow/           # update/install, uninstall, rollback as resumable flows; PEP 668 preamble
│  │  ├─ src/graph/          # reverse-dependency graph from env introspection, PEP 508 markers
│  │  ├─ src/snapshot/       # freeze snapshots, diff, minimal-ops rollback
│  │  ├─ src/pins/           # pin store + auto-suggest scoring
│  │  ├─ src/health/         # tools-venv manager, deptry/vulture/ruff runners
│  │  ├─ src/errors/         # error catalog (codes + stderr classifiers)
│  │  ├─ src/store/          # the SQLite store, index.db (§6)
│  │  ├─ src/*.rs            # single-file modules: exec (argv, watchdogs, temp files), compat +
│  │  │                      #   pep440, release (per-release Requires-Python), logging, audit,
│  │  │                      #   cache, report, requirements, settings
│  │  └─ src/probe.py        # embedded env-introspection helper (see §4)
│  └─ pipdock-cli/           # clap binary `pipdock`
├─ src-tauri/                # Tauri 2 app (commands + events only)
├─ ui/                       # React 19 + TS + Tailwind 4 + Vite 8 + Zustand + i18next
├─ legal/                    # EULA, Disclaimer, Privacy, Third-Party (public — legal gate links here)
├─ .github/                  # caller workflows into poli0981/.github + issue templates
└─ docs/
```

Naming conventions: crates `pipdock-*`; React components prefixed `Pd*` (`PdPackageRow`, `PdConflictRow`); Tauri identifier `com.skullmute.pipdock`; CLI binary `pipdock`.

## 3. The Engine trait

```rust
#[async_trait]
pub trait Engine: Send + Sync {
    fn id(&self) -> EngineId;                                  // Pip | Uv
    async fn info(&self, env: &PyEnv) -> EngineInfo;           // version, availability
    async fn list_installed(&self, env: &PyEnv) -> Result<Vec<Dist>>;
    async fn list_outdated(&self, env: &PyEnv) -> Result<Vec<OutdatedDist>>;
    async fn resolve(&self, env: &PyEnv, req: &PlanRequest, ctx: &ResolveCtx)
        -> Result<ResolutionReport>;
    async fn install(&self, env: &PyEnv, set: &InstallSet, mode: ExecMode,
                     sink: ProgressSink) -> Result<StepResult>;
    async fn uninstall(&self, env: &PyEnv, names: &[PkgName], system: SystemPackages,
                       sink: ProgressSink) -> Result<StepResult>;
    async fn check(&self, env: &PyEnv) -> Result<CheckReport>;      // pip check / uv pip check
    async fn freeze(&self, env: &PyEnv) -> Result<String>;          // snapshots, export, audit
    async fn upgrade_pip(&self, env: &PyEnv) -> Result<StepResult>; // pip only; uv refuses (PD-ENG-001)
    async fn probe(&self, env: &PyEnv) -> Result<ProbeResult> {     // provided: probe.py, §4
        envs::probe(&env.interpreter, env.source).await
    }
}

pub struct ResolveCtx  { cancel: CancellationToken, adjust: GuardAdjust, system: SystemPackages }
pub struct GuardAdjust { pins: Vec<PinnedSpec>, release: Vec<PkgName>, exclude: Vec<PinnedSpec> }
pub struct InstallSet  { specs: Vec<PinnedSpec>, constraints: Vec<PinnedSpec>, system: SystemPackages }
pub enum SystemPackages { Protected /* default */, OverrideAllowed }
```

The signatures moved in 1.4.0, each for a defect the old one made expressible:

- **`ResolveCtx`.** A resolve was the one engine call nothing could stop — *Stop* while pip backtracked left it running for up to ten minutes (H13) — so it now carries the flow's cancellation token, and a stopped resolve is a `nothing` step, not an error. `GuardAdjust` is what the *flow* changes about the requirement lines, never an adapter: `pins` are exact requirements whose names leave the upgrade list and the guard set (*Force latest*, `Hold` pins), `release` takes installed packages out of the guard set (what a forced package will break), and `exclude` rides along as `name!=version` (a release whose `Requires-Python` excludes this interpreter). `system` is for uv, which refuses even a dry run in an externally managed environment without the flag; pip's `--dry-run --report` is exempt and is never given it.
- **`InstallSet`.** SP-1's rule — restate the installed set or the resolver breaks it — had been applied to the resolve only, and Phase B ran a bare `pip install name==v`, free to move any dependency. `constraints` is everything that must stay where it is, passed as `-c` (§8).
- **`SystemPackages`** is a named value rather than a `bool` for the reason `SnapshotProof` is a named waiver: hard invariant 5 says `--break-system-packages` is never passed implicitly, and this makes "implicitly" greppable. Only a flow's preamble produces `OverrideAllowed` (DATA-FLOW §2).
- **`probe` is provided and no adapter overrides it** — it is `probe.py`, not an engine command. It is on the trait so a whole flow can be driven against a scripted engine (`crates/pipdock-core/tests/flows.rs`): every flow used to probe through a free function, so nothing `start` decides — the PEP 668 gate, the graph, the drift basis — could be tested without a real interpreter, and the bugs in them were not. `freeze` was on the trait before 1.4.0 and missing from this block.

Key model types:

- `PlanRequest` — the user's intent: `{ upgrades: Vec<PkgName>, installs: Vec<Spec>, strategy: Compatible | ForceLatest(per-pkg overrides) }`. A `Spec`'s version requirement is a PEP 440 specifier set, checked by `Spec::parse` and on deserialization.
- `ResolutionReport` — normalized output of the dry-run: `{ changes: Vec<Change>, held_back: Vec<HeldBack{ pkg, resolved, latest, blockers: Vec<Blocker{by, version, constraint}> }>, impossible: Option<ImpossibleDetail>, incompatible: Vec<PythonMismatch{pkg, version, requiresPython, python}>, forced: Vec<HeldBack>, yanked: Vec<YankWarning{pkg, version, reason}> }`; the last three are omitted when empty. **Both adapters must emit this same shape** — this is the whole point of the trait. The pip adapter fills it from `--dry-run --report` JSON; the uv adapter from its dry-run plan output (format pinned during spike SP-1; fixtures under `crates/pipdock-core/tests/fixtures/` — `uv/` is uv 0.13's `error:`/`cause:` rendering, `uv-0.11/` the frozen miette one, and both are under test). Both read the **exit status first**: zero is a plan, an impossibility is `impossible`, and anything else is classified like every other engine failure. uv's adapter used to hand every outcome to the plan parser, so a failed build that still printed `Resolved 1 package` previewed as "no changes" (C3); pip's impossibility was a hard error that failed the whole preview where uv's offered a per-package *Skip* (H15).
- **What is not on the wire.** `raw` — what the engine wrote while resolving, now pip's stderr and uv's plan text — and `declared_python` — each planned release's `Requires-Python` as pip's report states it — are `#[serde(skip)]`, kept for the log ring and the flow's own check. `raw` used to cross IPC as pip's whole `--report` document, every package's README included, cloned into each `FlowStep` and printed by `--json`: 33 KB for two packages, megabytes for "update all" (H22).
- `incompatible`, `forced` and `yanked` are new in 1.4.0. `incompatible` is DATA-FLOW §2's Requires-Python screening, structured so the preview can show the range against the interpreter (invariant 4); `forced` is the "Will break" section DISCLAIMER §2 requires before anything runs; `yanked` is PD-PKG-003's preview warning, which both parsers had extracted and then dropped (`.map(|p| p.report)`), so it never showed.
- `Blocker` computation: the engine's report says *what* was held back; *who* is responsible comes from cross-referencing the reverse-dependency graph (§4) with each blocker's `Requires-Dist` constraints. If attribution is ambiguous, show constraints without a culprit rather than guessing.

### Requires-Python is enforced by PipDock, not the engine

**Owner decision 2026-07-27 (spike SP-2).** The two engines disagree: `scipy==1.7.3` declares `Requires-Python >=3.7,<3.11`, and on Python 3.12 pip refuses it while uv plans to install it. Since G5 promises one behavior across heads, the preview must not change shape because the user flipped the engine radio — so `pipdock-core` evaluates `Requires-Python` itself, before any mutating engine command, and reports rejects as `PD-PKG-001` with the required range against the environment's version.

**Until 1.4.0 nothing did** (C2): this section described the decision, `PD-PKG-001` sat in the catalog, and no code path raised it. pip refused an incompatible exact pin with the "No matching distribution" it gives a typo, and uv — which honours a lower bound and ignores an upper one — planned `scipy==1.7.3` on Python 3.14 and failed at install time, building it from source. `UpdateFlow` now checks three places, with `src/compat.rs` evaluating the specifier each time:

1. **Exact pins the user typed** (`name==v`, `name===v`), before any engine sees them. A request that is nothing but such pins is `PD-PKG-001`; otherwise the pin is left out and listed in `report.incompatible`.
2. **Forced pins.** Forcing a latest release this interpreter cannot run is dropped from the force and listed the same way.
3. **Every change the engine planned.** pip's report carries each release's `requires_python`, so nothing is asked; uv's plan carries nothing, so PyPI is asked (`src/release.rs`). A violating release is ruled out as `name!=version` (`GuardAdjust.exclude`) and the engine asked again, at most `PYTHON_EXCLUDE_ROUNDS` (2) times — the engine, not PipDock, picks what comes next. Whatever still violates after that stays in the plan and is listed in `report.incompatible` as a warning.

`release.rs` reads `https://pypi.org/pypi/{name}/{version}/json` → `info.requires_python`, at most `LOOKUP_CONCURRENCY` (8) at once with a `LOOKUP_TIMEOUT` (15 s) each, and remembers every answer **forever** in the store's `release_meta` table (§6): an uploaded release's metadata never changes. A failure is not remembered, and is asked again next time. Not screened: a rollback's restore set, which reinstalls what was there (audit backlog N4).

This does not violate "explain, don't reimplement" (§1.2): resolution is still entirely the engine's. `compat.rs` only filters candidates the engine should never have been offered, using a narrow slice of PEP 440 — the comparison, wildcard and compatible-release operators as they appear in `Requires-Python` — and a release it rules out is excluded *by name* for the engine to resolve around. **Unknown never blocks**: a specifier it cannot parse, a release PyPI does not know (a private index), a lookup that is offline or slow — each is treated as **compatible**, because PipDock failing to read metadata must never be the reason an installable package is refused.

### Engine selection

`Settings.engine ∈ {pip, uv}`, pip by default; user can change any time. First run was specified to probe `uv --version` on PATH and preselect uv if present — **that is not implemented**: an empty store reads as pip (`Settings::default`), and detecting uv at first run is owed (PRD P0-9, audit backlog X19). The status bar always shows the active engine, read when the app starts rather than when Settings is first opened (1.4.0). Per-env override is P2.

## 4. Environment introspection: `probe.py`

Rust cannot cheaply read a foreign env's installed metadata. PipDock embeds a single-file, stdlib-only helper (`probe.py`, ~330 lines, no third-party imports) executed as `<env-python> -X utf8 -I <temp>.py --json` under a 60 s watchdog (`exec::watchdog::PROBE`), printing one JSON document, schema 3:

```json
{ "schema": 3, "python": "3.12.4", "implementation": "cpython",
  "prefix": "...", "base_prefix": "...", "executable": "...",
  "is_venv": true, "externally_managed": false, "hidden_user_site": null,
  "markers": { "sys_platform": "win32", "platform_system": "Windows",
               "python_version": "3.12", "python_full_version": "3.12.4", "...": "..." },
  "dists": [ { "name": "requests", "version": "2.32.3",
               "requires_dist": ["urllib3<3,>=1.21.1", "..."],
               "requires_python": ">=3.8", "size_bytes": 131072 } ] }
```

`hidden_user_site` is non-null only when `-I` is actually hiding packages from the listing — see SECURITY §2 for the decision and the UI note it drives.

**`-X utf8` is not redundant.** `-I` implies `-E`, which ignores `PYTHONUTF8` and `PYTHONIOENCODING`, so the probe wrote through the ANSI code page: a venv under a folder named with Vietnamese diacritics crashed it with a `UnicodeEncodeError` (`PD-ENV-003`, reproduced) and a Latin-1 path came back as mojibake (H10). `-X` is a command-line option, which isolated mode honours. Schema 3 also reads each distribution's `METADATA` **once** — `dist.metadata`, then `dist.version` and `dist.requires`, had parsed it three times on the path every environment open pays (154 → 118 ms for 64 distributions on CPython 3.14).

From `requires_dist` the core builds the **reverse-dependency graph** used by: held-back attribution, uninstall guard, pin auto-suggest, and the dependency view (P1-6). The helper is written to a temp file per invocation and never installed into the env. Compatibility floor: Python 3.10 (uses `importlib.metadata` only).

**Markers are evaluated against the interpreter the probe ran in** (`ReverseDeps::for_probe`, which every production graph uses — held-back attribution in the update flow, the uninstall guard, `deps_graph`, pin suggestions). Until 1.4.0 only `python_version` and `python_full_version` were read and every other variable counted as unknown, which keeps a requirement in force: ipython's `pexpect; sys_platform != "win32"` made the dependency view report pexpect missing from every Windows environment with ipython, and the guard and suggestions counted an edge that does not exist there (H25). Platform variables compare as strings with `==`, `!=`, `in` and `not in`. Schema 3's `markers` is built from `sys` and `os`, not `platform`: `platform.uname()` is a WMI query on Windows (55 ms on 3.14) for `platform_release` and `platform_version`, which almost no requirement mentions — so those two are omitted on Windows, and a requirement that names them stays in force.

`ReverseDeps` holds **both directions** as of 1.3.0. The first three consumers each look exactly one edge out and only ever asked "who depends on this", so the forward map was never needed; the dependency view asks the other question. Both are built in the same pass from the same parse and filtered by the same `Requirement::applies_in`, and `edges_to` is a projection of `breaking_dependents` rather than a second walk — so the dependents column and the uninstall guard are the same set **by construction**. Two features applying two edge rules is the failure this module exists to prevent, and a second traversal is how it would happen.

**The graph crosses the bridge whole, once per environment** (`deps_graph` → `DepsGraph`), not a package at a time. The view re-centres on a click, so a per-package command would pay the probe again for each one; measured in `--release` on a 352-package environment, the probe is 605 ms while building the graph is 1.8 ms and computing every node's transitive counts is 43 ms, for a 249 KB payload. Nothing about which edges are in force is recomputed in the frontend — marker evaluation and extra-gating live in `graph/markers.rs`, and a JavaScript reimplementation would drift from the guard the user is warned by.

## 5. Index & metadata

- **Name index:** PyPI Simple Index in PEP 691 JSON form → SQLite (`index.db`, table `names(name, normalized)`), refreshed on request — nothing refreshes it on a timer; older than 7 days it is reported stale (`index::is_stale`; `pipdock search` says so). The download is parsed as a typed, borrowing document rather than a `serde_json::Value` of 700k objects, and ingested on a blocking thread; an empty project list is refused rather than ingested over the index (1.4.0). Fuzzy search runs in Rust (nucleo matcher) over an in-memory copy of the normalized column (SP-3), which the GUI loads in the background on a connection of its own: a search that arrives first answers `ready: false` and the screen asks again, rather than holding the store's lock for the 140 ms load (U35). Target < 50 ms per keystroke.
- **Metadata on demand:** `GET https://pypi.org/pypi/<name>/json` → summary, latest version, requires-python, license, project URLs. Cached in `meta_cache` with 24 h TTL. Strict HTTPS with verification always on; redirects may not leave `pypi.org` or `files.pythonhosted.org`. A TLS failure is recognised by reading the error's whole `source()` chain — reqwest's top-level message never mentions the certificate, so every TLS failure used to read as `PD-NET-001` and `PD-NET-002`'s proxy guidance never showed (M32).
- **Per-release `Requires-Python`** (`release.rs`, 1.4.0): `GET https://pypi.org/pypi/<name>/<version>/json`, for uv's plans and exact pins (§3), through one client shared by the process; remembered forever in `release_meta`.
- Offline: search still works over the cached index; metadata panel shows a cached/offline badge; a `Requires-Python` lookup that cannot be answered never blocks a plan.

## 6. Storage (`%LOCALAPPDATA%\PipDock\data\`)

```text
index.db             # SQLite, schema v4: names, meta_cache, envs (recents, default),
                     #   pins(env_hash, pkg, mode, version, reason), health_projects,
                     #   kv (settings, consent, index refresh stamp), release_meta
snapshots/<env_hash>/<id>.freeze.txt + <id>.meta.json   # id: 20261016T145809-423560600Z
tools/.venv/         # Code Health tools env (see CODE-HEALTH-SPEC)
audit/.venv/         # the Security tab's pip-audit env (PRD P1-1), a sibling cleared separately
logs/pipdock.<date>.log, logs/pipdock-cli.<date>.log   # one file per head per UTC day, 14 days
```

**There is no `config.json`.** This block listed one until 1.4.0; settings and the consent record have always been rows in `index.db`'s `kv` table (`settings::load`), because the store already exists, already handles concurrent access, and a second on-disk format is a second thing to migrate.

`env_hash` = SHA-256 of the canonicalized interpreter path, case-folded on Windows (SP-6). Snapshot `.meta.json` records trigger (which plan), engine, package count, and app version; the engine is what a rollback with the other engine reads (DATA-FLOW §8). Snapshot ids are fixed width, so they sort as the times they name — the timestamp with its punctuation swapped out used to trim its fraction, so `…09-42Z` sorted after `…09-4235606Z` — and the timeline orders by `created_at` either way (M43).

**The store migrates only when it is behind.** `user_version` is compared with `store::SCHEMA_VERSION` (4) and the batch runs only when the database is older: every open used to run it — a `journal_mode` write and eight `CREATE TABLE`s, on a store some paths open twice per lookup — and an older build set the version back down, so the next newer one re-ran them (M38). v4 added `release_meta(normalized, version, requires_python)`: a release's `Requires-Python` as PyPI publishes it, never refreshed because an uploaded release's metadata never changes. A NULL there means "declares none"; "unknown" is never stored.

**The log is new in 1.4.0.** `tracing` had been a dependency of three crates and was called nowhere, and `LOG_RETENTION_DAYS` had no reader. `core::logging` writes one file per UTC day, prefixed by head so the GUI and the CLI never interleave, at `PIPDOCK_LOG` or INFO: every spawn's argv, exit code and duration, every stop and its reason, each resolve's outcome, each snapshot, each run's counts or refusal, and the engine's own lines at DEBUG. Each head sweeps files older than `LOG_RETENTION_DAYS` (14) when it starts and never touches a file it did not name; a panic hook records a crash before the default hook prints it. Nothing is ever written to stdout — the CLI's `--verbose` adds stderr at DEBUG and `--log-file` a copy. The cache manager does not clear the log, and Settings shows where it is (`logs_tail`, §7).

The GUI opens the store **before Tauri starts**, so a data folder that cannot be opened is said in a native dialog — the code, the message and the log's path, in both languages, the one place GUI copy is written in Rust because no webview exists yet to load a catalog. It used to be opened in Tauri's `setup` hook, whose failure is a panic in the event loop; a release build has no console, so the user double-clicked and nothing happened (U18).

## 7. Tauri IPC surface

Commands return `Result<T, WireError>` — a `PdError` flattened to `{ code, message, stderrTail? }`, so it carries a catalog code and never the typed `stop` an engine run records — and are `async` except `app_info`, which is a compile-time constant. Each is a **thin** wrapper over a `pipdock_core` function — a wrapper that starts making decisions is logic the CLI will not inherit, and G5 promises the two heads never diverge.

This table is the surface. A command that is not listed here does not exist; adding one means amending this section in the same commit. It was written as a prose list of 26 with `a|b|c` groupings, which made the count ambiguous and let two Stage 1 additions land undocumented — hence the table.

| Command | Returns | Purpose |
|---|---|---|
| `app_info` | `AppInfo` | PipDock's version, and the hash of the legal documents this build ships against. |
| `env_scan` | `EnvRow[]` | Discover environments, streaming `scan-progress`. A probe failure is reported on its own row, never fatal to the scan. Carries `pipVersion` and `healthProject`, both read before the probe loop so the store guard is never held across a probe. Probes run four at a time (they ran in series until 1.4.0), each under its own 60 s watchdog, so one hung interpreter costs its own row; rows keep discovery order. |
| `env_add_manual` | `EnvRow` | Persist an interpreter chosen through *Browse…*. **Not implemented** — the one entry left in `NOT_YET` (`src-tauri/src/lib.rs`), owed by M3. |
| `env_probe` | `EnvRow` | Probe one interpreter without persisting it. Takes an optional `source`: absent means *Browse…*, and a **refresh must pass the row's own**, or the row comes back relabelled `manual`. Fills every field `env_scan` does, because `upgradePip` replaces the row wholesale. |
| `env_export` | `string` | Write the environment's `Engine::freeze` document to a user-chosen path and return it (PRD P1-3). Byte for byte, with no formatter: a freeze *is* a requirements file, and a constraints file is the same body under a different name. Rust-side for `health_save_report`'s reason — `dialog:allow-save` asks for a path and grants no `fs`. |
| `requirements_read` | `ParsedRequirements` | Read a `requirements.txt` into install specs **and whatever it could not use**. The skipped lines are the point: an include or an editable install means the file asks for something PipDock will not do, and the user has to see that before a preview claims to represent their file. |
| `pkg_list` | `Dist[]` | Installed distributions, read from `probe.py` rather than the engine — see §4 and DATA-FLOW §7. |
| `pkg_outdated` | `OutdatedDist[]` | Outdated distributions, via the configured engine. |
| `index_search` | `SearchResults` | Fuzzy search over the in-memory name index: `{ hits: Hit[], ready, unavailable? }`. A struct, not the `Hit[]` this row said, because "no such package" and "the index is still loading" are different answers. Never blocks on the load: the first search starts it in the background and answers `ready: false`, and the screen asks again. A load that failed — most often, an index never downloaded — is reported in `unavailable` and **not** retried by the next search, only by a refresh; retrying reset it to warming before the reason could be read, and Search showed "preparing" for ever on a fresh profile. |
| `index_refresh` | `RefreshReport` | Re-download the PEP 691 name index. |
| `pkg_metadata` | `[PackageMeta, Freshness]` | Cached PyPI metadata for the details panel, and how fresh it is (`fresh`, `cached`, `stale`) — a tuple on the wire, which this row used to write as `PackageMeta` alone. |
| `plan_resolve` | `FlowStep` | Begin an update or install flow: probe, PEP 668 gate, dry-run resolve, then derive held-back items. The cancellation token it registers is **the flow's own**, handed in before the first resolve, so *Stop* reaches that resolve — the one it registered used to be a fresh token nothing listened to (H13). A stopped resolve answers `{ step: "nothing", reason: "cancelled" }`. |
| `plan_decide` | `FlowStep` | Apply the user's 3-way conflict decisions and re-resolve. The flow stays parked whether or not the round succeeds: a failed re-resolve leaves it exactly as it was, so the preview on screen can still be decided or confirmed — it used to be dropped, leaving a Confirm that answered "no plan" (U24). |
| `plan_execute` | `ExecutionOutcome` | Take the snapshot, then run the two-phase execution (§8). The environment is probed again first, and a preview older than ten minutes or no longer describing the environment is refused with `PD-RES-002` before the snapshot is written (DATA-FLOW §9.3). Returns the summary **and** the snapshot meta: the CLI prints the id before execution and the summary sheet needs it afterwards to offer the rollback, so one envelope beats a second command to go and look it up. |
| `plan_cancel` | `bool` | Stop the session, and say whether there was one — a resolve as well as an execution. A *parked* flow counts and is discarded — it has no process to kill, and leaving it refuses the next plan on behalf of a preview nobody is looking at. |
| `uninstall_guard` | `GuardReport` | Reverse-dependency check over a removal set, parking the flow that would run it. Called **again** with `withDependents` for DATA-FLOW §5's *Remove dependents too*, so a dependent of a dependent surfaces on the next pass. |
| `uninstall_execute` | `ExecutionOutcome` | Snapshot, then remove. Takes `force` — §5's *Force remove only X*; without it a removal the guard objected to is refused with `PD-RES-004` before the snapshot is written, and the flow stays parked, so the dialog's *Force remove* is still a choice rather than a button answering "no plan". The environment is probed again before the snapshot: a package installed since the guard ran may depend on what is about to go, so a changed environment is `PD-RES-002` (M44). |
| `pin_list` | `Pin[]` | Pins for an environment. |
| `pin_add` | `()` | Add or replace a pin. |
| `pin_remove` | `bool` | Remove a pin; reports whether one existed. |
| `deps_graph` | `DepsGraph` | The in-force dependency graph of one environment (PRD P1-6): every installed package, its dependents and dependencies with the specifier on each edge, and its transitive `impact` and `reach`. **One call per environment, not one per package** — the view re-centres on a click, and a per-package command would pay a 605 ms probe for each one. 249 KB and 45 ms on the 352-package fixture. Uncapped, because the view's "+ N more" count has to come from the full set. |
| `pin_suggestions` | `PinSuggestion[]` | Packages worth pinning, by in-force reverse-dependency count (PRD P1-2). **One probe per call**, which is why UI-SPEC §4 puts this on the Pins screen rather than a sidebar badge — the cost is paid only by someone who opened the tab. |
| `snapshot_list` | `SnapshotMeta[]` | Snapshots for an environment, newest first. Takes the `env_hash`, not a `PyEnv`: snapshots outlive the interpreter that made them, and an environment whose Python is gone still has a history worth showing. |
| `snapshot_create` | `SnapshotMeta` | Take a snapshot on demand, outside any plan. |
| `snapshot_diff` | `Diff` | The environment against a snapshot. Claims no session — browsing a timeline must not start a flow. |
| `snapshot_rollback_preview` | `RollbackPreview` | What restoring a snapshot would do, parking the flow that would do it. Split from the execute for the reason `plan_resolve` is: what the user confirms must be the plan they were shown. Gated on PEP 668 like every mutating flow — the rollback flow had no gate at all until 1.4.0 (H12). |
| `snapshot_rollback` | `ExecutionOutcome` | Restore the parked snapshot — itself snapshotted first, per DATA-FLOW §8, which is why the outcome always carries one. The freeze taken for that snapshot must match the one the preview was diffed against, or the restore is refused with `PD-RES-002` and nothing is written. |
| `health_run` | `HealthReport` | Run the Code Health tools against a project folder. **Not `CheckReport`** — that name is taken by `engine.check()` and is already a published `pipdock schema` contract, so two unrelated shapes would have shared one name on the wire. |
| `health_fix` | `FixReport` | Apply the gated `ruff` fix (P5). **Not `ExecutionSummary`** — there is no plan, no phase and no per-package counts in a fix, and inventing them would be four lies for one operation. Same correction §7 needed for `pip_upgrade`. Its argv is the read-only run's plus `--fix`, so it rewrites only what the report covered: it ran `check . --fix` with none of the preview's exclusions until 1.4.0, and rewrote every notebook in the tree (C4, DATA-FLOW §9.1). |
| `health_dirty` | `number \| null` | Uncommitted entries in the parked report's project, so the fix confirm can name one. `null` means no repository, no git, or git failed — a courtesy, not a precondition. Asked again inside `health_fix`: this decides what to render, that decides what to allow. |
| `health_save_report` | `string[]` | Write a finished report as Markdown **and** JSON beside a user-chosen path, returning both. Rust-side rather than a filesystem capability: `dialog:allow-save` lets the webview ask for a path and nothing more. |
| `audit_run` | `AuditReport` | pip-audit over an environment (PRD P1-1). Takes the freeze itself, because pip is invoked with `--all` and uv is not, so the configured engine changes what is audited. Only the freeze's `name==version` pins reach pip-audit: an editable install or a direct reference, which no index can answer for, failed the whole audit until 1.4.0 (H21). Syncs its **own** venv on first use — a `msgpack` wheel gap fails this and leaves Code Health working. pip-audit failing is not an error: it lands in `problems` and the report still returns. |
| `audit_cancel` | `bool` | Stop a running audit, reporting whether there was one. Exists where `health_run` has no equivalent because an audit is **18-68 s** rather than 1.3 s, dominated by a one-off advisory-database fetch. A cancel is a *state* on the report, never `PD-INT-001`. `audit_run` registers its token before anything slow, so a Stop during the freeze is honoured — it was registered after, and such a Stop did nothing (U37). |
| `audit_save_report` | `string[]` | Write a finished audit as Markdown **and** JSON beside a user-chosen path, returning both. `health_save_report`'s shape, for its reasons: `dialog:allow-save` asks for a path and grants no `fs`, and the report comes from the frontend because what the user asked to save is what is on their screen. |
| `engine_info` | `EngineInfo[]` | Detected version and availability per engine, for one environment — pip's version is that environment's pip. Settings shows both, so this returns both. The wrapper had no caller until 1.4.0, when Settings began showing the version beside each engine radio for the selected environment. |
| `pip_upgrade` | `StepResult` | Upgrade pip itself in the selected environment. **Not `ExecutionSummary`** — there is no plan, no phase and no per-package counts, and inventing them would be four lies for one step. Carries no versions either (`from`/`to` are always absent): the caller re-probes, which it must do anyway to refresh the row. |
| `settings_get` | `Settings` | Read stored settings. |
| `settings_set` | `Settings` | Persist settings, returning what was actually stored rather than what was sent. |
| `cache_usage` | `Usage` | What PipDock has written to its own data directory (PRD P1-4). Never fails as a whole; an unreadable path reports zero bytes. |
| `cache_clear` | `number` | Delete one cache target, resolving to the bytes freed. Takes a `Target` **enum, never a path** — `cache::clear` is the only thing that turns one into a directory. `index.db` is deliberately not a target: it holds settings, pins and the consent record as well as the package index, and *clear the cache* must never take a user's pins. Holds the slot of the feature that owns the target — the mutation slot for snapshots, Health's for the tools venv, the Security tab's for the audit venv — so a clear is refused with `PD-RES-003` while that feature runs instead of deleting the venv it is executing from; a session merely parked there is put back (U36). The deletion runs on a blocking thread. |
| `legal_consent_get` | `ConsentState` | Whether the legal gate can be skipped for this build's documents. |
| `legal_consent_set` | `Consent` | Record acceptance. |
| `logs_tail` | `LogTail` | The end of PipDock's own log files (`data\logs\`, kept `LOG_RETENTION_DAYS`) and the folder they are in, for Settings' Logs row. Never fails: no log yet is an empty tail. |
| `report_bug_url` | `BugReportLink` | `{ url, log }`: the prefilled GitHub issue URL (ERROR-CATALOG §4) and the whole log ring for *Copy full log* — two fields, not the `string` this row said, because §4.3 splits the truncated excerpt from the full log. Takes an optional environment and code; given the environment, the URL carries its Python and the engine's version (U31). Built in Rust so it cannot drift from `pipdock self report-bug`. **Nothing is ever sent automatically.** |

Events (Tauri event channel, the closed set `EVENTS` in `ui/src/ipc/index.ts`): `plan-progress` streams live subprocess output to the in-app console; `scan-progress` (`ScanProgress`, one per discovery source); `health-progress` and `audit-progress`, which carry `plan-progress`'s `ProgressEvent` lifecycle for a tools-venv sync and the tool runs. `audit-progress` shipped with the Security tab in 1.2.0, and this line did not name it until 1.4.0.

**What the webview may call besides these commands is exact.** `src-tauri/capabilities/` grants seven permissions — `core:event:allow-listen` and `-unlisten` for the four channels, `core:window:allow-close` for the legal gate's *Decline*, `dialog:allow-open` and `dialog:allow-save` (both only ask for a path; no `fs` permission exists), `clipboard-manager:allow-write-text`, and `opener:allow-open-url` scoped to four hosts — and `the_webview_is_granted_exactly_what_it_calls` in `src-tauri/src/lib.rs` pins the set. `core:default` stood there until 1.4.0: it granted some sixty core commands nothing called and not `allow-close`, so *Decline* did nothing in the installed app while its component test, which mocks `close()`, passed (U1, U32).

`plan-progress` carries a **tagged lifecycle**, not a bare line — amended in S3, where the console drawer and the live region were first built:

| `kind` | Payload | Meaning |
|---|---|---|
| `stepStarted` | `step, total, pkg?, phase` | Opens a section in the console drawer |
| `line` | `step, pkg?, phase, stream, line` | One line of engine output, verbatim and never localized |
| `stepFinished` | `step, total, pkg?, phase, status` | Closes the section; advances the "13 of 15 complete" live region |

It was originally specified as `{ step, pkg, phase, line }`, which made both of those features unimplementable: there was no event meaning "a step began" to group a section under, and none meaning "one finished" to count. Neither is recoverable from the text — engine output does not reliably name the package it concerns, and counting lines is not counting steps. Every step now emits exactly one `stepStarted`, any number of `line`s, and exactly one `stepFinished`, whichever way it ended. `stream` distinguishes stdout from stderr, which matters for uv in particular: uv writes its **plan** to stderr (SP-1), so stderr does not mean failure.

**One mutation session at a time, across all of them.** Update, install, uninstall and rollback share a single slot, so a second one is refused with `PD-RES-003` rather than allowed to interleave engine commands against the same environment. The commands that resume a session (`plan_decide`, `plan_execute`, `uninstall_execute`, `snapshot_rollback`) name the flow they expect and get `PD-INT-001` if a different one is parked — a frontend sequencing bug, reported as one instead of running the wrong plan. Health and the Security tab each have a slot of their own, and `cache_clear` borrows whichever owns its target.

**A claimed slot releases itself.** A claim is a guard that frees the slot when it is dropped unless it was parked first (1.4.0, U4). Release used to be a call every exit path had to remember, and `plan_resolve`, `uninstall_guard` and the rollback preview each had a `?` between claim and release: one failed settings read left the slot `Busy`, and every later plan answered `PD-RES-003` until a restart. The slot is a `std` mutex, held only to swap a value, so the drop can take it.

Long operations are cancellable: `plan_cancel` trips the plan's token, which kills the child **and its whole process tree** (a Windows Job Object — `python -m pip` spawns build backends). The token is the flow's own from the first resolve (`FlowOptions.cancel`), so a stopped resolve ends as a `nothing` step with reason `cancelled`; a stopped run reports its remaining steps `Skipped` with `ExecutionSummary.cancelled` set — during Phase A as much as Phase B, where Phase A used to surface as `PD-INT-001` with no summary (C5). Already-completed steps stay applied; the snapshot covers full revert.

## 8. Execution model (two-phase)

Given a confirmed `ResolutionReport` (see DATA-FLOW §3 for the state machine):

1. **Phase A — batch fast path:** one engine invocation installing the full pinned set, with everything installed that the plan does not touch held at its current version by `-c`. Fast (especially uv) and atomic-ish. If it exits 0 → done. **Its failure is an outcome, never an abort:** a Stop marks every row `Skipped` with `cancelled` set; a failure that is the *environment's* rather than one package's — the network, a read-only site-packages, a full disk, a missing or too-old engine, the watchdog, an interpreter that is gone or managed (`plan::isolates`) — fails every row with the batch's code, the stderr tail on the first row only, and skips Phase B, which would meet the same wall N times after pip's own retry backoff. Until 1.4.0 the adapters' `Err` for a stop, a watchdog or a spawn failure went out of `execute` through a `?` as `PD-INT-001`, with no summary and no rollback offer (C5).
2. **Phase B — isolation pass:** on any other Phase-A failure, re-run per package sequentially in resolver-report order, collecting individual `StepResult`s. Each step installs one pin with the rest of the plan at its planned versions and the untouched set at its current ones as `-c` — it ran a bare `<engine> install pkg==ver`, free to move a dependency past an installed package's bound, which pip only warns about (H16). Failures are classified via the error catalog and **do not stop the loop** (owner requirement: skip and continue).
3. Post-run: `engine.check()`; summary aggregates Phase A/B results + check findings. A check that could not run is `ok: false` with its code in `CheckReport.error` — it reported `ok: true`, "verified" exactly when nothing was, such as after a run that broke pip (M30).

Every row says what it moved **from** (`StepResult.from`, taken from the plan's changes — the engines cannot know it, so every upgrade used to read as a fresh install). The summary's `planId` is the flow's for an update or install — the same id its snapshot's trigger records, unique per plan (M31: the snapshot said `update-<env>` for every plan an environment ever had) — and the pre-execution snapshot's id for a removal or a rollback.

**Requirement files, not argv.** Every plan restates the whole installed set (SP-1), and as one argv entry per package that passes Windows' 32 767-character command line at about 1 300 packages: the spawn failed with os error 206 and the user was told to install an engine (H20). `engine::RequirementFiles` writes the requirement lines and the constraints to `-r`/`-c` temp files under fresh random names, created with `create_new` and deleted on drop (`exec::TempFile`), holding exactly the tokens argv held. pip names a requirement read from a file `-r <path> (line N)`, so its messages are rewritten back to the line's own text (`RequirementFiles::humanize`) before they are parsed or shown.

**Watchdogs are per kind of work** (`exec::watchdog`): install and uninstall 30 min, a resolve and `list --outdated` 10 min, local queries (`list`, `freeze`, `check`, `--version`) 3 min, `probe.py` 60 s, the discovery listings (`py -0p`, `uv python list`) 30 s. One 600 s budget used to cover everything, which held the Environments screen for ten minutes on a hung `py -0p` and killed a slow scipy build. A fired watchdog is `PD-ENG-004`, and a stopped run says why in its type — `PdError.stop` is `Cancelled` or `TimedOut`, never on the wire — rather than in a message prefix three modules used to match. A spawn failure maps by cause: a missing interpreter is `PD-ENV-001`, a command line Windows refuses as too long `PD-SYS-001`, anything else `PD-ENG-001`. Output is read as bytes and decoded lossily: tokio's `lines()` stopped at the first non-UTF-8 byte — MSVC prints in the OEM code page — and dropped the pipe, killing the child mid-install.

Uninstalls are always sequential (cheap) with the reverse-dep guard evaluated once up front against the full removal set. A rollback removes what the snapshot does not have, then restores what it does, one package at a time in `Isolated` mode — exactness is the whole job, and a batch resolve is free to pick another version — each restore holding the rest of the restore at the snapshot's versions with `-c`. It no longer runs `pip check` between its removals and its restores.

## 9. Frontend architecture

Zustand stores: `useEnvStore`, `usePlanStore`, `useIndexStore`, `useSettingsStore`, `useHealthStore`, `useSecurityStore`, `useDepsStore`, `useLegalStore`, `useUiStore`. **An answer is applied only if it still answers the question being asked** (1.4.0): a store that fetches per environment checks, after each await, that the environment is still the selected one — the plan store, that its generation is still current — because a slow answer for an environment the user had just left used to land on the one they switched to, and a plan the user had backed out of re-opened its panel (`ui/src/stores/races.test.ts`). All engine data enters via typed IPC wrappers (`ui/src/ipc/*.ts`) mirroring the Rust types. **This section originally specified `specta`/`tauri-specta` and that is not what shipped:** `ui/src/ipc/generated.ts` comes from a first-party `xtask` over `schemars`, which makes drift a *test* failure rather than a compile error. `crates/pipdock-core/src/bindings.rs` records why — specta 2 is still `2.0.0-rc`, and a second derive stack over ~40 types was not worth it — and names the condition for revisiting. No component calls `invoke` directly. i18next with `en`/`vi` JSON catalogs (see I18N.md). Styling via Tailwind 4 tokens defined in UI-SPEC.

## 10. Version manifest (verified 2026-10-17 for 1.4.0, against `rust-toolchain.toml`, `Cargo.lock` and `package-lock.json`)

| Component | Version policy |
|---|---|
| Tauri | 2.12.x (2.12.4; `tauri-build` 2.7.3), **in lockstep with `@tauri-apps/*` 2.12** — Tauri's own version check requires the crate and the npm package to agree, and CI's `tauri lockstep` job fails when `Cargo.lock` and `package-lock.json` drift apart. **No `tauri-plugin-updater`** — PipDock does not update itself (SECURITY §5) |
| Rust | 1.99.1, pinned via `rust-toolchain.toml`; the workspace's `rust-version` (1.99) moves with the pin, so it never claims a floor it is not built with |
| React / TypeScript | 19.x (19.3.1) / see the TypeScript row |
| Vite / Vitest / Tailwind | 8.x (8.3.4, Rolldown line) / 5.x (5.0.5) / 4.x (4.3.3) |
| Node (dev only) | 24 LTS (Active LTS; EOL 2028-04) |
| reqwest / tokio | 0.13.x (0.13.5; rustls, no native TLS) / 1.x (1.53.3) |
| pip engine | ≥ 24.0 supported; latest is 26.2.x (the fixtures are 26.2.1). `--dry-run --report` requires ≥ 22.2 — PipDock offers to upgrade older pips before first plan. |
| uv engine | latest stable (0.13.x; the fixtures are 0.13.1, whose `error:`/`cause:` rendering replaced miette's, with 0.11's kept as a frozen corpus); **minimum 0.10.0**, pinned by SP-1 (`UV_MIN_VERSION`, `engine/mod.rs`) — a declared floor nothing enforces yet (audit backlog) |
| Managed Pythons | 3.10 – 3.14 (3.9 is EOL) |
| Windows | 10 (1809+) / 11; WebView2 Evergreen assumed present; `longPathAware` manifest enabled |
| rusqlite | 0.40.x (0.40.2; `libsqlite3-sys` 0.38.2, SQLite 3.53.2). **Held at 0.37 until 1.4.0**: `libsqlite3-sys` 0.38's build script used the then-unstable `cfg_select!`, which did not compile on Rust 1.94.1. It is stable since 1.95, so the toolchain move met the hold's own condition. Nothing here used the `u64`/`usize` `ToSql` impls (off by default since 0.38) or `prepare_cached` (optional since 0.38). |
| TypeScript | **held at 6.0.x** (6.0.3). `typescript-eslint` 8.71.2 peers on `>=4.8.4 <6.1.0`, so anything from 6.1 on — 7.x included — fails `npm ci` with `ERESOLVE` before a single check runs. Revisit when typescript-eslint widens its range. |
| ESLint | **10.x (10.12.0), with `eslint-plugin-react` deliberately absent.** That plugin caps at ESLint 9, which drags in a `minimatch`/`brace-expansion` chain carrying six high-severity advisories and would fail `npm audit` in CI. The one rule it was wanted for — no hardcoded JSX strings (I18N §1) — is a local rule in `eslint.config.js`, which also implements the documented allowlist precisely. |

**TypeScript is the one hold left, and Dependabot no longer proposes it:** `.github/dependabot.yml` ignores `typescript >=6.1.0`, with the reason beside the rule. A hold is a compile or install failure, not a preference, so such a PR would be closed rather than merged until the stated condition changes. `cargo audit` runs with `--deny warnings`, so a new informational advisory fails CI unless `.cargo/audit.toml` records why it is tolerated (SECURITY §7).

Dependabot + `cargo audit` + `cargo deny` + `npm audit` in CI, and both audits daily against `main` (`security-scan.yml`), keep this table honest (RELEASE-CI.md §2).
