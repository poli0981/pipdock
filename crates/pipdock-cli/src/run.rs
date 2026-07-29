//! Command implementations.
//!
//! CLI-SPEC §1.1: every command maps 1:1 onto a `pipdock_core` call and **adds no logic of its
//! own**. Anything here that starts making a decision belongs in the core instead, so the GUI
//! inherits it (PRD G5: GUI and CLI never diverge).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use pipdock_core::engine::{Engine, ProgressEvent, pip::PipEngine, uv::UvEngine};
use pipdock_core::envs::{self, Candidate};
use pipdock_core::errors::{Code, PdError, Result};
use pipdock_core::flow::{
    FlowStep, NothingReason, RollbackFlow, SnapshotPolicy, UninstallFlow, UpdateFlow,
};
use pipdock_core::model::{EngineId, EnvSource, PkgName, PyEnv, StepStatus};

/// Re-exported so `main.rs` keeps referring to `run::Intent`. It lives in the core now, because
/// translating what the user asked for into a `PlanRequest` is flow logic the GUI needs too.
pub use pipdock_core::flow::Intent;
use pipdock_core::store::Store;
use pipdock_core::{index, pins, plan, snapshot};

use crate::{EngineArg, Exit, GlobalOpts};

/// Resolve which engine to drive.
///
/// ARCHITECTURE §3: first run probes `uv --version` on PATH and preselects uv when present. The
/// `--engine` flag overrides for one invocation.
#[must_use]
pub fn engine_for(opts: &GlobalOpts) -> Box<dyn Engine> {
    match opts.engine {
        Some(EngineArg::Uv) => Box::new(UvEngine),
        Some(EngineArg::Pip) => Box::new(PipEngine),
        // No flag: use what the user configured. Falling back to pip rather than probing for uv
        // here keeps a read-only command from silently changing behaviour based on PATH; first-run
        // uv detection belongs to Settings, where the choice is shown and can be changed.
        None => match configured_engine() {
            Some(EngineId::Uv) => Box::new(UvEngine),
            _ => Box::new(PipEngine),
        },
    }
}

/// `kv` key holding the configured engine.
const KEY_ENGINE: &str = "settings.engine";

/// The engine the user configured, if any. A store that cannot be opened is not worth failing a
/// command over — the default is safe.
fn configured_engine() -> Option<EngineId> {
    let store = Store::open(&app_data_dir()).ok()?;
    match store.get(KEY_ENGINE).ok()??.as_str() {
        "uv" => Some(EngineId::Uv),
        "pip" => Some(EngineId::Pip),
        _ => None,
    }
}

/// `pipdock engine <pip|uv>` — set the configured engine.
///
/// # Errors
/// `PD-ENG-001` when the chosen engine is not actually available, because storing a preference
/// that cannot be honoured just moves the failure to the next command.
pub async fn engine_set(opts: &GlobalOpts, engine: EngineArg) -> Result<Exit> {
    let id = match engine {
        EngineArg::Pip => EngineId::Pip,
        EngineArg::Uv => EngineId::Uv,
    };

    // Availability is checked against the selected environment when there is one; uv is a
    // standalone binary, so it can be checked without one.
    let info = match select_env(opts).await {
        Ok(env) => match id {
            EngineId::Pip => PipEngine.info(&env).await,
            EngineId::Uv => UvEngine.info(&env).await,
        },
        Err(e) if id == EngineId::Uv => {
            let env = PyEnv {
                interpreter: PathBuf::new(),
                prefix: PathBuf::new(),
                python_version: String::new(),
                externally_managed: false,
                hidden_user_site: None,
                source: EnvSource::Manual,
            };
            let _ = e;
            UvEngine.info(&env).await
        }
        Err(e) => return Err(e),
    };

    if !info.available {
        return Err(PdError::new(
            Code::EngNotFound,
            format!(
                "{} is not available here, so it has not been set as the engine",
                id.as_str()
            ),
        ));
    }

    Store::open(&app_data_dir())?.set(KEY_ENGINE, id.as_str())?;
    println!(
        "engine set to {} {}",
        id.as_str(),
        info.version.unwrap_or_default()
    );
    Ok(Exit::Success)
}

/// Map a catalog code onto the exit code CLI-SPEC §5 assigns it.
///
/// Scripts branch on these numbers, so the mapping is part of the public contract — and it is
/// derived from the code's area rather than written out per call site, which is what keeps a new
/// code from silently exiting 10.
#[must_use]
pub fn exit_for(code: Code) -> Exit {
    use pipdock_core::errors::Area;
    match code.area() {
        Area::Env => Exit::EnvError,
        Area::Eng => Exit::EngineError,
        Area::Net => Exit::NetworkError,
        Area::Snp => Exit::SnapshotError,
        Area::Res => Exit::PlanAborted,
        // Package, build, permission, system and health failures are per-package outcomes; a
        // command that hit one still ran.
        Area::Pkg | Area::Bld | Area::Prm | Area::Sys | Area::Hlt => Exit::PartialFailure,
        Area::Int => Exit::Internal,
    }
}

