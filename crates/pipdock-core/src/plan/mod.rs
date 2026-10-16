//! Planning: `PlanRequest` → `ResolutionReport` → `ExecutionSummary`.
//!
//! See `docs/DATA-FLOW.md` §3 for the state machine and §9 for the invariants this module owns.

pub mod preview;

pub use preview::{
    Decision, ForcedPlan, apply_decisions, default_decision, derive_forced, derive_held_back,
    forced_collisions, forced_requirements,
};

use crate::engine::{Engine, InstallSet, ProgressSink, SystemPackages};
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
    /// The installed version of [`Self::by`], when the probe knew it.
    ///
    /// Split out for the same reason [`crate::graph::BrokenDependent`] splits it: the two heads
    /// phrase this differently — the CLI writes `scipy 1.11.4 requires …` itself, the GUI
    /// interpolates `plan.blocker` — and neither can un-join a string Rust joined for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The requirement holding the package back, e.g. `"requests<2.31"`.
    ///
    /// **Data, not a sentence** — hard invariant 4. Until Slice 0 this carried
    /// `"scipy 1.11.4 requires numpy <1.28.0,>=1.21.6"`, a phrase assembled in Rust, which the
    /// GUI then wrapped in `"{{by}} requires {{constraint}}"` and rendered naming the dependent
    /// **twice**. Only the CLI looked right, because it prints this field alone. The shape here
    /// now matches `BrokenDependent.constraint`, which the guard dialog has always composed
    /// correctly.
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

/// A pin the user asked for that cannot install on this environment's Python (`PD-PKG-001`).
///
/// Hard invariant 10 says PipDock enforces `Requires-Python` itself, because the engines disagree:
/// pip refuses such a pin with the same "No matching distribution" it uses for a typo, and uv
/// ignores an upper bound and plans it. Structured, not a sentence (invariant 4): the preview shows
/// the required range against the environment's version, and a catalog cannot un-join a string.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct PythonMismatch {
    /// The package.
    pub pkg: PkgName,
    /// The version that was asked for or planned.
    pub version: Version,
    /// That release's `Requires-Python`, verbatim.
    pub requires_python: String,
    /// The environment's Python, e.g. `"3.12.4"`.
    pub python: String,
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
    /// Pins left out because their `Requires-Python` excludes this environment (`PD-PKG-001`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub incompatible: Vec<PythonMismatch>,
    /// Packages the user chose to **force** past what installed dependents allow, each with what
    /// it will break — the "Will break" section of the preview (DISCLAIMER §2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forced: Vec<HeldBack>,
    /// Yanked releases the plan would install. A warning, not a failure (`PD-PKG-003`, SP-2):
    /// both engines exit 0 and install them. Both adapters parsed these and then dropped them, so
    /// the preview never showed one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub yanked: Vec<crate::engine::parse::YankWarning>,
    /// What the engine wrote while resolving — uv's plan, pip's warnings — for the log ring and
    /// the debug log. **Never on the wire.** It used to be, as pip's whole `--report` document
    /// with every package's README inside it, cloned into each `FlowStep` and serialized over IPC
    /// on every round (33 KB for two packages; megabytes for "update all").
    #[serde(skip)]
    #[schemars(skip)]
    pub raw: String,
    /// What the engine itself said each planned release requires of Python — pip's report carries
    /// it, uv's plan does not. For the flow's own check (hard invariant 10); never on the wire.
    #[serde(skip)]
    #[schemars(skip)]
    pub declared_python: std::collections::BTreeMap<PkgName, String>,
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
        "PinSuggestion" => crate::pins::PinSuggestion,
        "ParsedRequirements" => crate::requirements::ParsedRequirements,
        "CacheUsage" => crate::cache::Usage,
        "CacheTarget" => crate::cache::Target,
        "GuardReport" => crate::graph::GuardReport,
        "DepsGraph" => crate::graph::DepsGraph,
        "Diff" => crate::snapshot::Diff,
        "SnapshotMeta" => crate::snapshot::Meta,
        "EngineInfo" => crate::model::EngineInfo,
        "RollbackPlan" => crate::snapshot::RollbackPlan,
        "RollbackPreview" => crate::flow::RollbackPreview,
        "Hit" => crate::index::Hit,
        "PackageMeta" => crate::index::PackageMeta,
        "Freshness" => crate::index::Freshness,
        "RefreshReport" => crate::index::RefreshReport,
        "ProgressEvent" => crate::engine::ProgressEvent,
        "FlowStep" => crate::flow::FlowStep,
        "Decision" => Decision,
        "Intent" => crate::flow::Intent,
        "Code" => crate::errors::Code,
        // Only `HealthReport`, not its parts: the bindings generator hoists every `$defs` entry
        // into its own TS declaration, so `DeptryIssue` and the rest arrive without being listed.
        // Registering them separately would emit each twice.
        "HealthReport" => crate::health::HealthReport,
        "AuditReport" => crate::audit::AuditReport,
        "FixReport" => crate::health::fix::FixReport,
        "LogTail" => crate::logging::LogTail,
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
    "PinSuggestion",
    "ParsedRequirements",
    "CacheUsage",
    "CacheTarget",
    "GuardReport",
    "DepsGraph",
    "Diff",
    "SnapshotMeta",
    "EngineInfo",
    "RollbackPlan",
    "RollbackPreview",
    "Hit",
    "PackageMeta",
    "Freshness",
    "RefreshReport",
    "ProgressEvent",
    "FlowStep",
    "Decision",
    "Intent",
    "Code",
    "HealthReport",
    "AuditReport",
    "FixReport",
    "LogTail",
];

