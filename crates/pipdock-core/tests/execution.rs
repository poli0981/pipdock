//! Two-phase execution semantics, against a scripted engine.
//!
//! `docs/TESTING.md` §1.4 lists skip-and-continue and summary counts as things that must never
//! regress. Proving them needs an engine whose failures are controllable, so this uses a fake
//! rather than a real venv — the disposable-venv suite (TESTING L2) covers the real thing.
//!
//! What is asserted here is the behaviour a user would notice:
//!
//! - a batch of fifteen with two bad packages applies **thirteen**, not zero;
//! - the headline counts match the rows;
//! - a preview the user approved ten minutes ago, or one made before the environment changed
//!   under them, is refused rather than executed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use pipdock_core::engine::{Engine, InstallSet, ProgressEvent, ProgressSink, ResolveCtx};
use pipdock_core::errors::Result;
use pipdock_core::model::{
    CheckFinding, CheckReport, Dist, EngineId, EngineInfo, EnvSource, ExecMode, OutdatedDist,
    PinnedSpec, PkgName, PyEnv, StepResult, StepStatus, Version,
};
use pipdock_core::plan::{
    AcceptedPlan, Change, ChangeKind, ExecutionSummary, PlanRequest, ResolutionReport, execute,
};
use pipdock_core::snapshot::{self, SnapshotProof, Trigger};

/// One recorded install invocation: the packages it covered, and which phase made it.
type Call = (Vec<String>, ExecMode);

/// How the **batch** install (Phase A) ends, when it does not simply run the packages.
///
/// The real adapters do not return `Ok` when they are stopped: `exec` drops the child and the
/// adapter's `?` turns that into an `Err`. A fake that answered `Ok` on cancel is why
/// `plan::execute`'s cancelled-Phase-A branch passed its test for months while being unreachable
/// in production — every real Stop during a batch surfaced as PD-INT-001.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Batch {
    /// Install what was asked; `failing` decides each package's outcome.
    Runs,
    /// The token was tripped mid-batch: `Err` with `Stop::Cancelled`, as the adapters return.
    Cancelled,
    /// The watchdog fired: `Err` with `Stop::TimedOut` and PD-ENG-004.
    TimedOut,
    /// The engine could not be started at all.
    WontStart,
    /// The batch ran and failed for a reason that is the environment's, not a package's.
    Offline,
}

/// An engine whose per-package outcomes are scripted.
#[derive(Clone)]
struct FakeEngine {
    /// Packages that fail whenever they are installed.
    failing: Vec<String>,
    /// Every install invocation, so phase behaviour can be inspected.
    calls: Arc<Mutex<Vec<Call>>>,
    /// What `check` reports afterwards.
    check: CheckReport,
    /// How Phase A ends.
    batch: Batch,
    /// The constraints each install was given, in call order.
    constraints: Arc<Mutex<Vec<Vec<PinnedSpec>>>>,
    /// `check` cannot run at all — a pip broken by the run it was meant to verify.
    check_breaks: bool,
}

impl FakeEngine {
    fn new(failing: &[&str]) -> Self {
        Self {
            failing: failing.iter().map(|s| (*s).to_owned()).collect(),
            calls: Arc::new(Mutex::new(Vec::new())),
            check: CheckReport {
                ok: true,
                error: None,
                findings: Vec::new(),
            },
            batch: Batch::Runs,
            constraints: Arc::new(Mutex::new(Vec::new())),
            check_breaks: false,
        }
    }

