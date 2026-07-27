//! The uv adapter: `uv pip … --python <env-python>`.
//!
//! **SP-1 answered: GO.** uv has no JSON report — it prints a hard-wrapped text plan, and it
//! prints it to **stderr**, not stdout. That turned out not to be the weakness the roadmap feared:
//! on the output that matters most, an unsatisfiable set, uv names the exact blocking constraint
//! where pip says only that versions conflict.
//!
//! Two consequences shape this adapter:
//!
//! - `resolve` parses the plan whether or not uv exited zero, because its failure message is the
//!   most useful thing it produces.
//! - Every plan restates the installed set as explicit requirements. Without that, uv upgrades
//!   straight past an installed package's constraint and exits 0 — see [`super::plan_requirements`].
//!
//! `PD-ENG-003` remains the documented outcome when uv emits a shape the parser does not know,
//! which the weekly latest-engine job in CI exists to catch before users do.

use async_trait::async_trait;

use crate::errors::Result;
use crate::exec::Command;
use crate::model::{
    CheckReport, Dist, EngineId, EngineInfo, ExecMode, OutdatedDist, PinnedSpec, PkgName, PyEnv,
    StepResult,
};
use crate::plan::{PlanRequest, ResolutionReport};

use super::{Engine, EventSink};

/// Drives uv as a subprocess.
#[derive(Debug, Clone, Copy, Default)]
pub struct UvEngine;

impl UvEngine {
    /// argv for `uv pip list --format=json --python <py>`.
    #[must_use]
    pub fn argv_list(python: &str) -> Vec<String> {
        vec![
            "pip".into(),
            "list".into(),
            "--format=json".into(),
            "--python".into(),
            python.into(),
        ]
    }

    /// argv for `uv pip install -U --dry-run --python <py> [specs…]`.
    #[must_use]
    pub fn argv_dry_run(python: &str, specs: &[PinnedSpec]) -> Vec<String> {
        let mut argv = vec![
            "pip".into(),
            "install".into(),
            "-U".into(),
            "--dry-run".into(),
            "--python".into(),
            python.into(),
        ];
        argv.extend(specs.iter().map(PinnedSpec::to_requirement));
        argv
    }

    /// argv for `uv pip freeze --python <py>`. Note uv has no `--all`, so snapshots taken with
    /// the uv engine omit pip/setuptools — the snapshot metadata records the engine for this
    /// reason (DATA-FLOW §7).
    #[must_use]
    pub fn argv_freeze(python: &str) -> Vec<String> {
        vec![
            "pip".into(),
            "freeze".into(),
            "--python".into(),
            python.into(),
        ]
    }

    /// Parse uv's text dry-run plan into the normalized report.
    ///
    /// # Errors
    /// Returns `PD-ENG-003` when the output does not match any shape the adapter knows.
    pub fn parse_dry_run(_stdout: &str) -> Result<ResolutionReport> {
        todo!("SP-1: pin uv's text plan format against captured fixtures before parsing it")
    }
}

#[async_trait]
impl Engine for UvEngine {
    fn id(&self) -> EngineId {
        EngineId::Uv
    }

    async fn info(&self, _env: &PyEnv) -> EngineInfo {
        // uv is a standalone binary on PATH, not a module inside the environment, so this is the
        // one engine query that does not involve the interpreter.
        match Command::new("uv").arg("--version").run().await {
            Ok(o) if o.ok() => EngineInfo {
                id: EngineId::Uv,
                // "uv 0.11.32 (3010295ae 2026-07-23 x86_64-pc-windows-msvc)" -> "0.11.32"
                version: o.stdout.split_whitespace().nth(1).map(str::to_owned),
                available: true,
            },
            _ => EngineInfo {
                id: EngineId::Uv,
                version: None,
                available: false,
            },
        }
    }

    async fn list_installed(&self, env: &PyEnv) -> Result<Vec<Dist>> {
        let python = env.interpreter.display().to_string();
        let out = Command::new("uv")
            .args(Self::argv_list(&python))
            .run()
            .await?;
        if !out.ok() {
            return Err(out.into_error());
        }
        super::parse::list_json(&out.stdout)
    }