/// DATA-FLOW §9.3: a report older than this is refused by [`execute`] and must be re-resolved.
pub const PLAN_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// DATA-FLOW §3: after this many conflict-decision rounds the UI requires manual pruning, which
/// prevents decision ping-pong.
pub const MAX_CONFLICT_ROUNDS: u8 = 3;

/// How many times one forced resolve may release guard pins the engine names as colliding with a
/// forced package, before the impossibility is shown as it stands. See
/// [`preview::forced_collisions`].
pub const FORCE_RELEASE_ROUNDS: usize = 3;

/// How many times one resolve may rule out releases whose `Requires-Python` excludes this
/// environment and ask again, before what remains is shown as a warning.
pub const PYTHON_EXCLUDE_ROUNDS: usize = 2;

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
    /// Whether the PEP 668 override applies to the installs this plan runs.
    pub system: SystemPackages,
}

impl AcceptedPlan {
    /// Record the user's confirmation of a report.
    ///
    /// `installed` and `resolved_at` are the **basis the report was made against** — the listing
    /// the preview was resolved over, and when — not the moment of confirmation. The flow used to
    /// pass the current time and the resolve-time listing back to a check against the same
    /// listing, so neither half of §9.3 could ever fire (C1). `id` is the flow's, so the summary
    /// and the snapshot taken for it carry one id (M31).
    #[must_use]
    pub fn accept(
        id: String,
        report: ResolutionReport,
        env_hash: String,
        installed: &[crate::model::Dist],
        resolved_at: jiff::Timestamp,
    ) -> Self {
        Self {
            id,
            report,
            accepted_at: resolved_at,
            env_hash,
            probe_hash: fingerprint(installed),
            system: SystemPackages::Protected,
        }
    }

    /// The same plan, run with the PEP 668 override. Only a flow's preamble decides this.
    #[must_use]
    pub const fn with_system_packages(mut self, system: SystemPackages) -> Self {
        self.system = system;
        self
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
        if env_hash != self.env_hash {
            return Err(PdError::new(
                Code::ResPlanStale,
                "this preview was made for a different environment",
            ));
        }
        verify_basis(self.accepted_at, &self.probe_hash, installed, now)
    }
}

/// DATA-FLOW §9.3's two refusals, for any decision made against a listing of an environment: it
/// is older than [`PLAN_MAX_AGE`], or the environment no longer matches the listing.
///
/// Shared by the three flows. A removal's guard and a rollback's restore plan are decisions made
/// against a listing just as a preview is, and both were executed against whatever the environment
/// had become (M44).
///
/// # Errors
/// `PD-RES-002` in either case.
pub fn verify_basis(
    made_at: jiff::Timestamp,
    fingerprint_then: &str,
    installed_now: &[crate::model::Dist],
    now: jiff::Timestamp,
) -> Result<()> {
    verify_age(made_at, now)?;
    if fingerprint(installed_now) != fingerprint_then {
        return Err(PdError::new(
            Code::ResPlanStale,
            "the environment changed since the preview was made — re-run it",
        ));
    }
    Ok(())
}