    fn with_batch(mut self, batch: Batch) -> Self {
        self.batch = batch;
        self
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[async_trait]
impl Engine for FakeEngine {
    fn id(&self) -> EngineId {
        EngineId::Pip
    }
    async fn info(&self, _env: &PyEnv) -> EngineInfo {
        EngineInfo {
            id: EngineId::Pip,
            version: Some("26.1.2".into()),
            available: true,
        }
    }
    async fn list_installed(&self, _env: &PyEnv) -> Result<Vec<Dist>> {
        Ok(Vec::new())
    }
    async fn list_outdated(&self, _env: &PyEnv) -> Result<Vec<OutdatedDist>> {
        Ok(Vec::new())
    }
    async fn resolve(
        &self,
        _env: &PyEnv,
        _req: &PlanRequest,
        _ctx: &ResolveCtx,
    ) -> Result<ResolutionReport> {
        Ok(ResolutionReport {
            changes: Vec::new(),
            held_back: Vec::new(),
            impossible: None,
            incompatible: Vec::new(),
            forced: Vec::new(),
            yanked: Vec::new(),
            raw: String::new(),
        })
    }
    async fn install(
        &self,
        _env: &PyEnv,
        set: &InstallSet,
        mode: ExecMode,
        _sink: ProgressSink,
    ) -> Result<StepResult> {
        let specs = &set.specs;
        self.constraints
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(set.constraints.clone());
        let names: Vec<String> = specs.iter().map(|s| s.name.to_string()).collect();
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((names.clone(), mode));

        if mode == ExecMode::Batch {
            use pipdock_core::errors::{Code, PdError};
            use pipdock_core::exec::Stop;
            match self.batch {
                Batch::Runs => {}
                Batch::Cancelled => {
                    return Err(PdError::new(Code::IntUnexpected, "cancelled: python.exe")
                        .with_stop(Stop::Cancelled));
                }
                Batch::TimedOut => {
                    return Err(PdError::new(Code::EngTimedOut, "timed out after 1800s")
                        .with_stop(Stop::TimedOut(std::time::Duration::from_secs(1800))));
                }
                Batch::WontStart => {
                    return Err(PdError::new(Code::EngNotFound, "could not run python.exe"));
                }
                Batch::Offline => {
                    return Ok(StepResult {
                        pkg: specs[0].name.clone(),
                        from: None,
                        to: Some(specs[0].version.clone()),
                        status: StepStatus::Failed,
                        code: Some(Code::NetUnreachable),
                        stderr_tail: Some("Failed to establish a new connection".into()),
                    });
                }
            }
        }

        let bad = names.iter().find(|n| self.failing.contains(n));
        let pkg = specs
            .first()
            .map_or_else(|| PkgName::parse("batch").unwrap(), |s| s.name.clone());
        Ok(match bad {
            None => StepResult {
                pkg,
                from: None,
                to: specs.first().map(|s| s.version.clone()),
                status: StepStatus::Ok,
                code: None,
                stderr_tail: None,
            },
            Some(_) => StepResult {
                pkg,
                from: None,
                to: specs.first().map(|s| s.version.clone()),
                status: StepStatus::Failed,
                code: Some(pipdock_core::errors::Code::BldBackendFailed),
                stderr_tail: Some("metadata-generation-failed".into()),
            },
        })
    }
    async fn uninstall(
        &self,
        _env: &PyEnv,
        names: &[PkgName],
        _sink: ProgressSink,
    ) -> Result<StepResult> {
        // Recorded like installs are. Without this, "the engine was not invoked for the packages
        // after the cancel" is a claim no test can make — and a removal loop that ignores the
        // token still produces a summary that says `cancelled`.
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).push((
            names.iter().map(ToString::to_string).collect(),
            ExecMode::Isolated,
        ));

        let pkg = names
            .first()
            .cloned()
            .unwrap_or_else(|| PkgName::parse("batch").unwrap());
        let failed = self.failing.contains(&pkg.to_string());
        Ok(StepResult {
            pkg,
            from: None,
            to: None,
            status: if failed {
                StepStatus::Failed
            } else {
                StepStatus::Ok
            },
            code: failed.then_some(pipdock_core::errors::Code::PrmFileLocked),
            stderr_tail: None,
        })
    }
    async fn check(&self, _env: &PyEnv) -> Result<CheckReport> {
        if self.check_breaks {
            return Err(pipdock_core::errors::PdError::new(
                pipdock_core::errors::Code::EngNotFound,
                "No module named pip",
            ));
        }
        Ok(self.check.clone())
    }
    async fn freeze(&self, _env: &PyEnv) -> Result<String> {
        Ok("idna==3.4\n".to_owned())
    }
    async fn upgrade_pip(&self, _env: &PyEnv) -> Result<StepResult> {
        unreachable!("not exercised")
    }
}

fn env() -> PyEnv {
    PyEnv {
        interpreter: "python.exe".into(),
        prefix: "prefix".into(),
        python_version: "3.12.4".into(),
        externally_managed: false,
        hidden_user_site: None,
        source: EnvSource::Manual,
    }
}

fn report(packages: &[(&str, &str)]) -> ResolutionReport {
    ResolutionReport {
        changes: packages
            .iter()
            .map(|(n, v)| Change {
                name: PkgName::parse(n).unwrap(),
                // An upgrade from 0.9 for every package whose name starts with "up", so a test can
                // tell whether the summary carried the version it moved *from*.
                from: n.starts_with("up").then(|| Version("0.9".to_owned())),
                to: Version((*v).to_owned()),
                kind: if n.starts_with("up") {
                    ChangeKind::Upgrade
                } else {
                    ChangeKind::NewInstall
                },
            })
            .collect(),
        held_back: Vec::new(),
        impossible: None,
        incompatible: Vec::new(),
        forced: Vec::new(),
        yanked: Vec::new(),
        raw: String::new(),
    }
}

/// The identity of [`env`], which a plan is now verified against.
fn env_hash() -> String {
    pipdock_core::envs::env_hash(std::path::Path::new("python.exe"))
}

fn now() -> jiff::Timestamp {
    "2026-07-27T12:00:00Z".parse().unwrap()
}

fn scratch() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "pd-exec-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ))
}

