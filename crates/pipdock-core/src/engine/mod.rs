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

/// uv's floor, pinned by spike SP-1: 0.10.12 and 0.11.32 produce identical plan formats.
pub const UV_MIN_VERSION: (u32, u32) = (0, 10);

/// Build the extra requirements a dry-run resolve must be given alongside the user's own.
///
/// **This is the single most important consequence of spike SP-1.** `<engine> install -U <pkg>`
/// ignores the constraints of packages already installed: seeded with `httpx 0.23.0`, which
/// requires `httpcore<0.16`, both pip and uv plan `httpcore 1.0.9` and exit 0, silently breaking
/// it. Restating the installed set as explicit `name==version` requirements is what makes the
/// resolver hold packages back instead.
///
/// Returned here is the guard group: **everything installed that the user is not moving**, pinned
/// to its current version. Packages the user chose to upgrade or install are deliberately absent
/// so they remain free to move.
///
/// Without this, PipDock would preview a plan that breaks the environment — the exact failure the
/// product exists to prevent.
#[must_use]
pub fn plan_requirements(req: &PlanRequest, installed: &[crate::model::Dist]) -> Vec<PinnedSpec> {
    use std::collections::BTreeSet;

    let moving: BTreeSet<&PkgName> = req
        .upgrades
        .iter()
        .chain(req.installs.iter().map(|s| &s.name))
        .collect();

    installed
        .iter()
        .filter(|d| !moving.contains(&d.name))
        .map(|d| PinnedSpec {
            name: d.name.clone(),
            version: d.version.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Dist, Version};
    use crate::plan::Strategy;

    fn dist(name: &str, version: &str) -> Dist {
        Dist {
            name: PkgName::parse(name).unwrap(),
            version: Version(version.into()),
            requires_dist: Vec::new(),
            requires_python: None,
        }
    }

    #[test]
    fn documented_pip_floor() {
        // DATA-FLOW §7 and ERROR-CATALOG PD-ENG-002 both name 22.2; keep them in step.
        assert_eq!(PIP_MIN_VERSION_FOR_REPORT, (22, 2));
        // SP-1 verified two uv minors produce identical plan text.
        assert_eq!(UV_MIN_VERSION, (0, 10));
    }

    #[test]
    fn the_installed_set_is_restated_so_the_resolver_cannot_break_it() {
        // The SP-1 scenario: upgrading httpcore while httpx 0.23.0 is installed. httpx must be
        // pinned in the request, or both engines plan httpcore 1.0.9 and break it at exit 0.
        let installed = [
            dist("httpx", "0.23.0"),
            dist("httpcore", "0.15.0"),
            dist("h11", "0.12.0"),
        ];
        let req = PlanRequest {
            upgrades: vec![PkgName::parse("httpcore").unwrap()],
            installs: Vec::new(),
            strategy: Strategy::Compatible,
        };

        let guards = plan_requirements(&req, &installed);
        let rendered: Vec<String> = guards.iter().map(PinnedSpec::to_requirement).collect();

        assert!(
            rendered.contains(&"httpx==0.23.0".to_owned()),
            "httpx must be pinned"
        );
        assert!(rendered.contains(&"h11==0.12.0".to_owned()));
        assert!(
            !rendered.iter().any(|r| r.starts_with("httpcore==")),
            "the package being upgraded must stay free to move"
        );
    }

    #[test]
    fn a_package_being_installed_is_not_pinned_to_its_old_version() {
        let installed = [dist("idna", "3.4")];
        let req = PlanRequest {
            upgrades: Vec::new(),
            installs: vec![crate::model::Spec {
                name: PkgName::parse("idna").unwrap(),
                version_req: None,
            }],
            strategy: Strategy::Compatible,
        };
        assert!(plan_requirements(&req, &installed).is_empty());
    }

    #[test]
    fn an_empty_environment_needs_no_guards() {
        let req = PlanRequest {
            upgrades: Vec::new(),
            installs: Vec::new(),
            strategy: Strategy::Compatible,
        };
        assert!(plan_requirements(&req, &[]).is_empty());
    }
}
