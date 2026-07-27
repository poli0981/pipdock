//! Planning: `PlanRequest` → `ResolutionReport` → `ExecutionSummary`.
//!
//! See `docs/DATA-FLOW.md` §3 for the state machine and §9 for the invariants this module owns.

use crate::model::{CheckReport, ExecMode, PinnedSpec, PkgName, Spec, StepResult, Version};

/// How aggressively to resolve conflicts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    /// Accept whatever the resolver can satisfy. The safe default; `[C]ompatible` in the CLI.
    Compatible,
    /// Force the latest version of the listed packages even though it violates another
    /// package's requirement. The UI must name what breaks before this is selectable
    /// (UI-SPEC §4, DISCLAIMER §2).
    ForceLatest {
        /// Only these packages are forced; everything else stays `Compatible`.
        overrides: Vec<PkgName>,
    },
}

/// The user's intent, before the engine has said anything about it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanRequest {
    /// Already-installed packages the user selected for upgrade.
    #[serde(default)]
    pub upgrades: Vec<PkgName>,
    /// New packages queued in the dock bay.
    #[serde(default)]
    pub installs: Vec<Spec>,
    /// Conflict-handling strategy.
    pub strategy: Strategy,
}

/// Which section of the preview a change belongs to (UI-SPEC §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChangeKind {
    /// An installed package moving to a newer version.
    Upgrade,
    /// A package pulled in to satisfy something else.
    NewDependency,
    /// A package the user explicitly asked to install.
    NewInstall,
    /// A package moving to an older version to satisfy a constraint.
    Downgrade,
}

/// One line of the preview diff.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Change {
    /// Normalized distribution name.
    pub name: PkgName,
    /// Version before, absent for a fresh install.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Version>,
    /// Version the resolver chose.
    pub to: Version,
    /// Which preview section this belongs in.
    pub kind: ChangeKind,
}

/// The package responsible for holding another one back.
///
/// ARCHITECTURE §3: the engine's report says *what* was held back; *who* is responsible comes from
/// cross-referencing the reverse-dependency graph with each blocker's `Requires-Dist` constraint.
/// **If attribution is ambiguous, show the constraint without a culprit rather than guessing.**
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Blocker {
    /// The package imposing the constraint, absent when attribution is ambiguous.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<PkgName>,
    /// The constraint text, e.g. `"requests<2.31"`.
    pub constraint: String,
}

/// A package the resolver could not take all the way to `latest`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HeldBack {
    /// Normalized distribution name.
    pub pkg: PkgName,
    /// The version the resolver settled on.
    pub resolved: Version,
    /// The version the index offers.
    pub latest: Version,
    /// Why it could not go further. Empty means the engine gave no attributable reason.
    #[serde(default)]
    pub blockers: Vec<Blocker>,
}

/// A `ResolutionImpossible` outcome and whatever detail the engine gave (`PD-RES-001`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImpossibleDetail {
    /// Packages involved in the unsatisfiable set.
    #[serde(default)]
    pub packages: Vec<PkgName>,
    /// The engine's own explanation, verbatim.
    pub explanation: String,
}

/// The normalized dry-run result.
///
/// **Both adapters must emit this same shape** — that is the whole point of the `Engine` trait
/// (ARCHITECTURE §3). The pip adapter fills it from `--dry-run --report` JSON; the uv adapter from
/// uv's text plan output.
///
/// The exact field set is **provisional until spike SP-1** confirms uv's output is rich enough to
/// populate `held_back.blockers`. If it is not, SP-1's go/no-go says v1.0 ships pip-primary with uv
/// behind a beta flag.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResolutionReport {
    /// Everything that would change if the user confirms.
    #[serde(default)]
    pub changes: Vec<Change>,
    /// Packages that could not reach `latest`, with attribution.
    #[serde(default)]
    pub held_back: Vec<HeldBack>,
    /// Present when the whole resolution failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impossible: Option<ImpossibleDetail>,
    /// The engine's untouched output, kept for the log and for bug reports.
    pub raw: String,
}

