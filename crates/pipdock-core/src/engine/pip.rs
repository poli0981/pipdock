//! The pip adapter: `<env-python> -m pip …`.
//!
//! Command mapping is fixed by `docs/DATA-FLOW.md` §7. Implementation lands in M1; the argv
//! builders below are already exercised by tests because they are the surface SECURITY §2 cares
//! about — nothing else in this file may construct engine arguments.

use async_trait::async_trait;

use crate::errors::{Code, PdError, Result};
use crate::exec::Command;
use crate::model::{
    CheckReport, Dist, EngineId, EngineInfo, ExecMode, OutdatedDist, PinnedSpec, PkgName, PyEnv,
    StepResult,
};
use crate::plan::{PlanRequest, ResolutionReport};

use super::{Engine, ProgressSink, single_pkg};

/// Drives pip as a subprocess.
#[derive(Debug, Clone, Copy, Default)]
pub struct PipEngine;

impl PipEngine {
    /// argv for `list --format=json`.
    #[must_use]
    pub fn argv_list() -> Vec<String> {
        vec![
            "-m".into(),
            "pip".into(),
            "list".into(),
            "--format=json".into(),
        ]
    }

    /// argv for `list --outdated --format=json`.
    #[must_use]
    pub fn argv_outdated() -> Vec<String> {
        vec![
            "-m".into(),
            "pip".into(),
            "list".into(),
            "--outdated".into(),
            "--format=json".into(),
        ]
    }

    /// argv for the dry-run resolve, whose JSON report goes to stdout.
    ///
    /// `requirements` comes from [`super::plan_argv_specs`] and carries all three groups: bare
    /// names to move, explicit installs, and the pinned guard set.
    #[must_use]
    pub fn argv_dry_run(requirements: &[String]) -> Vec<String> {
        let mut argv = vec![
            "-m".into(),
            "pip".into(),
            "install".into(),
            // -U is load-bearing, and its absence is silent: without it pip sees an installed
            // package as already satisfying the requirement, plans nothing, and every selected
            // package is then reported held back at its current version for no visible reason.
            // DATA-FLOW §7 specifies `install -U --dry-run --quiet --report -`.
            "-U".into(),
            "--dry-run".into(),
            "--quiet".into(),
            "--report".into(),
            "-".into(),
        ];
        argv.extend(requirements.iter().cloned());
        argv
    }

    /// argv for `freeze --all`, used for snapshots. `--all` keeps pip/setuptools in the freeze;
    /// the snapshot metadata records which engine produced it (DATA-FLOW §7).
    #[must_use]
    pub fn argv_freeze() -> Vec<String> {
        vec!["-m".into(), "pip".into(), "freeze".into(), "--all".into()]
    }

    /// argv for `check`.
    #[must_use]
    pub fn argv_check() -> Vec<String> {
        vec!["-m".into(), "pip".into(), "check".into()]
    }
}

#[async_trait]
impl Engine for PipEngine {
    fn id(&self) -> EngineId {
        EngineId::Pip
    }

    async fn info(&self, env: &PyEnv) -> EngineInfo {
        let out = Command::python(&env.interpreter)
            .args(["-m", "pip", "--version"])
            .run()
            .await;
        match out {
            Ok(o) if o.ok() => EngineInfo {
                id: EngineId::Pip,
                // "pip 26.1.2 from C:\... (python 3.12)" -> "26.1.2"
                version: o.stdout.split_whitespace().nth(1).map(str::to_owned),
                available: true,
            },
            _ => EngineInfo {
                id: EngineId::Pip,
                version: None,
                available: false,
            },
        }
    }

    async fn list_installed(&self, env: &PyEnv) -> Result<Vec<Dist>> {
        let out = Command::python(&env.interpreter)
            .args(Self::argv_list())
            .run()
            .await?;
        if !out.ok() {
            return Err(out.into_error());
        }
        super::parse::list_json(&out.stdout)
    }

    async fn list_outdated(&self, env: &PyEnv) -> Result<Vec<OutdatedDist>> {
        let out = Command::python(&env.interpreter)
            .args(Self::argv_outdated())
            .run()
            .await?;
        if !out.ok() {
            return Err(out.into_error());
        }
        super::parse::outdated_json(&out.stdout)
    }

    async fn resolve(&self, env: &PyEnv, req: &PlanRequest) -> Result<ResolutionReport> {
        // SP-1: the installed set must be restated or the resolver ignores it and plans an
        // upgrade that breaks installed dependents. The caller assembles that set into `req`.
        let installed = self.list_installed(env).await?;
        let requirements = super::plan_argv_specs(req, &installed);
        let out = Command::python(&env.interpreter)
            .args(Self::argv_dry_run(&requirements))
            .run()
            .await?;
        if !out.ok() {
            return Err(out.into_error());
        }
        super::parse::pip_report(&out.stdout, &out.stderr, &installed).map(|p| p.report)
    }

    async fn install(
        &self,
        env: &PyEnv,
        specs: &[PinnedSpec],
        mode: ExecMode,
        sink: ProgressSink,
    ) -> Result<StepResult> {
        let mut argv = vec!["-m".to_owned(), "pip".to_owned(), "install".to_owned()];
        argv.extend(specs.iter().map(PinnedSpec::to_requirement));
        let out = Command::python(&env.interpreter)
            .args(argv)
            .cancel(sink.cancel.clone())
            .run_streaming(&sink, single_pkg(specs), mode)
            .await?;
        Ok(super::step_result(specs, &out))
    }

