//! Running the three tools and assembling a [`HealthReport`].
//!
//! The shape that matters here is **skip-and-continue**. `PD-HLT-003` promises a "partial report
//! shown", so a tool that fails does not fail the run: it lands in `HealthReport.problems` and the
//! other two still report. That is the same rule `plan::execute` follows for a failed package, and
//! it is the thing most likely to be built as a whole-run failure by accident.

use std::path::Path;

use crate::engine::ProgressSink;
use crate::errors::{Code, PdError, Result};
use crate::exec::Command;
use crate::model::{ExecMode, PkgName, PyEnv, StepStatus};

use super::report::{HealthReport, RuffFindings, ToolProblem};
use super::{DEFAULT_EXCLUDES, DEFAULT_MIN_CONFIDENCE, HEALTH_TOOLS, TOOL_TIMEOUT};

/// What the caller wants run, and how.
///
/// **Off the wire on purpose.** CODE-HEALTH-SPEC §4 says the confidence floor comes "from
/// settings", but `settings::Settings` has three fields and none of them is this. Adding them is a
/// golden, a bindings regeneration and a Settings screen, which is its own slice; until then the
/// constants are the answer and the doc is ahead of the code.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Which tools to run. Empty means all of [`HEALTH_TOOLS`].
    pub tools: Vec<String>,
    /// vulture's `--min-confidence`.
    pub min_confidence: u8,
    /// Extra exclusions on top of [`DEFAULT_EXCLUDES`].
    pub excludes: Vec<String>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            tools: HEALTH_TOOLS.iter().map(|t| (*t).to_owned()).collect(),
            min_confidence: DEFAULT_MIN_CONFIDENCE,
            excludes: Vec::new(),
        }
    }
}

impl RunOptions {
    /// The tools to run, in [`HEALTH_TOOLS`] order and with anything unknown dropped.
    ///
    /// Ordered by the constant rather than by what the caller passed, so `--tool ruff --tool
    /// deptry` and `--tool deptry --tool ruff` produce the same report and the same progress.
    #[must_use]
    pub fn selected(&self) -> Vec<&'static str> {
        HEALTH_TOOLS
            .iter()
            .filter(|t| self.tools.is_empty() || self.tools.iter().any(|want| want == *t))
            .copied()
            .collect()
    }
}

/// How many steps a run reports, given the tools selected.
#[must_use]
pub fn run_steps(opts: &RunOptions) -> usize {
    opts.selected().len()
}

/// Run Code Health over `project` for `env`.
///
/// * `tools_dir` — passed, not derived, for the reason `sync_tools_venv` documents: a test or CI
///   run must not clobber the developer's real `%LOCALAPPDATA%\PipDock\tools`.
/// * `project` — each tool's CWD, via the `Command::cwd` added for exactly this.
/// * `env` — recorded on the report so a stale one can be told from a current one. It is
///   deliberately **not** passed to any tool; see below.
/// * `sink` — carries the `CancellationToken`, and its `total` must already account for whatever
///   the caller did before this (an implicit tools sync, above all).
///
/// # Errors
/// Only failures that make **every** tool meaningless: `PD-HLT-001` when a tool is missing from
/// the tools venv, `PD-ENV-003` when `project` cannot be read. A single tool failing is not one of
/// them — see `problems`.
///
/// # deptry does not see the user's environment, and cannot be made to
///
/// CODE-HEALTH-SPEC §3 says the environment is "passed to deptry so 'installed vs declared vs
/// imported' compares against reality". **deptry 0.25.1 has no such option.** It ignores
/// `VIRTUAL_ENV`, and reads whatever its own interpreter can import — verified by watching it
/// report `click`, which exists only in PipDock's tools venv, as `DEP003` transitive. Putting the
/// tools venv on a user interpreter's `PYTHONPATH` does not help: deptry's own dependencies come
/// with it. §2's isolation and §3's comparison are simply in conflict at this version.
///
/// What that costs, precisely: deptry classifies an undeclared import as `DEP003` rather than
/// `DEP001` when it can see the package, so the split is wrong for the nine packages the tools venv
/// holds, and `DEP003` under-reports anything genuinely transitive in the *user's* environment.
/// Both still mean "you imported something you did not declare", which is what §6 tells the user to
/// fix either way. Kept rather than suppressed: `--ignore DEP003` would turn a mislabelled finding
/// into a missing one.
///
/// §3 is amended to say this, and `deptry_is_never_told_about_an_environment` pins it so a future
/// deptry gaining the flag is noticed rather than silently continuing to be worked around.
pub async fn run(
    tools_dir: &Path,
    project: &Path,
    env: &PyEnv,
    opts: &RunOptions,
    sink: &ProgressSink,
) -> Result<HealthReport> {
    let declared = super::declared_source(project)?;
    let selected = opts.selected();

    let mut report = HealthReport {
        project: project.display().to_string(),
        env: crate::exec::canonical_interpreter(&env.interpreter),
        ran_at: String::new(),
        tool_versions: super::read_manifest(tools_dir)
            .map(|m| m.tools)
            .unwrap_or_default(),
        declared,
        ran: selected.iter().map(|t| (*t).to_owned()).collect(),
        deptry: Vec::new(),
        vulture: Vec::new(),
        ruff: RuffFindings::default(),
        problems: Vec::new(),
    };

    for (i, tool) in selected.iter().enumerate() {
        let step = sink.at(sink.step + i);
        let name = PkgName::parse(tool)?;
        step.started(Some(name.clone()), ExecMode::Isolated);

        match run_one(tools_dir, project, opts, tool).await {
            Ok(out) => {
                collect(&mut report, tool, &out)?;
                step.finished(Some(name), ExecMode::Isolated, StepStatus::Ok);
            }
            Err(e) => {
                // The skip-and-continue point. `Failed`, not an early return: the console drawer
                // and the live region both need to see this step end, and the next tool still runs.
                report.problems.push(ToolProblem {
                    tool: (*tool).to_owned(),
                    code: e.code,
                    message: e.message,
                    stderr_tail: e.stderr_tail,
                });
                step.finished(Some(name), ExecMode::Isolated, StepStatus::Failed);
            }
        }
    }

    report.ran_at = jiff::Timestamp::now().to_string();
    Ok(report)
}