/// The age half of [`verify_basis`], for a decision whose drift is checked another way.
///
/// # Errors
/// `PD-RES-002` when `made_at` is more than [`PLAN_MAX_AGE`] before `now`, or after it.
pub fn verify_age(made_at: jiff::Timestamp, now: jiff::Timestamp) -> Result<()> {
    let age = now
        .as_millisecond()
        .saturating_sub(made_at.as_millisecond());
    if age < 0 || u128::try_from(age).unwrap_or(u128::MAX) > PLAN_MAX_AGE.as_millis() {
        return Err(PdError::new(
            Code::ResPlanStale,
            format!(
                "this preview is older than {} minutes — re-run it",
                PLAN_MAX_AGE.as_secs() / 60
            ),
        ));
    }
    Ok(())
}

/// A cheap, order-independent fingerprint of an installed set.
///
/// Order-independent on purpose: engines do not promise a stable listing order, and a reordering
/// is not drift. Only the set of `name==version` pairs matters.
#[must_use]
pub fn fingerprint(installed: &[crate::model::Dist]) -> String {
    let mut pairs: Vec<String> = installed
        .iter()
        .map(|d| format!("{}=={}", d.name, d.version))
        .collect();
    pairs.sort_unstable();

    let mut hex = crate::model::sha256_hex(pairs.join("\n").as_bytes());
    hex.truncate(32);
    hex
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
    // Against the environment actually being changed. This compared the plan's hash with itself.
    plan.verify(&crate::envs::env_hash(&env.interpreter), installed_now, now)?;

    let pinned = plan.report.pinned_set();
    if pinned.is_empty() {
        return Ok(ExecutionSummary {
            plan_id: plan.id.clone(),
            phase: ExecMode::Batch,
            results: Vec::new(),
            check: post_check(engine, env).await,
            counts: Counts::default(),
            cancelled: false,
        });
    }

    // The plan knows its own length; the caller cannot, so it is set here rather than trusted.
    let sink = ProgressSink {
        total: pinned.len(),
        ..sink
    };
    // What each package moves *from*. The engine cannot say — pip's report has no "before" and an
    // adapter runs one command — so every row used to carry `from: None`, and the summary and
    // `--json` called every upgrade a fresh install.
    let before: std::collections::BTreeMap<&crate::model::PkgName, &crate::model::Version> = plan
        .report
        .changes
        .iter()
        .filter_map(|c| c.from.as_ref().map(|v| (&c.name, v)))
        .collect();
    let row = |spec: &PinnedSpec, status: StepStatus| StepResult {
        pkg: spec.name.clone(),
        from: before.get(&spec.name).map(|v| (*v).clone()),
        to: Some(spec.version.clone()),
        status,
        code: None,
        stderr_tail: None,
    };

    // Phase A. The markers are emitted here rather than inside the adapter for the same reason
    // `step` is: an adapter runs one command and cannot know its position in the plan.
    let batch_sink = sink.at(0);
    batch_sink.started(None, ExecMode::Batch);
    // **An `Err` here is an outcome, not a reason to abandon the run.** The adapters return one
    // when the batch is stopped (cancel, watchdog) or cannot start, and the `?` this replaced sent
    // all three out of `execute` as PD-INT-001: no summary, no "cancelled", no rollback offer, and
    // the cancelled-Phase-A branch below unreachable outside a test whose fake returned `Ok`.
    let batch = engine
        .install(
            env,
            &InstallSet {
                specs: pinned.clone(),
                constraints: hold_still(installed_now, &pinned, None),
                system: plan.system,
            },
            ExecMode::Batch,
            batch_sink.clone(),
        )
        .await;
    let stopped_by_user = sink.is_cancelled() || batch.as_ref().is_err_and(PdError::is_cancelled);
    batch_sink.finished(
        None,
        ExecMode::Batch,
        match &batch {
            Ok(step) => step.status,
            Err(_) if stopped_by_user => StepStatus::Skipped,
            Err(_) => StepStatus::Failed,
        },
    );

    // A cancelled Phase A must **not** fall through to Phase B. Isolating would re-run every
    // package the user just stopped, one at a time — the opposite of what cancelling means.
    if stopped_by_user {
        let results: Vec<StepResult> = pinned
            .iter()
            .map(|spec| row(spec, StepStatus::Skipped))
            .collect();
        return Ok(summary(plan, ExecMode::Batch, results, engine, env, true).await);
    }

    let failure = match batch {
        Ok(step) if step.status == StepStatus::Ok => {
            let results: Vec<StepResult> = pinned
                .iter()
                .map(|spec| row(spec, StepStatus::Ok))
                .collect();
            return Ok(summary(plan, ExecMode::Batch, results, engine, env, false).await);
        }
        Ok(step) => (step.code, step.stderr_tail),
        Err(e) => (Some(e.code), e.stderr_tail),
    };

    // Phase B only helps when the failure could be one package's. Offline is offline for every
    // package, a missing engine is missing for every retry, and a batch the watchdog killed would
    // be the same hang N times over — so those fail every row with the batch's code, and the trace
    // is attached once rather than N times.
    if !isolates(failure.0) {
        let results: Vec<StepResult> = pinned
            .iter()
            .enumerate()
            .map(|(i, spec)| StepResult {
                status: StepStatus::Failed,
                code: failure.0,
                stderr_tail: if i == 0 { failure.1.clone() } else { None },
                ..row(spec, StepStatus::Failed)
            })
            .collect();
        return Ok(summary(plan, ExecMode::Batch, results, engine, env, false).await);
    }

    // Phase B: isolate. One failure must not cost the user the other fourteen packages.
    let mut results = Vec::with_capacity(pinned.len());
    for (index, spec) in pinned.iter().enumerate() {
        // Checked before each package so the remaining ones are reported as never attempted,
        // which is what `Skipped` already means. The one in flight when the token tripped is
        // handled below.
        if sink.is_cancelled() {
            results.extend(
                pinned[index..]
                    .iter()
                    .map(|remaining| row(remaining, StepStatus::Skipped)),
            );
            break;
        }

        let step_sink = sink.at(index);
        step_sink.started(Some(spec.name.clone()), ExecMode::Isolated);
        let step = engine
            .install(
                env,
                &InstallSet {
                    specs: vec![spec.clone()],
                    constraints: hold_still(installed_now, &pinned, Some(&spec.name)),
                    system: plan.system,
                },
                ExecMode::Isolated,
                step_sink.clone(),
            )
            .await;

        // A step that failed *because we killed it* is not the package's failure. Reclassifying
        // here rather than inventing a "cancelled" catalog code keeps the summary honest without
        // widening the error catalog: it was not attempted to completion, so it is Skipped.
        if sink.is_cancelled() && step.as_ref().is_err_and(PdError::is_cancelled) {
            step_sink.finished(
                Some(spec.name.clone()),
                ExecMode::Isolated,
                StepStatus::Skipped,
            );
            results.push(row(spec, StepStatus::Skipped));
            continue;
        }

        let result = match step {
            Ok(r) => StepResult {
                from: before.get(&spec.name).map(|v| (*v).clone()),
                ..r
            },
            // An engine that could not even be spawned, or a step its watchdog killed, is still
            // one package's outcome here; aborting would discard the packages that already
            // succeeded.
            Err(e) => StepResult {
                status: StepStatus::Failed,
                code: Some(e.code),
                stderr_tail: e.stderr_tail,
                ..row(spec, StepStatus::Failed)
            },
        };
        // Every StepStarted gets exactly one StepFinished, whichever way the step ended — the
        // drawer closes a section on it and the live region counts it.
        step_sink.finished(Some(spec.name.clone()), ExecMode::Isolated, result.status);
        results.push(result);
    }

    let cancelled = sink.is_cancelled();
    Ok(summary(plan, ExecMode::Isolated, results, engine, env, cancelled).await)
}