fn snapshot_for(dir: &std::path::Path) -> snapshot::Snapshot {
    snapshot::create(
        dir,
        "envhash01",
        "idna==3.4\n".to_owned(),
        Trigger::Manual,
        EngineId::Pip,
        now(),
    )
    .expect("snapshot")
}

fn sink() -> ProgressSink {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    // Dropping the receiver would make every send fail; keeping it alive proves the executor does
    // not depend on anyone listening either way.
    std::mem::forget(rx);
    ProgressSink::new(tx, 0, tokio_util::sync::CancellationToken::new())
}

async fn run(engine: &FakeEngine, packages: &[(&str, &str)]) -> ExecutionSummary {
    run_with(engine, packages, sink()).await
}

async fn run_with(
    engine: &FakeEngine,
    packages: &[(&str, &str)],
    sink: ProgressSink,
) -> ExecutionSummary {
    let dir = scratch();
    let _ = std::fs::remove_dir_all(&dir);
    let snap = snapshot_for(&dir);
    let plan = AcceptedPlan::accept(
        "plan-1".to_owned(),
        report(packages),
        env_hash(),
        &[],
        now(),
    );
    let summary = execute(
        engine,
        &env(),
        &plan,
        SnapshotProof::Taken(&snap),
        &[],
        now(),
        sink,
    )
    .await
    .expect("execute");
    let _ = std::fs::remove_dir_all(&dir);
    summary
}

/// Run and collect every `plan-progress` event, rather than discarding them.
async fn run_collecting(
    engine: &FakeEngine,
    packages: &[(&str, &str)],
) -> (ExecutionSummary, Vec<ProgressEvent>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let sink = ProgressSink::new(tx, 0, tokio_util::sync::CancellationToken::new());
    let summary = run_with(engine, packages, sink).await;

    let mut events = Vec::new();
    // The sender is dropped with the sink inside `run_with`, so this drains and stops.
    while let Some(e) = rx.recv().await {
        events.push(e);
    }
    (summary, events)
}

#[tokio::test]
async fn every_step_is_opened_and_closed_exactly_once() {
    // The property the console drawer's sections and the "13 of 15 complete" live region both
    // stand on. Neither can be recovered from the output text, which is why the payload became a
    // lifecycle: a section that is never closed stays open forever, and a counter that misses a
    // step never reaches its total.
    let engine = FakeEngine::new(&["badone"]);
    let (summary, events) = run_collecting(
        &engine,
        &[("good", "1.0"), ("badone", "1.0"), ("other", "2.0")],
    )
    .await;

    // Phase A fails, so this is the isolated pass: one step per package, plus Phase A's own.
    let started: Vec<usize> = events
        .iter()
        .filter_map(|e| match e {
            ProgressEvent::StepStarted { step, .. } => Some(*step),
            _ => None,
        })
        .collect();
    let finished: Vec<usize> = events
        .iter()
        .filter_map(|e| match e {
            ProgressEvent::StepFinished { step, .. } => Some(*step),
            _ => None,
        })
        .collect();

    assert_eq!(started, finished, "every step opened must also be closed");
    assert_eq!(
        finished.len(),
        summary.results.len() + 1,
        "one marker pair per package, plus Phase A's"
    );

    // A failed step still closes, carrying the status the summary reports.
    let closes: Vec<(String, StepStatus)> = events
        .iter()
        .filter_map(|e| match e {
            ProgressEvent::StepFinished {
                pkg: Some(p),
                status,
                ..
            } => Some((p.as_str().to_owned(), *status)),
            _ => None,
        })
        .collect();
    assert!(
        closes.contains(&("badone".to_owned(), StepStatus::Failed)),
        "the failing package must still close its section: {closes:?}"
    );
}

