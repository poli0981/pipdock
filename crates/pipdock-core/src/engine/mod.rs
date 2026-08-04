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
    StepResult, StepStatus,
};
use crate::plan::{PlanRequest, ResolutionReport};
use tokio_util::sync::CancellationToken;

/// Where live subprocess output goes.
///
/// The GUI forwards these to the `plan-progress` Tauri event feeding the console drawer
/// (ARCHITECTURE §7); the CLI writes them as NDJSON when `--json` is set (CLI-SPEC §6).
pub type EventSink = tokio::sync::mpsc::UnboundedSender<ProgressEvent>;

/// Everything one step needs from whoever is driving it.
///
/// Replaces a bare [`EventSink`] on the mutating trait methods, because an adapter needs three
/// things from its caller and passing them separately is how the first two got lost:
///
/// * where to send output — the sink;
/// * **which step this is, and how many there are.** `step` was previously hardcoded to `0` at
///   all four call sites, since an adapter has no way to know its own index. That made
///   UI-SPEC §3's per-package section markers and §8's "13 of 15 complete" live region
///   unimplementable — the data they need was never emitted;
/// * whether to stop — the cancellation token, which the adapter hands to [`crate::exec::Command`].
#[derive(Debug, Clone)]
pub struct ProgressSink {
    /// Where lines go.
    pub tx: EventSink,
    /// Zero-based index of this step within the plan.
    pub step: usize,
    /// How many steps the plan has, so a caller can render progress without counting.
    pub total: usize,
    /// Tripped by `plan_cancel` (ARCHITECTURE §7).
    pub cancel: CancellationToken,
}

impl ProgressSink {
    /// Announce that step `step` is starting.
    ///
    /// Emitted by the executor rather than the adapter: an adapter runs one command and cannot
    /// know whether it is step 3 of 15, which is the same reason `step` itself lives here.
    pub fn started(&self, pkg: Option<PkgName>, phase: ExecMode) {
        let _ = self.tx.send(ProgressEvent::StepStarted {
            step: self.step,
            total: self.total,
            pkg,
            phase,
        });
    }

    /// Announce that step `step` has finished, and how.
    pub fn finished(&self, pkg: Option<PkgName>, phase: ExecMode, status: StepStatus) {
        let _ = self.tx.send(ProgressEvent::StepFinished {
            step: self.step,
            total: self.total,
            pkg,
            phase,
            status,
        });
    }

    /// A sink for a plan of `total` steps, starting at step zero.
    #[must_use]
    pub fn new(tx: EventSink, total: usize, cancel: CancellationToken) -> Self {
        Self {
            tx,
            step: 0,
            total,
            cancel,
        }
    }

    /// The same sink, reporting as step `step`.
    #[must_use]
    pub fn at(&self, step: usize) -> Self {
        Self {
            step,
            ..self.clone()
        }
    }

    /// True once the plan has been cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

/// Which stream a line came from.
///
/// The console drawer renders them differently, and for uv the distinction is load-bearing in a
/// way it is not for pip: uv writes its **plan** to stderr (SP-1), so "stderr" here does not mean
/// "something went wrong".
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    /// The engine's stdout.
    Stdout,
    /// The engine's stderr.
    Stderr,
}

/// One event on the `plan-progress` channel (ARCHITECTURE §7).
///
/// Deferred from Stage 1 to the slice that could verify it. It was a bare line, which made two
/// documented features unimplementable: UI-SPEC §3's per-package section markers in the console
/// drawer had nothing to mark a section with, and §8's "13 of 15 complete" live region had no
/// event that meant "one finished". Neither can be recovered from the text — an engine's output
/// does not reliably say which package it is about, and counting lines is not counting steps.
///
/// A tagged lifecycle instead: every step emits exactly one [`Self::StepStarted`], any number of
/// [`Self::Line`]s, and exactly one [`Self::StepFinished`]. That makes the drawer's grouping and
/// the live region's counter both mechanical rather than inferred.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProgressEvent {
    /// A step is about to run. Opens a section in the console drawer.
    StepStarted {
        /// Zero-based index of the step within the plan.
        step: usize,
        /// How many steps the plan has, so the caller can render progress without counting.
        total: usize,
        /// The package this step is for, absent for a batch covering the whole set.
        #[serde(skip_serializing_if = "Option::is_none")]
        pkg: Option<PkgName>,
        /// Which execution phase is running.
        phase: ExecMode,
    },
    /// One line of the engine's output, verbatim and never localized (I18N §2).
    Line {
        /// Zero-based index of the step within the plan.
        step: usize,
        /// The package this line belongs to, absent for batch-wide output.
        #[serde(skip_serializing_if = "Option::is_none")]
        pkg: Option<PkgName>,
        /// Which execution phase is running.
        phase: ExecMode,
        /// Which stream produced it.
        stream: Stream,
        /// The line itself.
        line: String,
    },
    /// A step has finished. Closes its section, and advances the live region's counter.
    StepFinished {
        /// Zero-based index of the step within the plan.
        step: usize,
        /// How many steps the plan has.
        total: usize,
        /// The package this step was for.
        #[serde(skip_serializing_if = "Option::is_none")]
        pkg: Option<PkgName>,
        /// Which execution phase produced it.
        phase: ExecMode,
        /// How it ended.
        status: StepStatus,
    },
}