impl ResolutionReport {
    /// True when the preview has nothing for the user to decide, so the UI can go straight to the
    /// confirm step (part of the 4-click budget in UI-SPEC §5).
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.held_back.is_empty() && self.impossible.is_none()
    }

    /// The pinned set a confirmed plan would execute.
    #[must_use]
    pub fn pinned_set(&self) -> Vec<PinnedSpec> {
        self.changes
            .iter()
            .map(|c| PinnedSpec {
                name: c.name.clone(),
                version: c.to.clone(),
            })
            .collect()
    }
}

/// Aggregate counts rendered as "13 successful, 2 failed, 1 skipped" (DATA-FLOW §6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Counts {
    /// Steps that applied.
    pub ok: usize,
    /// Steps that failed, each with a catalog code.
    pub failed: usize,
    /// Steps not attempted.
    pub skipped: usize,
}

/// The end-of-run report shown in the summary sheet and emitted by `--json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionSummary {
    /// Correlates the summary with its snapshot and log ring buffer.
    pub plan_id: String,
    /// Which phase produced these results.
    pub phase: ExecMode,
    /// One row per package.
    #[serde(default)]
    pub results: Vec<StepResult>,
    /// Post-run `engine.check()`.
    pub check: CheckReport,
    /// Derived from `results`; see [`ExecutionSummary::tally`].
    pub counts: Counts,
}

impl ExecutionSummary {
    /// Recompute `counts` from `results`.
    ///
    /// The counts drive the user-visible headline, so they are derived rather than accumulated by
    /// hand — a mismatch between the rows and the headline is exactly the kind of bug
    /// `docs/TESTING.md` §1.4 says must never regress.
    #[must_use]
    pub fn tally(results: &[StepResult]) -> Counts {
        use crate::model::StepStatus;
        let mut counts = Counts::default();
        for r in results {
            match r.status {
                StepStatus::Ok => counts.ok += 1,
                StepStatus::Failed => counts.failed += 1,
                StepStatus::Skipped => counts.skipped += 1,
            }
        }
        counts
    }
}

/// DATA-FLOW §9.3: a report older than this is refused by `plan_execute` and must be re-resolved.
pub const PLAN_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// DATA-FLOW §3: after this many conflict-decision rounds the UI requires manual pruning, which
/// prevents decision ping-pong.
pub const MAX_CONFLICT_ROUNDS: u8 = 3;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StepStatus;

    fn step(pkg: &str, status: StepStatus) -> StepResult {
        StepResult {
            pkg: PkgName::parse(pkg).unwrap(),
            from: None,
            to: None,
            status,
            code: None,
            stderr_tail: None,
        }
    }

    #[test]
    fn tally_matches_the_documented_headline() {
        // The example from DATA-FLOW §6: "13 successful, 2 failed, 1 skipped".
        let mut results: Vec<StepResult> = (0..13)
            .map(|i| step(&format!("ok{i}"), StepStatus::Ok))
            .collect();
        results.extend((0..2).map(|i| step(&format!("bad{i}"), StepStatus::Failed)));
        results.push(step("skipped0", StepStatus::Skipped));

        assert_eq!(
            ExecutionSummary::tally(&results),
            Counts {
                ok: 13,
                failed: 2,
                skipped: 1
            }
        );
    }

    #[test]
    fn empty_run_tallies_to_zero() {
        assert_eq!(ExecutionSummary::tally(&[]), Counts::default());
    }

    #[test]
    fn a_report_with_held_back_items_is_not_clean() {
        let held = HeldBack {
            pkg: PkgName::parse("requests").unwrap(),
            resolved: Version("2.30.0".into()),
            latest: Version("2.32.3".into()),
            blockers: vec![Blocker {
                by: Some(PkgName::parse("apiclient").unwrap()),
                constraint: "requests<2.31".into(),
            }],
        };
        let report = ResolutionReport {
            changes: vec![],
            held_back: vec![held],
            impossible: None,
            raw: String::new(),
        };
        assert!(!report.is_clean());
    }

    #[test]
    fn pinned_set_mirrors_the_changes() {
        let report = ResolutionReport {
            changes: vec![Change {
                name: PkgName::parse("httpx").unwrap(),
                from: Some(Version("0.27.0".into())),
                to: Version("0.28.1".into()),
                kind: ChangeKind::Upgrade,
            }],
            held_back: vec![],
            impossible: None,
            raw: String::new(),
        };
        assert!(report.is_clean());
        let pinned = report.pinned_set();
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].to_requirement(), "httpx==0.28.1");
    }
}
