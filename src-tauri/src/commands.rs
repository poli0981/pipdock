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
use pipdock_core::pins::{self, Pin, PinSuggestion};
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
    /// The environment's own pip, when it has one (PRD P0-10).
    ///
    /// **Read out of the probe's distribution list, not from `engine_info`.** `engine_info` spawns
    /// two subprocesses per call — pip's `--version` and uv's — so calling it per row would add
    /// 2N spawns to the landing screen on top of the N `probe.py` runs already happening. pip is
    /// an ordinary distribution in site-packages and the probe has already read it; this is the
    /// same list `packages` counts.
    ///
    /// `None` means the probe found no pip, which is a real state — a `--without-pip` venv, or a
    /// system Python where `-I` hid a user-site install (ARCHITECTURE §4's trade-off, disclosed as
    /// `hidden_user_site`). It is not "unknown".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pip_version: Option<String>,
    /// The project folder Code Health last ran in for this environment (CODE-HEALTH-SPEC §3).
    ///
    /// Carried on the row for `pip_version`'s reason — the alternative is a command whose entire
    /// job is to return one string the caller is about to be handed anyway. `None` means Health
    /// has never run here, which is what makes the first Run cost one extra click.
    ///
    /// Read out of the store **before** the probe loop and joined afterwards: `Store` is not
    /// `Sync`, so holding the guard across `envs::probe` does not even compile at the command
    /// boundary — the good kind of enforcement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_project: Option<String>,
    /// Present when the probe failed; the row renders as unusable with this code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

async fn probe_row(
    path: std::path::PathBuf,
    source: EnvSource,
    health_project: Option<String>,
) -> EnvRow {
    let env_hash = envs::env_hash(&path);
    let interpreter = path.display().to_string();
    match envs::probe(&path, source).await {
        Ok(probed) => EnvRow {
            interpreter,
            source,
            env_hash,
            packages: Some(probed.dists.len()),
            pip_version: pip_version_of(&probed.dists),
            health_project,
            env: Some(probed.env),
            error: None,
        },
        Err(e) => EnvRow {
            interpreter,
            source,
            env_hash,
            env: None,
            packages: None,
            pip_version: None,
            // Kept on a failed row too. The probe failing says nothing about where Health ran,
            // and dropping it would make a transiently broken interpreter forget the folder.
            health_project,
            error: Some(e.into()),
        },
    }
}

/// pip's version out of a probed distribution list.
///
/// Names arrive PEP 503-normalized from `PkgName::parse`, so an exact match is enough — there is no
/// casing or separator variant of `pip` to worry about.
fn pip_version_of(dists: &[pipdock_core::Dist]) -> Option<String> {
    dists
        .iter()
        .find(|d| d.name.as_str() == "pip")
        .map(|d| d.version.0.clone())
}

/// Discover environments, streaming `scan-progress` as each source is read.
///
/// # Errors
/// Never fails as a whole — a probe failure is reported on its own row.
#[tauri::command]
pub async fn env_scan(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Wire<Vec<EnvRow>> {
    let emitter = app.clone();
    let candidates = envs::scan_reporting(&move |progress: ScanProgress| {
        // A UI that stopped listening is not a reason to abandon the scan.
        let _ = emitter.emit("scan-progress", &progress);
    })
    .await;

    // Every remembered folder, read in one pass with the guard taken and dropped before the first
    // probe. One query per row inside the loop would hold the store across N subprocess spawns.
    let mut folders: std::collections::HashMap<String, String> = {
        let store = state.store.lock().await;
        candidates
            .iter()
            .filter_map(|c| {
                let hash = envs::env_hash(&c.path);
                // A store read failing is not a reason to fail discovery: the cost is a Health
                // screen that asks for the folder again, and the alternative is no Environments
                // screen at all.
                store
                    .health_project(&hash)
                    .ok()
                    .flatten()
                    .map(|f| (hash, f))
            })
            .collect()
    };

    let mut rows = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let folder = folders.remove(&envs::env_hash(&candidate.path));
        rows.push(probe_row(candidate.path, candidate.source, folder).await);
    }
    Ok(rows)
}

