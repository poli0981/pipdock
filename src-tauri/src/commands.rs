//! Tauri commands.
//!
//! ARCHITECTURE §1.1: each command is a **thin** wrapper over a `pipdock_core` function. If a
//! wrapper starts making decisions, that logic belongs in the core instead, so the CLI inherits it
//! too (G5: GUI and CLI never diverge).
//!
//! Errors cross as [`WireError`] rather than `PdError` directly. `#[tauri::command]` wants a plain
//! `Serialize` error type, and the frontend contract in `ui/src/ipc` is a flat
//! `{ code, message, stderrTail }`. One conversion here beats a shape the UI has to unwrap.

use pipdock_core::engine::{self, Engine as _, pip::PipEngine};
use pipdock_core::envs::{self, ScanProgress};
use pipdock_core::flow;
use pipdock_core::index::{self, NameIndex};
use pipdock_core::model::{EnvSource, StepResult};
use pipdock_core::pins::{self, Pin};
use pipdock_core::plan::{Decision, ExecutionSummary};
use pipdock_core::settings::{self, Consent, Settings};
use pipdock_core::snapshot;
use pipdock_core::{PdError, PkgName, PyEnv};
use tauri::{Emitter as _, Manager as _};
use tokio_util::sync::CancellationToken;

use crate::state::{AppState, Session};

/// The error shape `ui/src/ipc` declares.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireError {
    /// Catalog code, e.g. `PD-ENV-001`. Never localized (I18N §1).
    pub code: String,
    /// Developer-facing detail; the user-facing one-liner is looked up from `code`.
    pub message: String,
    /// Tail of the engine's stderr, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_tail: Option<String>,
}

impl From<PdError> for WireError {
    fn from(e: PdError) -> Self {
        Self {
            code: e.code.as_str().to_owned(),
            message: e.message,
            stderr_tail: e.stderr_tail,
        }
    }
}

/// Every command returns this.
type Wire<T> = Result<T, WireError>;

/// What the shell can report about itself.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    /// PipDock's own version, from Cargo.
    pub version: &'static str,
    /// Hash of the legal documents this build ships against (UI-SPEC §4).
    pub docs_hash: &'static str,
}

/// Version and the legal-documents hash.
#[tauri::command]
pub const fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        docs_hash: settings::docs_hash(),
    }
}

/// One row of the Environments screen.
///
/// A probe failure is carried **per row** rather than failing the scan: one broken interpreter
/// must not hide the rest, which is the rule `pipdock env list` already follows.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvRow {
    /// Interpreter path, and the row's identity.
    pub interpreter: String,
    /// How it was discovered — the source chip (UI-SPEC §4).
    pub source: EnvSource,
    /// Identity used for pins and snapshots.
    pub env_hash: String,
    /// Present when the probe succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<PyEnv>,
    /// How many distributions the probe saw.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packages: Option<usize>,
    /// Present when the probe failed; the row renders as unusable with this code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

async fn probe_row(path: std::path::PathBuf, source: EnvSource) -> EnvRow {
    let env_hash = envs::env_hash(&path);
    let interpreter = path.display().to_string();
    match envs::probe(&path, source).await {
        Ok(probed) => EnvRow {
            interpreter,
            source,
            env_hash,
            packages: Some(probed.dists.len()),
            env: Some(probed.env),
            error: None,
        },
        Err(e) => EnvRow {
            interpreter,
            source,
            env_hash,
            env: None,
            packages: None,
            error: Some(e.into()),
        },
    }
}

/// Discover environments, streaming `scan-progress` as each source is read.
///
/// # Errors
/// Never fails as a whole — a probe failure is reported on its own row.
#[tauri::command]
pub async fn env_scan(app: tauri::AppHandle) -> Wire<Vec<EnvRow>> {
    let emitter = app.clone();
    let candidates = envs::scan_reporting(&move |progress: ScanProgress| {
        // A UI that stopped listening is not a reason to abandon the scan.
        let _ = emitter.emit("scan-progress", &progress);
    })
    .await;

    let mut rows = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        rows.push(probe_row(candidate.path, candidate.source).await);
    }
    Ok(rows)
}

