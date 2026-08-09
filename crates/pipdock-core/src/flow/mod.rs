//! The mutation flows, as resumable state machines.
//!
//! DATA-FLOW §3 is one process: resolve → derive held-back → decide → re-resolve → confirm →
//! snapshot → two-phase execute → post-check → summary. It used to live in `pipdock-cli`, which
//! meant the GUI would have had to reimplement it — and with it every hard invariant: the PEP 668
//! gate, pins never reaching `upgrades`, the conflict-round cap, snapshot-before-mutation,
//! `AcceptedPlan`. Two implementations of those is one too many (PRD G5: the GUI and the CLI never
//! diverge).
//!
//! # Why resumable rather than callback-driven
//!
//! The flow stops at two points that need a human: which conflicts to force or skip, and the final
//! confirm. In the CLI those are a prompt; in the GUI they are separate IPC round trips with a
//! screen render in between. A callback interface would invert that — `plan_resolve` could not
//! return until the whole flow finished, so the preview could never be drawn.
//!
//! So each step returns a [`FlowStep`] describing what is needed next, and the caller drives:
//!
//! ```text
//!   start() ──► NeedsDecisions ──decide()──► NeedsDecisions ──► …
//!      │              │                            │
//!      │              └──────────┬─────────────────┘
//!      ▼                         ▼
//!    Nothing              NeedsConfirm / RoundsExhausted
//!                                │
//!                    take_snapshot() ──► execute() ──► ExecutionSummary
//! ```
//!
//! # The flow never prints
//!
//! Every message the CLI used to emit mid-flow is returned as data instead: the pinned-excluded
//! notice is [`UpdateFlow::excluded_pins`], "nothing to do" is [`NothingReason`], the snapshot id
//! comes back from [`UpdateFlow::take_snapshot`]. I18N §1 requires this — Rust emits codes and
//! structured data, and all human phrasing lives in the frontend catalogs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::engine::{Engine, EventSink, ProgressSink};
use crate::errors::{Code, PdError, Result};
use crate::graph::ReverseDeps;
use crate::model::{Dist, OutdatedDist, PkgName, PyEnv, Spec};
use crate::pins::{self, Pin};
use crate::plan::{self, Decision, PlanRequest, ResolutionReport, Strategy};
use crate::snapshot::{self, Snapshot, SnapshotProof};
use tokio_util::sync::CancellationToken;

/// What the user asked for, before it becomes a [`PlanRequest`].
///
/// Names arrive as strings and are parsed here rather than by the caller, so both heads get the
/// same `PD-PKG-002` on a malformed name and SECURITY §2's "validated before it reaches argv"
/// holds for the GUI as well as the CLI.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "intent",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Intent {
    /// `pipdock update`
    Update {
        /// Every outdated package.
        all: bool,
        /// Specific packages.
        #[serde(default)]
        pkgs: Vec<String>,
        /// Ad-hoc exclusions on top of pins.
        #[serde(default)]
        except: Vec<String>,
        /// `--strategy latest`.
        #[serde(default)]
        force_latest: bool,
    },
    /// `pipdock install`
    Install {
        /// `name` or `name==version`.
        specs: Vec<String>,
    },
}

/// Why a flow ended before there was anything to confirm.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum NothingReason {
    /// The request was empty once pins and `--except` were applied.
    NothingToDo,
    /// Every candidate was skipped while resolving conflicts, so no plan remains.
    EverythingSkipped,
}

/// What the flow needs next.
///
/// Crosses IPC, so the GUI's two decision points are a round trip apart (see the module docs).
/// Tagged on `step` rather than externally, because the frontend switches on it and a bare
/// `{ needsDecisions: { … } }` would make every consumer unwrap before it can look.
#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "step",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FlowStep {
    /// Conflicts need a per-package choice. Feed the answers to [`UpdateFlow::decide`].
    NeedsDecisions {
        /// The preview as it stands.
        report: ResolutionReport,
        /// How many decision rounds have been applied.
        round: u8,
        /// Rounds still available before the cap (DATA-FLOW §3).
        ///
        /// Surfaced because `MAX_CONFLICT_ROUNDS` was invisible: the cap existed in core and
        /// nothing told the user it was approaching, so they would hit the wall unwarned.
        rounds_remaining: u8,
    },
    /// The preview is ready to confirm.
    NeedsConfirm {
        /// The preview as it stands.
        report: ResolutionReport,
    },
    /// The conflict-round cap is reached; the remaining conflicts cannot be re-decided.
    ///
    /// Distinct from [`Self::NeedsConfirm`] so the GUI can say so, but the plan is still
    /// confirmable — which is what the CLI has always done at this point.
    RoundsExhausted {
        /// The preview as it stands.
        report: ResolutionReport,
    },
    /// There is nothing to do.
    Nothing {
        /// Which of the two empty cases this is.
        reason: NothingReason,
    },
}

