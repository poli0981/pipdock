//! Command implementations.
//!
//! CLI-SPEC §1.1: every command maps 1:1 onto a `pipdock_core` call and **adds no logic of its
//! own**. Anything here that starts making a decision belongs in the core instead, so the GUI
//! inherits it (PRD G5: GUI and CLI never diverge).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use pipdock_core::engine::{
    self, CancellationToken, Engine, ProgressEvent, pip::PipEngine, uv::UvEngine,
};
use pipdock_core::envs::{self, Candidate};
use pipdock_core::errors::{Code, PdError, Result};
use pipdock_core::flow::{
    FlowStep, GuardAck, NothingReason, RollbackFlow, SnapshotPolicy, UninstallFlow, UpdateFlow,
};
use pipdock_core::model::{EngineId, EnvSource, PkgName, PyEnv, StepStatus};

/// Re-exported so `main.rs` keeps referring to `run::Intent`. It lives in the core now, because
/// translating what the user asked for into a `PlanRequest` is flow logic the GUI needs too.
pub use pipdock_core::flow::Intent;
use pipdock_core::store::Store;
use pipdock_core::{health, index, pins, plan, report, settings, snapshot};

use crate::{EngineArg, Exit, GlobalOpts, ToolArg};

/// Resolve which engine to drive.
///
/// ARCHITECTURE §3: first run probes `uv --version` on PATH and preselects uv when present. The
/// `--engine` flag overrides for one invocation.
#[must_use]
pub fn engine_for(opts: &GlobalOpts) -> Box<dyn Engine> {
    let id = match opts.engine {
        Some(EngineArg::Uv) => EngineId::Uv,
        Some(EngineArg::Pip) => EngineId::Pip,
        // No flag: use what the user configured. Falling back to pip rather than probing for uv
        // here keeps a read-only command from silently changing behaviour based on PATH; first-run
        // uv detection belongs to Settings, where the choice is shown and can be changed.
        //
        // A store that cannot be opened is not worth failing a command over, and `settings::load`
        // already answers pip for a missing or unrecognized value — so one `map_or` covers every
        // way this can go wrong.
        None => Store::open(&app_data_dir())
            .and_then(|store| settings::load(&store))
            .map_or(EngineId::Pip, |s| s.engine),
    };
    engine::for_id(id)
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
        Ok(env) => engine::for_id(id).info(&env).await,
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

    // Through `settings::save`, not a raw kv write: the GUI's Settings screen reads this back with
    // `settings::load`, and a key written by one head that the other cannot find is the failure
    // mode `core::settings` exists to prevent.
    let store = Store::open(&app_data_dir())?;
    let mut settings = settings::load(&store)?;
    settings.engine = id;
    settings::save(&store, &settings)?;
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
                    // Same field the GUI's EnvRow carries, out of the same list, so `env list`
                    // and the Environments screen cannot disagree about which pip is installed.
                    "pipVersion": p.dists.iter()
                        .find(|d| d.name.as_str() == "pip")
                        .map(|d| d.version.0.clone()),
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

    // Whether Code Health can run at all. Deliberately **not** part of the exit rule below: Health
    // is optional, and a fresh install exiting 1 because it has not built a tools venv yet would
    // make `doctor` useless as a health check for everything else.
    let tools = health::tools_dir(&app_data_dir());
    let tools_need = health::needs_sync(&tools, health::HEALTH_TOOLS);

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
                "toolsVenv": {
                    "path": tools,
                    "need": tools_need.as_ref().ok(),
                    "manifest": health::read_manifest(&tools),
                },
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
        match &tools_need {
            Ok(need) => {
                println!("code health : {}", describe_need(need));
                if need.is_needed() {
                    println!("              run `pipdock tools sync`");
                }
            }
            Err(e) => println!("code health : could not tell ({})", e.code),
        }
    }

    // A doctor that found real problems should say so in its exit code, or scripts cannot use it.
    Ok(if pip_ok && check_ok {
        Exit::Success
    } else {
        Exit::PartialFailure
    })
}