    async fn list_outdated(&self, env: &PyEnv) -> Result<Vec<OutdatedDist>> {
        let python = env.interpreter.display().to_string();
        let out = Command::new("uv")
            .args([
                "pip".to_owned(),
                "list".to_owned(),
                "--outdated".to_owned(),
                "--format=json".to_owned(),
                "--python".to_owned(),
                python,
            ])
            .run()
            .await?;
        if !out.ok() {
            return Err(out.into_error());
        }
        super::parse::outdated_json(&out.stdout)
    }

    async fn resolve(&self, env: &PyEnv, req: &PlanRequest) -> Result<ResolutionReport> {
        // SP-1: without the installed set restated, uv plans an upgrade that breaks installed
        // dependents and exits 0. Identical requirement to pip's adapter.
        let installed = self.list_installed(env).await?;
        let specs = super::plan_requirements(req, &installed);
        let python = env.interpreter.display().to_string();
        let out = Command::new("uv")
            .args(Self::argv_dry_run(&python, &specs))
            .run()
            .await?;
        // uv exits non-zero on an unsatisfiable set, but its message is the most useful thing it
        // produces (SP-1), so the plan is parsed either way and the parser decides.
        super::parse::uv_plan(&out.stdout, &out.stderr, &installed).map(|p| p.report)
    }

    async fn install(
        &self,
        _env: &PyEnv,
        _specs: &[PinnedSpec],
        _mode: ExecMode,
        _sink: EventSink,
    ) -> Result<StepResult> {
        todo!("M1: two-phase execution per ARCHITECTURE §8")
    }

    async fn uninstall(
        &self,
        _env: &PyEnv,
        _names: &[PkgName],
        _sink: EventSink,
    ) -> Result<StepResult> {
        todo!("M1: `uv pip uninstall`, sequential, skip-and-continue")
    }

    async fn check(&self, env: &PyEnv) -> Result<CheckReport> {
        let python = env.interpreter.display().to_string();
        let out = Command::new("uv")
            .args([
                "pip".to_owned(),
                "check".to_owned(),
                "--python".to_owned(),
                python,
            ])
            .run()
            .await?;
        // uv reports findings on stderr where pip uses stdout, so both are given to the parser.
        let combined = [out.stdout.as_str(), out.stderr.as_str()].join("\n");
        Ok(super::parse::check_text(&combined, out.ok()))
    }

    async fn upgrade_pip(&self, _env: &PyEnv) -> Result<StepResult> {
        // Documented behaviour, not a gap: pip upkeep is a pip-engine concern (DATA-FLOW §7).
        Err(crate::errors::PdError::new(
            crate::errors::Code::EngNotFound,
            "pip upkeep is not available while uv is the active engine",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PY: &str = r"C:\proj\.venv\Scripts\python.exe";

    #[test]
    fn dry_run_argv_targets_the_selected_interpreter() {
        let argv = UvEngine::argv_dry_run(PY, &[]);
        assert_eq!(argv, ["pip", "install", "-U", "--dry-run", "--python", PY]);
    }

    #[test]
    fn a_windows_path_stays_one_argv_entry() {
        // The path contains backslashes and could contain spaces; because it is its own argv
        // entry there is nothing to quote or escape (SECURITY §2).
        let argv = UvEngine::argv_freeze(r"C:\Program Files\Python312\python.exe");
        assert_eq!(argv.len(), 4);
        assert_eq!(argv[3], r"C:\Program Files\Python312\python.exe");
    }

    #[tokio::test]
    async fn upgrade_pip_is_refused_with_a_catalog_code() {
        let env = PyEnv {
            interpreter: PY.into(),
            prefix: r"C:\proj\.venv".into(),
            python_version: "3.12.4".into(),
            externally_managed: false,
            hidden_user_site: None,
            source: crate::model::EnvSource::VenvScan,
        };
        let err = UvEngine
            .upgrade_pip(&env)
            .await
            .expect_err("uv cannot upgrade pip");
        assert_eq!(err.code, crate::errors::Code::EngNotFound);
    }
}