/// Whether to take the pre-execution snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotPolicy {
    /// The default. DATA-FLOW §9.2.
    Take,
    /// `--no-snapshot`: disposable environments only, and the caller must warn.
    Waive,
}

/// Tracks DATA-FLOW §9.2 across the two calls that used to be one.
#[derive(Debug)]
enum SnapshotState {
    NotTaken,
    Taken(Box<Snapshot>),
    Waived,
}

/// The update/install flow (DATA-FLOW §3).
pub struct UpdateFlow {
    env: PyEnv,
    engine: Box<dyn Engine>,
    env_hash: String,
    /// The installed set at resolve time; also what `AcceptedPlan` fingerprints against.
    installed: Vec<Dist>,
    graph: ReverseDeps,
    outdated: Vec<OutdatedDist>,
    req: PlanRequest,
    report: ResolutionReport,
    round: u8,
    plan_id: String,
    snapshot: SnapshotState,
    /// Pinned packages kept out of this plan, so the caller can say which and why.
    excluded_pins: Vec<Pin>,
    cancel: CancellationToken,
}

impl UpdateFlow {
    /// Probe, build the request, and resolve once.
    ///
    /// Takes the pins rather than the [`Store`] they came from. That is not only tidier — it is
    /// required. `Store` wraps a `rusqlite::Connection` and is `Send` but **not `Sync`**, so a
    /// future holding a `&Store` across an await is not `Send`, and a Tauri command must be. The
    /// caller reads the pins first, which is one synchronous line, and the flow no longer reaches
    /// into a database at all — it can be driven in a test without one.
    ///
    /// # Errors
    /// `PD-ENV-002` when the environment is externally managed (PEP 668), which is checked before
    /// any engine command runs. Otherwise propagates probe and engine failures.
    pub async fn start(
        env: PyEnv,
        engine: Box<dyn Engine>,
        intent: &Intent,
        pins: &[Pin],
    ) -> Result<(Self, FlowStep)> {
        let env_hash = crate::envs::env_hash(&env.interpreter);

        // DATA-FLOW §2: PEP 668 environments are blocked at step zero, before any engine command.
        if env.externally_managed {
            return Err(PdError::new(
                Code::EnvExternallyManaged,
                "this Python is externally managed (PEP 668). Use a virtual environment; \
                 the override lives in Settings and is discouraged",
            ));
        }

        let probed = crate::envs::probe(&env.interpreter, env.source).await?;
        // Built against this interpreter so marker-gated requirements are read correctly: without
        // it a `python_version < "3.11"` branch is reported as a blocker on 3.12 (SP-5 dogfood).
        let graph = ReverseDeps::build_for(&probed.dists, &probed.env.python_version);
        let outdated = engine.list_outdated(&env).await?;
        let (req, excluded_pins) = build_request(intent, &outdated, pins)?;
        let plan_id = format!("update-{env_hash:.8}");

        let mut flow = Self {
            env,
            engine,
            env_hash,
            installed: probed.dists,
            graph,
            outdated,
            req,
            report: ResolutionReport::default(),
            round: 0,
            plan_id,
            snapshot: SnapshotState::NotTaken,
            excluded_pins,
            cancel: CancellationToken::new(),
        };

        if flow.req.upgrades.is_empty() && flow.req.installs.is_empty() {
            return Ok((
                flow,
                FlowStep::Nothing {
                    reason: NothingReason::NothingToDo,
                },
            ));
        }

        let step = flow.resolve().await?;
        Ok((flow, step))
    }