/// The app data root, `%LOCALAPPDATA%\PipDock\data` (ARCHITECTURE §6).
///
/// Re-exported rather than computed here. The CLI carried a byte-for-byte copy of the core's
/// version, which is the same failure `KEY_ENGINE` was: two functions deriving one path are one
/// edit away from the two heads reading different directories, and the pins and snapshots the
/// user was relying on silently not being there.
pub use pipdock_core::store::default_app_data as app_data_dir;

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

    // Read before the flow starts: `UpdateFlow` takes the pins rather than the store they came
    // from, so its future stays `Send` for the GUI's sake.
    let env_pins = pins::list(&store, &envs::env_hash(&env.interpreter))?;
    let (mut flow, mut step) = UpdateFlow::start(env, engine_for(opts), &intent, &env_pins).await?;

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
            // The lifecycle markers carry no text; the CLI streams engine output only.
            if let Some(line) = event.line().filter(|_| !quiet) {
                eprintln!("{line}");
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

/// One blocker, as a sentence.
///
/// Assembled here because this head is English-only (PRD P0-14) and `plan::Blocker` carries data
/// rather than phrasing (hard invariant 4). The GUI builds the same sentence from `plan.blocker`
/// in its own catalogs; the shape mirrors the guard's, which composes `BrokenDependent` the same
/// way a few hundred lines below.
///
/// No culprit means no culprit: ARCHITECTURE §3 says show the constraint rather than guess.
fn blocker_line(b: &plan::Blocker) -> String {
    match (&b.by, &b.version) {
        (Some(by), Some(v)) => format!("{by} {v} requires {}", b.constraint),
        (Some(by), None) => format!("{by} requires {}", b.constraint),
        (None, _) => b.constraint.clone(),
    }
}

/// The per-package prompt from CLI-SPEC §4.
fn prompt_decision(held: &plan::HeldBack) -> plan::Decision {
    use std::io::Write as _;

    println!(
        "\n{}  held back at {} (latest {})",
        held.pkg, held.resolved, held.latest
    );
    for blocker in &held.blockers {
        println!("  — {}", blocker_line(blocker));
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
                println!("      {}", blocker_line(b));
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

/// `pipdock tools sync`
///
/// The re-sync `PD-HLT-001`'s shipped copy already tells users to run — until now there was no way
/// to do it. Not `--env`-scoped: the tools venv is PipDock's own, one per installation, and never
/// the user's environment (CODE-HEALTH-SPEC §1).
///
/// # Errors
/// As `health::sync_tools_venv`. `PD-ENV-001` when no Python 3.10+ is discoverable.
pub async fn tools_sync(opts: &GlobalOpts, force: bool, python: Option<&Path>) -> Result<Exit> {
    let dir = health::tools_dir(&app_data_dir());

    if !force {
        let need = health::needs_sync(&dir, health::HEALTH_TOOLS)?;
        if !need.is_needed() {
            if opts.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&need).unwrap_or_default()
                );
            } else {
                println!("tools environment is current ({})", dir.display());
            }
            return Ok(Exit::Success);
        }
        if !opts.quiet && !opts.json {
            println!("{}", describe_need(&need));
        }
    }

    let base = match python {
        Some(path) => path.to_path_buf(),
        None => {
            // One sweep, driven here rather than hidden inside the sync — `scan` spawns four
            // subprocesses and the caller owns the "newest >= 3.10" policy.
            let (path, version) = health::choose_tools_python(&envs::scan().await).await?;
            if !opts.quiet && !opts.json {
                println!("building on Python {version} ({})", path.display());
            }
            path
        }
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();
    let quiet = opts.quiet;
    let pump = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            // The lifecycle markers carry no text; the CLI streams tool output only.
            if let Some(line) = event.line().filter(|_| !quiet) {
                eprintln!("{line}");
            }
        }
    });

    // A fresh token nothing trips, matching every other CLI path: the CLI installs no Ctrl-C
    // handler, and the flows build their own tokens for the GUI's `plan_cancel` to reach. Killing
    // the process still tears the tree down — `exec::TreeGuard`'s job object closes with the last
    // handle — which is verified rather than assumed: a killed sync leaves no orphan pip.
    let sink = engine::ProgressSink::new(
        tx,
        health::sync_steps(health::HEALTH_TOOLS),
        CancellationToken::new(),
    );
    let manifest = health::sync_tools_venv(&dir, &base, health::HEALTH_TOOLS, &sink).await;
    drop(sink);
    let _ = pump.await;
    let manifest = manifest?;

    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest).unwrap_or_default()
        );
    } else if !opts.quiet {
        println!("tools environment synced at {}", dir.display());
        for (tool, version) in &manifest.tools {
            println!("  {tool} {version}");
        }
    }
    Ok(Exit::Success)
}