/// Select the environment to act on.
///
/// CLI-SPEC §2: `--env` wins; otherwise an auto-detected `.venv` in the working directory; the
/// "last used" fallback arrives with the settings store.
///
/// # Errors
/// `PD-ENV-001` when nothing usable can be found, so the user gets a code rather than a silent
/// default that mutates the wrong environment.
pub async fn select_env(opts: &GlobalOpts) -> Result<PyEnv> {
    if let Some(path) = &opts.env {
        let interpreter = interpreter_in(path);
        return envs::probe(&interpreter, EnvSource::Manual)
            .await
            .map(|p| p.env);
    }

    let cwd = std::env::current_dir().unwrap_or_default();
    if let Some(found) = envs::venv_scan(&cwd).into_iter().next() {
        return envs::probe(&found, EnvSource::VenvScan)
            .await
            .map(|p| p.env);
    }

    Err(PdError::new(
        Code::EnvInterpreterMissing,
        "no environment selected: pass --env, or run from a directory containing a .venv",
    ))
}

/// Accept either an interpreter path or an environment directory, as CLI-SPEC §2 promises.
fn interpreter_in(path: &Path) -> PathBuf {
    if path.is_file() {
        return path.to_path_buf();
    }
    let exe: PathBuf = if cfg!(windows) {
        ["Scripts", "python.exe"].iter().collect()
    } else {
        ["bin", "python"].iter().collect()
    };
    let candidate = path.join(&exe);
    if candidate.is_file() {
        candidate
    } else {
        path.to_path_buf()
    }
}

/// `pipdock env list`
///
/// # Errors
/// Never fails as a whole: an interpreter that cannot be probed is reported in place rather than
/// aborting the listing, because one broken environment must not hide the rest.
pub async fn env_list(opts: &GlobalOpts) -> Result<Exit> {
    let candidates = envs::scan().await;

    let mut rows = Vec::new();
    for Candidate { path, source } in candidates {
        let probed = envs::probe(&path, source).await;
        rows.push((path, source, probed));
    }

    if opts.json {
        let payload: Vec<serde_json::Value> = rows
            .iter()
            .map(|(path, source, probed)| match probed {
                Ok(p) => serde_json::json!({
                    "interpreter": path,
                    "source": source,
                    "python": p.env.python_version,
                    "externallyManaged": p.env.externally_managed,
                    "hiddenUserSite": p.env.hidden_user_site,
                    "packages": p.dists.len(),
                    "envHash": envs::env_hash(path),
                }),
                Err(e) => serde_json::json!({
                    "interpreter": path,
                    "source": source,
                    "error": { "code": e.code.as_str(), "message": e.message },
                }),
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
        return Ok(Exit::Success);
    }

    if rows.is_empty() {
        println!("no Python environments found");
        return Ok(Exit::Success);
    }

    for (path, source, probed) in &rows {
        let source = format!("{source:?}").to_lowercase();
        match probed {
            Ok(p) => {
                let mut chips = Vec::new();
                if p.env.externally_managed {
                    chips.push("MANAGED".to_owned());
                }
                if p.env.listing_is_partial() {
                    chips.push("PARTIAL LISTING".to_owned());
                }
                println!(
                    "{:<52} py {:<9} {:>4} pkgs  [{}] {}",
                    path.display(),
                    p.env.python_version,
                    p.dists.len(),
                    source,
                    chips.join(" ")
                );
            }
            Err(e) => println!("{:<52} error[{}]  [{source}]", path.display(), e.code),
        }
    }
    Ok(Exit::Success)
}

/// `pipdock list [--outdated]`
///
/// # Errors
/// Propagates environment and engine failures with their catalog codes.
pub async fn list(opts: &GlobalOpts, outdated: bool) -> Result<Exit> {
    let env = select_env(opts).await?;
    let engine = engine_for(opts);

    if outdated {
        let rows = engine.list_outdated(&env).await?;
        if opts.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&rows).unwrap_or_default()
            );
        } else if rows.is_empty() {
            println!("all packages up to date");
        } else {
            for row in &rows {
                println!("{:<40} {:<16} -> {}", row.name, row.current, row.latest);
            }
        }
        return Ok(Exit::Success);
    }

    // The probe is the richer source: it carries requires_dist, which `list --format=json` does
    // not, and it is what the reverse-dependency graph is built from.
    let probed = envs::probe(&env.interpreter, env.source).await?;
    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&probed.dists).unwrap_or_default()
        );
    } else {
        if probed.env.listing_is_partial() {
            // SECURITY §2: say so rather than quietly under-reporting.
            if let Some(hidden) = &probed.env.hidden_user_site {
                println!(
                    "note: packages installed for your user account are not shown ({})",
                    hidden.display()
                );
            }
        }
        for dist in &probed.dists {
            println!("{:<40} {}", dist.name, dist.version);
        }
    }
    Ok(Exit::Success)
}

