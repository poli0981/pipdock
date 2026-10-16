//! The flows, driven end to end against a scripted engine.
//!
//! `execution.rs` drives `plan::execute` directly. Everything a flow decides *before* that — the
//! PEP 668 gate, the drift basis, what reaches the resolver, what a forced package releases — sat
//! behind a free-function probe and could not be reached without a real interpreter, and the bugs
//! in it were not found by a test. `Engine::probe` is the seam; this suite is what it is for.
//!
//! The engine here owns a small world — what is installed, what the index offers, whether the
//! interpreter is externally managed — and installs really change it, so a test can move the
//! environment between two calls the way another terminal would.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use pipdock_core::engine::{Engine, GuardAdjust, InstallSet, ProgressSink, ResolveCtx};
use pipdock_core::envs::ProbeResult;
use pipdock_core::errors::{Code, PdError, Result};
use pipdock_core::exec::Stop;
use pipdock_core::flow::{
    FlowOptions, FlowStep, Intent, NothingReason, RollbackFlow, SnapshotPolicy, UninstallFlow,
    UpdateFlow,
};
use pipdock_core::model::{
    CheckReport, Dist, EngineId, EngineInfo, EnvSource, ExecMode, OutdatedDist, PkgName, PyEnv,
    StepResult, StepStatus, Version,
};
use pipdock_core::plan::{Change, ChangeKind, PlanRequest, ResolutionReport};
use pipdock_core::snapshot::{self, Trigger};

/// One package in the scripted world.
#[derive(Clone)]
struct Installed {
    version: String,
    requires: Vec<String>,
}

/// The environment and index a [`Scripted`] engine answers from.
#[derive(Default)]
struct World {
    installed: BTreeMap<String, Installed>,
    /// The newest release the index offers, per package.
    latest: BTreeMap<String, String>,
    externally_managed: bool,
    /// The resolve blocks until its token is tripped, as a backtracking pip does.
    resolve_hangs: bool,
    /// Every resolve the flow asked for: the request and what it changed about the guard set.
    resolves: Vec<(PlanRequest, GuardAdjust)>,
    /// Every install invocation.
    installs: Vec<InstallSet>,
}

#[derive(Clone, Default)]
struct Scripted(Arc<Mutex<World>>);

impl Scripted {
    fn world(&self) -> MutexGuard<'_, World> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn with(self, name: &str, version: &str, requires: &[&str]) -> Self {
        self.world().installed.insert(
            name.to_owned(),
            Installed {
                version: version.to_owned(),
                requires: requires.iter().map(|r| (*r).to_owned()).collect(),
            },
        );
        self
    }

    fn offering(self, name: &str, latest: &str) -> Self {
        self.world()
            .latest
            .insert(name.to_owned(), latest.to_owned());
        self
    }

    fn version_of(&self, name: &str) -> Option<String> {
        self.world().installed.get(name).map(|i| i.version.clone())
    }
}

