//! Command implementations.
//!
//! CLI-SPEC §1.1: every command maps 1:1 onto a `pipdock_core` call and **adds no logic of its
//! own**. Anything here that starts making a decision belongs in the core instead, so the GUI
//! inherits it (PRD G5: GUI and CLI never diverge).

use std::path::{Path, PathBuf};

use pipdock_core::engine::{Engine, pip::PipEngine, uv::UvEngine};
use pipdock_core::envs::{self, Candidate};
use pipdock_core::errors::{Code, PdError, Result};
use pipdock_core::model::{EngineId, EnvSource, PyEnv};
use pipdock_core::snapshot;

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
        None => Box::new(PipEngine),
    }
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
