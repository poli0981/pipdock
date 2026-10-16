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
use pipdock_core::engine::{
    Engine, GuardAdjust, InstallSet, ProgressSink, ResolveCtx, SystemPackages,
};
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
use pipdock_core::plan::{
    Change, ChangeKind, Decision, ImpossibleDetail, PlanRequest, ResolutionReport,
};
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
    /// Older releases the index also offers, newest first — what a resolver falls back to when
    /// the latest is ruled out.
    older: BTreeMap<String, Vec<String>>,
    /// Which engine this pretends to be; the flow asks PyPI about uv's plans, not pip's.
    engine: Option<EngineId>,
    /// What a package's newest release needs at least, per dependency: `httpcore` 1.0.9 needs
    /// `h11>=0.16`.
    needs: BTreeMap<String, Vec<(String, String)>>,
    externally_managed: bool,
    /// The resolve blocks until its token is tripped, as a backtracking pip does.
    resolve_hangs: bool,
    /// Every resolve the flow asked for.
    resolves: Vec<ResolveCall>,
    /// Every install invocation.
    installs: Vec<InstallSet>,
    /// Every removal: the names, and whether the PEP 668 override applied.
    removals: Vec<(Vec<String>, SystemPackages)>,
}

/// What one resolve was asked: the request, what the flow changed about the guard set, and
/// whether the PEP 668 override applied.
struct ResolveCall {
    req: PlanRequest,
    adjust: GuardAdjust,
    system: SystemPackages,
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

    fn also_offering(self, name: &str, older: &[&str]) -> Self {
        self.world().older.insert(
            name.to_owned(),
            older.iter().map(|v| (*v).to_owned()).collect(),
        );
        self
    }

    fn pretending_to_be(self, id: EngineId) -> Self {
        self.world().engine = Some(id);
        self
    }

    fn needing(self, name: &str, dep: &str, at_least: &str) -> Self {
        self.world()
            .needs
            .entry(name.to_owned())
            .or_default()
            .push((dep.to_owned(), at_least.to_owned()));
        self
    }

    fn version_of(&self, name: &str) -> Option<String> {
        self.world().installed.get(name).map(|i| i.version.clone())
    }
}

fn pkg(name: &str) -> PkgName {
    PkgName::parse(name).unwrap()
}