/// `pipdock health`
///
/// Exits **1 when any tool reported a finding**, 0 when all of them were clean. A linter that
/// exits 0 on findings is useless in a pre-commit hook, and `doctor` already returns
/// `PartialFailure` for "found real problems" — CLI-SPEC §5's description of code 1 is amended to
/// cover it. Failures still map through `exit_for`, so a tools venv that will not build is
/// distinguishable from a project with lint.
///
/// # Errors
/// `PD-ENV-003` when the project folder cannot be read, plus whatever the implicit tools-venv sync
/// raises. A single tool failing is **not** an error: it lands in `problems` and the rest report.
pub async fn health(
    opts: &GlobalOpts,
    path: Option<&Path>,
    tools: &[ToolArg],
    fix: bool,
) -> Result<Exit> {
    if fix && !tools.is_empty() && !tools.iter().any(|t| matches!(t, ToolArg::Ruff)) {
        // ruff is the only tool with a write path (CODE-HEALTH-SPEC §1). Asking to fix while
        // excluding it is a mistake worth naming rather than a no-op worth performing.
        eprintln!("error[PD-PKG-002]: `--fix` needs ruff; it is the only tool that writes");
        return Ok(Exit::PlanAborted);
    }

    let env = select_env(opts).await?;
    let env_hash = envs::env_hash(&env.interpreter);
    let app_data = app_data_dir();
    let store = Store::open(&app_data)?;

    // CLI-SPEC §3's `--path` default: what was used here last, else the working directory. An
    // error naming the flag beats silently scanning wherever the shell happened to be.
    let project = match path {
        Some(p) => p.to_path_buf(),
        None => match store.health_project(&env_hash)? {
            Some(folder) => PathBuf::from(folder),
            None => std::env::current_dir().map_err(|e| {
                PdError::new(
                    Code::EnvProbeFailed,
                    format!("no project folder: pass --path ({e})"),
                )
            })?,
        },
    };

    let tools_dir = health::tools_dir(&app_data);
    let sync_needed = health::needs_sync(&tools_dir, health::HEALTH_TOOLS)?.is_needed();
    let run_opts = health::RunOptions {
        tools: tools.iter().map(|t| t.as_str().to_owned()).collect(),
        ..health::RunOptions::default()
    };

    // The total is decided **before** the first event. A progress bar cannot learn its own total
    // halfway, and whether a sync is owed is exactly the thing that changes it.
    let total = health::run_steps(&run_opts)
        + if sync_needed {
            health::sync_steps(health::HEALTH_TOOLS)
        } else {
            0
        };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();
    let quiet = opts.quiet;
    let pump = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let Some(line) = event.line().filter(|_| !quiet) {
                eprintln!("{line}");
            }
        }
    });
    let sink = engine::ProgressSink::new(tx, total, CancellationToken::new());

    let outcome = async {
        if sync_needed {
            if !opts.quiet && !opts.json {
                println!("building the Code Health tools environment (first run)…");
            }
            let (python, _) = health::choose_tools_python(&envs::scan().await).await?;
            health::sync_tools_venv(&tools_dir, &python, health::HEALTH_TOOLS, &sink).await?;
        }
        health::run_tools(
            &tools_dir,
            &project,
            &env,
            &run_opts,
            &sink.at(if sync_needed {
                health::sync_steps(health::HEALTH_TOOLS)
            } else {
                0
            }),
        )
        .await
    }
    .await;

    drop(sink);
    let _ = pump.await;
    let report = outcome?;

    // Remembered on every run, not only successful ones: the question it answers is "where did we
    // last do this", and a run that found problems still happened here.
    store.set_health_project(
        &env_hash,
        &project.display().to_string(),
        &jiff::Timestamp::now().to_string(),
    )?;

    // With `--fix`, `--json` emits the **`FixReport` alone**: the pre-fix report describes a state
    // that no longer exists by the time the command returns, and printing both put two documents
    // on stdout for a contract CLI-SPEC §6 states as one. The human form still prints both,
    // because there the report is what makes the fix summary legible.
    if !(opts.json && fix) {
        print_health(opts, &report);
    }
    if fix {
        return apply_fix(opts, &tools_dir, &project, &report).await;
    }
    Ok(if health::has_findings(&report) {
        Exit::PartialFailure
    } else {
        Exit::Success
    })
}