/// Invoke one tool and hand back its stdout.
///
/// # Errors
/// `PD-HLT-001` when it is not in the tools venv, `PD-HLT-003` on its watchdog, `PD-HLT-002` when
/// it exited a way [`super::is_findings_exit`] does not recognize.
async fn run_one(
    tools_dir: &Path,
    project: &Path,
    opts: &RunOptions,
    tool: &str,
) -> Result<String> {
    let exe = super::tool_exe(tools_dir, tool);
    if !exe.is_file() {
        return Err(PdError::new(
            Code::HltToolMissing,
            format!("{tool} is not in the tools environment"),
        ));
    }

    // deptry writes its JSON to a path rather than a stream, so it needs a scratch file — under
    // the tools directory, never in the project (§1). Unique per process so two runs cannot read
    // each other's report.
    let report_path = tools_dir
        .join("runs")
        .join(format!("{tool}-{}.json", std::process::id()));
    if tool == "deptry"
        && let Some(parent) = report_path.parent()
    {
        std::fs::create_dir_all(parent).map_err(|e| {
            PdError::new(
                Code::HltToolFailed,
                format!("could not make room for deptry's report: {e}"),
            )
        })?;
    }

    let out = Command::new(&exe)
        .args(argv(tool, opts, &report_path))
        .cwd(project)
        .timeout(TOOL_TIMEOUT)
        .run()
        .await
        .map_err(|e| watchdog(e, tool));

    let stdout = match out {
        Ok(out) if super::is_findings_exit(tool, out.code) => {
            if tool == "deptry" {
                std::fs::read_to_string(&report_path).map_err(|e| {
                    PdError::new(
                        Code::HltToolFailed,
                        format!("deptry exited cleanly but wrote no report: {e}"),
                    )
                })
            } else {
                Ok(out.stdout)
            }
        }
        Ok(out) => Err(PdError::new(
            Code::HltToolFailed,
            format!("{tool} exited {}", out.code.unwrap_or(-1)),
        )
        .with_stderr(&out.stderr)),
        Err(e) => Err(e),
    };

    // Cleaned on both paths: `%LOCALAPPDATA%\PipDock` is what SECURITY §8 promises a delete
    // resets, and leaving a report per run turns that into a slow leak.
    let _ = std::fs::remove_file(&report_path);
    stdout
}