    /// Apply one round of conflict decisions and re-resolve.
    ///
    /// Answering every conflict with [`Decision::KeepCompatible`] settles the preview without a
    /// re-resolve, because that is what the resolver already chose.
    ///
    /// # Errors
    /// Propagates engine failures from the re-resolve.
    pub async fn decide(&mut self, decisions: &BTreeMap<PkgName, Decision>) -> Result<FlowStep> {
        if decisions.values().all(|d| *d == Decision::KeepCompatible) {
            return Ok(FlowStep::NeedsConfirm {
                report: self.report.clone(),
            });
        }

        self.req = plan::apply_decisions(&self.req, decisions);
        self.round = self.round.saturating_add(1);

        if self.req.upgrades.is_empty() && self.req.installs.is_empty() {
            return Ok(FlowStep::Nothing {
                reason: NothingReason::EverythingSkipped,
            });
        }

        self.resolve().await
    }

    /// Resolve the current request and classify what the caller must do next.
    async fn resolve(&mut self) -> Result<FlowStep> {
        let mut report = self.engine.resolve(&self.env, &self.req).await?;
        plan::derive_held_back(&mut report, &self.req.upgrades, &self.outdated, &self.graph);
        self.report = report;

        let report = self.report.clone();
        Ok(if self.report.is_clean() {
            FlowStep::NeedsConfirm { report }
        } else if self.round >= plan::MAX_CONFLICT_ROUNDS {
            FlowStep::RoundsExhausted { report }
        } else {
            FlowStep::NeedsDecisions {
                report,
                round: self.round,
                rounds_remaining: plan::MAX_CONFLICT_ROUNDS.saturating_sub(self.round),
            }
        })
    }

    /// Write the pre-execution snapshot, or record that it was waived.
    ///
    /// Separate from [`Self::execute`] because DATA-FLOW §3 makes `Snapshotting` and `Executing`
    /// distinct states the UI renders separately, and because a snapshot failure has to abort
    /// before anything is touched.
    ///
    /// # Errors
    /// `PD-SNP-001` when the snapshot cannot be written. **Nothing has been executed** at that
    /// point, and nothing will be: [`Self::execute`] refuses without this step.
    pub async fn take_snapshot(
        &mut self,
        policy: SnapshotPolicy,
        app_data: &Path,
    ) -> Result<Option<snapshot::Meta>> {
        if policy == SnapshotPolicy::Waive {
            self.snapshot = SnapshotState::Waived;
            return Ok(None);
        }
        let snap = snapshot::create(
            app_data,
            &self.env_hash,
            self.engine.freeze(&self.env).await?,
            snapshot::Trigger::Plan {
                plan_id: self.plan_id.clone(),
            },
            self.engine.id(),
            jiff::Timestamp::now(),
        )?;
        let meta = snap.meta.clone();
        self.snapshot = SnapshotState::Taken(Box::new(snap));
        Ok(Some(meta))
    }

    /// Accept the plan and run it, two-phase.
    ///
    /// # Errors
    /// `PD-SNP-001` when [`Self::take_snapshot`] has not run — invariant DATA-FLOW §9.2 is not
    /// something a caller gets to skip by forgetting. `PD-RES-002` when the preview has gone
    /// stale or the environment drifted. Per-package failures are **not** errors; they appear in
    /// the summary with their codes.
    pub async fn execute(&self, tx: EventSink) -> Result<crate::plan::ExecutionSummary> {
        let proof = proof_from(&self.snapshot)?;
        // `total` is corrected inside plan::execute, which is the only place that knows the
        // pinned-set length.
        let sink = ProgressSink::new(tx, 0, self.cancel.clone());

        let accepted = plan::AcceptedPlan::accept(
            self.report.clone(),
            self.env_hash.clone(),
            &self.installed,
            jiff::Timestamp::now(),
        );

        plan::execute(
            self.engine.as_ref(),
            &self.env,
            &accepted,
            proof,
            &self.installed,
            jiff::Timestamp::now(),
            sink,
        )
        .await
    }

    /// Pinned packages kept out of this plan (DATA-FLOW §9.5).
    #[must_use]
    pub fn excluded_pins(&self) -> &[Pin] {
        &self.excluded_pins
    }

    /// The preview as it currently stands.
    #[must_use]
    pub const fn report(&self) -> &ResolutionReport {
        &self.report
    }

    /// Correlates the summary, the snapshot and the log ring buffer.
    #[must_use]
    pub fn plan_id(&self) -> &str {
        &self.plan_id
    }

    /// The environment this flow is acting on.
    #[must_use]
    pub const fn env(&self) -> &PyEnv {
        &self.env
    }