/// `pipdock doctor`
///
/// # Errors
/// Propagates environment failures; engine unavailability is reported in the output rather than
/// raised, because naming what is wrong is the command's entire job.
pub async fn doctor(opts: &GlobalOpts) -> Result<Exit> {
    let env = select_env(opts).await?;
    let pip = PipEngine.info(&env).await;
    let uv = UvEngine.info(&env).await;
    let check = engine_for(opts).check(&env).await;

    let pip_ok = pip.available;
    let check_ok = check.as_ref().map(|c| c.ok).unwrap_or(false);

    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "interpreter": env.interpreter,
                "python": env.python_version,
                "envHash": envs::env_hash(&env.interpreter),
                "externallyManaged": env.externally_managed,
                "hiddenUserSite": env.hidden_user_site,
                "engines": { "pip": pip, "uv": uv },
                "check": check.as_ref().ok(),
            }))
            .unwrap_or_default()
        );
    } else {
        println!("interpreter : {}", env.interpreter.display());
        println!("python      : {}", env.python_version);
        println!("env hash    : {}", envs::env_hash(&env.interpreter));
        println!(
            "pip         : {}",
            pip.version
                .clone()
                .unwrap_or_else(|| "not available".into())
        );
        println!(
            "uv          : {}",
            uv.version.clone().unwrap_or_else(|| "not available".into())
        );
        if env.externally_managed {
            println!("PEP 668     : externally managed — mutation blocked (PD-ENV-002)");
        }
        if let Some(hidden) = &env.hidden_user_site {
            println!(
                "note        : user-site packages not listed ({})",
                hidden.display()
            );
        }
        match &check {
            Ok(c) if c.ok => println!("check       : no broken requirements"),
            Ok(c) => {
                println!("check       : {} broken requirement(s)", c.findings.len());
                for f in &c.findings {
                    println!("              {}", f.requirement);
                }
            }
            Err(e) => println!("check       : could not run ({})", e.code),
        }
    }

    // A doctor that found real problems should say so in its exit code, or scripts cannot use it.
    Ok(if pip_ok && check_ok {
        Exit::Success
    } else {
        Exit::PartialFailure
    })
}

/// The app data root, `%LOCALAPPDATA%\PipDock` (ARCHITECTURE §6).
#[must_use]
pub fn app_data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(pipdock_core::APP_DATA_DIR_NAME)
}

/// `pipdock snapshot list`
///
/// # Errors
/// Propagates environment failures.
pub async fn snapshot_list(opts: &GlobalOpts) -> Result<Exit> {
    let env = select_env(opts).await?;
    let hash = envs::env_hash(&env.interpreter);
    let metas = snapshot::list(&app_data_dir(), &hash)?;

    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&metas).unwrap_or_default()
        );
    } else if metas.is_empty() {
        println!("no snapshots for this environment");
    } else {
        for m in &metas {
            let trigger = match &m.trigger {
                snapshot::Trigger::Plan { plan_id } => format!("plan {plan_id}"),
                snapshot::Trigger::Rollback { restoring } => format!("rollback of {restoring}"),
                snapshot::Trigger::Manual => "manual".to_owned(),
            };
            println!(
                "{:<24} {:>4} pkgs  {:<6} {}",
                m.id,
                m.package_count,
                m.engine.as_str(),
                trigger
            );
        }
    }
    Ok(Exit::Success)
}

/// `pipdock snapshot create`
///
/// # Errors
/// `PD-SNP-001` when the snapshot cannot be written.
pub async fn snapshot_create(opts: &GlobalOpts) -> Result<Exit> {
    let env = select_env(opts).await?;
    let engine = engine_for(opts);
    let freeze = engine.freeze(&env).await?;
    let hash = envs::env_hash(&env.interpreter);

    let snap = snapshot::create(
        &app_data_dir(),
        &hash,
        freeze,
        snapshot::Trigger::Manual,
        engine.id(),
        jiff::Timestamp::now(),
    )?;

    println!(
        "snapshot {} written ({} packages)",
        snap.meta.id, snap.meta.package_count
    );
    Ok(Exit::Success)
}

/// `pipdock snapshot diff <id>`
///
/// # Errors
/// `PD-SNP-002` when the snapshot does not exist.
pub async fn snapshot_diff(opts: &GlobalOpts, id: &str) -> Result<Exit> {
    let env = select_env(opts).await?;
    let engine = engine_for(opts);
    let hash = envs::env_hash(&env.interpreter);

    let snap = snapshot::load(&app_data_dir(), &hash, id)?;
    let current = snapshot::parse_freeze(&engine.freeze(&env).await?);
    let d = snapshot::diff(&current, &snap.entries());

    if opts.json {
        println!("{}", serde_json::to_string_pretty(&d).unwrap_or_default());
        return Ok(Exit::Success);
    }

    if d.is_empty() {
        println!("environment matches snapshot {}", snap.meta.id);
        return Ok(Exit::Success);
    }
    for s in &d.added {
        println!("+ {} {}   (not in snapshot)", s.name, s.version);
    }
    for s in &d.removed {
        println!("- {} {}   (in snapshot, not installed)", s.name, s.version);
    }
    for c in &d.changed {
        println!("~ {} {} -> {} (snapshot)", c.name, c.current, c.snapshot);
    }

    // Honesty about what a rollback could not put back — the same reason PD-SNP-002 exists.
    let stuck = snapshot::unrestorable_lines(&snap.freeze);
    if !stuck.is_empty() {
        println!(
            "\n{} entr(y/ies) cannot be restored from an index:",
            stuck.len()
        );
        for line in &stuck {
            println!("  {line}");
        }
    }
    Ok(Exit::Success)
}