fn release(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

fn older(a: &str, b: &str) -> bool {
    release(a) < release(b)
}

/// `requirement` names `target` with an upper bound at or below `version` — `httpcore<0.16.0`
/// caps `httpcore` 1.0.9. Just enough of PEP 440 for the scenarios here.
fn caps(requirement: &str, target: &str, version: &str) -> bool {
    let Some(rest) = requirement.strip_prefix(target) else {
        return false;
    };
    rest.split(',')
        .filter_map(|c| c.trim().strip_prefix('<'))
        .any(|bound| !bound.starts_with('=') && !older(version, bound))
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
        self.world().engine.unwrap_or(EngineId::Pip)
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
            world.resolves.push(ResolveCall {
                req: req.clone(),
                adjust: ctx.adjust.clone(),
                system: ctx.system,
            });
            world.resolve_hangs
        };
        if hangs {
            ctx.cancel.cancelled().await;
            return Err(
                PdError::new(Code::IntUnexpected, "cancelled: pip").with_stop(Stop::Cancelled)
            );
        }

        // Two rules of a real resolver, just enough of them. A guard-pinned package that caps an
        // upgrade below its latest holds it where it is (SP-1). A forced pin that needs a newer
        // dependency than the guard set pins is impossible, and the engine names both.
        let world = self.world();
        let pinned: Vec<&str> = ctx.adjust.pins.iter().map(|p| p.name.as_str()).collect();
        let moving = |name: &str| {
            req.upgrades.iter().any(|u| u.as_str() == name)
                || pinned.contains(&name)
                || ctx.adjust.release.iter().any(|r| r.as_str() == name)
        };
        let guard: Vec<(&String, &Installed)> =
            world.installed.iter().filter(|(n, _)| !moving(n)).collect();

        for pin in &ctx.adjust.pins {
            for (dep, min) in world.needs.get(pin.name.as_str()).into_iter().flatten() {
                if let Some((_, held)) = guard.iter().find(|(n, _)| *n == dep)
                    && older(&held.version, min)
                {
                    return Ok(ResolutionReport {
                        impossible: Some(ImpossibleDetail {
                            packages: vec![pin.name.clone(), pkg(dep)],
                            explanation: format!(
                                "Cannot install {}=={} and {dep}=={} because these package \
                                 versions have conflicting dependencies.",
                                pin.name, pin.version, held.version
                            ),
                        }),
                        ..ResolutionReport::default()
                    });
                }
            }
        }

        let excluded = |name: &str, version: &str| {
            ctx.adjust
                .exclude
                .iter()
                .any(|e| e.name.as_str() == name && e.version.0 == version)
        };
        let mut changes = Vec::new();
        for spec in &req.installs {
            let Some(version) = spec
                .version_req
                .as_deref()
                .and_then(|v| v.strip_prefix("=="))
            else {
                continue;
            };
            changes.push(Change {
                name: spec.name.clone(),
                from: None,
                to: Version(version.to_owned()),
                kind: ChangeKind::NewInstall,
            });
        }
        for name in &req.upgrades {
            if pinned.contains(&name.as_str()) {
                continue;
            }
            let Some(from) = world.installed.get(name.as_str()) else {
                continue;
            };
            let candidates = world
                .latest
                .get(name.as_str())
                .into_iter()
                .chain(world.older.get(name.as_str()).into_iter().flatten());
            let Some(to) = candidates.into_iter().find(|v| !excluded(name.as_str(), v)) else {
                continue;
            };
            let capped = guard
                .iter()
                .any(|(_, g)| g.requires.iter().any(|r| caps(r, name.as_str(), to)));
            if capped {
                continue;
            }
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
            // What it needs moved, now that nothing pins it, goes to the index's latest.
            for (dep, min) in world.needs.get(pin.name.as_str()).into_iter().flatten() {
                if let (Some(i), Some(latest)) = (world.installed.get(dep), world.latest.get(dep))
                    && older(&i.version, min)
                {
                    changes.push(Change {
                        name: pkg(dep),
                        from: Some(Version(i.version.clone())),
                        to: Version(latest.clone()),
                        kind: ChangeKind::Upgrade,
                    });
                }
            }
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
        system: SystemPackages,
        _sink: ProgressSink,
    ) -> Result<StepResult> {
        let mut world = self.world();
        world
            .removals
            .push((names.iter().map(ToString::to_string).collect(), system));
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

fn managed(engine: Scripted) -> Scripted {
    engine.world().externally_managed = true;
    engine
}

#[tokio::test]
async fn a_managed_environment_is_refused_on_what_the_probe_says_not_what_the_caller_sent() {
    // H12. The caller's PyEnv says "not managed" — a value cached from some earlier scan. The
    // gate trusted it and ran the resolve; the probe it does first now decides.
    let engine = managed(
        Scripted::default()
            .with("idna", "3.4", &[])
            .offering("idna", "3.10"),
    );
    assert!(!env().externally_managed, "the caller believes otherwise");
    let Err(err) = UpdateFlow::start(
        env(),
        Box::new(engine.clone()),
        &update_all(),
        &[],
        FlowOptions::default(),
    )
    .await
    else {
        panic!("a managed environment must be refused");
    };
    assert_eq!(err.code, Code::EnvExternallyManaged);
    assert!(
        engine.world().resolves.is_empty(),
        "refused before the engine ran"
    );
}

#[tokio::test]
async fn a_rollback_into_a_managed_environment_needs_the_override_too() {
    // H12: the rollback flow had no PEP 668 gate at all.
    let engine = managed(Scripted::default().with("idna", "3.4", &[]));
    let dir = app_data("rollback-managed");
    let hash = pipdock_core::envs::env_hash(std::path::Path::new("python.exe"));
    let target = snapshot::create(
        &dir,
        &hash,
        "idna==3.2\n".to_owned(),
        Trigger::Manual,
        EngineId::Pip,
        jiff::Timestamp::now(),
    )
    .expect("target snapshot");
    let refused = RollbackFlow::start(
        env(),
        Box::new(engine.clone()),
        &dir,
        &target.meta.id,
        FlowOptions::default(),
    )
    .await;
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        refused.err().map(|e| e.code),
        Some(Code::EnvExternallyManaged)
    );
}

#[tokio::test]
async fn with_the_override_the_flag_reaches_every_mutating_call() {
    // H11: the Settings checkbox saved and nothing read it. With it on, a managed environment's
    // resolve, installs and removals all carry the override.
    let engine = managed(
        Scripted::default()
            .with("idna", "3.4", &[])
            .offering("idna", "3.10"),
    );
    let opts = FlowOptions {
        allow_externally_managed: true,
        ..FlowOptions::default()
    };
    let mut flow = start_update(&engine, opts.clone()).await;
    assert!(flow.overrides_pep668());
    let dir = app_data("override");
    flow.take_snapshot(SnapshotPolicy::Take, &dir)
        .await
        .expect("snapshot");
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    flow.execute(tx).await.expect("execute");

    let (mut removal, _) =
        UninstallFlow::start(env(), Box::new(engine.clone()), &["idna".to_owned()], opts)
            .await
            .expect("start");
    removal
        .take_snapshot(SnapshotPolicy::Take, &dir)
        .await
        .expect("snapshot");
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    removal
        .execute(pipdock_core::flow::GuardAck::Clear, tx)
        .await
        .expect("execute");
    let _ = std::fs::remove_dir_all(&dir);

    let world = engine.world();
    assert!(
        world
            .resolves
            .iter()
            .all(|r| r.system == SystemPackages::OverrideAllowed)
    );
    assert!(!world.installs.is_empty());
    assert!(
        world
            .installs
            .iter()
            .all(|i| i.system == SystemPackages::OverrideAllowed)
    );
    assert_eq!(
        world.removals,
        [(vec!["idna".to_owned()], SystemPackages::OverrideAllowed)]
    );
}

#[tokio::test]
async fn an_unmanaged_environment_never_gets_the_flag_even_with_the_override_on() {
    // Hard invariant 5: the override is permission, not an instruction. Without the marker the
    // flag is never passed.
    let engine = Scripted::default()
        .with("idna", "3.4", &[])
        .offering("idna", "3.10");
    let opts = FlowOptions {
        allow_externally_managed: true,
        ..FlowOptions::default()
    };
    let mut flow = start_update(&engine, opts).await;
    assert!(!flow.overrides_pep668());
    let dir = app_data("no-override");
    flow.take_snapshot(SnapshotPolicy::Waive, &dir)
        .await
        .expect("waived");
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    flow.execute(tx).await.expect("execute");

    let world = engine.world();
    assert!(
        world
            .resolves
            .iter()
            .all(|r| r.system == SystemPackages::Protected)
    );
    assert!(
        world
            .installs
            .iter()
            .all(|i| i.system == SystemPackages::Protected)
    );
}

#[tokio::test]
async fn force_latest_forces_and_names_what_it_breaks() {
    // C6, end to end on the SP-1 scenario. httpx 0.23 caps httpcore below 0.16, so httpcore is
    // held back, and the user forces it. Forcing has to pin httpcore to its latest, release httpx
    // from the guard set — httpx is what breaks — and let h11 move, which httpcore 1.0.9 needs at
    // >=0.16 and the guard set had pinned at 0.12.0. Before 1.4.0 the decision was recorded and
    // nothing read it: the "forced" resolve was the held-back one again.
    let engine = Scripted::default()
        .with("httpx", "0.23.0", &["httpcore<0.16.0,>=0.15.0"])
        .with("httpcore", "0.15.0", &["h11<0.13,>=0.11"])
        .with("h11", "0.12.0", &[])
        .offering("httpcore", "1.0.9")
        .offering("h11", "0.16.0")
        .needing("httpcore", "h11", "0.16");
    let intent = Intent::Update {
        all: false,
        pkgs: vec!["httpcore".to_owned()],
        except: Vec::new(),
        force_latest: false,
    };
    let (mut flow, step) = UpdateFlow::start(
        env(),
        Box::new(engine.clone()),
        &intent,
        &[],
        FlowOptions::default(),
    )
    .await
    .expect("start");
    let FlowStep::NeedsDecisions { report, .. } = step else {
        panic!("httpx holds httpcore back: {step:?}");
    };
    assert_eq!(report.held_back.len(), 1);

    let step = flow
        .decide(&BTreeMap::from([(pkg("httpcore"), Decision::ForceLatest)]))
        .await
        .expect("decide");
    let FlowStep::NeedsConfirm { report } = step else {
        panic!("a forced plan is confirmable: {step:?}");
    };
    let moved: BTreeMap<String, String> = report
        .changes
        .iter()
        .map(|c| (c.name.to_string(), c.to.0.clone()))
        .collect();
    assert_eq!(moved.get("httpcore").map(String::as_str), Some("1.0.9"));
    assert_eq!(
        moved.get("h11").map(String::as_str),
        Some("0.16.0"),
        "the pin the engine named as colliding was released"
    );
    assert!(report.held_back.is_empty());

    // "Will break": httpx, by name, the package whose declared constraint was overridden.
    assert_eq!(report.forced.len(), 1);
    let breaks: Vec<String> = report.forced[0]
        .blockers
        .iter()
        .filter_map(|b| b.by.as_ref().map(ToString::to_string))
        .collect();
    assert_eq!(breaks, ["httpx"]);

    let world = engine.world();
    let last = &world.resolves.last().expect("resolved").adjust;
    assert!(last.release.contains(&pkg("httpx")));
    assert!(last.release.contains(&pkg("h11")));
}

/// `Requires-Python` per release, as PyPI would answer; anything absent is unknown.
#[derive(Debug, Default)]
struct Releases {
    known: BTreeMap<(String, String), String>,
    asked: std::sync::atomic::AtomicUsize,
}

impl Releases {
    fn with(mut self, name: &str, version: &str, requires: &str) -> Self {
        self.known
            .insert((name.to_owned(), version.to_owned()), requires.to_owned());
        self
    }
}

#[async_trait]
impl pipdock_core::release::ReleaseMeta for Releases {
    async fn requires_python(&self, name: &PkgName, version: &Version) -> Option<String> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.known
            .get(&(name.to_string(), version.0.clone()))
            .cloned()
    }
}

