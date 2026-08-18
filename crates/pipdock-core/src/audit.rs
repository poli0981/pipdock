//! pip-audit over an installed environment — PRD P1-1, SECURITY §6.
//!
//! Runs out of **its own venv** ([`crate::health::AUDIT_TOOLS`], [`crate::health::audit_dir`]) and
//! in freeze-file mode, which SP-4 established is the only mode there is: `--path <site-packages>`
//! is rejected outright when combined with `--no-deps`, so the environment reaches the tool as a
//! `pip freeze` document rather than as a directory.
//!
//! Four things this module exists to get right, every one measured against the pinned 2.10.1
//! rather than read out of a document (`tests/fixtures/audit/`):
//!
//! 1. **The ids are `PYSEC-*`, because the default vulnerability service is PyPI.** Left as the
//!    default deliberately: PipDock querying `api.osv.dev` itself would add a second network
//!    destination and falsify `legal/PRIVACY-POLICY.md` §3's "exactly one destination". Findings
//!    still *link* to OSV, which is a visit the user's browser makes after a click — the case §3
//!    already carves out.
//! 2. **There is no severity, under either service.** The vuln object carries exactly `id`,
//!    `fix_versions`, `aliases` and `description`. PRD P1-1's "severity-sorted" and SECURITY §6's
//!    "(CVE/GHSA id, severity, fixed-in)" were written against a field that has never existed, so
//!    [`sort_advisories`] sorts by what is real instead.
//! 3. **Ids repeat.** The captured run returns ten rows for **eight** advisories; the same
//!    environment under `--vulnerability-service osv` returns sixteen rows and the *same* eight.
//!    A count taken before [`dedupe`] is wrong under either service.
//! 4. **stderr is not a failure signal.** pip-audit writes a `pip-compile`/hashes advisory on
//!    every single run. Parse stdout first and classify only if that fails — the rule SP-1 left
//!    behind for uv, which writes its plan to stderr for the same reason.

use std::collections::BTreeSet;
use std::path::Path;

use crate::engine::ProgressSink;
use crate::errors::{Code, PdError, Result};
use crate::exec::Command;
use crate::health::{ToolProblem, read_manifest, tool_exe};
use crate::model::{ExecMode, PkgName, PyEnv, StepStatus, Version};

/// Where a finding links. The id is appended after validation — never interpolated raw.
const OSV_BASE: &str = "https://osv.dev/vulnerability/";

/// One advisory against one installed distribution.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct Advisory {
    /// The installed distribution the advisory is against.
    pub pkg: PkgName,
    /// The version installed, not the version that fixes it.
    pub version: Version,
    /// pip-audit's primary id — `PYSEC-*` under the default service.
    pub id: String,
    /// Other ids for the same advisory, which is where the `CVE-*` and `GHSA-*` live.
    ///
    /// PRD P1-1 says "known CVEs"; a CVE is never the primary id here, so a screen that wants to
    /// show one reads this.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Versions that fix it. **Empty is meaningful** — an advisory with nowhere to upgrade to —
    /// and [`sort_advisories`] puts those last rather than hiding them.
    #[serde(default)]
    pub fix_versions: Vec<String>,
    /// The advisory prose, verbatim. Never translated (I18N §2) and never trimmed here.
    pub description: String,
    /// The OSV entry, when the id is shaped like one.
    ///
    /// **Built from a validated id rather than taken from the tool**, which is the whole
    /// difference between this and `PdHealthReport`'s `finding.url`.
    /// `capabilities/external-links.json` records that ruff's URL is the only one in the
    /// application derived from a third party's output; this one is derived from a string PipDock
    /// has checked, so widening the allowlist to `osv.dev` does not widen what can reach it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// The OSV entry for an advisory id, or `None` if the id is not shaped like one.