/// Probe one interpreter, for the manual *Browse…* path.
///
/// # Errors
/// `PD-ENV-001` when there is no usable interpreter there, `PD-ENV-003` when the probe output
/// cannot be read.
#[tauri::command]
pub async fn env_probe(interpreter: String) -> Wire<EnvRow> {
    let path = std::path::PathBuf::from(&interpreter);
    let probed = envs::probe(&path, EnvSource::Manual).await?;
    Ok(EnvRow {
        interpreter,
        source: EnvSource::Manual,
        env_hash: envs::env_hash(&path),
        packages: Some(probed.dists.len()),
        env: Some(probed.env),
        error: None,
    })
}

/// Everything installed in one environment — the Installed table (UI-SPEC §4).
///
/// Goes through `envs::probe`, **not** `Engine::list_installed`. The probe carries
/// `requires_dist`, which `list --format=json` zeroes out, and it is the source the
/// reverse-dependency graph and the uninstall guard are built from; it is also the only producer
/// of `size_bytes`. `pipdock list` makes the same choice, and if these two disagreed the Installed
/// screen and the guard protecting it would disagree about which packages exist (DATA-FLOW §7).
///
/// Takes the whole `PyEnv` rather than a path so it can be handed straight back from `env_scan`,
/// and so the `source` chip survives the round trip instead of being re-guessed as `Manual`.
///
/// # Errors
/// `PD-ENV-001` when the interpreter has gone, `PD-ENV-003` when the probe output is unreadable.
#[tauri::command]
pub async fn pkg_list(env: PyEnv) -> Wire<Vec<pipdock_core::Dist>> {
    Ok(envs::probe(&env.interpreter, env.source).await?.dists)
}

/// Installed packages with a newer release available.
///
/// Kept separate from [`pkg_list`] per ARCHITECTURE §7, and because it is the one of the two that
/// touches the network: the Installed table renders from `pkg_list` immediately and badges itself
/// when this resolves. It takes a `PyEnv` for the same reason — building one here would mean a
/// second probe on the slower of the two calls.
///
/// The engine is the configured one, read from the store on every call so flipping the Settings
/// radio takes effect on the next refresh. The store guard is dropped before the engine runs;
/// holding it across the await would compile, pass every test, and freeze `settings_get` and the
/// pin commands for the length of a network round trip.
///
/// # Errors
/// `PD-ENG-001` when the configured engine cannot be spawned — uv is a standalone binary and is
/// often not on PATH — and the `PD-NET-*` family when the index is unreachable.
#[tauri::command]
pub async fn pkg_outdated(
    state: tauri::State<'_, AppState>,
    env: PyEnv,
) -> Wire<Vec<pipdock_core::OutdatedDist>> {
    let engine = {
        let store = state.store.lock().await;
        engine::for_id(settings::load(&store)?.engine)
    };
    Ok(engine.list_outdated(&env).await?)
}

/// Pins for an environment, ordered by package name.
///
/// The 🔒 chip and *Select all*'s "N pinned excluded" note both read this. Neither is the
/// enforcement point: DATA-FLOW §9.5 is enforced by `pins::filter_upgrades` when a plan is built.
///
/// # Errors
/// `PD-INT-001` when the pin table cannot be read.
#[tauri::command]
pub async fn pin_list(state: tauri::State<'_, AppState>, env_hash: String) -> Wire<Vec<Pin>> {
    let store = state.store.lock().await;
    Ok(pins::list(&store, &env_hash)?)
}

/// Add or replace a pin.
///
/// # Errors
/// `PD-PKG-002` when the name or the held version is not well formed — `Pin` arrives from the
/// frontend, and a `Hold` pin becomes a `PinnedSpec` that reaches argv (SECURITY §2).
/// `PD-INT-001` when the write fails.
#[tauri::command]
pub async fn pin_add(state: tauri::State<'_, AppState>, env_hash: String, pin: Pin) -> Wire<()> {
    let store = state.store.lock().await;
    pins::add(&store, &env_hash, &pin)?;
    Ok(())
}