/// Probe one interpreter, for the manual *Browse…* path — and to refresh a single row.
///
/// The second use is what P0-10 needs: after `pip_upgrade` the Environments row must show the new
/// version without a full rescan, and one probe is the cheapest honest way to get it. That makes
/// carrying **every** per-row field here load-bearing rather than tidy — a row refreshed through a
/// path that omitted one would blank the field, and `upgradePip` replaces the row wholesale.
///
/// That warning was written about `pip_version` and then not applied twice over:
///
/// * `health_project` is read here for the same reason, or *Upgrade pip* makes the Health screen
///   forget where it was pointed.
/// * **`source` used to be hardcoded `Manual`**, so refreshing a registry-discovered interpreter
///   relabelled it *Added manually* in the chip at `PdEnvironments.tsx:31` — and, because
///   `PyEnv` carries the source too, handed that relabelled env to every later `pkg_list`. It is
///   now a parameter: `None` is the *Browse…* path, which really is manual; a refresh passes the
///   source the row already had.
///
/// # Errors
/// `PD-ENV-001` when there is no usable interpreter there, `PD-ENV-003` when the probe output
/// cannot be read.
#[tauri::command]
pub async fn env_probe(
    state: tauri::State<'_, AppState>,
    interpreter: String,
    source: Option<EnvSource>,
) -> Wire<EnvRow> {
    let path = std::path::PathBuf::from(&interpreter);
    let env_hash = envs::env_hash(&path);
    let source = source.unwrap_or(EnvSource::Manual);
    let folder = {
        let store = state.store.lock().await;
        store.health_project(&env_hash).ok().flatten()
    };
    // Not `probe_row`, despite the shape: that one folds a probe failure onto the row because
    // `env_scan` must not let one broken interpreter hide the rest. Here the caller asked about
    // exactly this interpreter, so the failure is the answer and `?` is right.
    let probed = envs::probe(&path, source).await?;
    Ok(EnvRow {
        interpreter,
        source,
        env_hash,
        packages: Some(probed.dists.len()),
        pip_version: pip_version_of(&probed.dists),
        health_project: folder,
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

/// Packages worth pinning, most-depended-upon first — PRD P1-2, UI-SPEC §4.
///
/// Takes the whole `PyEnv` rather than an `env_hash` for [`pkg_list`]'s reason: it is handed
/// straight back from `env_scan`, and the hash is a one-way digest of the interpreter path, so
/// there is nothing to resolve it *from*. The hash is derived here instead — the same thing
/// `flow::UpdateFlow::start` does.
///
/// **One probe per call**, and that is why UI-SPEC §4 puts this on the Pins screen rather than on
/// a sidebar badge: the count is only paid by someone who opened the tab. The alternative — asking
/// the frontend for the `Dist` list it already holds — would send several hundred packages' worth
/// of `requires_dist` back across the bridge to save a subprocess.
///
/// The store guard is dropped before the probe, per [`pkg_outdated`]'s note: holding it across the
/// await compiles, passes every test, and freezes `settings_get` and the pin commands for as long
/// as the probe takes.
///
/// # Errors
/// `PD-ENV-001` when the interpreter has gone, `PD-ENV-003` when the probe output is unreadable,
/// `PD-INT-001` when the pin table cannot be read.
#[tauri::command]
pub async fn pin_suggestions(
    state: tauri::State<'_, AppState>,
    env: PyEnv,
) -> Wire<Vec<PinSuggestion>> {
    let (existing, threshold) = {
        let store = state.store.lock().await;
        let existing = pins::list(&store, &envs::env_hash(&env.interpreter))?;
        (existing, settings::load(&store)?.pin_suggest_threshold)
    };
    let probed = envs::probe(&env.interpreter, env.source).await?;
    Ok(pins::suggest(
        &probed.dists,
        &probed.env.python_version,
        &existing,
        threshold,
    ))
}

/// Write the environment out as a `requirements.txt` — PRD P1-3.
///
/// **The document is `Engine::freeze`'s**, byte for byte, which is the same one a snapshot
/// records. A freeze *is* a requirements file, so there is no formatter here and no second idea
/// of what an exported environment looks like. A constraints file is the same body under a
/// different name; the difference is how the user feeds it back, not what it contains.
///
/// Writing happens in Rust because `capabilities/default.json` grants `dialog:allow-open` and
/// `dialog:allow-save` and deliberately no `fs` permission — the picker returns a path and nothing
/// in the webview can act on it. `health_save_report` is the same shape.
///
/// # Errors
/// `PD-ENG-001` when the engine cannot be spawned, `PD-ENV-*` when the environment cannot be read,
/// `PD-SYS-002` when the file cannot be written.
#[tauri::command]
pub async fn env_export(
    state: tauri::State<'_, AppState>,
    env: PyEnv,
    path: String,
) -> Wire<String> {
    let engine = {
        let store = state.store.lock().await;
        engine::for_id(settings::load(&store)?.engine)
    };
    let freeze = engine.freeze(&env).await?;
    let target = std::path::PathBuf::from(&path);
    std::fs::write(&target, freeze).map_err(|e| {
        PdError::new(
            pipdock_core::errors::Code::SysDiskFull,
            format!("write {}: {e}", target.display()),
        )
    })?;
    Ok(target.display().to_string())
}

/// Read a `requirements.txt` into install specs, with whatever it could not use.
///
/// Reading happens in Rust for the same reason writing does: the webview has no `fs` permission,
/// only the ability to ask for a path.
///
/// The result is deliberately **not** fed straight into a plan. The skipped lines are the point —
/// an include or an editable install means the file asks for something PipDock will not do, and
/// the user has to see that before a preview claims to represent their file.
///
/// # Errors
/// `PD-SYS-002` when the file cannot be read.
#[tauri::command]
pub async fn requirements_read(
    path: String,
) -> Wire<pipdock_core::requirements::ParsedRequirements> {
    let text = std::fs::read_to_string(&path).map_err(|e| {
        PdError::new(
            pipdock_core::errors::Code::SysDiskFull,
            format!("read {path}: {e}"),
        )
    })?;
    Ok(pipdock_core::requirements::parse(&text))
}

/// What PipDock has written to disk — PRD P1-4.
///
/// # Errors
/// Never fails as a whole; an unreadable path reports zero bytes.
#[tauri::command]
pub async fn cache_usage(state: tauri::State<'_, AppState>) -> Wire<pipdock_core::cache::Usage> {
    Ok(pipdock_core::cache::usage(&state.app_data)?)
}

/// Remove one cache target, resolving to the bytes freed.
///
/// Takes a `Target` enum, **never a path** — `cache::clear` is the only thing that turns one into
/// a location, and it checks the result is inside the data root by canonicalized prefix before
/// removing anything. This is the first delete-a-tree in the application, so the surface it is
/// reachable through is deliberately as narrow as an enum.
///
/// `index.db` is not a target: it holds settings, pins and the consent record as well as the
/// package index, and "clear the cache" must never take a user's pins.
///
/// # Errors
/// `PD-PRM-002` when something holds the files open — a tools venv whose Python is still running
/// is the ordinary way to hit this on Windows. `PD-INT-001` if the containment check ever fails,
/// which would be a bug.
#[tauri::command]
pub async fn cache_clear(
    state: tauri::State<'_, AppState>,
    target: pipdock_core::cache::Target,
) -> Wire<u64> {
    Ok(pipdock_core::cache::clear(&state.app_data, target)?)
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
    forward_progress(&app, "plan-progress", rx);

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
    forward_progress(&app, "plan-progress", rx);

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
    forward_progress(&app, "plan-progress", rx);

    let result = rollback.execute(tx).await;
    state.release().await;

    Ok(ExecutionOutcome {
        summary: result?,
        snapshot: Some(snapshot),
    })
}

/// Forward progress to the webview on `channel`, teeing every line into the log ring.
///
/// One helper for every streaming command. A UI that stopped listening is not a reason to abandon
/// an install, so the emit result is dropped; the ring is fed regardless, because the report is
/// most wanted precisely when nobody was watching.
///
/// `channel` is a `&'static str` rather than a `String` because the names are the closed set in
/// `ui/src/ipc/index.ts`'s `EVENTS`. A literal typo is then a compile error instead of an event
/// nobody is listening for — which is the failure mode that leaves a progress bar at zero while the
/// work runs to completion behind it.
fn forward_progress(
    app: &tauri::AppHandle,
    channel: &'static str,
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
            let _ = emitter.emit(channel, &event);
        }
    });
}

