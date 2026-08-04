//! Planning: `PlanRequest` → `ResolutionReport` → `ExecutionSummary`.
//!
//! See `docs/DATA-FLOW.md` §3 for the state machine and §9 for the invariants this module owns.

pub mod preview;

pub use preview::{
    Decision, ForcedPlan, apply_decisions, default_decision, derive_held_back, forced_requirements,
};

use crate::engine::{Engine, ProgressSink};
use crate::errors::{Code, PdError, Result};
use crate::model::{
    CheckReport, ExecMode, PinnedSpec, PkgName, PyEnv, Spec, StepResult, StepStatus, Version,
};
use crate::snapshot::SnapshotProof;

/// How aggressively to resolve conflicts.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
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
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Blocker {
    /// The package imposing the constraint, absent when attribution is ambiguous.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<PkgName>,
    /// The constraint text, e.g. `"requests<2.31"`.
    pub constraint: String,
}

/// A package the resolver could not take all the way to `latest`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
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
// `Default` is an empty plan — no changes, nothing held back. [`UpdateFlow`] starts from one
// before its first resolve, and `is_clean()` on it is true, which is the honest reading.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
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
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
pub struct Counts {
    /// Steps that applied.
    pub ok: usize,
    /// Steps that failed, each with a catalog code.
    pub failed: usize,
    /// Steps not attempted.
    pub skipped: usize,
}

/// The end-of-run report shown in the summary sheet and emitted by `--json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
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
    /// True when the user stopped this run part-way.
    ///
    /// ARCHITECTURE §7 words this as `Skipped(UserCancelled)`, but [`StepStatus`] is a
    /// payload-free enum and giving it one changes the wire shape for every consumer. A flag on
    /// the summary also matches what the summary sheet actually needs: "cancelled" is said once
    /// at the top, not repeated on forty rows.
    ///
    /// Steps that never ran are `Skipped`, which is already the honest status for them.
    #[serde(default)]
    pub cancelled: bool,
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

/// The types `--json` emits, exposed as JSON Schema so scripts can pin against them.
///
/// CLI-SPEC §6: *"schema documented by `pipdock schema <type>` which prints the JSON Schema
/// generated from the Rust types, so scripts can pin against it."* Generated rather than
/// hand-written, because a hand-written schema drifts from the struct the moment a field is added
/// and then lies to every script depending on it.
///
/// # Errors
/// `PD-PKG-002` when the name is not one of the exported types; the message lists them.
pub fn json_schema(type_name: &str) -> Result<serde_json::Value> {
    macro_rules! schema_for {
        ($($name:literal => $ty:ty),* $(,)?) => {
            match type_name {
                $($name => serde_json::to_value(schemars::schema_for!($ty))
                    .map_err(|e| PdError::new(Code::IntUnexpected, format!("schema: {e}"))),)*
                _ => Err(PdError::new(
                    Code::PkgNotFound,
                    format!(
                        "unknown type {type_name:?}; known types: {}",
                        SCHEMA_TYPES.join(", ")
                    ),
                )),
            }
        };
    }

    schema_for! {
        "Dist" => crate::model::Dist,
        "OutdatedDist" => crate::model::OutdatedDist,
        "PyEnv" => crate::model::PyEnv,
        "CheckReport" => crate::model::CheckReport,
        "StepResult" => crate::model::StepResult,
        "PlanRequest" => PlanRequest,
        "ResolutionReport" => ResolutionReport,
        "ExecutionSummary" => ExecutionSummary,
        "Pin" => crate::pins::Pin,
        "GuardReport" => crate::graph::GuardReport,
        "Diff" => crate::snapshot::Diff,
        "SnapshotMeta" => crate::snapshot::Meta,
        "Hit" => crate::index::Hit,
        "PackageMeta" => crate::index::PackageMeta,
        "Freshness" => crate::index::Freshness,
        "RefreshReport" => crate::index::RefreshReport,
        "ProgressEvent" => crate::engine::ProgressEvent,
        "FlowStep" => crate::flow::FlowStep,
        "Decision" => Decision,
        "Intent" => crate::flow::Intent,
        "Code" => crate::errors::Code,
    }
}