/// Remove a pin, reporting whether one existed.
///
/// # Errors
/// `PD-PKG-002` when `pkg` is not a valid distribution name; `PD-INT-001` when the delete fails.
#[tauri::command]
pub async fn pin_remove(
    state: tauri::State<'_, AppState>,
    env_hash: String,
    pkg: String,
) -> Wire<bool> {
    let name = pipdock_core::PkgName::parse(&pkg)?;
    let store = state.store.lock().await;
    Ok(pins::remove(&store, &env_hash, &name)?)
}

/// What `index_search` returns.
///
/// A struct rather than a bare `Hit[]` because "no results" and "the index is not loaded yet" are
/// different answers and the screen must say different things about them. Conflating them would
/// tell a user their package does not exist during the 613 ms it takes to load 858k names.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    /// Matches, best first. Empty when `ready` is false.
    pub hits: Vec<index::Hit>,
    /// False while the index is still loading.
    pub ready: bool,
    /// Set when the index could not be loaded at all — usually "never refreshed".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
}

/// Fuzzy search over the local name index.
///
/// **Never blocks on the load.** SP-3 measured scanning SQLite at 218 ms per keystroke against a
/// 50 ms budget, so the index is held in memory — and loading it costs 140 ms on the real 864k
/// index, which is why a search that arrives first is answered `ready: false` rather than queued
/// behind it. The load is kicked off here so the screen does not have to coordinate; calling this
/// on every keystroke is safe.
///
/// # Errors
/// Never as a whole — an index that cannot be loaded is reported in `unavailable`, because
/// "refresh the index" is an action, not a failure.
#[tauri::command]
pub async fn index_search(
    state: tauri::State<'_, AppState>,
    query: String,
    limit: usize,
) -> Wire<SearchResults> {
    if let Some(hits) = state.search_index(&query, limit) {
        return Ok(SearchResults {
            hits,
            ready: true,
            unavailable: None,
        });
    }

    if state.begin_index_load() {
        let loaded = {
            let store = state.store.lock().await;
            NameIndex::load(&store)
        };
        state.finish_index_load(loaded);
        // The load that this keystroke paid for is done, so answer from it rather than making the
        // user press another key.
        if let Some(hits) = state.search_index(&query, limit) {
            return Ok(SearchResults {
                hits,
                ready: true,
                unavailable: None,
            });
        }
    }

    Ok(SearchResults {
        hits: Vec::new(),
        ready: false,
        unavailable: state.index_failure(),
    })
}

/// Cached PyPI metadata for the details panel, with how fresh it is.
///
/// # Errors
/// `PD-PKG-002` when the name is malformed or PyPI does not know it; `PD-NET-001` when it is
/// neither cached nor reachable.
#[tauri::command]
pub async fn pkg_metadata(
    state: tauri::State<'_, AppState>,
    pkg: String,
) -> Wire<(index::PackageMeta, index::Freshness)> {
    let name = PkgName::parse(&pkg)?;
    // Takes the directory, not a `Store`: `index::metadata` opens one for each synchronous stretch
    // and drops it before the PyPI call, so its future stays `Send`.
    Ok(index::metadata(&state.app_data, &name, jiff::Timestamp::now()).await?)
}

/// Re-download the PEP 691 name index.
///
/// # Errors
/// `PD-NET-010` when the index cannot be fetched; the previously cached index stays searchable.
#[tauri::command]
pub async fn index_refresh(state: tauri::State<'_, AppState>) -> Wire<index::RefreshReport> {
    let report = index::refresh(&state.app_data, jiff::Timestamp::now()).await?;
    // Otherwise a refresh reports thousands of new projects and search cannot find any of them.
    state.invalidate_index();
    Ok(report)
}

/// What `plan_execute` returns: the summary, plus the snapshot it took first.
///
/// The snapshot id is not in `ExecutionSummary` because the CLI prints it *before* execution
/// starts (DATA-FLOW §3 draws them as distinct states), and the summary sheet needs it afterwards
/// to offer the rollback. One envelope beats a second command to go and look it up.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionOutcome {
    /// The summary (DATA-FLOW §6).
    pub summary: ExecutionSummary,
    /// The snapshot taken before anything was mutated, absent only when waived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<snapshot::Meta>,
}