/// The argv for one tool (CODE-HEALTH-SPEC §4, plus what running them taught).
fn argv(tool: &str, opts: &RunOptions, report_path: &Path) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    match tool {
        "deptry" => {
            args.push(".".into());
            // A real path, because `--json-output` is a *location* and not a stream: passing `-`
            // creates a file literally called `-` in whatever the CWD is, which here is the user's
            // project — the precise thing CODE-HEALTH-SPEC §1 forbids. The caller supplies a path
            // under the tools directory and deletes it either way.
            args.push("--json-output".into());
            args.push(report_path.display().to_string());
            // deptry colours its text report unconditionally, and the escapes are noise in the
            // console drawer and in the bug-report log ring.
            args.push("--no-ansi".into());
            // §7's notebook non-goal.
            args.push("--ignore-notebooks".into());
            // deptry's exclusions are **regexes** matched with `re.match`, so they are anchored:
            // a bare `build` would not match `pkg/build/`. Every other tool here takes globs.
            for pattern in excludes(opts) {
                args.push("--extend-exclude".into());
                args.push(format!(".*{}/.*", regex_escape(&pattern)));
            }
            // **No environment flag exists.** CODE-HEALTH-SPEC §3 says the env is "passed to deptry
            // so 'installed vs declared vs imported' compares against reality"; deptry 0.25.1 has
            // no such option, ignores VIRTUAL_ENV, and reads whatever is importable from the
            // interpreter running it — verified by watching it report a tools-venv-only package as
            // DEP003. See `run`'s doc comment for what that costs and what is disclosed.
        }
        "vulture" => {
            args.push(".".into());
            args.push("--min-confidence".into());
            args.push(opts.min_confidence.to_string());
            // vulture takes one comma-separated list of globs, not repeated flags.
            let globs = excludes(opts)
                .iter()
                .map(|p| format!("*/{p}/*"))
                .collect::<Vec<_>>()
                .join(",");
            if !globs.is_empty() {
                args.push("--exclude".into());
                args.push(globs);
            }
        }
        "ruff" => {
            args.push("check".into());
            args.push(".".into());
            args.push("--output-format".into());
            args.push("json".into());
            // §7 again. ruff's excludes are globs and are *additive* to the project's own config.
            args.push("--exclude".into());
            args.push("*.ipynb".into());
            for pattern in excludes(opts) {
                args.push("--exclude".into());
                args.push(pattern);
            }
        }
        _ => {}
    }
    args
}

/// The default exclusions plus the caller's.
fn excludes(opts: &RunOptions) -> Vec<String> {
    DEFAULT_EXCLUDES
        .iter()
        .map(|d| (*d).to_owned())
        .chain(opts.excludes.iter().cloned())
        .collect()
}

/// Escape the characters a directory name can contain that a regex would read as syntax.
///
/// `.venv` is the case that matters and it is in the defaults: unescaped, `.` matches any
/// character, so the pattern would also exclude `avenv`, `1venv` and anything else shaped like it.
fn regex_escape(literal: &str) -> String {
    literal.chars().fold(String::new(), |mut acc, c| {
        if ".^$*+?()[]{}|\\".contains(c) {
            acc.push('\\');
        }
        acc.push(c);
        acc
    })
}

/// Fold one tool's stdout into the report.
fn collect(report: &mut HealthReport, tool: &str, stdout: &str) -> Result<()> {
    match tool {
        "deptry" => report.deptry = super::deptry::parse(stdout)?,
        "vulture" => report.vulture = super::vulture::parse(stdout),
        "ruff" => report.ruff = super::ruff::parse(stdout)?,
        _ => {}
    }
    Ok(())
}

/// Re-read an `exec` timeout as the tool's own watchdog code.
///
/// `exec::Command::stopped` raises `PD-INT-001` for both its watchdog and cancellation, so without
/// this a tool that ran long would tell the user PipDock hit an internal error — and `PD-HLT-003`,
/// which exists for exactly this, would never be reachable.
fn watchdog(e: PdError, tool: &str) -> PdError {
    if e.code == Code::IntUnexpected && e.message.contains("timed out") {
        return PdError::new(
            Code::HltTimeout,
            format!("{tool} exceeded its {TOOL_TIMEOUT:?} watchdog; its results are missing"),
        );
    }
    e
}

/// Whether any tool reported something.
///
/// Drives the CLI's exit code: a linter that exits 0 on findings is useless in a pre-commit hook.
#[must_use]
pub fn has_findings(report: &HealthReport) -> bool {
    !report.deptry.is_empty() || !report.vulture.is_empty() || !report.ruff.findings.is_empty()
}