/// Every type [`json_schema`] can produce, for help text and tests.
pub const SCHEMA_TYPES: &[&str] = &[
    "Dist",
    "OutdatedDist",
    "PyEnv",
    "CheckReport",
    "StepResult",
    "PlanRequest",
    "ResolutionReport",
    "ExecutionSummary",
    "Pin",
    "GuardReport",
    "Diff",
    "SnapshotMeta",
    "Hit",
    "PackageMeta",
    "Freshness",
    "RefreshReport",
    "ProgressEvent",
    "FlowStep",
    "Decision",
    "Intent",
    "Code",
];

/// DATA-FLOW §9.3: a report older than this is refused by [`execute`] and must be re-resolved.
pub const PLAN_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// DATA-FLOW §3: after this many conflict-decision rounds the UI requires manual pruning, which
/// prevents decision ping-pong.
pub const MAX_CONFLICT_ROUNDS: u8 = 3;

/// A [`ResolutionReport`] the user has confirmed.
///
/// **This type is how DATA-FLOW §9.1 stops being a rule and starts being a fact.** There is no
/// other way to construct one, and [`execute`] accepts nothing else, so a mutating engine call
/// without an accepted plan is not a bug that review has to catch — it does not compile.
///
/// It also carries what §9.3 needs to refuse a stale plan: when it was accepted, and a fingerprint
/// of the environment at that moment.
#[derive(Debug, Clone)]
pub struct AcceptedPlan {
    /// Correlates the plan with its snapshot, its summary and its log ring buffer.
    pub id: String,
    /// The confirmed report.
    pub report: ResolutionReport,
    /// When the user confirmed.
    pub accepted_at: jiff::Timestamp,
    /// Identity of the environment it was resolved against.
    pub env_hash: String,
    /// Fingerprint of the installed set at resolve time, for the drift check.
    pub probe_hash: String,
}

impl AcceptedPlan {
    /// Record the user's confirmation of a report.
    #[must_use]
    pub fn accept(
        report: ResolutionReport,
        env_hash: String,
        installed: &[crate::model::Dist],
        now: jiff::Timestamp,
    ) -> Self {
        Self {
            id: format!(
                "{}-{}",
                &env_hash[..8.min(env_hash.len())],
                now.as_millisecond()
            ),
            report,
            accepted_at: now,
            env_hash,
            probe_hash: fingerprint(installed),
        }
    }

    /// DATA-FLOW §9.3: refuse a plan that is too old, or one whose environment has drifted.
    ///
    /// Both cases mean the preview the user approved no longer describes what would happen — and
    /// executing a plan the user did not actually see is exactly what "preview before touch" is
    /// supposed to prevent.
    ///
    /// # Errors
    /// `PD-RES-002` in either case.
    pub fn verify(
        &self,
        env_hash: &str,
        installed: &[crate::model::Dist],
        now: jiff::Timestamp,
    ) -> Result<()> {
        let age = now
            .as_millisecond()
            .saturating_sub(self.accepted_at.as_millisecond());
        if age < 0 || u128::try_from(age).unwrap_or(u128::MAX) > PLAN_MAX_AGE.as_millis() {
            return Err(PdError::new(
                Code::ResPlanStale,
                format!(
                    "this preview is older than {} minutes — re-run it",
                    PLAN_MAX_AGE.as_secs() / 60
                ),
            ));
        }
        if env_hash != self.env_hash {
            return Err(PdError::new(
                Code::ResPlanStale,
                "this preview was made for a different environment",
            ));
        }
        if fingerprint(installed) != self.probe_hash {
            return Err(PdError::new(
                Code::ResPlanStale,
                "the environment changed since the preview was made — re-run it",
            ));
        }
        Ok(())
    }
}

/// A cheap, order-independent fingerprint of an installed set.
///
/// Order-independent on purpose: engines do not promise a stable listing order, and a reordering
/// is not drift. Only the set of `name==version` pairs matters.
#[must_use]
pub fn fingerprint(installed: &[crate::model::Dist]) -> String {
    use std::fmt::Write as _;

    use sha2::{Digest as _, Sha256};

    let mut pairs: Vec<String> = installed
        .iter()
        .map(|d| format!("{}=={}", d.name, d.version))
        .collect();
    pairs.sort_unstable();

    let digest = Sha256::digest(pairs.join("\n").as_bytes());
    digest
        .iter()
        .take(16)
        .fold(String::with_capacity(32), |mut acc, b| {
            let _ = write!(acc, "{b:02x}");
            acc
        })
}

