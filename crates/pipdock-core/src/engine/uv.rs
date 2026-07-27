//! The uv adapter: `uv pip … --python <env-python>`.
//!
//! **This adapter is gated on spike SP-1.** Unlike pip, uv has no stable JSON report for
//! `install --dry-run` — it prints a text plan. SP-1 must establish whether that text carries
//! enough information to populate [`ResolutionReport::held_back`] blockers; if it does not,
//! `docs/ROADMAP.md` says v1.0 ships pip-primary with uv behind a beta-engine flag.
//!
//! Until SP-1 lands, the parsing entry point below is deliberately unimplemented rather than
//! guessed at, and `PD-ENG-003` is the documented outcome when uv emits a shape the adapter does
//! not recognize.

use async_trait::async_trait;

use crate::errors::Result;
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
        todo!("M1: run `uv --version`")
    }

    async fn list_installed(&self, _env: &PyEnv) -> Result<Vec<Dist>> {
        todo!("M1: normalize uv's list output into Dist (shape differs slightly from pip)")
    }

    async fn list_outdated(&self, _env: &PyEnv) -> Result<Vec<OutdatedDist>> {
        todo!("M1 (SP-1 fixtures): parse uv's outdated output")
    }

    async fn resolve(&self, _env: &PyEnv, _req: &PlanRequest) -> Result<ResolutionReport> {
        todo!("SP-1 go/no-go gates this")
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

    async fn check(&self, _env: &PyEnv) -> Result<CheckReport> {
        todo!("M1: normalize `uv pip check` findings")
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