///
/// The check is a whitelist, not an escape: ASCII letters, digits and `-`, 4 to 64 characters,
/// leading letter. Nothing that passes can carry a `/`, `?`, `#`, `:` or `%`, so the result cannot
/// be anything but a path segment under [`OSV_BASE`] — which is what lets the opener allowlist
/// stay scoped to one host. SECURITY §2's obligation follows the *input*, and this input is a
/// third-party tool's JSON.
#[must_use]
pub fn advisory_url(id: &str) -> Option<String> {
    let ok = (4..=64).contains(&id.len())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && id.starts_with(|c: char| c.is_ascii_alphabetic());
    ok.then(|| format!("{OSV_BASE}{id}"))
}

/// pip-audit's `-f json` document, only the parts PipDock reads.
mod raw {
    #[derive(serde::Deserialize)]
    pub struct Doc {
        #[serde(default)]
        pub dependencies: Vec<Dep>,
    }

    #[derive(serde::Deserialize)]
    pub struct Dep {
        pub name: String,
        #[serde(default)]
        pub version: String,
        #[serde(default)]
        pub vulns: Vec<Vuln>,
    }

    #[derive(serde::Deserialize)]
    pub struct Vuln {
        pub id: String,
        #[serde(default)]
        pub fix_versions: Vec<String>,
        #[serde(default)]
        pub aliases: Vec<String>,
        #[serde(default)]
        pub description: String,
    }
}

/// How many distributions the run covered, and what it found.
///
/// The count is separate from `advisories.len()` on purpose: "audited 145 packages, found nothing"
/// and "audited nothing" are different answers, and a screen that cannot tell them apart repeats
/// P4's *no issues found before anything had run*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Audited {
    /// Distributions present in the freeze document.
    pub packages: usize,
    /// Deduplicated and sorted.
    pub advisories: Vec<Advisory>,
}

/// Parse a pip-audit `-f json` document.
///
/// # Errors
/// `PD-HLT-002` when stdout is not the document `--format json` promises. That is the only failure
/// mode here: a non-zero exit is **not** one, because findings exit 1.
pub fn parse(stdout: &str) -> Result<Audited> {
    let doc: raw::Doc = serde_json::from_str(stdout).map_err(|e| {
        PdError::new(
            Code::HltToolFailed,
            format!("pip-audit did not emit the JSON document --format json promises: {e}"),
        )
    })?;

    let mut advisories = Vec::new();
    for dep in &doc.dependencies {
        // A name pip-audit echoed back out of a freeze document PipDock wrote. Validated anyway:
        // it is the join key against the installed table, and an unparseable one would be a row
        // matching nothing rather than a row that is merely wrong.
        let Ok(pkg) = PkgName::parse(&dep.name) else {
            continue;
        };
        for vuln in &dep.vulns {
            advisories.push(Advisory {
                pkg: pkg.clone(),
                version: Version(dep.version.clone()),
                url: advisory_url(&vuln.id),
                id: vuln.id.clone(),
                aliases: vuln.aliases.clone(),
                fix_versions: vuln.fix_versions.clone(),
                description: vuln.description.clone(),
            });
        }
    }

    Ok(Audited {
        packages: doc.dependencies.len(),
        advisories: sort_advisories(dedupe(advisories)),
    })
}

/// Collapse advisories repeated for one package.
///
/// pip-audit emits the same id more than once — twice each for two of the eight in the captured
/// run, and eight times each under the OSV service. The key is `(pkg, id)` rather than `id` alone,
/// because one advisory legitimately applies to two different installed packages.
#[must_use]
pub fn dedupe(advisories: Vec<Advisory>) -> Vec<Advisory> {
    let mut seen = BTreeSet::new();
    advisories
        .into_iter()
        .filter(|a| seen.insert((a.pkg.clone(), a.id.clone())))
        .collect()
}