/// What must not move while part of a plan installs: every **other** planned package at the
/// version the preview showed, and everything installed that the plan does not touch at the
/// version it has now.
///
/// Phase A passes `except: None` (the whole plan is the requirement); a Phase B step passes its
/// own package. Without this a Phase B `pip install alpha==2` was free to pull a dependency past
/// an installed package's upper bound — pip warns and exits 0 — leaving a state no preview showed.
fn hold_still(
    installed: &[crate::model::Dist],
    pinned: &[PinnedSpec],
    except: Option<&crate::model::PkgName>,
) -> Vec<PinnedSpec> {
    let planned: std::collections::BTreeSet<&crate::model::PkgName> =
        pinned.iter().map(|p| &p.name).collect();
    let others = pinned
        .iter()
        .filter(|p| except.is_some_and(|name| *name != p.name))
        .cloned();
    let untouched = installed
        .iter()
        .filter(|d| !planned.contains(&d.name))
        .filter(|d| crate::pep440::is_argv_safe(&d.version.0))
        .map(|d| PinnedSpec {
            name: d.name.clone(),
            version: d.version.clone(),
        });
    others.chain(untouched).collect()
}

/// Whether retrying package by package could get further than the batch did.
///
/// False for failures that are the **environment's**, not one package's: the network (PD-NET-*),
/// a read-only site-packages, a full disk, an engine that is missing or too old, the watchdog, an
/// environment that is gone or managed. Every per-package retry would meet the same wall — after
/// pip's own retry backoff, each time. A failure with no code isolates, as it always did.
fn isolates(code: Option<Code>) -> bool {
    !matches!(
        code,
        Some(
            Code::NetUnreachable
                | Code::NetTlsFailure
                | Code::PrmSitePackagesReadOnly
                | Code::SysDiskFull
                | Code::EngNotFound
                | Code::EngPipTooOld
                | Code::EngTimedOut
                | Code::EnvInterpreterMissing
                | Code::EnvExternallyManaged
        )
    )
}