fn pkg(name: &str) -> PkgName {
    PkgName::parse(name).unwrap()
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

#[async_trait]
impl Engine for Scripted {
    fn id(&self) -> EngineId {
        EngineId::Pip
    }

    async fn info(&self, _env: &PyEnv) -> EngineInfo {
        EngineInfo {
            id: EngineId::Pip,
            version: Some("26.2.1".into()),
            available: true,
        }
    }

    async fn list_installed(&self, _env: &PyEnv) -> Result<Vec<Dist>> {
        Ok(self.probe(&env()).await?.dists)
    }

    async fn list_outdated(&self, _env: &PyEnv) -> Result<Vec<OutdatedDist>> {
        let world = self.world();
        Ok(world
            .installed
            .iter()
            .filter_map(|(name, i)| {
                let latest = world.latest.get(name)?;
                (*latest != i.version).then(|| OutdatedDist {
                    name: pkg(name),
                    current: Version(i.version.clone()),
                    latest: Version(latest.clone()),
                })
            })
            .collect())
    }

    async fn resolve(
        &self,
        _env: &PyEnv,
        req: &PlanRequest,
        ctx: &ResolveCtx,
    ) -> Result<ResolutionReport> {
        let hangs = {
            let mut world = self.world();
            world.resolves.push((req.clone(), ctx.adjust.clone()));
            world.resolve_hangs
        };
        if hangs {
            ctx.cancel.cancelled().await;
            return Err(
                PdError::new(Code::IntUnexpected, "cancelled: pip").with_stop(Stop::Cancelled)
            );
        }

        // Every upgrade goes to the index's latest; a pin the flow added goes where it says.
        let world = self.world();
        let mut changes = Vec::new();
        for name in &req.upgrades {
            if ctx.adjust.pins.iter().any(|p| &p.name == name) {
                continue;
            }
            let (Some(from), Some(to)) = (
                world.installed.get(name.as_str()),
                world.latest.get(name.as_str()),
            ) else {
                continue;
            };
            changes.push(Change {
                name: name.clone(),
                from: Some(Version(from.version.clone())),
                to: Version(to.clone()),
                kind: ChangeKind::Upgrade,
            });
        }
        for pin in &ctx.adjust.pins {
            changes.push(Change {
                name: pin.name.clone(),
                from: world
                    .installed
                    .get(pin.name.as_str())
                    .map(|i| Version(i.version.clone())),
                to: pin.version.clone(),
                kind: ChangeKind::Upgrade,
            });
        }
        Ok(ResolutionReport {
            changes,
            ..ResolutionReport::default()
        })
    }

    async fn install(
        &self,
        _env: &PyEnv,
        set: &InstallSet,
        mode: ExecMode,
        _sink: ProgressSink,
    ) -> Result<StepResult> {
        let mut world = self.world();
        world.installs.push(set.clone());
        for spec in &set.specs {
            let requires = world
                .installed
                .get(spec.name.as_str())
                .map(|i| i.requires.clone())
                .unwrap_or_default();
            world.installed.insert(
                spec.name.to_string(),
                Installed {
                    version: spec.version.0.clone(),
                    requires,
                },
            );
        }
        let _ = mode;
        Ok(StepResult {
            pkg: set.specs[0].name.clone(),
            from: None,
            to: Some(set.specs[0].version.clone()),
            status: StepStatus::Ok,
            code: None,
            stderr_tail: None,
        })
    }

    async fn uninstall(
        &self,
        _env: &PyEnv,
        names: &[PkgName],
        _sink: ProgressSink,
    ) -> Result<StepResult> {
        let mut world = self.world();
        for name in names {
            world.installed.remove(name.as_str());
        }
        Ok(StepResult {
            pkg: names[0].clone(),
            from: None,
            to: None,
            status: StepStatus::Ok,
            code: None,
            stderr_tail: None,
        })
    }

    async fn check(&self, _env: &PyEnv) -> Result<CheckReport> {
        Ok(CheckReport {
            ok: true,
            findings: Vec::new(),
            error: None,
        })
    }

    async fn freeze(&self, _env: &PyEnv) -> Result<String> {
        Ok(self
            .world()
            .installed
            .iter()
            .map(|(name, i)| format!("{name}=={}\n", i.version))
            .collect())
    }

    async fn upgrade_pip(&self, _env: &PyEnv) -> Result<StepResult> {
        Err(PdError::new(Code::IntUnexpected, "not scripted"))
    }

    async fn probe(&self, env: &PyEnv) -> Result<ProbeResult> {
        let world = self.world();
        Ok(ProbeResult {
            env: PyEnv {
                externally_managed: world.externally_managed,
                ..env.clone()
            },
            dists: world
                .installed
                .iter()
                .map(|(name, i)| Dist {
                    name: pkg(name),
                    version: Version(i.version.clone()),
                    requires_dist: i.requires.clone(),
                    requires_python: None,
                    size_bytes: None,
                })
                .collect(),
            markers: BTreeMap::new(),
        })
    }
}

fn update_all() -> Intent {
    Intent::Update {
        all: true,
        pkgs: Vec::new(),
        except: Vec::new(),
        force_latest: false,
    }
}

/// A scratch app-data directory of this test's own.
fn app_data(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pd-flows-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[tokio::test]
async fn an_update_runs_end_to_end_through_the_seam() {
    let engine = Scripted::default()
        .with("idna", "3.4", &[])
        .offering("idna", "3.10");
    let (mut flow, step) = UpdateFlow::start(
        env(),
        Box::new(engine.clone()),
        &update_all(),
        &[],
        FlowOptions::default(),
    )
    .await
    .expect("start");
    let FlowStep::NeedsConfirm { report } = step else {
        panic!("a clean upgrade needs only a confirm: {step:?}");
    };
    assert_eq!(report.changes.len(), 1);

    let dir = app_data("e2e");
    flow.take_snapshot(SnapshotPolicy::Take, &dir)
        .await
        .expect("snapshot");
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let summary = flow.execute(tx).await.expect("execute");
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(summary.counts.ok, 1);
    assert_eq!(engine.version_of("idna").as_deref(), Some("3.10"));
}

#[tokio::test]
async fn a_stop_during_the_first_resolve_reaches_the_engine() {
    // The first resolve runs inside `start`. The GUI's Stop for that window tripped a token
    // nothing listened to, and pip went on backtracking for up to its ten-minute watchdog (H13).
    let engine = Scripted::default()
        .with("idna", "3.4", &[])
        .offering("idna", "3.10");
    engine.world().resolve_hangs = true;

    let opts = FlowOptions::default();
    let stop = opts.cancel.clone();
    let pressed = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        stop.cancel();
    });
    let started = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        UpdateFlow::start(env(), Box::new(engine.clone()), &update_all(), &[], opts),
    )
    .await
    .expect("Stop must end the resolve, not the watchdog")
    .expect("a stopped resolve is not an error");
    pressed.await.expect("joined");

    assert!(
        matches!(
            started.1,
            FlowStep::Nothing {
                reason: NothingReason::Cancelled
            }
        ),
        "{:?}",
        started.1
    );
    assert!(engine.world().installs.is_empty(), "nothing ran");
}