fn checking(releases: &Arc<Releases>) -> FlowOptions {
    FlowOptions {
        releases: Some(Arc::clone(releases) as Arc<dyn pipdock_core::release::ReleaseMeta>),
        ..FlowOptions::default()
    }
}

fn install(specs: &[&str]) -> Intent {
    Intent::Install {
        specs: specs.iter().map(|s| (*s).to_owned()).collect(),
    }
}

#[tokio::test]
async fn an_exact_pin_this_python_cannot_install_is_pd_pkg_001_before_any_engine_runs() {
    // C2. pip answered this with the "No matching distribution" it gives a typo, and uv planned
    // it and failed at install time, building scipy from source. PD-PKG-001 existed in the
    // catalog and nothing ever raised it.
    let releases = Arc::new(Releases::default().with("scipy", "1.7.3", ">=3.7,<3.10"));
    for engine_id in [EngineId::Pip, EngineId::Uv] {
        let engine = Scripted::default().pretending_to_be(engine_id);
        let Err(err) = UpdateFlow::start(
            env(),
            Box::new(engine.clone()),
            &install(&["scipy==1.7.3"]),
            &[],
            checking(&releases),
        )
        .await
        else {
            panic!("{engine_id:?}: an install this Python cannot run must be refused");
        };
        assert_eq!(err.code, Code::PkgRequiresPython, "{engine_id:?}");
        assert!(err.message.contains(">=3.7,<3.10") && err.message.contains("3.12.4"));
        assert!(
            engine.world().resolves.is_empty(),
            "refused before the engine ran"
        );
    }
}