/// Order: package, then whether anything fixes it, then id.
///
/// **Not severity.** PRD P1-1 asked for severity-sorted and pip-audit has no such field, so this
/// sorts by the one distinction that changes what a user can *do*: an advisory with a
/// `fix_versions` entry is one the normal Update flow can act on, and an advisory without one is
/// not. Inventing a severity — from the id's year, from the description's wording — would be a
/// number PipDock made up, shown beside real ones.
#[must_use]
pub fn sort_advisories(mut advisories: Vec<Advisory>) -> Vec<Advisory> {
    advisories.sort_by(|a, b| {
        a.pkg
            .cmp(&b.pkg)
            .then(a.fix_versions.is_empty().cmp(&b.fix_versions.is_empty()))
            .then(a.id.cmp(&b.id))
    });
    advisories
}

/// The tool this module drives — one entry of [`crate::health::AUDIT_TOOLS`].
///
/// Named once so the argv, the manifest lookup and the [`ToolProblem`] row cannot disagree about
/// which tool a failure is about.
const TOOL: &str = "pip-audit";

/// How long pip-audit gets before the watchdog takes it.
///
/// **Not Code Health's `TOOL_TIMEOUT`**, and the difference is not a rounding error. That 120 s is
/// sized for a linter walking a source tree; an audit is a *network* operation, and what dominates
/// it is a one-off advisory-database fetch rather than the number of packages. Measured on this
/// machine, release-irrelevant because the time is all I/O:
///
/// | Run | Wall clock |
/// |---|---|
/// | cold cache, **one** package | **68 s** |
/// | warm cache, one package | 18.6 s |
/// | warm cache, **twelve** packages | 20.0 s |
///
/// Twelve packages cost 1.4 s more than one, so a 352-package environment is not the risk — a cold
/// fetch on a slow link is. The first run written against `TOOL_TIMEOUT` duly hit the watchdog at
/// 120 s and reported `PD-HLT-003` for a tool that was working. Ten minutes matches
/// `exec::DEFAULT_TIMEOUT`, which is what every other network-bound subprocess already gets.
pub const AUDIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// How many steps a run reports, on top of whatever sync the caller had to do first.
///
/// One, because the freeze is the caller's — see [`run`].
pub const AUDIT_STEPS: usize = 1;

/// What a Security-tab run produced.
///
/// Shaped like [`crate::health::HealthReport`] deliberately: a `problems` list rather than an
/// early return, so a failed audit still *returns a report* and the screen renders one error row
/// instead of nothing at all. That shape is what makes a partial result expressible.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuditReport {
    /// `canonical_interpreter` of the environment audited. Data, never localized (I18N §2).
    pub env: String,
    /// When the run finished, RFC 3339.
    pub ran_at: String,
    /// pip-audit's version, out of the audit venv's manifest. Empty if it has never synced.
    pub tool_version: String,
    /// Distributions the freeze document listed.
    ///
    /// **Separate from `advisories.len()`**, because "audited 145 packages and found nothing" and
    /// "audited nothing" are different answers. A screen that cannot tell them apart tells P4's
    /// lie — *no issues found* before anything has run.
    pub packages: usize,
    /// Deduplicated and ordered by [`sort_advisories`].
    #[serde(default)]
    pub advisories: Vec<Advisory>,
    /// Why the run contributed nothing. Empty means it completed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<ToolProblem>,
    /// The user stopped it.
    ///
    /// A **state, not an error**, which is the shape `ExecutionSummary.cancelled` established: the
    /// user asking a run to stop is not a tool failure, so nothing lands in `problems` and no
    /// catalog code is minted for something that went right. It matters here in a way it did not
    /// for Code Health, whose runs are 1.3 s — an audit is 18-68 s, which is long enough that
    /// stopping one is an ordinary thing to want.
    #[serde(default)]
    pub cancelled: bool,
}

impl AuditReport {
    /// Whether anything was found — the CLI's exit rule.
    #[must_use]
    pub fn has_findings(&self) -> bool {
        !self.advisories.is_empty()
    }
}