    async fn uninstall(
        &self,
        env: &PyEnv,
        names: &[PkgName],
        sink: ProgressSink,
    ) -> Result<StepResult> {
        let mut argv = vec![
            "-m".to_owned(),
            "pip".to_owned(),
            "uninstall".to_owned(),
            "-y".to_owned(),
        ];
        argv.extend(names.iter().map(ToString::to_string));
        let out = Command::python(&env.interpreter)
            .args(argv)
            .cancel(sink.cancel.clone())
            .run_streaming(&sink, names.first().cloned(), ExecMode::Isolated)
            .await?;
        Ok(super::removal_result(names, &out))
    }

    async fn check(&self, env: &PyEnv) -> Result<CheckReport> {
        let out = Command::python(&env.interpreter)
            .args(Self::argv_check())
            .run()
            .await?;
        // `pip check` exits non-zero when it finds problems, which is a successful check with
        // findings -- not a command failure.
        Ok(super::parse::check_text(&out.stdout, out.ok()))
    }

    async fn freeze(&self, env: &PyEnv) -> Result<String> {
        let out = Command::python(&env.interpreter)
            .args(Self::argv_freeze())
            .run()
            .await?;
        if !out.ok() {
            return Err(out.into_error());
        }
        Ok(out.stdout)
    }

    async fn upgrade_pip(&self, env: &PyEnv) -> Result<StepResult> {
        // DATA-FLOW §2's preamble is "all mutating flows", and replacing pip in site-packages is
        // one. `UpdateFlow` and `UninstallFlow` have refused these at their entry points since S1
        // and S5; this had no guard at all, which did not matter while the only caller was a CLI
        // command nobody had wired to a button. P1 gives it one, so it needs the same refusal —
        // copied from `UninstallFlow::start`, not reworded, so the three cannot drift.
        if env.externally_managed {
            return Err(PdError::new(
                Code::EnvExternallyManaged,
                "this Python is externally managed (PEP 668). Use a virtual environment; \
                 the override lives in Settings and is discouraged",
            ));
        }

        let out = Command::python(&env.interpreter)
            .args(["-m", "pip", "install", "-U", "pip"])
            .run()
            .await?;
        if !out.ok() {
            return Err(PdError::from_engine_stderr(&out.stderr));
        }
        Ok(StepResult {
            pkg: PkgName::parse("pip").unwrap_or_else(|_| unreachable!("pip is a valid name")),
            from: None,
            to: None,
            status: crate::model::StepStatus::Ok,
            code: None,
            stderr_tail: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Version;

    #[test]
    fn dry_run_argv_matches_the_documented_command() {
        // DATA-FLOW §7 verbatim. An earlier version of this test omitted -U and still claimed to
        // match the document, which is how a silent planning failure passed CI: pip treated every
        // installed package as already satisfied and planned nothing at all.
        let argv = PipEngine::argv_dry_run(&[]);
        assert_eq!(
            argv,
            [
                "-m",
                "pip",
                "install",
                "-U",
                "--dry-run",
                "--quiet",
                "--report",
                "-"
            ]
        );
    }

    #[test]
    fn requirements_are_appended_as_separate_argv_entries() {
        // One token per argument is what makes shell quoting irrelevant (SECURITY §2).
        let specs = [
            PinnedSpec {
                name: PkgName::parse("httpx").unwrap(),
                version: Version("0.28.1".into()),
            },
            PinnedSpec {
                name: PkgName::parse("Requests").unwrap(),
                version: Version("2.32.3".into()),
            },
        ];
        let requirements: Vec<String> = specs.iter().map(PinnedSpec::to_requirement).collect();
        let argv = PipEngine::argv_dry_run(&requirements);
        assert_eq!(
            &argv[argv.len() - 2..],
            ["httpx==0.28.1", "requests==2.32.3"]
        );
        assert!(
            argv.iter().all(|a| !a.contains(' ')),
            "no argv entry may bundle multiple arguments"
        );
    }

    #[test]
    fn freeze_uses_all_so_snapshots_include_pip_itself() {
        assert!(PipEngine::argv_freeze().contains(&"--all".to_string()));
    }

    /// DATA-FLOW §2's preamble is "all mutating flows". `UpdateFlow` and `UninstallFlow` refuse a
    /// PEP 668 environment at their entry points; this path had no guard at all, which only stayed
    /// harmless while nothing but a CLI command could reach it.
    ///
    /// The interpreter path is deliberately nonsense: the guard must return *before* anything is
    /// spawned, so a test that needs a real Python would be testing the wrong thing.
    #[tokio::test]
    async fn an_externally_managed_python_is_refused_before_anything_runs() {
        let env = PyEnv {
            interpreter: r"C:\does\not\exist\python.exe".into(),
            prefix: r"C:\usr".into(),
            python_version: "3.12.4".into(),
            externally_managed: true,
            hidden_user_site: None,
            source: crate::model::EnvSource::Registry,
        };

        let err = PipEngine
            .upgrade_pip(&env)
            .await
            .expect_err("PEP 668 blocks pip upkeep");

        assert_eq!(err.code, Code::EnvExternallyManaged);
        // Not PD-ENG-001: reaching the spawn would mean the guard ran too late.
        assert!(!err.message.contains("could not run"), "{}", err.message);
    }
}