impl ProgressEvent {
    /// The text a plain consumer should show, if any.
    ///
    /// The CLI streams engine output to stderr and has no use for the markers; this keeps that a
    /// one-liner rather than a match at every call site.
    #[must_use]
    pub fn line(&self) -> Option<&str> {
        match self {
            Self::Line { line, .. } => Some(line),
            Self::StepStarted { .. } | Self::StepFinished { .. } => None,
        }
    }
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
        sink: ProgressSink,
    ) -> Result<StepResult>;

    /// Remove packages. Always sequential; the reverse-dependency guard runs once up front
    /// against the full removal set (ARCHITECTURE §8).
    async fn uninstall(
        &self,
        env: &PyEnv,
        names: &[PkgName],
        sink: ProgressSink,
    ) -> Result<StepResult>;

    /// `pip check` / `uv pip check`, normalized.
    async fn check(&self, env: &PyEnv) -> Result<CheckReport>;

    /// Capture the environment as a freeze document, for snapshots.
    ///
    /// The two engines do not capture the same thing: pip is invoked with `--all` so pip and
    /// setuptools are included, while uv has no such flag and omits them (DATA-FLOW §7). A
    /// snapshot therefore records which engine produced it, and a rollback must be planned
    /// against that same understanding — which is why [`crate::snapshot::Meta`] stores the engine.
    async fn freeze(&self, env: &PyEnv) -> Result<String>;

    /// Upgrade pip inside `env`. The uv adapter returns an `Unsupported` error — DATA-FLOW §7
    /// says pip upkeep is surfaced only when pip is the active engine or present in the env.
    async fn upgrade_pip(&self, env: &PyEnv) -> Result<StepResult>;
}