/// Run Code Health over `project`, streaming `health-progress` (PRD P0-11).
///
/// Syncs the tools venv first when one is owed — CODE-HEALTH-SPEC §2's "on first Health run (or
/// pin-set change)" — on the same channel, so the first run on a fresh install shows the ~15 s
/// bootstrap rather than an unexplained pause. The sink's total is decided before the first event
/// because whether a sync is owed is exactly what changes it.
///
/// Claims the **health** slot, not the mutation slot: a run touches no environment, so refusing it
/// while an install streams would be a lock with no invariant behind it. Two Run clicks would
/// otherwise put six subprocesses over one folder.
///
/// # Errors
/// `PD-ENV-003` when `project` cannot be read, `PD-HLT-004`/`PD-NET-011` from the implicit sync,
/// `PD-RES-003` when a run is already going. **A single tool failing is not an error** — it lands
/// in `HealthReport.problems` and the others still report.
#[tauri::command]
pub async fn health_run(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    env: PyEnv,
    project: String,
) -> Wire<pipdock_core::health::HealthReport> {
    use pipdock_core::health;

    state.health.claim().await?;

    let project = std::path::PathBuf::from(&project);
    let tools_dir = health::tools_dir(&state.app_data);
    let opts = health::RunOptions::default();

    let outcome = async {
        let sync_needed = health::needs_sync(&tools_dir, health::HEALTH_TOOLS)?.is_needed();
        let total = health::run_steps(&opts)
            + if sync_needed {
                health::sync_steps(health::HEALTH_TOOLS)
            } else {
                0
            };

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        forward_progress(&app, "health-progress", rx);
        let sink = engine::ProgressSink::new(tx, total, CancellationToken::new());

        if sync_needed {
            let (python, _) = health::choose_tools_python(&envs::scan().await).await?;
            health::sync_tools_venv(&tools_dir, &python, health::HEALTH_TOOLS, &sink).await?;
        }
        health::run_tools(
            &tools_dir,
            &project,
            &env,
            &opts,
            &sink.at(if sync_needed {
                health::sync_steps(health::HEALTH_TOOLS)
            } else {
                0
            }),
        )
        .await
    }
    .await;

    match outcome {
        Ok(report) => {
            // Remembered here rather than through a command of its own, so `EnvRow.healthProject`
            // has something to carry on the next scan and the second Run costs one click fewer.
            // Mirrors the CLI, which writes at the same point (`run::health`).
            //
            // Deliberately not `?`. The report the user asked for is in hand and P5 depends on it
            // being parked; failing the whole command over a bookkeeping write would throw away
            // findings that already cost three subprocesses.
            {
                let store = state.store.lock().await;
                let _ = store.set_health_project(
                    &envs::env_hash(&env.interpreter),
                    &project.display().to_string(),
                    &jiff::Timestamp::now().to_string(),
                );
            }
            // Parked rather than dropped: P5 checks its consent against this exact report, and the
            // folder it ran in is what a fix would rewrite.
            state
                .health
                .park(crate::state::HealthSession {
                    project,
                    env,
                    report: report.clone(),
                })
                .await;
            Ok(report)
        }
        Err(e) => {
            state.health.release().await;
            Err(e.into())
        }
    }
}