/// The CLI half of the write path — CLI-SPEC §3's `pipdock health --fix`.
///
/// Prompts the way the dialog does and refuses in the same places, because CLI-SPEC §1.1 says the
/// two heads share behaviour and this is the one operation neither of them can undo.
///
/// # The `--yes` decision, which is the sharpest one in the slice
///
/// `--yes` means "assume defaults", and **the default over uncommitted work is don't**. A clean
/// tree proceeds: one `git checkout .` puts it back, so an unattended fix there is recoverable,
/// and refusing would make `--fix` useless in the pre-commit hook the exit-code rule was written
/// for. A dirty tree — or no repository at all — refuses with exit 2, because those are the cases
/// where a script could destroy work with no way back.
///
/// The escape hatch is deliberately **not a flag**. It is `git commit` or `git stash`, which is the
/// actual remedy and leaves the user something to return to; a `--force-fix` would exist only to be
/// pasted into a CI file once and never reconsidered. CLI-SPEC §4 sketches a "second `-y`" idiom
/// for the resolve countdown, but `--yes` is a bool here, and inventing flag machinery inside the
/// slice that introduces source-tree writes is the wrong trade.
///
/// # Errors
/// Whatever the fix raises. `PD-PRM-003` arrives **before anything is written**.
async fn apply_fix(
    opts: &GlobalOpts,
    tools_dir: &Path,
    project: &Path,
    report: &health::HealthReport,
) -> Result<Exit> {
    use pipdock_core::health::fix;

    if report.ruff.fixable == 0 {
        if !opts.json {
            println!("\nnothing to fix: no finding carries a safe fix");
        }
        return Ok(if health::has_findings(report) {
            Exit::PartialFailure
        } else {
            Exit::Success
        });
    }

    let dirty = fix::dirty(project).await;
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());

    let approved = if opts.yes {
        if let Some(tree) = dirty {
            eprintln!(
                "error[PD-RES-002]: {} uncommitted change(s) here; --yes will not rewrite files \
                 you have not committed. Commit or stash first.",
                tree.entries
            );
            return Ok(Exit::PlanAborted);
        }
        true
    } else if interactive {
        if let Some(tree) = dirty {
            println!(
                "\n{} uncommitted change(s) in this repository. PipDock cannot undo a fix, and \
                 `git checkout .` would take your own edits with it.",
                tree.entries
            );
        }
        confirm_text(&format!(
            "Fix {} issue(s) in {} file(s)? This rewrites your source files",
            report.ruff.fixable, report.ruff.fixable_files
        ))
    } else {
        // A prompt with no terminal is not a prompt. Said here as well as in `confirm_text` so
        // this path names `--fix`'s own rule rather than the generic one.
        eprintln!("error[PD-RES-002]: not a terminal; pass --yes to fix a committed tree");
        return Ok(Exit::PlanAborted);
    };

    if !approved {
        if !opts.json {
            println!("nothing was changed");
        }
        return Ok(Exit::PlanAborted);
    }

    // No re-read here, unlike the GUI: the counts the prompt named came from the report this same
    // process produced seconds ago, so there is no window between confirming and fixing for the
    // tree to drift in. The consent is still built from what was actually observed.
    let consent = fix::consent_ok(report.ruff.fixable_files, dirty.is_some(), dirty)?;
    let fixed = fix::apply(tools_dir, project, &report.ruff, consent).await?;

    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&fixed).unwrap_or_default()
        );
    } else {
        println!(
            "\nfixed       : {} file(s); {} finding(s) remain",
            fixed.files_changed,
            fixed.remaining.findings.len()
        );
        if fixed.not_applied > 0 {
            println!(
                "warning     : {} safe fix(es) were not applied — check file permissions",
                fixed.not_applied
            );
        }
    }

    // Exit 0 only when the whole report is clean afterwards, so `pipdock health --fix && …` means
    // something. deptry and vulture are untouched by a ruff fix, so their findings still count.
    let anything_left = !report.deptry.is_empty()
        || !report.vulture.is_empty()
        || !fixed.remaining.findings.is_empty();
    Ok(if anything_left {
        Exit::PartialFailure
    } else {
        Exit::Success
    })
}