/// Start an update over `engine`'s world with default options.
async fn start_update(engine: &Scripted, opts: FlowOptions) -> UpdateFlow {
    UpdateFlow::start(env(), Box::new(engine.clone()), &update_all(), &[], opts)
        .await
        .expect("start")
        .0
}

#[tokio::test]
async fn a_preview_confirmed_after_the_environment_changed_is_refused_before_the_snapshot() {
    // C1. Something installs a package from another terminal while the preview is on screen.
    // The check compared the plan's hash with itself and the resolve-time listing with itself,
    // so it could never fire.
    let engine = Scripted::default()
        .with("idna", "3.4", &[])
        .offering("idna", "3.10");
    let mut flow = start_update(&engine, FlowOptions::default()).await;
    let _ = engine.clone().with("six", "1.17.0", &[]);

    let dir = app_data("drift");
    let err = flow
        .take_snapshot(SnapshotPolicy::Take, &dir)
        .await
        .expect_err("a preview of a different environment is refused");
    assert_eq!(err.code, Code::ResPlanStale);
    assert!(
        !dir.exists(),
        "no snapshot is written for a plan that will not run"
    );
    assert!(engine.world().installs.is_empty());
}

static NOW_MS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1_785_000_000_000);

fn test_clock() -> jiff::Timestamp {
    jiff::Timestamp::from_millisecond(NOW_MS.load(std::sync::atomic::Ordering::SeqCst))
        .expect("in range")
}