/// `pipdock update [--all | <pkg...>]` and `pipdock install <spec...>`.
///
/// The full DATA-FLOW §3 tail, shared by both: resolve → derive held-back → decide → re-resolve
/// → confirm → snapshot → two-phase execute → post-check → summary.
///
/// # Errors
/// `PD-SNP-001` when the pre-execution snapshot fails, in which case **nothing is executed**;
/// otherwise propagates environment and engine failures. Per-package failures are not errors —
/// they appear in the summary.
pub async fn plan_and_run(opts: &GlobalOpts, intent: Intent, dry_run: bool) -> Result<Exit> {
    let env = select_env(opts).await?;
    let store = Store::open(&app_data_dir())?;

    let (mut flow, mut step) = UpdateFlow::start(env, engine_for(opts), &intent, &store).await?;

    // The flow returns the exclusion as data; the wording is the head's business (I18N §1).
    if !flow.excluded_pins().is_empty() && !opts.json {
        println!(
            "{} pinned package(s) excluded: {}",
            flow.excluded_pins().len(),
            flow.excluded_pins()
                .iter()
                .map(|p| p.pkg.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let force_everything = matches!(
        intent,
        Intent::Update {
            force_latest: true,
            ..
        }
    );

    // Drive the resolve/decide loop. The cap itself lives in the flow; this only answers.
    let report = loop {
        match step {
            FlowStep::Nothing { reason } => {
                return Ok(match reason {
                    NothingReason::NothingToDo => {
                        println!("nothing to do");
                        Exit::Success
                    }
                    NothingReason::EverythingSkipped => {
                        println!("every package was skipped; nothing to do");
                        Exit::PlanAborted
                    }
                });
            }
            FlowStep::NeedsConfirm { report } | FlowStep::RoundsExhausted { report } => {
                break report;
            }
            FlowStep::NeedsDecisions { ref report, .. } => {
                let decisions = decide(report, force_everything, opts);
                step = flow.decide(&decisions).await?;
            }
        }
    };

    print_preview(opts, &report);

    if dry_run {
        return Ok(Exit::Success);
    }
    if report.changes.is_empty() {
        println!("no changes to apply");
        return Ok(Exit::Success);
    }
    if !opts.yes && !confirm(&report) {
        println!("aborted");
        return Ok(Exit::PlanAborted);
    }

    // DATA-FLOW §9.2: the snapshot comes before anything is touched, and its failure aborts.
    let policy = if opts.no_snapshot {
        // CLI-SPEC §2 documents this for CI images only, and requires the warning.
        eprintln!(
            "warning: --no-snapshot given. If this goes wrong there is no way back — \
             only use it on a disposable environment."
        );
        SnapshotPolicy::Waive
    } else {
        SnapshotPolicy::Take
    };
    if let Some(meta) = flow.take_snapshot(policy, &app_data_dir()).await?
        && !opts.json
    {
        println!("snapshot {} written", meta.id);
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();
    let quiet = opts.quiet;
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if !quiet {
                eprintln!("{}", event.line);
            }
        }
    });

    let summary = flow.execute(tx).await?;

    print_summary(opts, &summary);
    Ok(if summary.counts.failed > 0 {
        Exit::PartialFailure
    } else {
        Exit::Success
    })
}

/// Decide what to do about each package needing a decision.
///
/// CLI-SPEC §4: on a TTY the user is prompted per package; with `--yes` or off a TTY the defaults
/// apply, and those defaults **never force**.
fn decide(
    report: &plan::ResolutionReport,
    force_everything: bool,
    opts: &GlobalOpts,
) -> BTreeMap<PkgName, plan::Decision> {
    let interactive = !opts.yes && std::io::IsTerminal::is_terminal(&std::io::stdin());

    let mut out = BTreeMap::new();
    for held in &report.held_back {
        let decision = if interactive {
            prompt_decision(held)
        } else {
            plan::default_decision(false, force_everything)
        };
        out.insert(held.pkg.clone(), decision);
    }
    if let Some(detail) = &report.impossible {
        for pkg in &detail.packages {
            let decision = plan::default_decision(true, false);
            out.insert(pkg.clone(), decision);
        }
    }
    out
}

/// The per-package prompt from CLI-SPEC §4.
fn prompt_decision(held: &plan::HeldBack) -> plan::Decision {
    use std::io::Write as _;

    println!(
        "\n{}  held back at {} (latest {})",
        held.pkg, held.resolved, held.latest
    );
    for blocker in &held.blockers {
        println!("  — {}", blocker.constraint);
    }
    print!("  [C]ompatible (default)   [S]kip   [F]orce latest: ");
    let _ = std::io::stdout().flush();

    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return plan::Decision::KeepCompatible;
    }
    match line.trim().to_ascii_lowercase().as_str() {
        "s" => plan::Decision::Skip,
        "f" => {
            // DISCLAIMER §2: forcing is an expert action and the user is told what it costs.
            let breaks: Vec<String> = held
                .blockers
                .iter()
                .filter_map(|b| b.by.as_ref().map(ToString::to_string))
                .collect();
            if !breaks.is_empty() {
                println!("  this will break: {}", breaks.join(", "));
            }
            plan::Decision::ForceLatest
        }
        _ => plan::Decision::KeepCompatible,
    }
}

