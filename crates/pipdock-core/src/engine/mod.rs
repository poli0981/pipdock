//! The `Engine` trait and its two adapters.
//!
//! ARCHITECTURE §1.2: **explain, don't reimplement.** Resolution is always performed by the
//! selected engine in dry-run mode; this module parses and normalizes, and never computes version
//! resolution itself.
//!
//! ARCHITECTURE §1.5: engines are invoked with argv arrays via `tokio::process::Command`,
//! **never through a shell**.

pub mod parse;
pub mod pip;
pub mod uv;

use async_trait::async_trait;

use crate::errors::Result;
use crate::model::{
    CheckReport, Dist, EngineId, EngineInfo, ExecMode, OutdatedDist, PinnedSpec, PkgName, PyEnv,
    StepResult,
};
use crate::plan::{PlanRequest, ResolutionReport};

/// Where live subprocess output goes.
///
/// The GUI forwards these to the `plan-progress` Tauri event feeding the console drawer
/// (ARCHITECTURE §7); the CLI writes them as NDJSON when `--json` is set (CLI-SPEC §6).
pub type EventSink = tokio::sync::mpsc::UnboundedSender<ProgressEvent>;

/// A line of progress from a running engine command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProgressEvent {
    /// Zero-based index of the step within the plan.
    pub step: usize,
    /// The package this line belongs to, absent for batch-wide output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pkg: Option<PkgName>,
    /// Which execution phase is running.
    pub phase: ExecMode,
    /// One line of the engine's stdout or stderr, verbatim and never localized.
    pub line: String,
}

/// The contract both adapters implement. See ARCHITECTURE §3.
#[async_trait]
pub trait Engine: Send + Sync {
    /// Which engine this is.
    fn id(&self) -> EngineId;

    /// Version and availability for `env`.
    async fn info(&self, env: &PyEnv) -> EngineInfo;

    /// Everything installed in `env`.
    async fn list_installed(&self, env: &PyEnv) -> Result<Vec<Dist>>;

    /// Installed packages with a newer release available.
    async fn list_outdated(&self, env: &PyEnv) -> Result<Vec<OutdatedDist>>;

    /// Dry-run resolve `req` against `env`, normalized into a [`ResolutionReport`].
    ///
    /// This is the only way a plan may be produced; DATA-FLOW §9.1 forbids any mutating call
    /// without a report accepted in the same session.
    async fn resolve(&self, env: &PyEnv, req: &PlanRequest) -> Result<ResolutionReport>;

    /// Install an exact pinned set.
    ///
    /// `Batch` mode passes the whole set in one invocation (Phase A); `Isolated` mode is the
    /// per-package retry loop (Phase B), where a failure must **not** stop the caller's loop.
    async fn install(
        &self,
        env: &PyEnv,
        specs: &[PinnedSpec],
        mode: ExecMode,
        sink: EventSink,
    ) -> Result<StepResult>;

    /// Remove packages. Always sequential; the reverse-dependency guard runs once up front
    /// against the full removal set (ARCHITECTURE §8).
    async fn uninstall(
        &self,
        env: &PyEnv,
        names: &[PkgName],
        sink: EventSink,
    ) -> Result<StepResult>;

    /// `pip check` / `uv pip check`, normalized.
    async fn check(&self, env: &PyEnv) -> Result<CheckReport>;

    /// Upgrade pip inside `env`. The uv adapter returns an `Unsupported` error — DATA-FLOW §7
    /// says pip upkeep is surfaced only when pip is the active engine or present in the env.
    async fn upgrade_pip(&self, env: &PyEnv) -> Result<StepResult>;
}

/// pip's floor for `install --dry-run --report -`. Below this, planning is impossible and the app
/// offers a one-click pip upgrade (`PD-ENG-002`, DATA-FLOW §7).
pub const PIP_MIN_VERSION_FOR_REPORT: (u32, u32) = (22, 2);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_pip_floor() {
        // DATA-FLOW §7 and ERROR-CATALOG PD-ENG-002 both name 22.2; keep them in step.
        assert_eq!(PIP_MIN_VERSION_FOR_REPORT, (22, 2));
    }
}