/// Audit one environment against its freeze document.
///
/// **The freeze is passed, not taken.** `Engine::freeze` is the caller's call, for the reason
/// `UpdateFlow::start` takes pins rather than a `&Store`: this function would otherwise need an
/// engine, which means settings, which means the store — and the store guard cannot be held across
/// an await at a command boundary. It also keeps the module testable without a subprocess.
///
/// A failure does not propagate. It lands in [`AuditReport::problems`] and the report comes back
/// anyway, so the screen has an error row and a package count rather than an empty screen.
///
/// # Errors
/// Only if the step name cannot be parsed, which is a compile-time constant and so a bug.
pub async fn run(
    audit_dir: &Path,
    env: &PyEnv,
    freeze: &str,
    sink: &ProgressSink,
) -> Result<AuditReport> {
    let mut report = AuditReport {
        env: crate::exec::canonical_interpreter(&env.interpreter),
        ran_at: String::new(),
        tool_version: read_manifest(audit_dir)
            .and_then(|m| m.tools.get(TOOL).cloned())
            .unwrap_or_default(),
        packages: 0,
        advisories: Vec::new(),
        problems: Vec::new(),
        cancelled: false,
    };

    let step = sink.at(sink.step);
    let name = PkgName::parse(TOOL)?;
    step.started(Some(name.clone()), ExecMode::Isolated);

    match audit_once(audit_dir, freeze, sink).await {
        Ok(audited) => {
            report.packages = audited.packages;
            report.advisories = audited.advisories;
            step.finished(Some(name), ExecMode::Isolated, StepStatus::Ok);
        }
        // **Skipped, not Failed**, and no problem row — S1's rule that a step killed by us is not
        // a step that went wrong, applied to the one operation long enough for anyone to kill.
        Err(e) if is_cancellation(&e) => {
            report.cancelled = true;
            step.finished(Some(name), ExecMode::Isolated, StepStatus::Skipped);
        }
        Err(e) => {
            report.problems.push(ToolProblem {
                tool: TOOL.to_owned(),
                code: e.code,
                message: e.message,
                stderr_tail: e.stderr_tail,
            });
            step.finished(Some(name), ExecMode::Isolated, StepStatus::Failed);
        }
    }

    report.ran_at = jiff::Timestamp::now().to_string();
    Ok(report)
}

/// Spawn pip-audit over a freeze document and parse what it says.
///
/// # Errors
/// `PD-HLT-001` when the audit venv has no pip-audit, `PD-HLT-003` on its watchdog, `PD-HLT-002`
/// when stdout is not the promised document.
async fn audit_once(audit_dir: &Path, freeze: &str, sink: &ProgressSink) -> Result<Audited> {
    let exe = tool_exe(audit_dir, TOOL);
    if !exe.is_file() {
        return Err(PdError::new(
            Code::HltToolMissing,
            format!("{TOOL} is not in the audit environment"),
        ));
    }

    // The environment reaches pip-audit as a *document*, never as a directory: SP-4 found that
    // `--path <site-packages>` is rejected outright alongside `--no-deps`. Written to the system
    // temp directory rather than into the user's project or the environment being audited, and
    // removed on both paths.
    let freeze_path = crate::exec::write_temp("pipdock-audit", "txt", freeze)?;

    let out = Command::new(&exe)
        .args(argv(&freeze_path))
        .timeout(AUDIT_TIMEOUT)
        .cancel(sink.cancel.clone())
        .run()
        .await
        .map_err(watchdog);

    // **Not gated on the exit code**, which is the trap this module is arranged around. Findings
    // exit 1, and pip-audit writes a `pip-compile`/hashes advisory to stderr on every single run,
    // so neither the code nor stderr says whether it worked. stdout does: if the promised document
    // is there, it ran. SP-1 left the same rule behind for uv, which writes its plan to stderr.
    let parsed = match out {
        Ok(out) => parse(&out.stdout).map_err(|e| e.with_stderr(&out.stderr)),
        Err(e) => Err(e),
    };

    let _ = std::fs::remove_file(&freeze_path);
    parsed
}