/// Ensure `total` is not smaller than what will actually be emitted.
///
/// A sink whose total is wrong is a progress bar that stops short or overshoots, and the caller
/// cannot always know whether a tools-venv sync will be needed until it asks.
#[must_use]
pub fn sink_for(sink: &ProgressSink, first_step: usize) -> ProgressSink {
    sink.at(first_step)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts_for(tools: &[&str]) -> RunOptions {
        RunOptions {
            tools: tools.iter().map(|t| (*t).to_owned()).collect(),
            ..RunOptions::default()
        }
    }

    #[test]
    fn no_selection_means_every_tool() {
        assert_eq!(opts_for(&[]).selected(), HEALTH_TOOLS);
    }

    #[test]
    fn selection_order_is_the_constants_not_the_callers() {
        // `--tool ruff --tool deptry` and the reverse must produce the same report and the same
        // progress steps, or two identical runs disagree about what step 2 was.
        assert_eq!(
            opts_for(&["ruff", "deptry"]).selected(),
            opts_for(&["deptry", "ruff"]).selected()
        );
        assert_eq!(opts_for(&["ruff", "deptry"]).selected(), ["deptry", "ruff"]);
    }

    #[test]
    fn an_unknown_tool_is_dropped_rather_than_run() {
        assert_eq!(opts_for(&["pip-audit"]).selected(), Vec::<&str>::new());
    }

    #[test]
    fn run_steps_follows_the_selection() {
        assert_eq!(run_steps(&opts_for(&[])), HEALTH_TOOLS.len());
        assert_eq!(run_steps(&opts_for(&["ruff"])), 1);
    }

    /// The bug an unescaped default would ship: `.venv` as a regex also matches `avenv`.
    #[test]
    fn deptry_exclusions_are_escaped_regexes() {
        let args = args_for("deptry");

        assert!(
            args.iter().any(|a| a == r".*\.venv/.*"),
            "the dot must be escaped and the pattern anchored: {args:?}"
        );
        assert!(!args.iter().any(|a| a == ".*.venv/.*"));
    }

    #[test]
    fn vulture_takes_one_comma_separated_list_not_repeated_flags() {
        let args = args_for("vulture");
        let flags = args.iter().filter(|a| *a == "--exclude").count();

        assert_eq!(flags, 1, "vulture takes a single list: {args:?}");
        assert!(
            args.iter()
                .any(|a| a.contains("*/.venv/*") && a.contains(','))
        );
    }

    /// The limitation, pinned. `run`'s doc comment says why it cannot be otherwise.
    #[test]
    fn deptry_is_never_told_about_an_environment() {
        // deptry 0.25.1 has no such option — passing one fails the command outright — and
        // CODE-HEALTH-SPEC §3 assumed one existed. If a future deptry gains it, this is what makes
        // someone notice rather than leaving the workaround in place forever.
        let args = args_for("deptry");

        assert!(
            !args.iter().any(|a| a == "--python" || a == "--venv"),
            "deptry has no environment flag at the pinned version: {args:?}"
        );
    }

    #[test]
    fn deptrys_report_is_written_under_the_tools_directory_not_the_project() {
        // §1: PipDock never writes into the user's project. `--json-output` takes a *location*,
        // not a stream, so `-` would create a file called `-` in the CWD — which is the project.
        // Verified against `deptry --help`.
        let path = Path::new(r"C:\tools\runs\deptry-1.json");
        let args = argv("deptry", &RunOptions::default(), path);
        let idx = args
            .iter()
            .position(|a| a == "--json-output")
            .expect("--json-output");

        assert_eq!(args[idx + 1], path.display().to_string());
        assert_ne!(args[idx + 1], "-", "`-` is a filename here, not stdout");
    }

    #[test]
    fn notebooks_are_excluded_from_both_tools_that_can_see_them() {
        // §7's non-goal, which is contractual per ROADMAP's standing risks.
        assert!(args_for("deptry").iter().any(|a| a == "--ignore-notebooks"));
        assert!(args_for("ruff").iter().any(|a| a == "*.ipynb"));
    }

    #[test]
    fn no_argv_entry_bundles_two_arguments() {
        // SECURITY §2's argv rule, applied to the tools as it already is to the engines.
        for tool in HEALTH_TOOLS {
            for arg in args_for(tool) {
                assert!(
                    !arg.contains(' ') || arg.starts_with('*') || arg.contains(','),
                    "{tool}: {arg:?} bundles arguments"
                );
            }
        }
    }

    fn args_for(tool: &str) -> Vec<String> {
        argv(
            tool,
            &RunOptions::default(),
            Path::new(r"C:\tools\runs\r.json"),
        )
    }
    #[test]
    fn a_watchdog_reads_as_the_health_code_not_an_internal_error() {
        let timed_out = PdError::new(Code::IntUnexpected, "timed out after 120s: ruff.exe");
        assert_eq!(watchdog(timed_out, "ruff").code, Code::HltTimeout);
    }

    #[test]
    fn a_cancellation_is_not_a_watchdog() {
        let cancelled = PdError::new(Code::IntUnexpected, "cancelled: ruff.exe");
        assert_eq!(watchdog(cancelled, "ruff").code, Code::IntUnexpected);
    }
}