#[tokio::test]
async fn a_clean_batch_finishes_in_phase_a() {
    let engine = FakeEngine::new(&[]);
    let summary = run(&engine, &[("idna", "3.18"), ("certifi", "2025.1.1")]).await;

    assert_eq!(summary.phase, ExecMode::Batch);
    assert_eq!(summary.counts.ok, 2);
    assert_eq!(summary.counts.failed, 0);
    // One invocation for the whole set — the point of Phase A.
    assert_eq!(engine.calls().len(), 1);
    assert_eq!(engine.calls()[0].1, ExecMode::Batch);
}

#[tokio::test]
async fn thirteen_of_fifteen_still_apply_when_two_packages_fail() {
    // The owner requirement, and DATA-FLOW §6's worked example. Aborting the batch would cost the
    // user thirteen good packages to punish two bad ones.
    let mut packages: Vec<(String, String)> = (0..13)
        .map(|i| (format!("good{i}"), "1.0".to_owned()))
        .collect();
    packages.push(("badone".to_owned(), "1.0".to_owned()));
    packages.push(("badtwo".to_owned(), "1.0".to_owned()));
    let refs: Vec<(&str, &str)> = packages
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();

    let engine = FakeEngine::new(&["badone", "badtwo"]);
    let summary = run(&engine, &refs).await;

    assert_eq!(
        summary.phase,
        ExecMode::Isolated,
        "must fall through to Phase B"
    );
    assert_eq!(summary.counts.ok, 13);
    assert_eq!(summary.counts.failed, 2);
    assert_eq!(summary.results.len(), 15, "every package gets a row");

    // Phase B visits every package, including the ones after the first failure.
    let isolated = engine
        .calls()
        .iter()
        .filter(|(_, mode)| *mode == ExecMode::Isolated)
        .count();
    assert_eq!(isolated, 15, "the loop must not stop at the first failure");
}

#[tokio::test]
async fn every_failure_carries_a_catalog_code() {
    // DATA-FLOW §9.4. A failure the UI cannot label is a failure the user cannot act on.
    let engine = FakeEngine::new(&["oldlib"]);
    let summary = run(&engine, &[("httpx", "0.28.1"), ("oldlib", "2.0.0")]).await;

    for row in summary
        .results
        .iter()
        .filter(|r| r.status == StepStatus::Failed)
    {
        assert!(row.code.is_some(), "{} failed without a code", row.pkg);
    }
    assert_eq!(summary.counts.failed, 1);
}

#[tokio::test]
async fn the_headline_counts_match_the_rows() {
    // The summary sheet shows counts, the expandable list shows rows; a mismatch between them is
    // exactly the bug TESTING §1.4 says must never regress.
    let engine = FakeEngine::new(&["badone"]);
    let summary = run(&engine, &[("a", "1.0"), ("badone", "1.0"), ("c", "1.0")]).await;

    let recomputed = ExecutionSummary::tally(&summary.results);
    assert_eq!(summary.counts, recomputed);
    assert_eq!(
        summary.counts.ok + summary.counts.failed + summary.counts.skipped,
        summary.results.len()
    );
}

#[tokio::test]
async fn an_empty_plan_executes_nothing() {
    let engine = FakeEngine::new(&[]);
    let summary = run(&engine, &[]).await;
    assert!(summary.results.is_empty());
    assert!(
        engine.calls().is_empty(),
        "no engine call for an empty plan"
    );
}