/// The argv SP-4 settled on, and the only mode that works.
///
/// One flag per entry, so nothing can bundle two arguments into one. `-f json` rather than
/// `--format json`: SP-4 and ROADMAP pin the short form while SECURITY §6 writes the long one.
/// They are the same flag, and this is the spelling that was actually run.
fn argv(freeze: &Path) -> Vec<String> {
    vec![
        "-r".to_owned(),
        freeze.display().to_string(),
        "--no-deps".to_owned(),
        "-f".to_owned(),
        "json".to_owned(),
    ]
}

/// Did the user stop it, or did it break?
///
/// `exec` reports both through `Code::IntUnexpected`, distinguished only by the message it built
/// (`exec::Command::stopped`). Reading the message is unlovely, and it is what P4 warned about
/// when it recorded that wiring the token alone would render *"PipDock hit an internal error"* for
/// a cancel — the mapping has to be done, not merely enabled.
fn is_cancellation(e: &PdError) -> bool {
    e.code == Code::IntUnexpected && e.message.starts_with("cancelled:")
}

/// Translate `exec`'s vocabulary into this path's.
///
/// `exec` speaks the *engine's*, and two of its codes are actively wrong here: a watchdog is not
/// an internal error, and a tool that will not start is not a missing engine the user should be
/// told to go and install. Health's `run_one` needed the same translation for the same reason.
fn watchdog(e: PdError) -> PdError {
    if e.code == Code::IntUnexpected && e.message.contains("timed out") {
        return PdError::new(
            Code::HltTimeout,
            format!("{TOOL} exceeded its {AUDIT_TIMEOUT:?} watchdog; its results are missing"),
        );
    }
    if e.code == Code::EngNotFound {
        return PdError::new(
            Code::HltToolMissing,
            format!(
                "{TOOL} is in the audit environment but could not be run: {}",
                e.message
            ),
        );
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pinned 2.10.1 over `urllib3==2.0.0` — see the directory's README for provenance.
    const CAPTURE: &str = include_str!("../tests/fixtures/audit/urllib3-2.0.0.json");

    #[test]
    fn the_capture_parses_and_collapses_to_eight() {
        let got = parse(CAPTURE).expect("the committed capture must parse");

        assert_eq!(got.packages, 1);
        // Ten rows in the document, eight advisories. Asserted as a number rather than "fewer than
        // before", because this count is what a Security tab puts on screen.
        assert_eq!(got.advisories.len(), 8, "ten rows collapse to eight");

        let ids: BTreeSet<&str> = got.advisories.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids.len(), 8, "no id survives twice");
        assert!(ids.contains("PYSEC-2023-192"));
    }

    #[test]
    fn the_cve_is_an_alias_and_never_the_id() {
        let got = parse(CAPTURE).expect("parses");

        let a = got
            .advisories
            .iter()
            .find(|a| a.id == "PYSEC-2023-192")
            .expect("the capture carries it");
        assert!(a.aliases.contains(&"CVE-2023-43804".to_owned()));
        assert!(a.aliases.iter().any(|x| x.starts_with("GHSA-")));
        assert!(
            !a.id.starts_with("CVE-"),
            "PRD says CVEs; the id is not one"
        );
        assert_eq!(a.fix_versions, ["1.26.17", "2.0.6"]);
        assert_eq!(a.version.0, "2.0.0", "the installed version, not the fix");
    }

    #[test]
    fn every_advisory_links_to_its_osv_entry() {
        let got = parse(CAPTURE).expect("parses");

        for a in &got.advisories {
            assert_eq!(
                a.url,
                Some(format!("{OSV_BASE}{}", a.id)),
                "{} lost its link",
                a.id
            );
        }
    }

    #[test]
    fn an_id_that_could_escape_the_path_segment_gets_no_link() {
        // Why the URL is built rather than taken: these are what a compromised — or merely
        // surprising — tool output could otherwise carry into `opener:allow-open-url`.
        for bad in [
            "../../evil",
            "PYSEC-2023-192/../../x",
            "PYSEC 2023 192",
            "javascript:alert(1)",
            "https://evil.test/x",
            "PYSEC-2023-192?x=1",
            "PYSEC-2023-192#f",
            "%2e%2e",
            "1PYSEC",
            "ab",
        ] {
            assert_eq!(advisory_url(bad), None, "{bad:?} must not become a URL");
        }

        assert!(advisory_url("PYSEC-2023-192").is_some());
        assert!(advisory_url("GHSA-v845-jxx5-vc9f").is_some());
        assert!(advisory_url("CVE-2023-43804").is_some());
    }

    #[test]
    fn fixable_advisories_sort_ahead_of_unfixable_ones() {
        // The order PRD P1-1 gets instead of severity, so it is asserted rather than assumed.
        let mk = |pkg: &str, id: &str, fixes: &[&str]| Advisory {
            pkg: PkgName::parse(pkg).expect("valid"),
            version: Version("1.0".to_owned()),
            id: id.to_owned(),
            aliases: vec![],
            fix_versions: fixes.iter().map(|s| (*s).to_owned()).collect(),
            description: String::new(),
            url: None,
        };
        let got = sort_advisories(vec![
            mk("zlib-wrapper", "PYSEC-1", &["1.1"]),
            mk("aiohttp", "PYSEC-3", &[]),
            mk("aiohttp", "PYSEC-2", &["2.0"]),
        ]);

        let order: Vec<&str> = got.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(order, ["PYSEC-2", "PYSEC-3", "PYSEC-1"]);
    }

    #[test]
    fn stdout_that_is_not_the_promised_document_is_a_tool_failure() {
        // Not a panic, and not a silent empty report: pip-audit printing anything else means the
        // run did not do what was asked, and PD-HLT-002 is the code that says so.
        let err = parse("Found 10 known vulnerabilities in 1 package").expect_err("not JSON");
        assert_eq!(err.code, Code::HltToolFailed);
    }

    #[test]
    fn a_clean_environment_is_zero_advisories_over_a_real_package_count() {
        let doc =
            r#"{"dependencies":[{"name":"certifi","version":"2024.2.2","vulns":[]}],"fixes":[]}"#;
        let got = parse(doc).expect("parses");

        assert_eq!((got.packages, got.advisories.len()), (1, 0));
    }

    // -- the runner -------------------------------------------------------------

    fn a_python_env() -> PyEnv {
        PyEnv {
            interpreter: std::path::PathBuf::from(r"C:\Python312\python.exe"),
            prefix: std::path::PathBuf::from(r"C:\Python312"),
            python_version: "3.12.10".to_owned(),
            externally_managed: false,
            hidden_user_site: None,
            source: crate::model::EnvSource::Manual,
        }
    }

    #[test]
    fn no_argv_entry_bundles_two_arguments() {
        // The same guard Code Health's argv has, and for the same reason: a single entry reading
        // "-f json" is one argument to `CreateProcess`, and the tool would reject it as a flag it
        // does not have. SECURITY §2's argv-array rule only means anything if the array is split.
        let argv = argv(std::path::Path::new(r"C:\tmp\freeze.txt"));

        for entry in &argv {
            assert!(
                !entry.trim().contains(' ') || entry.ends_with(".txt"),
                "{entry:?} bundles two arguments"
            );
        }
        assert_eq!(argv[0], "-r");
        assert_eq!(argv[2], "--no-deps");
        assert_eq!(
            &argv[3..],
            ["-f", "json"],
            "SP-4's spelling, not SECURITY's"
        );
    }

    #[tokio::test]
    async fn a_missing_tool_is_a_problem_on_a_report_rather_than_an_error() {
        // The shape the screen depends on. An audit venv that has never synced must still produce
        // a report — with an error row and an honest zero — because returning `Err` here would
        // leave the Security tab with nothing to render but a blank screen, which is the failure
        // `PdHealthReport`'s `problems` list exists to prevent.
        let dir = std::env::temp_dir().join(format!("pipdock-audit-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let sink = ProgressSink::new(tx, AUDIT_STEPS, Default::default());

        let report = run(&dir, &a_python_env(), "urllib3==2.0.0\n", &sink)
            .await
            .expect("a failed run still returns a report");

        assert_eq!(report.problems.len(), 1);
        assert_eq!(report.problems[0].code, Code::HltToolMissing);
        assert_eq!(report.problems[0].tool, "pip-audit");
        assert!(report.advisories.is_empty());
        assert_eq!(report.packages, 0, "nothing was audited, and it says so");
        assert!(!report.has_findings());
        assert!(
            !report.ran_at.is_empty(),
            "a run that failed still happened"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The whole path against the real tool, which nothing above covers.
    ///
    /// Needs a synced audit venv, so it is opt-in the way `search_latency` is:
    ///
    /// ```text
    /// cargo test -p pipdock-core --lib audit::tests::the_real_tool -- --ignored --nocapture
    /// ```
    ///
    /// Point `PIPDOCK_AUDIT_DIR` at a directory whose `.venv\Scripts\pip-audit.exe` exists. The
    /// freeze below is the SP-4 environment, so a passing run must find the same eight advisories
    /// the committed fixture carries — which is what makes this a check on the *parser* against a
    /// live tool rather than only on the tool.
    #[tokio::test]
    #[ignore = "needs a synced audit venv; run with --ignored and PIPDOCK_AUDIT_DIR"]
    async fn the_real_tool_finds_what_the_fixture_says() {
        let Ok(dir) = std::env::var("PIPDOCK_AUDIT_DIR") else {
            panic!("set PIPDOCK_AUDIT_DIR to a directory holding a synced audit venv");
        };
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let sink = ProgressSink::new(tx, AUDIT_STEPS, Default::default());

        let report = run(
            std::path::Path::new(&dir),
            &a_python_env(),
            "urllib3==2.0.0\n",
            &sink,
        )
        .await
        .expect("returns a report");

        assert!(report.problems.is_empty(), "{:?}", report.problems);
        assert_eq!(report.packages, 1);
        assert_eq!(report.advisories.len(), 8, "the fixture's count, live");
        assert!(report.advisories.iter().all(|a| a.url.is_some()));
    }

    /// Cancelling one, against the real tool, because nothing smaller can.
    ///
    /// `audit_once` checks the executable exists before it spawns anything, so a fake directory
    /// returns `PD-HLT-001` and never reaches the token — the cancel path is only observable with
    /// a tool that actually runs. Same opt-in as above.
    #[tokio::test]
    #[ignore = "needs a synced audit venv; run with --ignored and PIPDOCK_AUDIT_DIR"]
    async fn a_cancelled_run_is_cancelled_rather_than_failed() {
        let Ok(dir) = std::env::var("PIPDOCK_AUDIT_DIR") else {
            panic!("set PIPDOCK_AUDIT_DIR to a directory holding a synced audit venv");
        };
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let token = tokio_util::sync::CancellationToken::new();
        let sink = ProgressSink::new(tx, AUDIT_STEPS, token.clone());

        // Cancelled while the advisory fetch is in flight — the 18-68 s window this feature exists
        // to make interruptible.
        let stopper = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            token.cancel();
        });
        let report = run(
            std::path::Path::new(&dir),
            &a_python_env(),
            "urllib3==2.0.0
",
            &sink,
        )
        .await
        .expect("a cancelled run still returns a report");
        stopper.await.expect("stopper");

        // The whole point of P4's warning: a cancel must not read as `PipDock hit an internal
        // error`, which is what `PD-INT-001` renders as.
        assert!(report.cancelled, "a stopped run says so");
        assert!(
            report.problems.is_empty(),
            "a cancel is not a failure: {:?}",
            report.problems
        );
        assert!(report.advisories.is_empty());
    }
}