#[tokio::test]
async fn a_preview_left_on_screen_for_eleven_minutes_is_refused() {
    // C1's other half: `accept` stamped the plan at execute time, so its age was always zero.
    let engine = Scripted::default()
        .with("idna", "3.4", &[])
        .offering("idna", "3.10");
    let opts = FlowOptions {
        clock: test_clock,
        ..FlowOptions::default()
    };
    let mut flow = start_update(&engine, opts).await;
    NOW_MS.fetch_add(11 * 60 * 1000, std::sync::atomic::Ordering::SeqCst);

    let dir = app_data("stale");
    let err = flow
        .take_snapshot(SnapshotPolicy::Waive, &dir)
        .await
        .expect_err("an eleven-minute-old preview is refused, waiver or not");
    assert_eq!(err.code, Code::ResPlanStale);
    assert!(engine.world().installs.is_empty());
}

#[tokio::test]
async fn the_summary_and_its_snapshot_carry_one_plan_id() {
    // M31: the snapshot said "update-<env>" for every plan ever made in an environment, and the
    // summary carried a second, different id — nothing tied a run to the snapshot taken for it.
    let engine = Scripted::default()
        .with("idna", "3.4", &[])
        .offering("idna", "3.10");
    let mut flow = start_update(&engine, FlowOptions::default()).await;
    let dir = app_data("plan-id");
    let meta = flow
        .take_snapshot(SnapshotPolicy::Take, &dir)
        .await
        .expect("snapshot")
        .expect("taken");
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let summary = flow.execute(tx).await.expect("execute");
    let _ = std::fs::remove_dir_all(&dir);

    let Trigger::Plan { plan_id } = &meta.trigger else {
        panic!("a plan's snapshot says so: {:?}", meta.trigger);
    };
    assert_eq!(plan_id, &summary.plan_id);
    assert_eq!(plan_id, flow.plan_id());
}

#[tokio::test]
async fn a_removal_guarded_against_an_older_environment_is_refused() {
    // M44: the guard found nothing that needs `x`. Since then `y`, which does, was installed.
    let engine = Scripted::default().with("x", "1.0", &[]);
    let (mut flow, report) = UninstallFlow::start(
        env(),
        Box::new(engine.clone()),
        &["x".to_owned()],
        FlowOptions::default(),
    )
    .await
    .expect("start");
    assert!(report.is_clear());
    let _ = engine.clone().with("y", "2.0", &["x>=1"]);

    let dir = app_data("guard-drift");
    let err = flow
        .take_snapshot(SnapshotPolicy::Take, &dir)
        .await
        .expect_err("a guard over a different environment is refused");
    assert_eq!(err.code, Code::ResPlanStale);
    assert_eq!(
        engine.version_of("x").as_deref(),
        Some("1.0"),
        "nothing was removed"
    );
}

#[tokio::test]
async fn a_rollback_previewed_against_an_older_environment_is_refused() {
    // M44: the restore plan is a diff against the freeze `start` took; applied to anything else
    // it removes and reinstalls the wrong things.
    let engine = Scripted::default().with("idna", "3.4", &[]);
    let dir = app_data("rollback-drift");
    let hash = pipdock_core::envs::env_hash(std::path::Path::new("python.exe"));
    let target = snapshot::create(
        &dir,
        &hash,
        "idna==3.2
"
        .to_owned(),
        Trigger::Manual,
        EngineId::Pip,
        jiff::Timestamp::now(),
    )
    .expect("target snapshot");
    let (mut flow, preview) = RollbackFlow::start(
        env(),
        Box::new(engine.clone()),
        &dir,
        &target.meta.id,
        FlowOptions::default(),
    )
    .await
    .expect("start");
    assert!(
        !preview.restore.is_empty(),
        "the fixture must have work to do"
    );
    let _ = engine.clone().with("six", "1.17.0", &[]);

    let err = flow
        .take_snapshot(&dir)
        .await
        .expect_err("a restore planned against a different environment is refused");
    assert_eq!(err.code, Code::ResPlanStale);
    assert_eq!(
        snapshot::list(&dir, &hash).expect("list").len(),
        1,
        "only the target exists: no pre-rollback snapshot was written"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