/// Audit an environment against the PyPI advisory database — PRD P1-1, SECURITY §6.
///
/// Syncs the audit venv on first use, exactly as [`health_run`] syncs Code Health's: the implicit
/// sync belongs to the command that needs it, or the frontend would have two ways to reach one
/// operation. It is a *different* venv, and that is the point — a CPython with no `msgpack` wheel
/// fails this and leaves Code Health working.
///
/// **The freeze is taken here, through the configured engine.** Which engine matters: pip is
/// invoked with `--all` and includes pip and setuptools, uv has no such flag and omits them, so
/// the engine selection changes what is audited. The store guard is read and dropped before any
/// subprocess, per [`pkg_outdated`]'s note.
///
/// **Cancellable, unlike `health_run`.** P4 deferred that for Code Health on a measurement of
/// 1.3 s; an audit measures 18-68 s, which is long enough that stopping one is an ordinary thing
/// to want. The token is registered beside the slot rather than inside it, so [`audit_cancel`] can
/// reach a run the slot no longer owns.
///
/// # Errors
/// `PD-RES-003` when an audit is already running; `PD-ENG-001` or `PD-ENV-*` when the freeze cannot
/// be taken; `PD-NET-011` when the audit venv cannot be bootstrapped. **pip-audit failing is not an
/// error** — it lands in `AuditReport.problems` and the report still returns.
#[tauri::command]
pub async fn audit_run(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    env: PyEnv,
) -> Wire<pipdock_core::audit::AuditReport> {
    use pipdock_core::{audit, health};

    state.audit.claim().await?;

    let dir = health::audit_dir(&state.app_data);
    let outcome = async {
        let engine = {
            let store = state.store.lock().await;
            engine::for_id(settings::load(&store)?.engine)
        };
        let freeze = engine.freeze(&env).await?;

        let sync_needed = health::needs_sync(&dir, health::AUDIT_TOOLS)?.is_needed();
        let synced = if sync_needed {
            health::sync_steps(health::AUDIT_TOOLS)
        } else {
            0
        };

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        forward_progress(&app, "audit-progress", rx);
        let token = CancellationToken::new();
        state.audit.set_cancel(Some(token.clone()));
        let sink = engine::ProgressSink::new(tx, audit::AUDIT_STEPS + synced, token);

        if sync_needed {
            let (python, _) = health::choose_tools_python(&envs::scan().await).await?;
            health::sync_tools_venv(&dir, &python, health::AUDIT_TOOLS, &sink).await?;
        }
        audit::run(&dir, &env, &freeze, &sink.at(synced)).await
    }
    .await;

    // Released on both paths, and the token cleared with it: a token left registered would let the
    // *next* `audit_cancel` stop a run that had not started, which is S5's wedge bug wearing a
    // different hat.
    state.audit.release().await;
    state.audit.set_cancel(None);
    Ok(outcome?)
}