/// Render the preview (UI-SPEC §4's three groups, in text).
fn print_preview(opts: &GlobalOpts, report: &plan::ResolutionReport) {
    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(report).unwrap_or_default()
        );
        return;
    }

    let upgrades: Vec<_> = report.changes.iter().filter(|c| c.from.is_some()).collect();
    let fresh: Vec<_> = report.changes.iter().filter(|c| c.from.is_none()).collect();

    if !upgrades.is_empty() {
        println!("\nWill upgrade:");
        for c in upgrades {
            let from = c.from.as_ref().map(ToString::to_string).unwrap_or_default();
            println!("  {:<32} {} -> {}", c.name, from, c.to);
        }
    }
    if !fresh.is_empty() {
        println!("\nNew:");
        for c in fresh {
            println!("  {:<32} {}", c.name, c.to);
        }
    }
    if !report.held_back.is_empty() {
        println!("\nNeeds decision — held back:");
        for h in &report.held_back {
            println!("  {:<32} {} (latest {})", h.pkg, h.resolved, h.latest);
            for b in &h.blockers {
                println!("      {}", b.constraint);
            }
            if h.blockers.is_empty() {
                // ARCHITECTURE §3: no culprit is invented, and the row says so plainly rather
                // than looking like a rendering bug.
                println!("      (nothing installed explains this)");
            }
        }
    }
    if let Some(detail) = &report.impossible {
        println!("\nImpossible:");
        println!("  {}", detail.explanation);
    }
    if report.changes.is_empty() && report.held_back.is_empty() {
        println!("\nno changes");
    }
}

/// Ask before touching anything (DATA-FLOW §3's Confirm step).
fn confirm(report: &plan::ResolutionReport) -> bool {
    use std::io::Write as _;

    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        // Off a TTY without --yes, refusing is the safe answer: nothing is there to confirm.
        eprintln!("error[PD-RES-002]: not a terminal; pass --yes to apply without confirming");
        return false;
    }
    print!("\nApply {} change(s)? [y/N]: ", report.changes.len());
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// `pipdock schema <type>`
///
/// # Errors
/// `PD-PKG-002` when the type is not one of the exported ones.
pub fn schema(type_name: &str) -> Result<Exit> {
    let schema = plan::json_schema(type_name)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&schema).unwrap_or_default()
    );
    Ok(Exit::Success)
}

/// `pipdock env use <path>`
///
/// # Errors
/// `PD-ENV-001` when the path is not a usable interpreter — chosen deliberately over accepting it
/// silently, because a default that does not work fails on every later command instead of this one.
pub async fn env_use(opts: &GlobalOpts, path: &Path) -> Result<Exit> {
    let interpreter = interpreter_in(path);
    let probed = envs::probe(&interpreter, EnvSource::Manual).await?;
    let store = Store::open(&app_data_dir())?;

    store.remember_env(
        &envs::env_hash(&interpreter),
        &interpreter.display().to_string(),
        &jiff::Timestamp::now().to_string(),
        true,
    )?;

    if !opts.quiet {
        println!(
            "default environment set: {} (Python {})",
            interpreter.display(),
            probed.env.python_version
        );
    }
    Ok(Exit::Success)
}

/// `pipdock pip-upgrade`
///
/// # Errors
/// `PD-ENG-001` when uv is the active engine, which cannot upgrade pip (DATA-FLOW §7).
pub async fn pip_upgrade(opts: &GlobalOpts) -> Result<Exit> {
    let env = select_env(opts).await?;
    let engine = engine_for(opts);

    let before = PipEngine.info(&env).await;
    engine.upgrade_pip(&env).await?;
    let after = PipEngine.info(&env).await;

    println!(
        "pip {} -> {}",
        before.version.unwrap_or_else(|| "?".into()),
        after.version.unwrap_or_else(|| "?".into())
    );
    Ok(Exit::Success)
}