/// Begin an update or install: resolve, and derive what needs a decision.
///
/// The first of the four calls that drive one `UpdateFlow` (DATA-FLOW §3). The flow is parked in
/// `AppState` between them, because it is resumable and IPC is not.
///
/// # Errors
/// `PD-RES-003` when a plan is already in flight, `PD-ENV-002` for a PEP 668 environment — checked
/// before any engine command runs — and whatever the resolve itself raises.
#[tauri::command]
pub async fn plan_resolve(
    state: tauri::State<'_, AppState>,
    env: PyEnv,
    intent: flow::Intent,
) -> Wire<flow::FlowStep> {
    // Claiming before any work means a second resolve is refused rather than racing this one.
    // Whatever flow was parked is dropped: starting a new plan abandons the old preview, which is
    // what the user just asked for.
    let _ = state.claim().await?;
    // A bug report carries one run, so the ring starts here rather than at execute.
    state.log.clear();

    let cancel = CancellationToken::new();
    state.set_cancel(Some(cancel));

    // Both store reads happen up front and the guard is dropped before any await. `Store` is not
    // `Sync`, so a future holding one is not `Send` and a Tauri command cannot return it — which
    // is what forced `UpdateFlow::start` to take the pins rather than the store.
    let (engine, env_pins) = {
        let store = state.store.lock().await;
        let engine = engine::for_id(settings::load(&store)?.engine);
        let env_pins = pins::list(&store, &envs::env_hash(&env.interpreter))?;
        (engine, env_pins)
    };

    match flow::UpdateFlow::start(env, engine, &intent, &env_pins).await {
        Ok((flow, step)) => {
            // The engine's own output, kept for a report. A resolve emits no progress events, so
            // without this the commonest thing anyone reports — a plan that would not resolve —
            // would attach an empty excerpt.
            state.log.push(flow.report().raw.as_str());
            state.park(Session::Update(Box::new(flow))).await;
            Ok(step)
        }
        Err(e) => {
            if let Some(tail) = e.stderr_tail.as_deref() {
                state.log.push(tail);
            }
            // Every failure path releases the slot, or the session refuses plans forever.
            state.release().await;
            Err(e.into())
        }
    }
}

/// Apply the user's 3-way conflict choices and re-resolve (DATA-FLOW §3's decision loop).
///
/// # Errors
/// `PD-RES-003` when another plan is in flight, `PD-PKG-002` for a name that is not a package,
/// `PD-INT-001` when there is no plan to decide on.
#[tauri::command]
pub async fn plan_decide(
    state: tauri::State<'_, AppState>,
    decisions: std::collections::BTreeMap<String, Decision>,
) -> Wire<flow::FlowStep> {
    let mut flow = state.claim_update().await?;

    let parsed = decisions
        .into_iter()
        .map(|(name, decision)| PkgName::parse(&name).map(|pkg| (pkg, decision)))
        .collect::<pipdock_core::Result<std::collections::BTreeMap<_, _>>>();

    let parsed = match parsed {
        Ok(p) => p,
        Err(e) => {
            // The flow is still good — only the argument was bad, so park it rather than losing
            // the preview the user is looking at.
            state.park(Session::Update(flow)).await;
            return Err(e.into());
        }
    };

    match flow.decide(&parsed).await {
        Ok(step) => {
            state.park(Session::Update(flow)).await;
            Ok(step)
        }
        Err(e) => {
            state.release().await;
            Err(e.into())
        }
    }
}