/// Stop a running audit, and say whether there was one.
///
/// Shaped like [`plan_cancel`] rather than returning `()`: "nothing was running" is a real answer
/// and the caller should not have to guess it from a silent success.
///
/// # Errors
/// Never fails.
#[tauri::command]
pub async fn audit_cancel(state: tauri::State<'_, AppState>) -> Wire<bool> {
    Ok(state.audit.stop().await)
}

/// Whether the parked report's project has uncommitted work, so the confirm can say so.
///
/// A command of its own rather than a field on `HealthReport`, because the answer expires: the
/// user may commit, or start editing, between the run and the Fix button. Asked when the dialog
/// opens; `health_fix` asks **again** before writing, because this one decides what to *render*
/// and that one decides what to allow.
///
/// `null` means no repository, no git, or git failed — CODE-HEALTH-SPEC §5's check is a courtesy,
/// not a precondition, and a folder outside version control is a choice rather than a warning.
///
/// # Errors
/// `PD-INT-001` when no run is parked.
#[tauri::command]
pub async fn health_dirty(state: tauri::State<'_, AppState>) -> Wire<Option<usize>> {
    let session = state.health.claim_one().await?;
    let found = pipdock_core::health::fix::dirty(&session.project).await;
    state.health.park(session).await;
    Ok(found.map(|tree| tree.entries))
}