/// The adapter for `id`.
///
/// Both heads reach this point by different routes — the CLI from `--engine` or the stored
/// setting, the GUI from the stored setting alone — and both then need a `Box<dyn Engine>`. The
/// mapping itself lives here so neither can grow its own idea of what "uv" selects (G5). The CLI
/// previously owned the only copy, along with a second declaration of the settings key it reads.
#[must_use]
pub fn for_id(id: EngineId) -> Box<dyn Engine> {
    match id {
        EngineId::Pip => Box::new(pip::PipEngine),
        EngineId::Uv => Box::new(uv::UvEngine),
    }
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

/// The complete requirement list a dry-run resolve is given.
///
/// Three groups, and **all three are required**:
///
/// 1. the packages the user wants moved, as **bare names**, so `-U` is free to take them anywhere;
/// 2. the packages the user wants installed, with whatever specifier they gave;
/// 3. the guard set from [`plan_requirements`], pinned to current versions.
///
/// Omitting group 1 is not a small mistake: the engine is then asked to upgrade nothing, reports
/// no changes, and every package the user selected looks "held back" for no discoverable reason.
/// Omitting group 3 is the SP-1 failure — the resolver breaks installed dependents at exit 0.
#[must_use]
pub fn plan_argv_specs(req: &PlanRequest, installed: &[crate::model::Dist]) -> Vec<String> {
    let mut out: Vec<String> = req.upgrades.iter().map(ToString::to_string).collect();

    out.extend(req.installs.iter().map(|s| match &s.version_req {
        Some(v) if v.starts_with(['=', '<', '>', '!', '~']) => format!("{}{v}", s.name),
        Some(v) => format!("{}=={v}", s.name),
        None => s.name.to_string(),
    }));

    out.extend(
        plan_requirements(req, installed)
            .iter()
            .map(PinnedSpec::to_requirement),
    );
    out
}

/// The package a step is about, when a step covers exactly one.
///
/// Phase A installs the whole set at once, so its progress lines belong to no single package;
/// Phase B is per-package and its lines do. The console drawer uses this to draw section markers
/// (UI-SPEC §3).
#[must_use]
pub fn single_pkg(specs: &[PinnedSpec]) -> Option<PkgName> {
    match specs {
        [only] => Some(only.name.clone()),
        _ => None,
    }
}

/// Build the [`StepResult`] for a finished install.
///
/// Failure is recorded, **not raised**. ARCHITECTURE §8 and the owner requirement behind it: a
/// failed package must not stop the batch, so this returns an `Ok` carrying a `Failed` status and
/// the classified code. The only thing that aborts a run is a snapshot failure.
#[must_use]
pub fn step_result(specs: &[PinnedSpec], out: &crate::exec::Output) -> StepResult {
    use crate::model::StepStatus;

    let pkg = single_pkg(specs).unwrap_or_else(|| {
        // A batch step is reported against the first package purely so the row has a name; the
        // summary shows Phase A as one line regardless.
        specs.first().map_or_else(
            || PkgName::parse("batch").unwrap_or_else(|_| unreachable!("literal is valid")),
            |s| s.name.clone(),
        )
    });

    if out.ok() {
        StepResult {
            pkg,
            from: None,
            to: specs.first().map(|s| s.version.clone()),
            status: StepStatus::Ok,
            code: None,
            stderr_tail: None,
        }
    } else {
        let err = crate::errors::PdError::from_engine_stderr(&out.stderr);
        StepResult {
            pkg,
            from: None,
            to: specs.first().map(|s| s.version.clone()),
            status: StepStatus::Failed,
            code: Some(err.code),
            stderr_tail: err.stderr_tail,
        }
    }
}

/// Build the [`StepResult`] for a finished removal. Same skip-and-continue rule as installs.
#[must_use]
pub fn removal_result(names: &[PkgName], out: &crate::exec::Output) -> StepResult {
    use crate::model::StepStatus;

    let pkg = names.first().cloned().unwrap_or_else(|| {
        PkgName::parse("batch").unwrap_or_else(|_| unreachable!("literal is valid"))
    });

    if out.ok() {
        StepResult {
            pkg,
            from: None,
            to: None,
            status: StepStatus::Ok,
            code: None,
            stderr_tail: None,
        }
    } else {
        let err = crate::errors::PdError::from_engine_stderr(&out.stderr);
        StepResult {
            pkg,
            from: None,
            to: None,
            status: StepStatus::Failed,
            code: Some(err.code),
            stderr_tail: err.stderr_tail,
        }
    }
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
            size_bytes: None,
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
    fn the_resolve_command_asks_for_the_upgrades_as_well_as_the_guards() {
        // Regression: an earlier version passed only the guard set, so the engine was asked to
        // upgrade nothing. It reported no changes, and every selected package then looked "held
        // back" with no discoverable cause — the feature appeared to work and told users nothing.
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

        let argv = plan_argv_specs(&req, &installed);

        assert!(
            argv.contains(&"httpcore".to_owned()),
            "the package being upgraded must be asked for, unpinned: {argv:?}"
        );
        assert!(
            argv.contains(&"httpx==0.23.0".to_owned()),
            "guards still present"
        );
        assert!(argv.contains(&"h11==0.12.0".to_owned()));
        assert!(
            !argv.iter().any(|a| a.starts_with("httpcore==")),
            "the upgrade target must not also be pinned: {argv:?}"
        );
    }

    #[test]
    fn updating_everything_still_asks_for_something() {
        // With --all every package is moving, so the guard set is empty. If only guards were
        // passed the command would carry no requirements at all.
        let installed = [dist("a", "1.0"), dist("b", "2.0")];
        let req = PlanRequest {
            upgrades: vec![PkgName::parse("a").unwrap(), PkgName::parse("b").unwrap()],
            installs: Vec::new(),
            strategy: Strategy::Compatible,
        };

        assert!(
            plan_requirements(&req, &installed).is_empty(),
            "no guards, as expected"
        );
        assert_eq!(plan_argv_specs(&req, &installed), ["a", "b"]);
    }

    #[test]
    fn install_specifiers_are_passed_through_intact() {
        let req = PlanRequest {
            upgrades: Vec::new(),
            installs: vec![
                crate::model::Spec {
                    name: PkgName::parse("httpx").unwrap(),
                    version_req: Some("0.28.1".into()),
                },
                crate::model::Spec {
                    name: PkgName::parse("idna").unwrap(),
                    version_req: None,
                },
                crate::model::Spec {
                    name: PkgName::parse("certifi").unwrap(),
                    version_req: Some(">=2025".into()),
                },
            ],
            strategy: Strategy::Compatible,
        };
        let argv = plan_argv_specs(&req, &[]);
        assert!(
            argv.contains(&"httpx==0.28.1".to_owned()),
            "bare version becomes =="
        );
        assert!(argv.contains(&"idna".to_owned()));
        assert!(
            argv.contains(&"certifi>=2025".to_owned()),
            "an operator is kept as given"
        );
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