    /// A handle that stops this flow's execution.
    ///
    /// Cloneable and safe to hold elsewhere, which is the point: `plan_cancel` arrives on a
    /// different IPC call while `execute` is still awaited, so it cannot go through `&mut self`.
    #[must_use]
    pub fn cancel_handle(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Stop this flow. Idempotent.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// The uninstall flow (DATA-FLOW §5).
///
/// Shorter than [`UpdateFlow`] because there is nothing to resolve, but the same shape and the
/// same invariant: the guard runs first, and nothing is removed before a snapshot exists.
///
/// "Remove dependents too" is not a variant here — it is the caller starting again with
/// [`crate::graph::GuardReport::with_dependents`] as the new set, which re-runs the guard. That
/// is what DATA-FLOW §5 means by re-guarding, and it keeps a widened removal from skipping the
/// check that justified widening it.
pub struct UninstallFlow {
    env: PyEnv,
    engine: Box<dyn Engine>,
    env_hash: String,
    names: Vec<PkgName>,
    plan_id: String,
    snapshot: SnapshotState,
    /// What the guard found, kept so [`Self::execute`] can refuse a removal the user never
    /// accepted. Returned to the caller as well, because the caller is what renders it.
    guard: crate::graph::GuardReport,
    cancel: CancellationToken,
}

/// Whether the user has accepted the breakage the guard found (DATA-FLOW §5).
///
/// A separate argument to [`UninstallFlow::execute`] rather than a `bool` on the flow, for the
/// reason [`SnapshotProof`] is a named waiver: `execute(true, …)` at a call site says nothing
/// about what was true, and the one thing this type exists to prevent is a removal proceeding
/// because somebody forgot to look at the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardAck {
    /// The guard found nothing, or the caller believes it did.
    ///
    /// Not a claim the caller gets to make freely: if the guard *did* find dependents this is
    /// refused with `PD-RES-004`, which is the whole point.
    Clear,
    /// The user was shown what breaks and chose to remove anyway — §5's *Force remove only X*.
    ForcedDespiteBreakage,
}

impl UninstallFlow {
    /// Parse the names, probe, and run the reverse-dependency guard.
    ///
    /// # Errors
    /// `PD-PKG-002` when a name does not parse; otherwise propagates probe failures.
    pub async fn start(
        env: PyEnv,
        engine: Box<dyn Engine>,
        pkgs: &[String],
    ) -> Result<(Self, crate::graph::GuardReport)> {
        // DATA-FLOW §2's preamble is "all mutating flows", and a removal is one. `UpdateFlow` has
        // refused these since S1 and this did not, so PipDock would happily strip packages out of
        // a system Python it declines to *upgrade* — the worse of the two operations, since there
        // is no resolver between the user and the damage.
        if env.externally_managed {
            return Err(PdError::new(
                Code::EnvExternallyManaged,
                "this Python is externally managed (PEP 668). Use a virtual environment; \
                 the override lives in Settings and is discouraged",
            ));
        }

        let names: Vec<PkgName> = pkgs
            .iter()
            .map(|p| PkgName::parse(p))
            .collect::<Result<_>>()?;

        // The graph is built from probe.py, which is the only source carrying requires_dist.
        let probed = crate::envs::probe(&env.interpreter, env.source).await?;
        let report =
            ReverseDeps::build_for(&probed.dists, &probed.env.python_version).guard(&names);

        let env_hash = crate::envs::env_hash(&env.interpreter);
        let plan_id = format!("uninstall-{env_hash:.8}");
        Ok((
            Self {
                env,
                engine,
                env_hash,
                names,
                plan_id,
                snapshot: SnapshotState::NotTaken,
                guard: report.clone(),
                cancel: CancellationToken::new(),
            },
            report,
        ))
    }

    /// What the guard found when this flow started.
    #[must_use]
    pub const fn guard(&self) -> &crate::graph::GuardReport {
        &self.guard
    }

    /// Would [`Self::execute`] accept this acknowledgement?
    ///
    /// The same rule `execute` enforces, exposed so a caller can refuse *before* writing a
    /// snapshot for a removal that will not happen. `execute` still checks: this is the polite
    /// early exit, not the guard.
    ///
    /// # Errors
    /// `PD-RES-004` when the guard found dependents and `ack` does not accept breaking them.
    pub fn check(&self, ack: GuardAck) -> Result<()> {
        ack_ok(&self.guard, ack)
    }