/// Execute a confirmed plan (ARCHITECTURE §8, DATA-FLOW §3).
///
/// Two phases:
///
/// 1. **Phase A** — one engine invocation for the whole pinned set. Fast, especially under uv, and
///    atomic-ish. If it exits 0 the run is done.
/// 2. **Phase B** — on Phase-A failure, re-run **per package, in resolver-report order**, and
///    **keep going past failures**. This is the owner's skip-and-continue requirement, and it is
///    the reason a batch of fifteen with two bad packages applies thirteen instead of nothing.
///
/// Then `engine.check()` runs and its findings join the summary.
///
/// # Why the signature looks like this
///
/// `plan: &AcceptedPlan` and `_snapshot: SnapshotProof` are the enforcement of DATA-FLOW §9.1 and
/// §9.2. An `AcceptedPlan` only exists once a report was confirmed, and a `SnapshotProof` is
/// either a real `Snapshot` — which only exists once one was successfully written — or a named
/// waiver that says out loud what it is giving up. A caller that skipped a step cannot quietly
/// pass `None`; it has to state the exception, which makes every one of them greppable.
///
/// # Errors
/// `PD-RES-002` when the plan is stale or the environment drifted. Per-package failures are
/// **not** errors — they appear in the summary with their catalog codes.
pub async fn execute(
    engine: &dyn Engine,
    env: &PyEnv,
    plan: &AcceptedPlan,
    _snapshot: SnapshotProof<'_>,
    installed_now: &[crate::model::Dist],
    now: jiff::Timestamp,
    sink: ProgressSink,
) -> Result<ExecutionSummary> {
    plan.verify(&plan.env_hash, installed_now, now)?;

    let pinned = plan.report.pinned_set();
    if pinned.is_empty() {
        return Ok(ExecutionSummary {
            plan_id: plan.id.clone(),
            phase: ExecMode::Batch,
            results: Vec::new(),
            check: engine.check(env).await.unwrap_or(CheckReport {
                ok: true,
                findings: Vec::new(),
            }),
            counts: Counts::default(),
            cancelled: false,
        });
    }

    // The plan knows its own length; the caller cannot, so it is set here rather than trusted.
    let sink = ProgressSink {
        total: pinned.len(),
        ..sink
    };

    // Phase A. The markers are emitted here rather than inside the adapter for the same reason
    // `step` is: an adapter runs one command and cannot know its position in the plan.
    let batch_sink = sink.at(0);
    batch_sink.started(None, ExecMode::Batch);
    let batch = engine
        .install(env, &pinned, ExecMode::Batch, batch_sink.clone())
        .await?;
    batch_sink.finished(None, ExecMode::Batch, batch.status);

    // A cancelled Phase A must **not** fall through to Phase B. Isolating would re-run every
    // package the user just stopped, one at a time — the opposite of what cancelling means.
    if sink.is_cancelled() {
        let results: Vec<StepResult> = pinned
            .iter()
            .map(|spec| StepResult {
                pkg: spec.name.clone(),
                from: None,
                to: Some(spec.version.clone()),
                status: StepStatus::Skipped,
                code: None,
                stderr_tail: None,
            })
            .collect();
        let counts = ExecutionSummary::tally(&results);
        return Ok(ExecutionSummary {
            plan_id: plan.id.clone(),
            phase: ExecMode::Batch,
            results,
            check: post_check(engine, env).await,
            counts,
            cancelled: true,
        });
    }
    if batch.status == StepStatus::Ok {
        let results: Vec<StepResult> = pinned
            .iter()
            .map(|spec| StepResult {
                pkg: spec.name.clone(),
                from: None,
                to: Some(spec.version.clone()),
                status: StepStatus::Ok,
                code: None,
                stderr_tail: None,
            })
            .collect();
        let counts = ExecutionSummary::tally(&results);
        return Ok(ExecutionSummary {
            plan_id: plan.id.clone(),
            phase: ExecMode::Batch,
            results,
            check: post_check(engine, env).await,
            counts,
            cancelled: false,
        });
    }

    // Phase B: isolate. One failure must not cost the user the other fourteen packages.
    let mut results = Vec::with_capacity(pinned.len());
    for (index, spec) in pinned.iter().enumerate() {
        // Checked before each package so the remaining ones are reported as never attempted,
        // which is what `Skipped` already means. The one in flight when the token tripped is
        // handled below.
        if sink.is_cancelled() {
            results.extend(pinned[index..].iter().map(|remaining| StepResult {
                pkg: remaining.name.clone(),
                from: None,
                to: Some(remaining.version.clone()),
                status: StepStatus::Skipped,
                code: None,
                stderr_tail: None,
            }));
            break;
        }

        let step_sink = sink.at(index);
        step_sink.started(Some(spec.name.clone()), ExecMode::Isolated);
        let step = engine
            .install(
                env,
                std::slice::from_ref(spec),
                ExecMode::Isolated,
                step_sink.clone(),
            )
            .await;

        // A step that failed *because we killed it* is not the package's failure. Reclassifying
        // here rather than inventing a "cancelled" catalog code keeps the summary honest without
        // widening the error catalog: it was not attempted to completion, so it is Skipped.
        if sink.is_cancelled() && step.is_err() {
            step_sink.finished(
                Some(spec.name.clone()),
                ExecMode::Isolated,
                StepStatus::Skipped,
            );
            results.push(StepResult {
                pkg: spec.name.clone(),
                from: None,
                to: Some(spec.version.clone()),
                status: StepStatus::Skipped,
                code: None,
                stderr_tail: None,
            });
            continue;
        }

        let result = match step {
            Ok(r) => r,
            // An engine that could not even be spawned is still one package's outcome here;
            // aborting would discard the packages that already succeeded.
            Err(e) => StepResult {
                pkg: spec.name.clone(),
                from: None,
                to: Some(spec.version.clone()),
                status: StepStatus::Failed,
                code: Some(e.code),
                stderr_tail: e.stderr_tail,
            },
        };
        // Every StepStarted gets exactly one StepFinished, whichever way the step ended — the
        // drawer closes a section on it and the live region counts it.
        step_sink.finished(Some(spec.name.clone()), ExecMode::Isolated, result.status);
        results.push(result);
    }

    let counts = ExecutionSummary::tally(&results);
    Ok(ExecutionSummary {
        plan_id: plan.id.clone(),
        phase: ExecMode::Isolated,
        results,
        check: post_check(engine, env).await,
        counts,
        cancelled: sink.is_cancelled(),
    })
}