/// Apply ruff's safe fixes to the project the parked report was produced from.
///
/// **Takes no `project`, no `env`, no file list and no findings.** The project and environment
/// come from the parked `HealthSession`, which is the point: what is fixed must be what was
/// reported on, the same reasoning that makes `plan_execute` take nothing. A file list would let
/// the frontend choose a different blast radius than the one it named in the dialog, so `files`
/// crosses as an **assertion to be checked**, never as an instruction.
///
/// # Errors
/// `PD-INT-001` when nothing is parked, when `files` disagrees with the parked report, or when
/// `acknowledged_dirty` is false against a tree the server just found dirty — all three are states
/// only a broken or out-of-date frontend can produce. `PD-RES-002` when a re-read of the project
/// no longer matches what the user confirmed. `PD-PRM-003` when a target cannot be written, raised
/// **before anything is**.
#[tauri::command]
pub async fn health_fix(
    state: tauri::State<'_, AppState>,
    files: usize,
    acknowledged_dirty: bool,
) -> Wire<pipdock_core::health::fix::FixReport> {
    use pipdock_core::health::{self, fix};

    // `claim_one`, not `claim`: finding nothing must leave the slot **idle** rather than writing
    // `Busy` and wedging every later command with `PD-RES-003` for a session that does not exist.
    let session = state.health.claim_one().await?;

    let outcome = async {
        // What the dialog named must be what the server is holding. Only a frontend that is out
        // of date with its own store can get here.
        if files != session.report.ruff.fixable_files {
            return Err(PdError::new(
                pipdock_core::errors::Code::IntUnexpected,
                format!(
                    "the confirmation named {files} file(s); the report has {}",
                    session.report.ruff.fixable_files
                ),
            ));
        }

        let tools_dir = health::tools_dir(&state.app_data);
        let opts = health::RunOptions::default();

        // Read the project again before writing to it. Minutes may have passed since the report,
        // and a source tree is not PipDock's to assume is still — DATA-FLOW §9.3's staleness rule,
        // applied to the one thing no snapshot describes.
        let fresh = fix::recheck(&tools_dir, &session.project, &opts).await?;
        if fresh.fixable != session.report.ruff.fixable
            || fresh.fixable_files != session.report.ruff.fixable_files
        {
            return Err(PdError::new(
                pipdock_core::errors::Code::ResPlanStale,
                format!(
                    "the project changed: {} fixable in {} file(s) now, {} in {} when confirmed",
                    fresh.fixable,
                    fresh.fixable_files,
                    session.report.ruff.fixable,
                    session.report.ruff.fixable_files
                ),
            ));
        }

        // Asked again rather than trusted: the dialog's answer decided what to *render*, this one
        // decides what to allow.
        let consent = fix::consent_ok(
            files,
            acknowledged_dirty,
            fix::dirty(&session.project).await,
        )?;
        fix::apply(&tools_dir, &session.project, &fresh, consent).await
    }
    .await;

    // Re-parked either way, with the post-fix ruff state on success. The screen drops its tab
    // count to `remaining` without a second Run, so the server has to be holding the same thing
    // the user is now looking at — otherwise the next fix is checked against a report nobody sees.
    let mut session = session;
    if let Ok(report) = &outcome {
        session.report.ruff = report.remaining.clone();
    }
    state.health.park(session).await;
    outcome.map_err(Into::into)
}

/// Write a finished report beside the path the user named, as Markdown **and** JSON.
///
/// CODE-HEALTH-SPEC §5's *Save report*. Two files from one prompt: the Markdown is for reading and
/// the JSON is what §7 means by "the JSON export enables users to wire their own" CI annotations,
/// and asking twice for what is conceptually one action would spend a click on nothing.
///
/// **Written in Rust rather than through `tauri-plugin-fs`.** A general `fs:allow-write-text-file`
/// is a write-anywhere primitive granted to the webview of a tool that already runs subprocesses
/// against the user's interpreters; this needs the webview to have no write permission at all.
/// `dialog:allow-save` only lets it *ask* for a path.
///
/// Takes the report from the frontend rather than the parked session on purpose: what the user
/// asked to save is what is on their screen. Nothing is executed from it, so the trust question
/// that makes `health_fix` read server-side does not arise here.
///
/// # Errors
/// `PD-SYS-002` when either file cannot be written; the path is the user's own choice, so a
/// failure is a real filesystem answer rather than something to retry.
#[tauri::command]
pub async fn health_save_report(
    report: pipdock_core::health::HealthReport,
    path: String,
) -> Wire<Vec<String>> {
    let base = std::path::PathBuf::from(&path);
    // The picker suggests `.md`, but the user may type anything or nothing; deriving both names
    // from the stem means the pair always matches rather than depending on what was typed.
    let md = base.with_extension("md");
    let json = base.with_extension("json");

    let body = pipdock_core::health::markdown(&report);
    let document = serde_json::to_string_pretty(&report).map_err(|e| {
        PdError::new(
            pipdock_core::errors::Code::IntUnexpected,
            format!("serialize report: {e}"),
        )
    })?;

    for (target, contents) in [(&md, body), (&json, document)] {
        std::fs::write(target, contents).map_err(|e| {
            PdError::new(
                pipdock_core::errors::Code::SysDiskFull,
                format!("write {}: {e}", target.display()),
            )
        })?;
    }
    Ok(vec![md.display().to_string(), json.display().to_string()])
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