#[tokio::test]
async fn a_stale_preview_is_refused() {
    // DATA-FLOW §9.3: the user approved a preview eleven minutes ago. What it described may no
    // longer be what would happen, and executing something they did not actually see is the
    // failure "preview before touch" exists to prevent.
    let dir = scratch();
    let _ = std::fs::remove_dir_all(&dir);
    let snap = snapshot_for(&dir);
    let plan = AcceptedPlan::accept(
        "plan-1".to_owned(),
        report(&[("idna", "3.18")]),
        env_hash(),
        &[],
        now(),
    );

    let later: jiff::Timestamp = "2026-07-27T12:11:00Z".parse().unwrap();
    let err = execute(
        &FakeEngine::new(&[]),
        &env(),
        &plan,
        SnapshotProof::Taken(&snap),
        &[],
        later,
        sink(),
    )
    .await
    .expect_err("a stale plan must be refused");

    assert_eq!(err.code.as_str(), "PD-RES-002");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_preview_made_before_the_environment_changed_is_refused() {
    // Same section: something else installed a package while the preview sat on screen.
    let dir = scratch();
    let _ = std::fs::remove_dir_all(&dir);
    let snap = snapshot_for(&dir);
    let plan = AcceptedPlan::accept(
        "plan-1".to_owned(),
        report(&[("idna", "3.18")]),
        env_hash(),
        &[],
        now(),
    );

    let drifted = [Dist {
        name: PkgName::parse("something-else").unwrap(),
        version: Version("1.0".into()),
        requires_dist: Vec::new(),
        requires_python: None,
        size_bytes: None,
    }];
    let err = execute(
        &FakeEngine::new(&[]),
        &env(),
        &plan,
        SnapshotProof::Taken(&snap),
        &drifted,
        now(),
        sink(),
    )
    .await
    .expect_err("drift must be refused");

    assert_eq!(err.code.as_str(), "PD-RES-002");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn reordering_the_installed_set_is_not_drift() {
    // Engines do not promise a stable listing order, and treating a reorder as drift would refuse
    // valid plans at random.
    let a = Dist {
        name: PkgName::parse("aaa").unwrap(),
        version: Version("1.0".into()),
        requires_dist: Vec::new(),
        requires_python: None,
        size_bytes: None,
    };
    let b = Dist {
        name: PkgName::parse("bbb").unwrap(),
        version: Version("2.0".into()),
        requires_dist: Vec::new(),
        requires_python: None,
        size_bytes: None,
    };
    let plan = AcceptedPlan::accept(
        "plan-1".to_owned(),
        report(&[("idna", "3.18")]),
        env_hash(),
        &[a.clone(), b.clone()],
        now(),
    );
    assert!(plan.verify(&env_hash(), &[b, a], now()).is_ok());
}

#[tokio::test]
async fn post_check_findings_reach_the_summary() {
    // DATA-FLOW §3: the check runs after execution and its findings are appended, so a batch that
    // "succeeded" but left the environment broken still says so.
    let mut engine = FakeEngine::new(&[]);
    engine.check = CheckReport {
        ok: false,
        findings: vec![CheckFinding {
            pkg: PkgName::parse("apiclient").unwrap(),
            requirement: "apiclient 1.4 requires requests<2.31, but you have requests 2.32.3"
                .into(),
        }],
        error: None,
    };
    let summary = run(&engine, &[("requests", "2.32.3")]).await;

    assert!(!summary.check.ok);
    assert_eq!(summary.check.findings.len(), 1);
    assert_eq!(summary.counts.ok, 1, "the install itself still succeeded");
}

#[tokio::test]
async fn a_cancelled_run_skips_the_rest_rather_than_isolating_them() {
    // The trap this guards: Phase A fails, and the cancelled flag is only checked inside Phase B.
    // Phase B would then re-run every package the user just stopped, one at a time — the exact
    // opposite of what cancelling means, and slower than the batch they cancelled.
    let packages = [("alpha", "1.0"), ("badone", "1.0"), ("gamma", "1.0")];
    let engine = FakeEngine::new(&["badone"]);

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    std::mem::forget(rx);
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    let summary = run_with(&engine, &packages, ProgressSink::new(tx, 0, token)).await;

    assert!(summary.cancelled, "the summary must say it was cancelled");
    assert_eq!(
        summary.phase,
        ExecMode::Batch,
        "a cancelled Phase A must not fall through to Phase B"
    );
    assert_eq!(summary.counts.skipped, 3, "{:?}", summary.results);
    assert_eq!(summary.counts.ok, 0);
    assert_eq!(
        summary.counts.failed, 0,
        "cancelling is not a package failure"
    );
}

#[tokio::test]
async fn a_batch_stopped_by_the_user_is_a_cancelled_summary_not_an_error() {
    // What the real adapters return when Stop arrives during Phase A — an `Err` with
    // `Stop::Cancelled`, the token already tripped. It used to propagate out of `execute` as
    // PD-INT-001: no summary, no "cancelled", and the rollback offer never drawn.
    let packages = [("alpha", "1.0"), ("beta", "1.0")];
    let engine = FakeEngine::new(&[]).with_batch(Batch::Cancelled);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    std::mem::forget(rx);
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    let summary = run_with(&engine, &packages, ProgressSink::new(tx, 0, token)).await;

    assert!(summary.cancelled);
    assert_eq!(summary.counts.skipped, 2, "{:?}", summary.results);
    assert_eq!(
        summary.counts.failed, 0,
        "cancelling is not a package failure"
    );
    assert_eq!(engine.calls().len(), 1, "nothing ran after the stop");
}

#[tokio::test]
async fn a_batch_watchdog_fails_every_row_with_pd_eng_004_and_does_not_isolate() {
    // A batch that ran 30 minutes and was killed. Retrying each package with its own 30 minutes
    // is not skip-and-continue, it is the same hang N times over.
    let packages = [("alpha", "1.0"), ("beta", "1.0"), ("gamma", "1.0")];
    let engine = FakeEngine::new(&[]).with_batch(Batch::TimedOut);
    let summary = run(&engine, &packages).await;

    assert!(!summary.cancelled, "a watchdog is not the user's cancel");
    assert_eq!(summary.counts.failed, 3, "{:?}", summary.results);
    assert!(
        summary
            .results
            .iter()
            .all(|r| r.code == Some(pipdock_core::errors::Code::EngTimedOut)),
        "{:?}",
        summary.results
    );
    assert_eq!(
        engine.calls().len(),
        1,
        "no per-package retry after a timeout"
    );
}

#[tokio::test]
async fn an_engine_that_will_not_start_is_reported_per_row_not_raised() {
    let packages = [("alpha", "1.0"), ("beta", "1.0")];
    let engine = FakeEngine::new(&[]).with_batch(Batch::WontStart);
    let summary = run(&engine, &packages).await;

    assert_eq!(summary.counts.failed, 2, "{:?}", summary.results);
    assert_eq!(
        summary.results[0].code,
        Some(pipdock_core::errors::Code::EngNotFound)
    );
    assert_eq!(
        engine.calls().len(),
        1,
        "the same missing engine would fail every retry"
    );
}

#[tokio::test]
async fn an_environmental_failure_is_not_retried_one_package_at_a_time() {
    // Offline is offline for every package. Isolating would re-run all N with pip's own retry
    // backoff each — minutes of waiting to learn the same thing N times.
    let packages = [("alpha", "1.0"), ("beta", "1.0"), ("gamma", "1.0")];
    let engine = FakeEngine::new(&[]).with_batch(Batch::Offline);
    let summary = run(&engine, &packages).await;

    assert_eq!(summary.counts.failed, 3, "{:?}", summary.results);
    assert_eq!(summary.phase, ExecMode::Batch);
    assert_eq!(engine.calls().len(), 1);
    // The stderr tail is on the first row only: three copies of one network trace is noise.
    assert!(summary.results[0].stderr_tail.is_some());
    assert!(summary.results[1].stderr_tail.is_none());
}

#[tokio::test]
async fn every_row_says_what_it_moved_from() {
    // `StepResult.from` is documented as "absent for a fresh install", and was absent for every
    // row — so `--json` and the summary sheet called every upgrade a fresh install.
    let summary = run(
        &FakeEngine::new(&[]),
        &[("upgraded", "2.0"), ("fresh", "1.0")],
    )
    .await;
    let from = |name: &str| {
        summary
            .results
            .iter()
            .find(|r| r.pkg.as_str() == name)
            .and_then(|r| r.from.as_ref().map(|v| v.0.clone()))
    };
    assert_eq!(from("upgraded").as_deref(), Some("0.9"));
    assert_eq!(from("fresh"), None);

    // And through the isolated pass, where the engine's own row has no `from` to give.
    let isolated = run(
        &FakeEngine::new(&["badone"]),
        &[("upgraded", "2.0"), ("badone", "1.0")],
    )
    .await;
    assert_eq!(isolated.phase, ExecMode::Isolated);
    assert_eq!(
        isolated
            .results
            .iter()
            .find(|r| r.pkg.as_str() == "upgraded")
            .and_then(|r| r.from.as_ref())
            .map(|v| v.0.as_str()),
        Some("0.9")
    );
}

#[tokio::test]
async fn an_isolated_step_holds_everything_else_where_the_preview_put_it() {
    // SP-1's rule was applied to the resolve and nowhere else: Phase B installed each package as a
    // bare `name==v`, free to move anything as a dependency. Every step now carries the rest of
    // the plan at its planned versions and the untouched packages at their current ones.
    let engine = FakeEngine::new(&["badone"]);
    let installed = [Dist {
        name: PkgName::parse("bystander").unwrap(),
        version: Version("1.5".into()),
        requires_dist: Vec::new(),
        requires_python: None,
        size_bytes: None,
    }];
    let dir = scratch();
    let _ = std::fs::remove_dir_all(&dir);
    let snap = snapshot_for(&dir);
    let plan = AcceptedPlan::accept(
        "plan-1".to_owned(),
        report(&[("alpha", "2.0"), ("badone", "1.0")]),
        env_hash(),
        &installed,
        now(),
    );
    let summary = execute(
        &engine,
        &env(),
        &plan,
        SnapshotProof::Taken(&snap),
        &installed,
        now(),
        sink(),
    )
    .await
    .expect("execute");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(summary.phase, ExecMode::Isolated);

    let rendered = |set: &[PinnedSpec]| -> Vec<String> {
        set.iter().map(PinnedSpec::to_requirement).collect()
    };
    let constraints = engine.constraints.lock().unwrap().clone();
    // Phase A: the plan is the requirement, the bystander is held.
    assert_eq!(rendered(&constraints[0]), ["bystander==1.5"]);
    // Phase B, alpha: badone at its planned version, and the bystander.
    assert_eq!(rendered(&constraints[1]), ["badone==1.0", "bystander==1.5"]);
    // Phase B, badone: alpha at its planned version, and the bystander.
    assert_eq!(rendered(&constraints[2]), ["alpha==2.0", "bystander==1.5"]);
}

#[tokio::test]
async fn a_check_that_could_not_run_is_not_reported_as_passed() {
    // `unwrap_or(ok: true)` said "verified" exactly when nothing was — for instance when the run
    // being checked had broken pip itself.
    let mut engine = FakeEngine::new(&[]);
    engine.check_breaks = true;
    let summary = run(&engine, &[("alpha", "1.0")]).await;
    assert!(!summary.check.ok, "{:?}", summary.check);
    assert_eq!(
        summary.check.error,
        Some(pipdock_core::errors::Code::EngNotFound)
    );
    assert_eq!(summary.counts.ok, 1, "the install itself still succeeded");
}

#[tokio::test]
async fn an_uncancelled_run_does_not_claim_it_was_cancelled() {
    let summary = run(&FakeEngine::new(&[]), &[("alpha", "1.0")]).await;
    assert!(!summary.cancelled);
}

/// Remove `names`, collecting every event, with `token` deciding whether it is cancelled.
async fn remove_collecting(
    engine: &FakeEngine,
    names: &[&str],
    token: tokio_util::sync::CancellationToken,
) -> (ExecutionSummary, Vec<ProgressEvent>) {
    let dir = scratch();
    let _ = std::fs::remove_dir_all(&dir);
    let snap = snapshot_for(&dir);
    let parsed: Vec<PkgName> = names.iter().map(|n| PkgName::parse(n).unwrap()).collect();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let summary = pipdock_core::plan::execute_uninstall(
        engine,
        &env(),
        "uninstall-envhash".to_owned(),
        &parsed,
        SnapshotProof::Taken(&snap),
        ProgressSink::new(tx, parsed.len(), token),
        0,
    )
    .await
    .expect("execute_uninstall");
    let _ = std::fs::remove_dir_all(&dir);

    let mut events = Vec::new();
    while let Some(e) = rx.recv().await {
        events.push(e);
    }
    (summary, events)
}

#[tokio::test]
async fn a_rollback_stops_when_the_token_is_tripped() {
    // `RollbackFlow` held a token from the day it was written and exposed no way to trip it, so a
    // GUI restore could not be stopped — a two-phase install of a whole snapshot, which is exactly
    // when a user reaches for Stop. Unlike `UninstallFlow::start`, this one never probes, so a
    // fake engine and a temp directory drive the whole flow with no interpreter involved.
    let dir = scratch();
    let _ = std::fs::remove_dir_all(&dir);
    let engine = FakeEngine::new(&[]);
    // A snapshot whose freeze differs from what the engine reports now, so there is work to do.
    let snap = snapshot::create(
        &dir,
        &pipdock_core::envs::env_hash(std::path::Path::new("python.exe")),
        "idna==3.4\nrequests==2.0.0\n".to_owned(),
        Trigger::Manual,
        EngineId::Pip,
        now(),
    )
    .expect("snapshot");

    let (mut flow, preview) = pipdock_core::flow::RollbackFlow::start(
        env(),
        Box::new(engine.clone()),
        &dir,
        &snap.meta.id,
        pipdock_core::flow::FlowOptions::default(),
    )
    .await
    .expect("start");
    assert!(
        !preview.restore.is_empty(),
        "the fixture must have work to do"
    );

    let token = flow.cancel_handle();
    token.cancel();
    flow.take_snapshot(&dir)
        .await
        .expect("pre-rollback snapshot");

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    std::mem::forget(rx);
    let before = engine.calls().len();
    let summary = flow.execute(tx).await.expect("execute");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(summary.cancelled, "the summary must say it was cancelled");
    assert_eq!(
        engine.calls().len(),
        before,
        "a cancelled restore must not install anything: {:?}",
        engine.calls()
    );
}

#[tokio::test]
async fn a_removal_opens_and_closes_a_step_per_package() {
    // Removals emitted neither marker and every line they produced carried `step: 0`. The CLI
    // never noticed, because it prints `event.line()` and nothing else — but the console drawer
    // groups on StepStarted and the live region counts StepFinished, so the GUI would have shown
    // an empty drawer and a counter stuck at zero, against a fully green suite.
    let engine = FakeEngine::new(&["locked"]);
    let (summary, events) = remove_collecting(
        &engine,
        &["alpha", "locked", "gamma"],
        tokio_util::sync::CancellationToken::new(),
    )
    .await;

    let started: Vec<usize> = events
        .iter()
        .filter_map(|e| match e {
            ProgressEvent::StepStarted { step, .. } => Some(*step),
            _ => None,
        })
        .collect();
    let finished: Vec<usize> = events
        .iter()
        .filter_map(|e| match e {
            ProgressEvent::StepFinished { step, .. } => Some(*step),
            _ => None,
        })
        .collect();

    assert_eq!(started, [0, 1, 2], "one open per package, in order");
    assert_eq!(finished, [0, 1, 2], "and one close each, however it ended");
    assert_eq!(summary.counts.ok, 2);
    assert_eq!(
        summary.counts.failed, 1,
        "a locked file is still that package's failure"
    );
}

#[tokio::test]
async fn a_cancelled_removal_leaves_the_rest_installed() {
    // The dangerous shape: the loop had no token check at all, so a cancel removed every package
    // and *then* reported `cancelled: true`. The summary looked right and the environment was
    // gone. Removals are fast, which is exactly why the window matters.
    let engine = FakeEngine::new(&[]);
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    let (summary, _) = remove_collecting(&engine, &["alpha", "beta", "gamma"], token).await;

    assert!(summary.cancelled);
    assert_eq!(summary.counts.skipped, 3, "{:?}", summary.results);
    assert_eq!(summary.counts.ok, 0);
    assert!(
        engine.calls().is_empty(),
        "the engine must not have been invoked at all: {:?}",
        engine.calls()
    );
}