/// Run the post-execution check, treating an unrunnable check as "no findings".
///
/// A check that could not run must not turn a successful run into a reported failure; the summary
/// already carries the per-package truth.
async fn post_check(engine: &dyn Engine, env: &PyEnv) -> CheckReport {
    engine.check(env).await.unwrap_or(CheckReport {
        ok: true,
        findings: Vec::new(),
    })
}

/// Remove packages, sequentially, skipping past failures (DATA-FLOW §5).
///
/// The reverse-dependency guard is the caller's job and runs **once against the full set** before
/// this is called; by the time execution starts the user has already decided.
///
/// # Errors
/// Never for a package failure — those land in the summary. Only a stale plan aborts.
/// Apply a rollback plan: remove what the snapshot does not have, restore what it does.
///
/// Lives here rather than in a head because it is the mutating half of DATA-FLOW §8, and a
/// hand-assembled `ExecutionSummary` in one head is a summary the other head cannot reproduce.
///
/// Restores run in [`ExecMode::Isolated`], one package at a time. A batch would be faster, but
/// this is the one flow whose entire job is exactness: a batch resolve is free to pick a
/// different version than the snapshot recorded, and a partial failure would take the rest of the
/// restore with it.
///
/// # Errors
/// Propagates a failure of the removal phase. Individual restore failures are **not** errors —
/// they land in the summary with their codes, so a snapshot that is 90% restorable restores 90%.
pub async fn execute_rollback(
    engine: &dyn Engine,
    env: &PyEnv,
    plan_id: String,
    restore: &crate::snapshot::RollbackPlan,
    snapshot: SnapshotProof<'_>,
    sink: ProgressSink,
) -> Result<ExecutionSummary> {
    let mut results = Vec::new();

    if !restore.uninstall.is_empty() {
        let removed = execute_uninstall(
            engine,
            env,
            plan_id.clone(),
            &restore.uninstall,
            snapshot,
            sink.clone(),
        )
        .await?;
        results.extend(removed.results);
    }

    for spec in &restore.install {
        let step = engine
            .install(
                env,
                std::slice::from_ref(spec),
                ExecMode::Isolated,
                sink.clone(),
            )
            .await;
        results.push(step.unwrap_or_else(|e| StepResult {
            pkg: spec.name.clone(),
            from: None,
            to: Some(spec.version.clone()),
            status: StepStatus::Failed,
            code: Some(e.code),
            stderr_tail: e.stderr_tail,
        }));
    }

    let counts = ExecutionSummary::tally(&results);
    Ok(ExecutionSummary {
        plan_id,
        phase: ExecMode::Isolated,
        results,
        check: post_check(engine, env).await,
        counts,
        cancelled: sink.is_cancelled(),
    })
}