/// Take the snapshot, then run the plan (ARCHITECTURE §8's two phases).
///
/// Streams `plan-progress` throughout. **The snapshot is not optional here** — `--no-snapshot` is
/// a CLI waiver for disposable environments and has no GUI surface, so DATA-FLOW §9.2 holds
/// unconditionally: a snapshot failure aborts with `PD-SNP-001` and executes nothing.
///
/// # Errors
/// `PD-SNP-001` when the snapshot cannot be written, `PD-RES-002` when the preview went stale or
/// the environment drifted, `PD-INT-001` when there is no plan to execute.
#[tauri::command]
pub async fn plan_execute(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Wire<ExecutionOutcome> {
    let mut flow = state.claim_update().await?;
    state.set_cancel(Some(flow.cancel_handle()));

    let snapshot = match flow
        .take_snapshot(flow::SnapshotPolicy::Take, &state.app_data)
        .await
    {
        Ok(meta) => meta,
        Err(e) => {
            state.release().await;
            return Err(e.into());
        }
    };

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    forward_progress(&app, rx);

    let result = flow.execute(tx).await;
    state.release().await;

    Ok(ExecutionOutcome {
        summary: result?,
        snapshot,
    })
}

/// Stop the plan, whichever half of it is on screen (DATA-FLOW §3: allowed while resolving or
/// executing).
///
/// Returns whether anything was actually there, so the UI can tell "stopped it" from "there was
/// nothing to stop" rather than guessing. A session that is merely *parked* — a preview, a guard
/// dialog — counts, and is discarded: it has no process to kill, and leaving it behind refuses
/// the next plan on behalf of something nobody is looking at.
///
/// # Errors
/// Never, in practice. An `async` command taking a borrowed `tauri::State<'_, _>` has to return a
/// `Result` for Tauri's generated glue to name the lifetime, so the signature says so even though
/// no path produces one. It became `async` when stopping grew to touch the slot as well as the
/// token — cancelling something that already finished is not an error, and making it one would
/// mean the UI has to race the thing it is trying to stop.
#[tauri::command]
pub async fn plan_cancel(state: tauri::State<'_, AppState>) -> Wire<bool> {
    Ok(state.stop().await)
}

/// Check what a removal would break, and park the flow that would do it (DATA-FLOW §5).
///
/// Called again with a wider set for *Remove dependents too*: that option is not a variant of the
/// flow but the caller starting over with `GuardReport.withDependents`, so a dependent of a
/// dependent surfaces on the next pass instead of being removed unannounced. The parked flow from
/// the previous pass is discarded here, exactly as `plan_resolve` discards a previous preview.
///
/// # Errors
/// `PD-RES-003` when a plan is already executing, `PD-ENV-002` for a PEP 668 environment,
/// `PD-PKG-002` for a name that is not a package, `PD-ENV-003` when the probe fails.
#[tauri::command]
pub async fn uninstall_guard(
    state: tauri::State<'_, AppState>,
    env: PyEnv,
    pkgs: Vec<String>,
) -> Wire<pipdock_core::graph::GuardReport> {
    let _ = state.claim().await?;

    // Read, then drop, before any await: `Store` is not `Sync`, so a future holding the guard is
    // not `Send` and this command would not compile — and where it does compile it serializes
    // every other command behind a subprocess.
    let engine = {
        let store = state.store.lock().await;
        engine::for_id(settings::load(&store)?.engine)
    };

    match flow::UninstallFlow::start(env, engine, &pkgs).await {
        Ok((flow, report)) => {
            state.park(Session::Uninstall(Box::new(flow))).await;
            Ok(report)
        }
        Err(e) => {
            state.release().await;
            Err(e.into())
        }
    }
}

/// Snapshot, then remove (DATA-FLOW §5's tail).
///
/// `force` is §5's *Force remove only X*: the user was shown what breaks and chose it. Without it
/// a removal the guard objected to is refused with `PD-RES-004` **before** the snapshot is
/// written, so a plan that will not run does not leave one behind.
///
/// # Errors
/// `PD-RES-004` when the guard objected and `force` is false, `PD-SNP-001` when the snapshot
/// cannot be written — in which case nothing is removed — and `PD-INT-001` when no guard has run.
#[tauri::command]
pub async fn uninstall_execute(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    force: bool,
) -> Wire<ExecutionOutcome> {
    let mut flow = state.sessions.claim_uninstall().await?;
    state.set_cancel(Some(flow.cancel_handle()));

    let ack = if force {
        flow::GuardAck::ForcedDespiteBreakage
    } else {
        flow::GuardAck::Clear
    };
    if let Err(e) = flow.check(ack) {
        state.release().await;
        return Err(e.into());
    }

    let snapshot = match flow
        .take_snapshot(flow::SnapshotPolicy::Take, &state.app_data)
        .await
    {
        Ok(meta) => meta,
        Err(e) => {
            state.release().await;
            return Err(e.into());
        }
    };

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    forward_progress(&app, rx);

    let result = flow.execute(ack, tx).await;
    state.release().await;

    Ok(ExecutionOutcome {
        summary: result?,
        snapshot,
    })
}

/// Snapshots for an environment, newest first.
///
/// Takes the `env_hash` rather than a `PyEnv`, because snapshots outlive the interpreter that made
/// them: an environment whose Python has been deleted still has a history worth showing, and its
/// `EnvRow.env` is `None` exactly then.
///
/// # Errors
/// Propagates read failures. A missing directory is not one — an environment with no snapshots
/// yet returns an empty list.
#[tauri::command]
pub async fn snapshot_list(
    state: tauri::State<'_, AppState>,
    env_hash: String,
) -> Wire<Vec<snapshot::Meta>> {
    Ok(snapshot::list(&state.app_data, &env_hash)?)
}

/// Take a snapshot on demand, outside any plan (`Trigger::Manual`).
///
/// # Errors
/// `PD-SNP-001` when it cannot be written; otherwise propagates the freeze.
#[tauri::command]
pub async fn snapshot_create(
    state: tauri::State<'_, AppState>,
    env: PyEnv,
) -> Wire<snapshot::Meta> {
    let engine = {
        let store = state.store.lock().await;
        engine::for_id(settings::load(&store)?.engine)
    };
    let freeze = engine.freeze(&env).await?;
    let snap = snapshot::create(
        &state.app_data,
        &envs::env_hash(&env.interpreter),
        freeze,
        snapshot::Trigger::Manual,
        engine.id(),
        jiff::Timestamp::now(),
    )?;
    Ok(snap.meta)
}

/// The environment as it is now, against a snapshot.
///
/// Claims no session: browsing a timeline must not start a flow, and this is one `engine.freeze()`
/// rather than the `RollbackFlow::start` a preview would need.
///
/// # Errors
/// `PD-SNP-002` when no such snapshot exists; otherwise propagates the freeze.
#[tauri::command]
pub async fn snapshot_diff(
    state: tauri::State<'_, AppState>,
    env: PyEnv,
    id: String,
) -> Wire<snapshot::Diff> {
    let engine = {
        let store = state.store.lock().await;
        engine::for_id(settings::load(&store)?.engine)
    };
    let hash = envs::env_hash(&env.interpreter);
    let snap = snapshot::load(&state.app_data, &hash, &id)?;
    let current = snapshot::parse_freeze(&engine.freeze(&env).await?);
    Ok(snapshot::diff(&current, &snap.entries()))
}

/// What restoring a snapshot would do, parking the flow that would do it (DATA-FLOW §8).
///
/// Split from `snapshot_rollback` for the same reason `plan_resolve` is split from `plan_execute`:
/// the user looks at the preview and answers in a separate message, and what they confirm has to
/// be the plan they were shown rather than one re-derived afterwards.
///
/// # Errors
/// `PD-RES-003` when a plan is already executing, `PD-SNP-002` when no such snapshot exists.
#[tauri::command]
pub async fn snapshot_rollback_preview(
    state: tauri::State<'_, AppState>,
    env: PyEnv,
    id: String,
) -> Wire<flow::RollbackPreview> {
    let _ = state.claim().await?;

    let engine = {
        let store = state.store.lock().await;
        engine::for_id(settings::load(&store)?.engine)
    };

    match flow::RollbackFlow::start(env, engine, &state.app_data, &id).await {
        Ok((rollback, preview)) => {
            state.park(Session::Rollback(Box::new(rollback))).await;
            Ok(preview)
        }
        Err(e) => {
            state.release().await;
            Err(e.into())
        }
    }
}

/// Snapshot the current state, then restore the parked target (DATA-FLOW §8).
///
/// The pre-rollback snapshot is what makes a rollback itself reversible, and it is why `latest`
/// moves twice across one restore — the reason no caller should ever name it.
///
/// # Errors
/// `PD-SNP-001` when the pre-rollback snapshot cannot be written, in which case nothing is
/// restored; `PD-INT-001` when no preview has run.
#[tauri::command]
pub async fn snapshot_rollback(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Wire<ExecutionOutcome> {
    let mut rollback = state.sessions.claim_rollback().await?;
    state.set_cancel(Some(rollback.cancel_handle()));

    let snapshot = match rollback.take_snapshot(&state.app_data).await {
        Ok(meta) => meta,
        Err(e) => {
            state.release().await;
            return Err(e.into());
        }
    };

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    forward_progress(&app, rx);

    let result = rollback.execute(tx).await;
    state.release().await;

    Ok(ExecutionOutcome {
        summary: result?,
        snapshot: Some(snapshot),
    })
}

/// Forward `plan-progress` to the webview, teeing every line into the log ring.
///
/// One helper for all three executing commands. A UI that stopped listening is not a reason to
/// abandon an install, so the emit result is dropped; the ring is fed regardless, because the
/// report is most wanted precisely when nobody was watching.
fn forward_progress(
    app: &tauri::AppHandle,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<pipdock_core::engine::ProgressEvent>,
) {
    let emitter = app.clone();
    let state = app.state::<AppState>();
    let log = std::sync::Arc::clone(&state.log);
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let Some(line) = event.line() {
                log.push(line);
            }
            let _ = emitter.emit("plan-progress", &event);
        }
    });
}