/// `pipdock snapshot rollback <id|latest>`
///
/// DATA-FLOW §8: diff, plan the minimal operations, **snapshot the current state first** because a
/// rollback is itself reversible, then execute two-phase.
///
/// # Errors
/// `PD-SNP-002` when the snapshot does not exist; `PD-SNP-001` when the pre-rollback snapshot
/// cannot be written, in which case nothing is executed.
pub async fn snapshot_rollback(opts: &GlobalOpts, id: &str) -> Result<Exit> {
    let env = select_env(opts).await?;
    let app_data = app_data_dir();

    let (mut flow, preview) = RollbackFlow::start(env, engine_for(opts), &app_data, id).await?;

    if preview.restore.is_empty() {
        println!("environment already matches snapshot {}", preview.target.id);
        return Ok(Exit::Success);
    }

    println!("Rolling back to {}:", preview.target.id);
    for name in &preview.restore.uninstall {
        println!("  remove  {name}");
    }
    for spec in &preview.restore.install {
        println!("  restore {} {}", spec.name, spec.version);
    }

    // Honesty about what cannot come back, rather than reporting a success that is not one.
    if !preview.unrestorable.is_empty() {
        println!(
            "\nwarning[PD-SNP-002]: {} entr(y/ies) in this snapshot cannot be restored from an \
             index and will be left as they are:",
            preview.unrestorable.len()
        );
        for line in &preview.unrestorable {
            println!("  {line}");
        }
    }

    if !opts.yes && !confirm_text(&format!("Apply {} operation(s)?", preview.restore.len())) {
        println!("aborted");
        return Ok(Exit::PlanAborted);
    }

    // A rollback is itself reversible (DATA-FLOW §8), so the state being left behind is captured
    // before it is replaced.
    let pre = flow.take_snapshot(&app_data).await?;
    if !opts.json {
        println!("snapshot {} written before rolling back", pre.id);
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();
    let quiet = opts.quiet;
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if !quiet {
                eprintln!("{}", event.line);
            }
        }
    });

    let summary = flow.execute(tx).await?;

    print_summary(opts, &summary);
    Ok(if summary.counts.failed > 0 {
        Exit::PartialFailure
    } else {
        Exit::Success
    })
}

/// `pipdock self report-bug`
///
/// ERROR-CATALOG §4: builds a prefilled GitHub issue URL and prints it. **Nothing is ever sent
/// automatically** — the user reviews the form in their browser. This is the entire telemetry
/// story (PRIVACY-POLICY §4).
///
/// # Errors
/// Never; environment details that cannot be read are simply left blank.
pub async fn report_bug(opts: &GlobalOpts) -> Result<Exit> {
    let env = select_env(opts).await.ok();
    let engine = engine_for(opts);

    let python = env
        .as_ref()
        .map(|e| e.python_version.clone())
        .unwrap_or_default();
    let engine_version = match &env {
        Some(e) => engine.info(e).await.version.unwrap_or_default(),
        None => String::new(),
    };

    let mut url = format!(
        "https://github.com/poli0981/pipdock/issues/new?template=bug_report.yml\
         &pd-version={}&os={}&engine={}",
        urlencode(env!("CARGO_PKG_VERSION")),
        urlencode(&os_description()),
        urlencode(engine.id().as_str()),
    );
    if !python.is_empty() {
        url.push_str(&format!(
            "&python={}",
            urlencode(&format!(
                "Python {python} · {} {engine_version}",
                engine.id().as_str()
            ))
        ));
    }

    println!("{url}");
    if !opts.quiet {
        eprintln!(
            "\nOpen that in a browser to review the prefilled issue. Nothing is sent until you \
             submit it yourself.\nCheck the log excerpt for paths or names you would rather not \
             make public."
        );
    }
    Ok(Exit::Success)
}

/// Percent-encode a query-string value.
///
/// Hand-rolled rather than pulling a dependency for one call site: the alphabet is small and the
/// consequence of getting it wrong is a broken link, not a security hole.
fn urlencode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn os_description() -> String {
    // The issue template asks for a Windows version; without a dependency the best honest answer
    // is the target family plus whatever the OS tells us for free.
    std::env::var("OS").unwrap_or_else(|_| std::env::consts::OS.to_owned())
}

/// A yes/no prompt that refuses off a TTY.
fn confirm_text(question: &str) -> bool {
    use std::io::Write as _;

    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        eprintln!("error[PD-RES-002]: not a terminal; pass --yes to proceed without confirming");
        return false;
    }
    print!("\n{question} [y/N]: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// `pipdock index refresh`
///
/// # Errors
/// `PD-NET-010` when the index cannot be fetched. The previous index stays in place and remains
/// searchable — a failed refresh must not cost the user the index they already had.
pub async fn index_refresh(opts: &GlobalOpts) -> Result<Exit> {
    let store = Store::open(&app_data_dir())?;
    if !opts.quiet {
        eprintln!("fetching {} …", index::SIMPLE_INDEX_URL);
    }

    let report = index::refresh(&store, jiff::Timestamp::now()).await?;

    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
    } else {
        println!(
            "indexed {} projects in {:.1}s ({:.1} MiB)",
            report.projects,
            report.elapsed_ms as f64 / 1000.0,
            report.wire_bytes as f64 / 1024.0 / 1024.0
        );
    }
    Ok(Exit::Success)
}