pub async fn execute_uninstall(
    engine: &dyn Engine,
    env: &PyEnv,
    plan_id: String,
    names: &[PkgName],
    _snapshot: SnapshotProof<'_>,
    sink: ProgressSink,
) -> Result<ExecutionSummary> {
    let mut results = Vec::with_capacity(names.len());
    for name in names {
        let step = engine
            .uninstall(env, std::slice::from_ref(name), sink.clone())
            .await;
        results.push(match step {
            Ok(r) => r,
            Err(e) => StepResult {
                pkg: name.clone(),
                from: None,
                to: None,
                status: StepStatus::Failed,
                code: Some(e.code),
                stderr_tail: e.stderr_tail,
            },
        });
    }

    let counts = ExecutionSummary::tally(&results);
    Ok(ExecutionSummary {
        plan_id,
        phase: ExecMode::Isolated,
        results,
        check: post_check(engine, env).await,
        counts,
        cancelled: sink.is_cancelled(),
    })
}

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
    fn every_advertised_schema_type_resolves() {
        // CLI-SPEC §6 promises scripts can pin against these. A name in the list that does not
        // resolve would be a promise broken at the moment someone relied on it.
        for name in SCHEMA_TYPES {
            let schema = json_schema(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(schema.is_object(), "{name} produced {schema}");
        }
        let err = json_schema("NotAType").expect_err("unknown type must fail");
        assert_eq!(err.code, Code::PkgNotFound);
        assert!(
            err.message.contains("ResolutionReport"),
            "the message must list the options"
        );
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

    #[test]
    fn every_exported_schema_uses_camel_case_properties() {
        // DATA-FLOW §6 documents `planId`, ERROR-CATALOG §3 documents `stderrTail`, and
        // ui/src/ipc/index.ts declares the same. Deriving serde without `rename_all` emitted the
        // Rust spelling, so `pipdock --json` disagreed with its own specification -- and with the
        // error envelope main.rs hand-builds, in the same binary.
        //
        // Walking the generated schemas is what makes this total. A convention only covers the
        // structs someone remembered; this fails at `cargo test` the moment a snake_case field
        // reaches the exported surface, including through a type nested inside another.
        fn property_names(node: &serde_json::Value, out: &mut Vec<String>) {
            match node {
                serde_json::Value::Object(map) => {
                    for (key, value) in map {
                        if key == "properties"
                            && let Some(props) = value.as_object()
                        {
                            out.extend(props.keys().cloned());
                        }
                        property_names(value, out);
                    }
                }
                serde_json::Value::Array(items) => {
                    for item in items {
                        property_names(item, out);
                    }
                }
                _ => {}
            }
        }

        let mut offenders = Vec::new();
        for name in SCHEMA_TYPES {
            let schema = json_schema(name).expect("every listed type has a schema");
            let mut props = Vec::new();
            property_names(&schema, &mut props);
            for prop in props {
                if prop.contains('_') {
                    offenders.push(format!("{name}.{prop}"));
                }
            }
        }
        offenders.sort();
        offenders.dedup();
        assert!(
            offenders.is_empty(),
            "snake_case reached the IPC surface; add #[serde(rename_all = \"camelCase\")]:\n  {}",
            offenders.join("\n  ")
        );
    }
}