/// Upgrade pip inside `env` (PRD P0-10).
///
/// **`PipEngine` unconditionally**, never `engine::for_id(settings.engine)`. Upgrading pip is a pip
/// operation by definition; the engine setting is a preference about how the *user's* environments
/// are resolved, and DATA-FLOW §7 (as amended by P1) says so. The CLI does the same.
///
/// **Returns `StepResult`, not `ExecutionSummary`.** ARCHITECTURE §7's row was wrong: there is no
/// plan, no phase and no per-package counts here, and inventing them would be four lies for one
/// step. The before/after version pair is not in it either — the caller re-probes, which it has to
/// do anyway to refresh the row.
///
/// **No snapshot** (DATA-FLOW §9.2's exemption, and it is deliberate). A snapshot's only restore
/// path is `pip install pip==X` executed *by pip*, so one taken to protect against a broken pip has
/// no consumer that could use it. The exemption is made visible rather than silent: the confirm
/// dialog says no snapshot is taken.
///
/// Claims the mutation slot because two pip invocations against one site-packages would interleave.
///
/// # Errors
/// `PD-ENV-002` on a PEP 668 environment; `PD-RES-003` when a plan is already in flight; whatever
/// `classify_stderr` makes of a failed install.
#[tauri::command]
pub async fn pip_upgrade(state: tauri::State<'_, AppState>, env: PyEnv) -> Wire<StepResult> {
    // `claim` rather than `claim_update`: there is nothing parked to expect. It discards a parked
    // session, which is reachable only if a preview is open — and the plan panel replaces the whole
    // content area while one is, so the Environments row is not clickable.
    state.sessions.claim().await?;

    let result = PipEngine.upgrade_pip(&env).await;
    // Released on **both** paths. A claim that is never released leaves every later command
    // answering PD-RES-003 for a plan that never existed — S5 bug 1, and the reason this is not an
    // early `?`.
    state.release().await;
    Ok(result?)
}