    /// Write the pre-removal snapshot, or record the waiver.
    ///
    /// Takes a [`SnapshotPolicy`] for the same reason [`UpdateFlow::take_snapshot`] does: without
    /// it the CLI's `--no-snapshot` was accepted, parsed and then silently ignored on this path,
    /// so a disposable-environment waiver behaved differently depending on which command it was
    /// given to. `Waive` returns `None` — there is no meta to print, and no id for the summary to
    /// correlate against.
    ///
    /// # Errors
    /// `PD-SNP-001` when it cannot be written, in which case nothing is removed.
    pub async fn take_snapshot(
        &mut self,
        policy: SnapshotPolicy,
        app_data: &Path,
    ) -> Result<Option<snapshot::Meta>> {
        if matches!(policy, SnapshotPolicy::Waive) {
            self.snapshot = SnapshotState::Waived;
            return Ok(None);
        }
        let snap = snapshot::create(
            app_data,
            &self.env_hash,
            self.engine.freeze(&self.env).await?,
            snapshot::Trigger::Plan {
                plan_id: self.plan_id.clone(),
            },
            self.engine.id(),
            jiff::Timestamp::now(),
        )?;
        let meta = snap.meta.clone();
        self.snapshot = SnapshotState::Taken(Box::new(snap));
        Ok(Some(meta))
    }

    /// Remove the packages, sequentially, skip-and-continue.
    ///
    /// # Errors
    /// `PD-RES-004` when the guard found dependents and `ack` does not accept breaking them;
    /// `PD-SNP-001` when [`Self::take_snapshot`] has not run. Per-package failures appear in the
    /// summary rather than as errors.
    pub async fn execute(
        &self,
        ack: GuardAck,
        tx: EventSink,
    ) -> Result<crate::plan::ExecutionSummary> {
        ack_ok(&self.guard, ack)?;
        let proof = proof_from(&self.snapshot)?;
        let sink = ProgressSink::new(tx, self.names.len(), self.cancel.clone());
        // The summary is correlated by the *snapshot* id here, not `plan_id`. That is what the
        // CLI has always emitted, and it is the more useful handle for a removal: the thing a
        // user wants after `uninstall` is the snapshot to roll back to.
        let correlation = match &self.snapshot {
            SnapshotState::Taken(snap) => snap.meta.id.clone(),
            _ => self.plan_id.clone(),
        };
        plan::execute_uninstall(
            self.engine.as_ref(),
            &self.env,
            correlation,
            &self.names,
            proof,
            sink,
            // The removals are the whole plan here, so they start at step zero. Only a rollback
            // has anything in front of them.
            0,
        )
        .await
    }

    /// The packages this flow will remove.
    #[must_use]
    pub fn names(&self) -> &[PkgName] {
        &self.names
    }

