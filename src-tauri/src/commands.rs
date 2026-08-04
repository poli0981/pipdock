//! Tauri commands.
//!
//! ARCHITECTURE §1.1: each command is a **thin** wrapper over a `pipdock_core` function. If a
//! wrapper starts making decisions, that logic belongs in the core instead, so the CLI inherits it
//! too (G5: GUI and CLI never diverge).
//!
//! Errors cross as [`WireError`] rather than `PdError` directly. `#[tauri::command]` wants a plain
//! `Serialize` error type, and the frontend contract in `ui/src/ipc` is a flat
//! `{ code, message, stderrTail }`. One conversion here beats a shape the UI has to unwrap.

use pipdock_core::engine;
use pipdock_core::envs::{self, ScanProgress};
use pipdock_core::errors::Code;
use pipdock_core::flow;
use pipdock_core::model::EnvSource;
use pipdock_core::pins::{self, Pin};
use pipdock_core::plan::{Decision, ExecutionSummary};
use pipdock_core::settings::{self, Consent, Settings};
use pipdock_core::snapshot;
use pipdock_core::{PdError, PkgName, PyEnv};
use tauri::Emitter as _;
use tokio_util::sync::CancellationToken;

use crate::state::AppState;

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
            state.park(Box::new(flow)).await;
            Ok(step)
        }
        Err(e) => {
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
    let mut flow = state.claim().await?.ok_or_else(no_plan)?;

    let parsed = decisions
        .into_iter()
        .map(|(name, decision)| PkgName::parse(&name).map(|pkg| (pkg, decision)))
        .collect::<pipdock_core::Result<std::collections::BTreeMap<_, _>>>();

    let parsed = match parsed {
        Ok(p) => p,
        Err(e) => {
            // The flow is still good — only the argument was bad, so park it rather than losing
            // the preview the user is looking at.
            state.park(flow).await;
            return Err(e.into());
        }
    };

    match flow.decide(&parsed).await {
        Ok(step) => {
            state.park(flow).await;
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
    let mut flow = state.claim().await?.ok_or_else(no_plan)?;
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

    // Forward every event to the webview. The receiver lives for the length of the execution and
    // the sender dies with it, so this task ends on its own.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let emitter = app.clone();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            // A UI that stopped listening is not a reason to abandon the install.
            let _ = emitter.emit("plan-progress", &event);
        }
    });

    let result = flow.execute(tx).await;
    state.release().await;

    Ok(ExecutionOutcome {
        summary: result?,
        snapshot,
    })
}

/// Stop the plan that is running (DATA-FLOW §3: allowed while resolving or executing).
///
/// Returns whether anything was actually in flight, so the UI can tell "stopped it" from "there
/// was nothing to stop" rather than guessing.
///
/// Never fails: cancelling something that already finished is not an error, and making it one
/// would mean the UI has to race the thing it is trying to stop.
#[tauri::command]
pub fn plan_cancel(state: tauri::State<'_, AppState>) -> bool {
    state.cancel_current()
}

/// There is no parked plan — the UI called out of order, or a previous call already consumed it.
fn no_plan() -> PdError {
    PdError::new(
        Code::IntUnexpected,
        "no plan is in progress; resolve one first",
    )
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