/// Detected version and availability for **both** engines (ARCHITECTURE §7).
///
/// Both, not the configured one: Settings shows a radio with a version beside each option, so
/// asking about the one already selected would leave the other blank until it was chosen.
///
/// Takes a `PyEnv` because `Engine::info` does in the trait and in both adapters — pip's version
/// comes from `<python> -m pip --version`, so there is no env-free answer for it. Settings shows
/// "pick an environment" until one is selected rather than reporting a pip that is not there.
///
/// # Errors
/// Never: an engine that cannot be spawned reports `available: false`, which is the answer, not a
/// failure. Returns `Wire` because an async command taking a borrowed `State` must.
#[tauri::command]
pub async fn engine_info(env: PyEnv) -> Wire<Vec<pipdock_core::model::EngineInfo>> {
    let mut out = Vec::with_capacity(2);
    for id in [
        pipdock_core::model::EngineId::Pip,
        pipdock_core::model::EngineId::Uv,
    ] {
        out.push(engine::for_id(id).info(&env).await);
    }
    Ok(out)
}

/// A prefilled GitHub issue URL, and the log to put on the clipboard (ERROR-CATALOG §4).
///
/// Two fields because §4.3 splits them: the URL carries a truncated, tail-biased excerpt so GitHub
/// accepts it, and the *full* log is copied separately with the dialog saying so. **Nothing is
/// sent** — this returns a string.
///
/// Built by `pipdock_core::report`, the same function `pipdock self report-bug` calls, so the two
/// heads cannot drift apart the day the template gains a field.
///
/// # Errors
/// Never. Returns `Wire` because an async command taking a borrowed `State` must.
#[tauri::command]
pub async fn report_bug_url(
    state: tauri::State<'_, AppState>,
    env: Option<PyEnv>,
    code: Option<String>,
) -> Wire<BugReportLink> {
    let log = state.log.read();
    let engine = {
        let store = state.store.lock().await;
        settings::load(&store).map(|s| s.engine).ok()
    };
    let engine_version = match (&env, engine) {
        (Some(e), Some(id)) => engine::for_id(id).info(e).await.version,
        _ => None,
    };

    let report = pipdock_core::report::BugReport {
        python: env.as_ref().map(|e| e.python_version.clone()),
        engine,
        engine_version,
        // Parsed rather than taken on trust: the frontend hands back the string it rendered, and
        // an unrecognised one is left off instead of pasted into a public URL.
        code: code.as_deref().and_then(code_from_wire),
        log: log.clone(),
    };
    Ok(BugReportLink {
        url: pipdock_core::report::bug_report_url(&report, &os_description()),
        log,
    })
}