    /// A handle that stops this flow's execution.
    ///
    /// Cloneable and safe to hold elsewhere, which is the point: `plan_cancel` arrives on a
    /// different IPC call while `execute` is still awaited, so it cannot go through `&mut self`.
    #[must_use]
    pub fn cancel_handle(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Stop this flow. Idempotent.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// What a rollback would do, for the caller to show before it happens.
#[derive(Debug, Clone)]
pub struct RollbackPreview {
    /// The snapshot being restored.
    pub target: snapshot::Meta,
    /// The minimal set of operations to get there.
    pub restore: snapshot::RollbackPlan,
    /// Freeze lines no index can restore — editable installs, direct URLs (`PD-SNP-002`).
    ///
    /// Reported rather than dropped: a rollback that silently leaves these behind is a success
    /// message for a restore that did not fully happen.
    pub unrestorable: Vec<String>,
}

/// The rollback flow (DATA-FLOW §8).
pub struct RollbackFlow {
    env: PyEnv,
    engine: Box<dyn Engine>,
    env_hash: String,
    target_id: String,
    restore: snapshot::RollbackPlan,
    /// The state being *replaced*, captured before it is — a rollback is itself reversible.
    pre: SnapshotState,
    cancel: CancellationToken,
}

impl RollbackFlow {
    /// Load the target snapshot and work out the minimal restore plan.
    ///
    /// # Errors
    /// `PD-SNP-002` when no such snapshot exists; otherwise propagates engine failures.
    pub async fn start(
        env: PyEnv,
        engine: Box<dyn Engine>,
        app_data: &Path,
        id: &str,
    ) -> Result<(Self, RollbackPreview)> {
        let env_hash = crate::envs::env_hash(&env.interpreter);
        let target = snapshot::load(app_data, &env_hash, id)?;
        let current = snapshot::parse_freeze(&engine.freeze(&env).await?);
        let diff = snapshot::diff(&current, &target.entries());
        let restore = snapshot::rollback_plan(&diff);

        let preview = RollbackPreview {
            target: target.meta.clone(),
            restore: restore.clone(),
            unrestorable: snapshot::unrestorable_lines(&target.freeze),
        };
        Ok((
            Self {
                env,
                engine,
                env_hash,
                target_id: target.meta.id,
                restore,
                pre: SnapshotState::NotTaken,
                cancel: CancellationToken::new(),
            },
            preview,
        ))
    }

    /// Capture the state being replaced, so the rollback is itself reversible (DATA-FLOW §8).
    ///
    /// # Errors
    /// `PD-SNP-001` when it cannot be written, in which case nothing is restored.
    pub async fn take_snapshot(&mut self, app_data: &Path) -> Result<snapshot::Meta> {
        let snap = snapshot::create(
            app_data,
            &self.env_hash,
            self.engine.freeze(&self.env).await?,
            snapshot::Trigger::Rollback {
                restoring: self.target_id.clone(),
            },
            self.engine.id(),
            jiff::Timestamp::now(),
        )?;
        let meta = snap.meta.clone();
        self.pre = SnapshotState::Taken(Box::new(snap));
        Ok(meta)
    }

    /// Apply the restore.
    ///
    /// # Errors
    /// `PD-SNP-001` when [`Self::take_snapshot`] has not run. Per-package failures appear in the
    /// summary rather than as errors.
    pub async fn execute(&self, tx: EventSink) -> Result<crate::plan::ExecutionSummary> {
        let proof = proof_from(&self.pre)?;
        let sink = ProgressSink::new(tx, self.restore.len(), self.cancel.clone());
        let correlation = match &self.pre {
            SnapshotState::Taken(snap) => snap.meta.id.clone(),
            _ => self.target_id.clone(),
        };
        plan::execute_rollback(
            self.engine.as_ref(),
            &self.env,
            correlation,
            &self.restore,
            proof,
            sink,
        )
        .await
    }

    /// A handle that stops this flow's execution.
    ///
    /// The same contract as [`UpdateFlow::cancel_handle`] and [`UninstallFlow::cancel_handle`],
    /// and the token has been threaded into this flow's `ProgressSink` since it was written — it
    /// simply had no way out, so a GUI rollback could not be stopped. A restore is a two-phase
    /// install of a whole snapshot's worth of packages, which is precisely the operation a user
    /// reaches for Stop during.
    #[must_use]
    pub fn cancel_handle(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Stop this flow. Idempotent.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// The proof that DATA-FLOW §9.2 was satisfied, or the refusal.
///
/// Split out so the refusal is unit-testable: splitting `confirm` into a snapshot call and an
/// execute call is what let the CLI keep printing the snapshot id before execution starts, but it
/// also created a way to skip the snapshot by forgetting. `SnapshotProof` makes that impossible
/// to express inside one function; across two, this is the guard.
/// The guard's half of DATA-FLOW §9.1, or the refusal.
///
/// Beside [`proof_from`] and for the same reason. The guard report is produced by `start` and
/// rendered by the caller, and between those two points nothing stopped a caller from simply
/// running the removal — DATA-FLOW §5's three options are a *dialog*, and a dialog is not an
/// enforcement point. This is.
fn ack_ok(report: &crate::graph::GuardReport, ack: GuardAck) -> Result<()> {
    if report.is_clear() || matches!(ack, GuardAck::ForcedDespiteBreakage) {
        return Ok(());
    }
    Err(PdError::new(
        Code::ResGuardTrip,
        format!(
            "removing this would break {} installed package(s); nothing was removed",
            report.all_broken().len()
        ),
    ))
}

fn proof_from(state: &SnapshotState) -> Result<SnapshotProof<'_>> {
    match state {
        SnapshotState::Taken(snap) => Ok(SnapshotProof::Taken(snap)),
        SnapshotState::Waived => Ok(SnapshotProof::WaivedForDisposableEnvironment),
        SnapshotState::NotTaken => Err(PdError::new(
            Code::SnpWriteFailed,
            "no snapshot was taken or waived for this plan; nothing was executed",
        )),
    }
}

/// Turn an [`Intent`] into a [`PlanRequest`], returning the pins that were excluded.
///
/// # Errors
/// `PD-PKG-002` when a package name or spec does not parse.
fn build_request(
    intent: &Intent,
    outdated: &[OutdatedDist],
    pin_list: &[Pin],
) -> Result<(PlanRequest, Vec<Pin>)> {
    match intent {
        Intent::Update {
            all,
            pkgs,
            except,
            force_latest,
        } => {
            let candidates: Vec<PkgName> = if *all {
                outdated.iter().map(|o| o.name.clone()).collect()
            } else {
                pkgs.iter()
                    .map(|p| PkgName::parse(p))
                    .collect::<Result<_>>()?
            };

            // Ad-hoc exclusions sit on top of pins (CLI-SPEC §3).
            let excluded: BTreeSet<PkgName> = except
                .iter()
                .map(|p| PkgName::parse(p))
                .collect::<Result<_>>()?;
            let candidates: Vec<PkgName> = candidates
                .into_iter()
                .filter(|c| !excluded.contains(c))
                .collect();

            // DATA-FLOW §9.5. Nothing here can put a pinned package into `upgrades`.
            let filtered = pins::filter_upgrades(&candidates, pin_list, &BTreeSet::new());

            Ok((
                PlanRequest {
                    upgrades: filtered.allowed,
                    installs: Vec::new(),
                    strategy: if *force_latest {
                        Strategy::ForceLatest {
                            overrides: Vec::new(),
                        }
                    } else {
                        Strategy::Compatible
                    },
                },
                filtered.excluded,
            ))
        }
        Intent::Install { specs } => Ok((
            PlanRequest {
                upgrades: Vec::new(),
                installs: specs
                    .iter()
                    .map(|raw| {
                        let (name, version_req) = match raw.split_once("==") {
                            Some((n, v)) => (n, Some(v.to_owned())),
                            None => (raw.as_str(), None),
                        };
                        PkgName::parse(name).map(|name| Spec { name, version_req })
                    })
                    .collect::<Result<_>>()?,
                strategy: Strategy::Compatible,
            },
            Vec::new(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Version;

    fn outdated(name: &str, current: &str, latest: &str) -> OutdatedDist {
        OutdatedDist {
            name: PkgName::parse(name).expect("valid name"),
            current: Version(current.into()),
            latest: Version(latest.into()),
        }
    }

    fn pin(name: &str) -> Pin {
        Pin {
            pkg: PkgName::parse(name).expect("valid name"),
            mode: pins::PinMode::Exclude,
            reason: None,
        }
    }

    #[test]
    fn update_all_takes_every_outdated_package() {
        let out = [
            outdated("idna", "3.4", "3.10"),
            outdated("httpx", "0.23", "0.28"),
        ];
        let (req, excluded) = build_request(
            &Intent::Update {
                all: true,
                pkgs: vec![],
                except: vec![],
                force_latest: false,
            },
            &out,
            &[],
        )
        .expect("builds");
        assert_eq!(req.upgrades.len(), 2);
        assert!(excluded.is_empty());
        assert!(matches!(req.strategy, Strategy::Compatible));
    }

    #[test]
    fn a_pinned_package_never_reaches_upgrades() {
        // DATA-FLOW §9.5, and the reason the exclusion is reported rather than silent: a bulk
        // update that quietly skips something looks like a bug to the person who pinned it.
        let out = [
            outdated("idna", "3.4", "3.10"),
            outdated("httpx", "0.23", "0.28"),
        ];
        let (req, excluded) = build_request(
            &Intent::Update {
                all: true,
                pkgs: vec![],
                except: vec![],
                force_latest: false,
            },
            &out,
            &[pin("httpx")],
        )
        .expect("builds");
        assert_eq!(req.upgrades.len(), 1);
        assert_eq!(req.upgrades[0].as_str(), "idna");
        assert_eq!(excluded.len(), 1);
        assert_eq!(excluded[0].pkg.as_str(), "httpx");
    }

    #[test]
    fn except_sits_on_top_of_pins() {
        let out = [
            outdated("idna", "3.4", "3.10"),
            outdated("httpx", "0.23", "0.28"),
        ];
        let (req, _) = build_request(
            &Intent::Update {
                all: true,
                pkgs: vec![],
                except: vec!["idna".into()],
                force_latest: false,
            },
            &out,
            &[],
        )
        .expect("builds");
        assert_eq!(req.upgrades.len(), 1);
        assert_eq!(req.upgrades[0].as_str(), "httpx");
    }

    #[test]
    fn an_install_spec_splits_on_the_version_pin() {
        let (req, _) = build_request(
            &Intent::Install {
                specs: vec!["httpx==0.28.1".into(), "idna".into()],
            },
            &[],
            &[],
        )
        .expect("builds");
        assert_eq!(req.installs.len(), 2);
        assert_eq!(req.installs[0].version_req.as_deref(), Some("0.28.1"));
        assert_eq!(req.installs[1].version_req, None);
    }

    #[test]
    fn a_malformed_name_is_refused_before_any_engine_runs() {
        // SECURITY §2: names are validated before they can reach argv, and both heads inherit
        // that by parsing here rather than at the caller.
        let err = build_request(
            &Intent::Update {
                all: false,
                pkgs: vec!["not a package name".into()],
                except: vec![],
                force_latest: false,
            },
            &[],
            &[],
        )
        .expect_err("must refuse");
        assert_eq!(err.code, Code::PkgNotFound);
    }

    #[test]
    fn executing_without_a_snapshot_is_refused() {
        // DATA-FLOW §9.2 and hard invariant 1: no mutating engine call without a successful
        // snapshot write. Inside one function `SnapshotProof` makes skipping it unrepresentable;
        // split across take_snapshot() and execute(), forgetting becomes expressible, so it has
        // to be refused explicitly — and with PD-SNP-001, the code that already means "nothing
        // was executed".
        let err = proof_from(&SnapshotState::NotTaken).expect_err("must refuse");
        assert_eq!(err.code, Code::SnpWriteFailed);

        // The two legitimate states still produce a proof.
        assert!(matches!(
            proof_from(&SnapshotState::Waived),
            Ok(SnapshotProof::WaivedForDisposableEnvironment)
        ));
    }

    /// The guard's own report, with `broken` dependents of `pkg`.
    fn guard_over(pkg: &str, broken: &[&str]) -> crate::graph::GuardReport {
        let name = |n: &str| PkgName::parse(n).expect("test name");
        let dists: Vec<crate::model::Dist> = std::iter::once(crate::model::Dist {
            name: name(pkg),
            version: Version("1.0".to_owned()),
            requires_dist: Vec::new(),
            requires_python: None,
            size_bytes: None,
        })
        .chain(broken.iter().map(|d| crate::model::Dist {
            name: name(d),
            version: Version("2.0".to_owned()),
            requires_dist: vec![format!("{pkg}>=1")],
            requires_python: None,
            size_bytes: None,
        }))
        .collect();
        ReverseDeps::build(&dists).guard(&[name(pkg)])
    }

    #[test]
    fn a_removal_that_breaks_something_is_refused_unless_it_was_accepted() {
        // The dialog in DATA-FLOW §5 is where the user answers, but a dialog is not an
        // enforcement point: between `start` returning the report and `execute` running the
        // engine, nothing stopped a caller from simply not looking. This is the same guard
        // `proof_from` is for the snapshot half.
        let breaking = guard_over("x", &["y"]);
        assert!(!breaking.is_clear());

        let err = ack_ok(&breaking, GuardAck::Clear).expect_err("must refuse");
        assert_eq!(err.code, Code::ResGuardTrip);
        assert!(
            err.message.contains("nothing was removed"),
            "the message must say the environment is untouched: {}",
            err.message
        );

        // The user was shown what breaks and chose it anyway — §5's *Force remove only X*.
        assert!(ack_ok(&breaking, GuardAck::ForcedDespiteBreakage).is_ok());

        // A clear guard needs no acknowledgement, or every plain removal would demand a force.
        assert!(ack_ok(&guard_over("x", &[]), GuardAck::Clear).is_ok());
    }

    #[test]
    fn force_latest_reaches_the_request() {
        let out = [outdated("idna", "3.4", "3.10")];
        let (req, _) = build_request(
            &Intent::Update {
                all: true,
                pkgs: vec![],
                except: vec![],
                force_latest: true,
            },
            &out,
            &[],
        )
        .expect("builds");
        assert!(matches!(req.strategy, Strategy::ForceLatest { .. }));
    }
}