/// Print a health report, as JSON or as the human summary.
fn print_health(opts: &GlobalOpts, report: &health::HealthReport) {
    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(report).unwrap_or_default()
        );
        return;
    }

    println!("project     : {}", report.project);
    match &report.declared {
        health::DeclaredSource::Pyproject => println!("declared in : pyproject.toml"),
        health::DeclaredSource::Requirements { files } => {
            println!("declared in : {}", files.join(", "));
        }
        health::DeclaredSource::None => {
            // §3's limited-mode notice. deptry still runs; its findings just mean less.
            println!("declared in : nothing found — deptry's results are limited");
        }
    }

    for issue in &report.deptry {
        println!("{} {} — {}", issue.code, issue.dep, issue.message);
        for loc in &issue.locations {
            match loc.line {
                Some(line) => println!("            {}:{line}", loc.file),
                None => println!("            {}", loc.file),
            }
        }
    }
    for finding in &report.vulture {
        println!(
            "{}:{} {} ({}%)",
            finding.path, finding.line, finding.message, finding.confidence
        );
    }
    for finding in &report.ruff.findings {
        println!(
            "{}:{}:{} {} {}",
            finding.filename,
            finding.row,
            finding.column,
            finding.code.as_deref().unwrap_or("-"),
            finding.message
        );
    }

    println!(
        "\n{} dependency, {} dead-code, {} lint ({} safely fixable in {} file(s))",
        report.deptry.len(),
        report.vulture.len(),
        report.ruff.findings.len(),
        report.ruff.fixable,
        report.ruff.fixable_files
    );
    // The limitation, said out loud rather than left in a doc comment: deptry compares against
    // PipDock's tools environment because it has no way to be told about another one.
    if !report.deptry.is_empty() {
        println!(
            "note        : deptry compares imports against PipDock's own tools environment, \
             not this one — DEP001 and DEP003 can be swapped for packages it happens to hold"
        );
    }
    for problem in &report.problems {
        eprintln!("error[{}]: {}", problem.code, problem.message);
    }
}

/// `pipdock tools status`
///
/// # Errors
/// `PD-PKG-002` when the shipped pin ledger is malformed — a build-time mistake.
pub async fn tools_status(opts: &GlobalOpts) -> Result<Exit> {
    let dir = health::tools_dir(&app_data_dir());
    let need = health::needs_sync(&dir, health::HEALTH_TOOLS)?;
    let manifest = health::read_manifest(&dir);

    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "path": dir,
                "need": need,
                "manifest": manifest,
            }))
            .unwrap_or_default()
        );
        return Ok(Exit::Success);
    }

    println!("tools environment: {}", dir.display());
    println!("{}", describe_need(&need));
    if let Some(m) = manifest {
        println!("python {} ({})", m.python_version, m.python);
        println!("synced {}", m.synced_at);
        for (tool, version) in &m.tools {
            println!("  {tool} {version}");
        }
    }
    Ok(Exit::Success)
}

/// One line saying what the tools environment needs, and why.
///
/// A sentence per variant rather than the debug form: `SyncNeed` exists so `doctor` and the Health
/// screen can *say* why, and a `PinsChanged` that does not name both hashes is no more useful than
/// a bool.
fn describe_need(need: &health::SyncNeed) -> String {
    match need {
        health::SyncNeed::Fresh => "up to date with the pins shipped in this build".to_owned(),
        health::SyncNeed::NeverSynced => "not built yet".to_owned(),
        health::SyncNeed::PinsChanged { from, to } => {
            format!(
                "pins changed: {} -> {}",
                &from[..8.min(from.len())],
                &to[..8.min(to.len())]
            )
        }
        health::SyncNeed::InterpreterGone => "its interpreter is gone".to_owned(),
        health::SyncNeed::ToolMissing { tool } => format!("{tool} is missing"),
    }
}