/// What `report_bug_url` returns.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BugReportLink {
    /// The prefilled issue URL, with a truncated excerpt.
    pub url: String,
    /// The complete buffer, for the clipboard. Empty when nothing has run.
    pub log: String,
}

/// Resolve a wire code string back to its variant.
fn code_from_wire(code: &str) -> Option<pipdock_core::errors::Code> {
    pipdock_core::errors::Code::ALL
        .iter()
        .copied()
        .find(|c| c.as_str() == code)
}

/// A short OS description for the issue template.
fn os_description() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Read the stored settings.
///
/// # Errors
/// Propagates store failures.
#[tauri::command]
pub async fn settings_get(state: tauri::State<'_, AppState>) -> Wire<Settings> {
    let store = state.store.lock().await;
    Ok(settings::load(&store)?)
}

/// Persist settings, returning what was actually stored.
///
/// # Errors
/// Propagates store failures.
#[tauri::command]
pub async fn settings_set(state: tauri::State<'_, AppState>, settings: Settings) -> Wire<Settings> {
    let store = state.store.lock().await;
    settings::save(&store, &settings)?;
    Ok(settings::load(&store)?)
}

/// Whether the legal gate has been satisfied for *this* build's documents.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsentState {
    /// True when the gate can be skipped.
    pub current: bool,
    /// The hash this build ships against.
    pub docs_hash: &'static str,
    /// What was recorded previously, if anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded: Option<Consent>,
}

/// Read the consent record (UI-SPEC §4).
///
/// # Errors
/// Propagates store failures.
#[tauri::command]
pub async fn legal_consent_get(state: tauri::State<'_, AppState>) -> Wire<ConsentState> {
    let store = state.store.lock().await;
    Ok(ConsentState {
        current: settings::consent_is_current(&store)?,
        docs_hash: settings::docs_hash(),
        recorded: settings::consent(&store)?,
    })
}

/// Record acceptance of this build's documents.
///
/// # Errors
/// Propagates store failures.
#[tauri::command]
pub async fn legal_consent_set(state: tauri::State<'_, AppState>) -> Wire<Consent> {
    let store = state.store.lock().await;
    Ok(settings::accept_consent(&store, jiff::Timestamp::now())?)
}