#[tokio::test]
async fn one_pin_this_python_cannot_install_does_not_cost_the_others() {
    let releases = Arc::new(Releases::default().with("scipy", "1.7.3", ">=3.7,<3.10"));
    let engine = Scripted::default();
    let (_, step) = UpdateFlow::start(
        env(),
        Box::new(engine.clone()),
        &install(&["scipy==1.7.3", "idna==3.10"]),
        &[],
        checking(&releases),
    )
    .await
    .expect("start");
    let FlowStep::NeedsConfirm { report } = step else {
        panic!("the rest of the request is confirmable: {step:?}");
    };
    let planned: Vec<String> = report.changes.iter().map(|c| c.name.to_string()).collect();
    assert_eq!(planned, ["idna"]);
    assert_eq!(report.incompatible.len(), 1);
    assert_eq!(report.incompatible[0].pkg.as_str(), "scipy");
    assert_eq!(report.incompatible[0].requires_python, ">=3.7,<3.10");
    let world = engine.world();
    let asked: Vec<String> = world.resolves[0]
        .req
        .installs
        .iter()
        .map(|s| s.name.to_string())
        .collect();
    assert_eq!(asked, ["idna"], "scipy never reached the engine");
}

#[tokio::test]
async fn a_release_uv_planned_past_its_python_cap_is_ruled_out_and_the_engine_asked_again() {
    // uv honours a lower Requires-Python bound and ignores an upper one, so it plans numba
    // 0.62.0 (<3.14) on 3.14. PipDock rules that release out by name and uv picks again.
    let releases = Arc::new(
        Releases::default()
            .with("numba", "0.62.0", ">=3.10,<3.14")
            .with("numba", "0.61.2", ">=3.10,<3.15"),
    );
    let engine = Scripted::default()
        .pretending_to_be(EngineId::Uv)
        .with("numba", "0.60.0", &[])
        .offering("numba", "0.62.0")
        .also_offering("numba", &["0.61.2"]);
    let mut py314 = env();
    py314.python_version = "3.14.7".into();
    let (_, step) = UpdateFlow::start(
        py314,
        Box::new(engine.clone()),
        &update_all(),
        &[],
        checking(&releases),
    )
    .await
    .expect("start");
    let FlowStep::NeedsConfirm { report } = step else {
        panic!("an older compatible release is a clean plan, not a held-back row: {step:?}");
    };
    assert_eq!(report.changes.len(), 1);
    assert_eq!(report.changes[0].to.0, "0.61.2");
    assert_eq!(
        report.incompatible.len(),
        1,
        "and the preview says what was ruled out"
    );
    assert_eq!(report.incompatible[0].version.0, "0.62.0");
    let world = engine.world();
    assert_eq!(world.resolves.len(), 2);
    assert_eq!(world.resolves[1].adjust.exclude[0].version.0, "0.62.0");
}