/// `pipdock pip-upgrade`
///
/// **`PipEngine`, not `engine_for(opts)`.** Upgrading pip is a pip operation by definition — there
/// is no uv way to do it, which is why `UvEngine::upgrade_pip` refuses. Dispatching on the
/// configured engine meant `--engine uv` failed at `PD-ENG-001` for a preference about the *user's*
/// environments, on the one command that has nothing to do with resolving anything; the line below
/// was already reaching past the abstraction to read the version. Same reasoning as the tools venv
/// (CODE-HEALTH-SPEC §2 as amended by P2): pip is present wherever Python is.
///
/// # Errors
/// `PD-ENV-002` on a PEP 668 environment; whatever `classify_stderr` makes of a failed install.
pub async fn pip_upgrade(opts: &GlobalOpts) -> Result<Exit> {
    let env = select_env(opts).await?;

    let before = PipEngine.info(&env).await;
    PipEngine.upgrade_pip(&env).await?;
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
            // The lifecycle markers carry no text; the CLI streams engine output only.
            if let Some(line) = event.line().filter(|_| !quiet) {
                eprintln!("{line}");
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

    let engine_version = match &env {
        Some(e) => engine.info(e).await.version,
        None => None,
    };
    // Built by `pipdock_core::report`, which the GUI's `report_bug_url` also calls. Two builders
    // means two URLs that agree until the template gains a field — and this is the one nobody
    // would notice had drifted.
    let url = report::bug_report_url(
        &report::BugReport {
            python: env.as_ref().map(|e| e.python_version.clone()),
            engine: Some(engine.id()),
            engine_version,
            code: None,
            // The CLI has no ring buffer; M3's logging subsystem is what fills this in.
            log: String::new(),
        },
        &os_description(),
    );

    println!("{url}");
    if !opts.quiet {
        // Concatenated rather than a multi-line literal: a bare string literal keeps the source
        // indentation, which shipped this line with fourteen stray spaces in the middle of a
        // sentence. Found by reading the actual output while writing docs/CLI-GUIDE.md.
        eprintln!(
            "\nOpen that in a browser to review the prefilled issue. \
             Nothing is sent until you submit it yourself.\n\
             Check the log excerpt for paths or names you would rather not make public."
        );
    }
    Ok(Exit::Success)
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
    if !opts.quiet {
        eprintln!("fetching {} …", index::SIMPLE_INDEX_URL);
    }

    let report = index::refresh(&app_data_dir(), jiff::Timestamp::now()).await?;

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
    let name = PkgName::parse(pkg)?;
    let (meta, freshness) = index::metadata(&app_data_dir(), &name, jiff::Timestamp::now()).await?;

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
                // Names alone say what breaks; the specifier says whether the user can live with
                // it. `pandas 2.1.4 (<2,>=1.26.0)` is checkable against what they know.
                let list = broken
                    .iter()
                    .map(|b| {
                        let named = match &b.version {
                            Some(v) => format!("{} {v}", b.pkg),
                            None => b.pkg.to_string(),
                        };
                        if b.constraint.is_empty() {
                            named
                        } else {
                            format!("{named} ({})", b.constraint)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("removing {pkg} breaks {list}");
            }
        }
        if !force {
            eprintln!(
                "error[{}]: refusing to remove packages other packages depend on.\n\
                 Re-run with --force to proceed anyway, or add the dependents to the removal set:\n\
                   pipdock uninstall {}",
                Code::ResGuardTrip,
                report
                    .with_dependents
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            // Exit 2, not 1. CLI-SPEC §5 defines 1 as "completed with per-package failures (see
            // JSON `counts.failed`)" — but a guard trip executes nothing, so there are no counts
            // to read and a script following the table finds an empty summary. 2 is "plan
            // aborted", which is exactly what happened. It is also what `exit_for` derives from
            // `Area::Res`, so the binary no longer disagrees with its own mapping.
            return Ok(Exit::PlanAborted);
        }
        eprintln!("warning: --force given; proceeding despite the breakage above");
    }

    // DATA-FLOW §9.2 applies to removals too: nothing is touched before a snapshot exists.
    // `--no-snapshot` was parsed and then ignored on this path, so the same flag meant "waive" for
    // an update and nothing at all for a removal — the one operation with no way back.
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
        println!("snapshot {} written before removing", meta.id);
    }

    // Engine output streams to stderr as it happens, which is the CLI's equivalent of the GUI's
    // console drawer: a long removal that prints nothing looks hung.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();
    let quiet = opts.quiet;
    let pump = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            // The lifecycle markers carry no text; the CLI streams engine output only.
            if let Some(line) = event.line().filter(|_| !quiet) {
                eprintln!("{line}");
            }
        }
    });

    // Reaching here means either the guard was clear or `--force` was given and the warning
    // printed; `ack_ok` refuses anything else with PD-RES-004 rather than trusting this call site.
    let ack = if force {
        GuardAck::ForcedDespiteBreakage
    } else {
        GuardAck::Clear
    };
    let summary = flow.execute(ack, tx).await?;
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
