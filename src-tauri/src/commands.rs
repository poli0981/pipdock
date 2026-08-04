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
use pipdock_core::model::EnvSource;
use pipdock_core::pins::{self, Pin};
use pipdock_core::settings::{self, Consent, Settings};
use pipdock_core::{PdError, PyEnv};
use tauri::Emitter as _;

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