/// `pipdock search <query> [--limit n]`
///
/// # Errors
/// `PD-NET-010` when the index has never been built; the message says to refresh.
pub async fn search(opts: &GlobalOpts, query: &str, limit: usize) -> Result<Exit> {
    let store = Store::open(&app_data_dir())?;
    let idx = index::NameIndex::load(&store)?;

    // ARCHITECTURE §5: search is entirely local, so it works offline. Only the staleness note
    // needs the clock.
    let now = jiff::Timestamp::now();
    if index::is_stale(index::last_refresh(&store)?, now) && !opts.quiet && !opts.json {
        eprintln!(
            "note: the package index is over a week old — `pipdock index refresh` updates it"
        );
    }

    let hits = idx.search(query, limit);

    // Installed packages are chipped rather than offered again (DATA-FLOW §4).
    let installed: BTreeSet<PkgName> = match select_env(opts).await {
        Ok(env) => envs::probe(&env.interpreter, env.source)
            .await
            .map(|p| p.dists.into_iter().map(|d| d.name).collect())
            .unwrap_or_default(),
        // Searching without an environment selected is legitimate; the chips just go away.
        Err(_) => BTreeSet::new(),
    };

    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&hits).unwrap_or_default()
        );
        return Ok(Exit::Success);
    }
    if hits.is_empty() {
        println!("no packages matching {query:?}");
        return Ok(Exit::Success);
    }
    for hit in &hits {
        let chip = if installed.contains(&hit.name) {
            "INSTALLED"
        } else {
            ""
        };
        println!(
            "{:<40} {:<10} {chip}",
            hit.display,
            format!("{:?}", hit.kind).to_lowercase()
        );
    }
    Ok(Exit::Success)
}

/// `pipdock info <pkg>`
///
/// # Errors
/// `PD-PKG-002` when PyPI does not know the name; `PD-NET-001` when it is neither cached nor
/// reachable.
pub async fn info(opts: &GlobalOpts, pkg: &str) -> Result<Exit> {
    let store = Store::open(&app_data_dir())?;
    let name = PkgName::parse(pkg)?;
    let (meta, freshness) = index::metadata(&store, &name, jiff::Timestamp::now()).await?;

    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "meta": meta,
                "freshness": freshness,
            }))
            .unwrap_or_default()
        );
        return Ok(Exit::Success);
    }

    println!("{}", meta.name);
    if let Some(v) = &meta.version {
        println!("  version        {v}");
    }
    if let Some(s) = &meta.summary {
        println!("  summary        {s}");
    }
    if let Some(r) = &meta.requires_python {
        println!("  requires-python {r}");
    }
    if let Some(l) = &meta.license {
        println!("  license        {l}");
    }
    if let Some(h) = &meta.home_page {
        println!("  home           {h}");
    }
    if freshness == index::Freshness::Stale {
        // UI-SPEC §7: offline shows a cached badge rather than an error, because the data is
        // still useful and search still works.
        println!("  (offline — showing cached data)");
    }
    Ok(Exit::Success)
}

/// `pipdock pin add|remove|list`
///
/// # Errors
/// Propagates environment and store failures.
pub async fn pin_list(opts: &GlobalOpts) -> Result<Exit> {
    let env = select_env(opts).await?;
    let store = Store::open(&app_data_dir())?;
    let pins = pins::list(&store, &envs::env_hash(&env.interpreter))?;

    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&pins).unwrap_or_default()
        );
    } else if pins.is_empty() {
        println!("no pins for this environment");
    } else {
        for p in &pins {
            let mode = match &p.mode {
                pins::PinMode::Exclude => "exclude".to_owned(),
                pins::PinMode::Hold { version } => format!("hold {version}"),
            };
            println!(
                "{:<32} {:<16} {}",
                p.pkg,
                mode,
                p.reason.as_deref().unwrap_or("")
            );
        }
    }
    Ok(Exit::Success)
}

/// `pipdock pin add <pkg> [--reason ...]`
///
/// # Errors
/// `PD-PKG-002` for an invalid name; propagates environment and store failures.
pub async fn pin_add(opts: &GlobalOpts, pkg: &str, reason: Option<&str>) -> Result<Exit> {
    let env = select_env(opts).await?;
    let store = Store::open(&app_data_dir())?;
    let name = PkgName::parse(pkg)?;

    pins::add(
        &store,
        &envs::env_hash(&env.interpreter),
        &pins::Pin {
            pkg: name.clone(),
            mode: pins::PinMode::Exclude,
            reason: reason.map(str::to_owned),
        },
    )?;
    println!("pinned {name} (excluded from bulk updates)");
    Ok(Exit::Success)
}

/// `pipdock pin remove <pkg>`
///
/// # Errors
/// `PD-PKG-002` for an invalid name; propagates environment and store failures.
pub async fn pin_remove(opts: &GlobalOpts, pkg: &str) -> Result<Exit> {
    let env = select_env(opts).await?;
    let store = Store::open(&app_data_dir())?;
    let name = PkgName::parse(pkg)?;

    if pins::remove(&store, &envs::env_hash(&env.interpreter), &name)? {
        println!("unpinned {name}");
    } else {
        println!("{name} was not pinned");
    }
    Ok(Exit::Success)
}