/// Assemble a summary: the rows, their tally, and the post-execution check.
async fn summary(
    plan: &AcceptedPlan,
    phase: ExecMode,
    results: Vec<StepResult>,
    engine: &dyn Engine,
    env: &PyEnv,
    cancelled: bool,
) -> ExecutionSummary {
    let counts = ExecutionSummary::tally(&results);
    ExecutionSummary {
        plan_id: plan.id.clone(),
        phase,
        results,
        check: post_check(engine, env).await,
        counts,
        cancelled,
    }
}

/// Run the post-execution check, reporting a check that could not run **as not having run**.
///
/// It must not turn a successful run into a failed one — the summary's rows still carry the
/// per-package truth — but it must not say "verified" either, which `unwrap_or(ok: true)` did:
/// exactly when pip had been broken by the run it was meant to check.
async fn post_check(engine: &dyn Engine, env: &PyEnv) -> CheckReport {
    engine.check(env).await.unwrap_or_else(|e| CheckReport {
        ok: false,
        findings: Vec::new(),
        error: Some(e.code),
    })
}

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
    _snapshot: SnapshotProof<'_>,
    sink: ProgressSink,
    system: SystemPackages,
) -> Result<ExecutionSummary> {
    let mut results = Vec::new();

    // One index space across both halves: the sink's `total` counts the removals and the restores
    // together, so the restores continue where the removals stopped rather than both starting at
    // zero and the live region counting to half.
    //
    // The removals are run as steps, not as a nested `execute_uninstall`: that one ends with its
    // own `pip check`, which here ran between the removals and the restores — a check of a state
    // that exists for a few seconds, against an environment about to change again.
    if !restore.uninstall.is_empty() {
        results.extend(removal_steps(engine, env, &restore.uninstall, &sink, 0, system).await);
    }

    for (index, spec) in restore.install.iter().enumerate() {
        // The restore half needs the same check the removal half has. A rollback is the longest
        // mutating run PipDock performs — a whole snapshot's worth of isolated installs — so it is
        // the one most likely to be stopped, and without this Stop killed the package in flight
        // and then quietly installed every remaining one.
        if sink.is_cancelled() {
            results.extend(restore.install[index..].iter().map(|remaining| StepResult {
                pkg: remaining.name.clone(),
                from: None,
                to: Some(remaining.version.clone()),
                status: StepStatus::Skipped,
                code: None,
                stderr_tail: None,
            }));
            break;
        }

        let step_sink = sink.at(restore.uninstall.len() + index);
        step_sink.started(Some(spec.name.clone()), ExecMode::Isolated);

        // The rest of the restore is the target state, so it is what this step must not move.
        let constraints: Vec<PinnedSpec> = restore
            .install
            .iter()
            .filter(|other| other.name != spec.name)
            .cloned()
            .collect();
        let step = engine
            .install(
                env,
                &InstallSet {
                    specs: vec![spec.clone()],
                    constraints,
                    system,
                },
                ExecMode::Isolated,
                step_sink.clone(),
            )
            .await;

        // Killed by us is not the package's failure — the same reclassification `execute` and
        // `execute_uninstall` make.
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

        let result = step.unwrap_or_else(|e| StepResult {
            pkg: spec.name.clone(),
            from: None,
            to: Some(spec.version.clone()),
            status: StepStatus::Failed,
            code: Some(e.code),
            stderr_tail: e.stderr_tail,
        });
        step_sink.finished(Some(spec.name.clone()), ExecMode::Isolated, result.status);
        results.push(result);
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

/// Remove packages, sequentially, skipping past failures (DATA-FLOW §5).
///
/// The reverse-dependency guard is the caller's job and runs **once against the full set** before
/// this is called; by the time execution starts the user has already decided.
///
/// The removals are the whole plan here, so they are steps `0..names.len()`; only
/// [`execute_rollback`], which installs on top under one sink, starts them anywhere else.
///
/// # Errors
/// Never for a package failure — those land in the summary. Only a stale plan aborts.
pub async fn execute_uninstall(
    engine: &dyn Engine,
    env: &PyEnv,
    plan_id: String,
    names: &[PkgName],
    _snapshot: SnapshotProof<'_>,
    sink: ProgressSink,
    system: SystemPackages,
) -> Result<ExecutionSummary> {
    let results = removal_steps(engine, env, names, &sink, 0, system).await;
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

/// The removal loop shared by [`execute_uninstall`] and [`execute_rollback`]'s first half.
///
/// `base` is where these steps begin in the sink's index space: a rollback removes and then
/// installs under **one** `ProgressSink` whose `total` counts both.
async fn removal_steps(
    engine: &dyn Engine,
    env: &PyEnv,
    names: &[PkgName],
    sink: &ProgressSink,
    base: usize,
    system: SystemPackages,
) -> Vec<StepResult> {
    let mut results = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        // Checked before each removal, so a cancelled run reports the rest as never attempted
        // instead of removing them and then admitting it was cancelled. Removals are fast, which
        // is exactly why the window matters: without this the whole set goes.
        if sink.is_cancelled() {
            results.extend(names[index..].iter().map(|remaining| StepResult {
                pkg: remaining.clone(),
                from: None,
                to: None,
                status: StepStatus::Skipped,
                code: None,
                stderr_tail: None,
            }));
            break;
        }

        // A removal used to emit neither marker, and every line it produced carried `step: 0`.
        // The CLI never noticed because it prints `event.line()` and nothing else, but the console
        // drawer groups on `StepStarted` and the live region counts `StepFinished` — both were
        // unimplementable for uninstall, and would have rendered an empty drawer against a green
        // suite.
        let step_sink = sink.at(base + index);
        step_sink.started(Some(name.clone()), ExecMode::Isolated);

        let step = engine
            .uninstall(env, std::slice::from_ref(name), system, step_sink.clone())
            .await;

        // Killed by us is not the package's failure — the same reclassification the install path
        // makes, and for the same reason: it was not attempted to completion.
        if sink.is_cancelled() && step.is_err() {
            step_sink.finished(Some(name.clone()), ExecMode::Isolated, StepStatus::Skipped);
            results.push(StepResult {
                pkg: name.clone(),
                from: None,
                to: None,
                status: StepStatus::Skipped,
                code: None,
                stderr_tail: None,
            });
            continue;
        }

        let result = match step {
            Ok(r) => r,
            Err(e) => StepResult {
                pkg: name.clone(),
                from: None,
                to: None,
                status: StepStatus::Failed,
                code: Some(e.code),
                stderr_tail: e.stderr_tail,
            },
        };
        step_sink.finished(Some(name.clone()), ExecMode::Isolated, result.status);
        results.push(result);
    }

    results
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
                version: None,
                constraint: "requests<2.31".into(),
            }],
        };
        let report = ResolutionReport {
            changes: vec![],
            held_back: vec![held],
            impossible: None,
            incompatible: Vec::new(),
            forced: Vec::new(),
            yanked: Vec::new(),
            raw: String::new(),
            declared_python: Default::default(),
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
            incompatible: Vec::new(),
            forced: Vec::new(),
            yanked: Vec::new(),
            raw: String::new(),
            declared_python: Default::default(),
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

    #[test]
    fn the_engine_output_never_crosses_the_wire() {
        // pip's whole `--report` — every README in the plan — rode in each FlowStep over IPC and
        // into `--json`. It stays in Rust, for the log.
        let report = ResolutionReport {
            raw: "a very large engine document".into(),
            ..ResolutionReport::default()
        };
        let wire = serde_json::to_value(&report).expect("serializes");
        assert!(wire.get("raw").is_none(), "{wire}");
        let schema = serde_json::to_value(schemars::schema_for!(ResolutionReport))
            .expect("schema serializes");
        assert!(
            schema["properties"].get("raw").is_none(),
            "the schema must not promise it either"
        );
    }
}