#[tokio::test]
async fn pip_s_own_answer_is_used_and_pypi_is_not_asked_about_its_plan() {
    let releases = Arc::new(Releases::default());
    let engine = Scripted::default()
        .with("idna", "3.4", &[])
        .offering("idna", "3.10");
    let (_, step) = UpdateFlow::start(
        env(),
        Box::new(engine.clone()),
        &update_all(),
        &[],
        checking(&releases),
    )
    .await
    .expect("start");
    assert!(matches!(step, FlowStep::NeedsConfirm { .. }), "{step:?}");
    assert_eq!(
        releases.asked.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "nothing exact was pinned and pip's plan says what it requires"
    );
}

#[tokio::test]
async fn a_requires_python_nobody_knows_never_blocks() {
    // A private index, a network that is down: refusing an installable package on PipDock's
    // uncertainty is the worse failure, and the engine still judges.
    let releases = Arc::new(Releases::default());
    let engine = Scripted::default().pretending_to_be(EngineId::Uv);
    let (_, step) = UpdateFlow::start(
        env(),
        Box::new(engine),
        &install(&["internal-tool==2.0"]),
        &[],
        checking(&releases),
    )
    .await
    .expect("start");
    let FlowStep::NeedsConfirm { report } = step else {
        panic!("{step:?}");
    };
    assert_eq!(report.changes.len(), 1);
    assert!(report.incompatible.is_empty());
    assert!(releases.asked.load(std::sync::atomic::Ordering::SeqCst) >= 1);
}