/// `pipdock uninstall <pkg...> [--force]`
///
/// DATA-FLOW §5: the reverse-dependency guard runs **once against the full removal set**, before
/// anything is touched. Bare `pip uninstall` performs no such check, which is the whole reason
/// this exists.
///
/// Without `--force`, a guard trip **aborts and executes nothing**, exiting non-zero so the CI
/// idiom in CLI-SPEC §7 (`pipdock uninstall legacylib --json || echo "dependents exist"`) works.
///
/// # Errors
/// `PD-PKG-002` for an invalid name; `PD-SNP-001` when the pre-removal snapshot cannot be written,
/// in which case nothing is executed.
pub async fn uninstall(opts: &GlobalOpts, pkgs: &[String], force: bool) -> Result<Exit> {
    let env = select_env(opts).await?;
    let engine = engine_for(opts);

    let (mut flow, report) = UninstallFlow::start(env, engine, pkgs).await?;

    if !report.is_clear() {
        if opts.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            );
        } else {
            for (pkg, broken) in &report.breaks {
                let list = broken
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("removing {pkg} breaks {list}");
            }
        }
        if !force {
            eprintln!(
                "error[PD-PKG-002]: refusing to remove packages other packages depend on.\n\
                 Re-run with --force to proceed anyway, or add the dependents to the removal set:\n\
                   pipdock uninstall {}",
                report
                    .with_dependents
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            // CLI-SPEC §7 documents this as exit 1 ("exit 1 if guard trips"), so scripts can use
            // `||` without matching a specific code.
            return Ok(Exit::PartialFailure);
        }
        eprintln!("warning: --force given; proceeding despite the breakage above");
    }

    // DATA-FLOW §9.2 applies to removals too: nothing is touched before a snapshot exists.
    let meta = flow.take_snapshot(&app_data_dir()).await?;
    if !opts.json {
        println!("snapshot {} written before removing", meta.id);
    }

    // Engine output streams to stderr as it happens, which is the CLI's equivalent of the GUI's
    // console drawer: a long removal that prints nothing looks hung.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();
    let quiet = opts.quiet;
    let pump = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if !quiet {
                eprintln!("{}", event.line);
            }
        }
    });

    let summary = flow.execute(tx).await?;
    drop(pump);

    print_summary(opts, &summary);
    Ok(if summary.counts.failed > 0 {
        Exit::PartialFailure
    } else {
        Exit::Success
    })
}

/// Render an execution summary (DATA-FLOW §6).
fn print_summary(opts: &GlobalOpts, summary: &plan::ExecutionSummary) {
    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(summary).unwrap_or_default()
        );
        return;
    }

    println!(
        "\n{} successful, {} failed, {} skipped",
        summary.counts.ok, summary.counts.failed, summary.counts.skipped
    );
    for row in &summary.results {
        if row.status == StepStatus::Failed {
            let code = row.code.map_or_else(String::new, |c| format!("[{c}] "));
            println!("  {} {}", row.pkg, code);
            if let Some(tail) = &row.stderr_tail {
                for line in tail.lines().take(6) {
                    println!("      {line}");
                }
            }
        }
    }
    if !summary.check.ok {
        println!(
            "\npost-check found {} problem(s):",
            summary.check.findings.len()
        );
        for f in &summary.check.findings {
            println!("  {}", f.requirement);
        }
    }
}

/// `pipdock engine [pip|uv]`
///
/// # Errors
/// Never; reports availability rather than failing.
pub async fn engine_status(opts: &GlobalOpts) -> Result<Exit> {
    let env = select_env(opts).await?;
    for (id, info) in [
        (EngineId::Pip, PipEngine.info(&env).await),
        (EngineId::Uv, UvEngine.info(&env).await),
    ] {
        println!(
            "{:<4} {}",
            id.as_str(),
            info.version.unwrap_or_else(|| "not available".into())
        );
    }
    Ok(Exit::Success)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_follow_the_catalog_area() {
        // CLI-SPEC §5, derived rather than hand-written per call site so a new code cannot
        // silently land on 10.
        assert_eq!(exit_for(Code::EnvExternallyManaged) as u8, 3);
        assert_eq!(exit_for(Code::EngNotFound) as u8, 4);
        assert_eq!(exit_for(Code::SnpWriteFailed) as u8, 5);
        assert_eq!(exit_for(Code::NetUnreachable) as u8, 6);
        assert_eq!(exit_for(Code::ResImpossible) as u8, 2);
        assert_eq!(exit_for(Code::BldBackendFailed) as u8, 1);
        assert_eq!(exit_for(Code::IntUnexpected) as u8, 10);
    }

    #[test]
    fn every_code_maps_to_a_documented_exit() {
        // The table in CLI-SPEC §5 is exhaustive; make sure the mapping is too.
        const DOCUMENTED: &[u8] = &[0, 1, 2, 3, 4, 5, 6, 10, 130];
        for code in Code::ALL {
            let exit = exit_for(*code) as u8;
            assert!(
                DOCUMENTED.contains(&exit),
                "{code} -> undocumented exit {exit}"
            );
        }
    }

    #[test]
    fn an_env_directory_resolves_to_its_interpreter() {
        // CLI-SPEC §2 accepts "interpreter or env dir"; a non-existent path is passed through so
        // the probe reports PD-ENV-001 against what the user actually typed.
        let missing = Path::new("definitely-not-here");
        assert_eq!(interpreter_in(missing), missing);
    }
}
