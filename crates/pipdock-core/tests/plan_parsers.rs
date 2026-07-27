//! Replay every captured engine output through the plan parsers.
//!
//! `docs/TESTING.md` §1.1 makes adapter parsing "the product stands on it" — the first thing that
//! must never regress. Unit tests in `engine::parse` cover the shapes; this covers the **actual
//! bytes two real engines wrote**, CRLF and wrapping and decoration included, which is what the
//! weekly latest-engine job in CI will re-run when pip or uv ships a new release.
//!
//! The assertions are deliberately about *meaning* rather than exact structures: that a plan is
//! recognised at all, that an upgrade knows what it is upgrading from, that impossibility is
//! reported as impossibility. A snapshot test would fail on every cosmetic engine change and
//! teach us to re-bless it without reading.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use pipdock_core::engine::parse::{ParsedPlan, pip_report, uv_plan};
use pipdock_core::model::{Dist, PkgName, Version};

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

struct Capture {
    stdout: String,
    stderr: String,
    exit_code: i64,
}

fn load(engine: &str, scenario: &str) -> Option<Capture> {
    let dir = fixtures_root().join(engine).join(scenario);
    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("meta.json")).ok()?).ok()?;
    Some(Capture {
        stdout: fs::read_to_string(dir.join("stdout.txt")).unwrap_or_default(),
        stderr: fs::read_to_string(dir.join("stderr.txt")).unwrap_or_default(),
        exit_code: meta["exit_code"].as_i64().unwrap_or(-1),
    })
}

fn parse(engine: &str, scenario: &str, installed: &[Dist]) -> ParsedPlan {
    let cap =
        load(engine, scenario).unwrap_or_else(|| panic!("missing fixture {engine}/{scenario}"));
    let result = match engine {
        "pip" => pip_report(&cap.stdout, &cap.stderr, installed),
        _ => uv_plan(&cap.stdout, &cap.stderr, installed),
    };
    result.unwrap_or_else(|e| panic!("{engine}/{scenario} failed to parse: {e}"))
}

fn dist(name: &str, version: &str) -> Dist {
    Dist {
        name: PkgName::parse(name).unwrap(),
        version: Version(version.into()),
        requires_dist: Vec::new(),
        requires_python: None,
    }
}

#[test]
fn both_engines_agree_on_the_clean_upgrade() {
    // The same real situation, parsed from JSON on stdout for pip and wrapped text on stderr for
    // uv, must produce the same normalized change. That equivalence is the entire premise of the
    // Engine trait (ARCHITECTURE §3).
    let installed = [dist("idna", "3.4")];
    for engine in ["pip", "uv"] {
        let plan = parse(engine, "clean-upgrade", &installed);
        let idna = plan
            .report
            .changes
            .iter()
            .find(|c| c.name.as_str() == "idna")
            .unwrap_or_else(|| panic!("{engine}: no idna change"));
        assert_eq!(
            idna.from.as_ref().map(|v| v.0.as_str()),
            Some("3.4"),
            "{engine}"
        );
        assert_eq!(idna.to.0, "3.18", "{engine}");
        assert!(plan.report.is_clean(), "{engine}");
    }
}

#[test]
fn both_engines_plan_the_same_environment_breaking_upgrade() {
    // SP-1's central finding, locked in as a test: unconstrained, both engines take httpcore to
    // 1.0.9 even though the installed httpx 0.23.0 requires <0.16. If a future engine version
    // starts holding it back instead, this test tells us the planning strategy can be simplified.
    let installed = [
        dist("httpx", "0.23.0"),
        dist("httpcore", "0.15.0"),
        dist("h11", "0.12.0"),
    ];
    for engine in ["pip", "uv"] {
        let plan = parse(engine, "held-back", &installed);
        let httpcore = plan
            .report
            .changes
            .iter()
            .find(|c| c.name.as_str() == "httpcore")
            .unwrap_or_else(|| panic!("{engine}: no httpcore change"));
        assert_eq!(
            httpcore.to.0, "1.0.9",
            "{engine} no longer breaks httpx — revisit SP-1"
        );
    }
}

#[test]
fn both_engines_produce_an_empty_plan_once_the_installed_set_is_restated() {
    // The other half of SP-1: with httpx restated as a requirement, the correct answer is "no
    // changes". This is what PipDock's planner relies on, so it is a regression gate.
    let installed = [dist("httpx", "0.23.0"), dist("httpcore", "0.15.0")];
    for engine in ["pip", "uv"] {
        let plan = parse(engine, "held-back-constrained", &installed);
        assert!(
            plan.report.changes.is_empty(),
            "{engine} planned {:?} where nothing should change",
            plan.report.changes
        );
    }
}

#[test]
fn uv_impossibility_carries_an_explanation_a_user_can_act_on() {
    let plan = parse("uv", "impossible", &[]);
    let detail = plan.report.impossible.expect("uv reports impossibility");

    // SP-1: uv names the blocking constraint where pip does not. That advantage is the reason the
    // preview can explain a conflict at all under uv, so assert the substance is present.
    assert!(
        detail.explanation.contains("httpcore>=0.15.0,<0.16.0"),
        "lost the constraint: {}",
        detail.explanation
    );
    assert!(detail.packages.iter().any(|p| p.as_str() == "httpx"));
}

#[test]
fn pip_impossibility_is_recognised_even_though_it_says_less() {
    // pip writes nothing to stdout when resolution fails, so the report parser must fail rather
    // than silently returning an empty plan — an empty plan would render as "no changes" and the
    // user would never learn the resolve failed.
    let cap = load("pip", "impossible").expect("fixture");
    assert_eq!(cap.exit_code, 1);
    let err = pip_report(&cap.stdout, &cap.stderr, &[]).expect_err("must not parse as empty plan");
    assert_eq!(err.code.as_str(), "PD-ENG-002");
}

#[test]
fn both_engines_flag_the_yanked_release_without_failing() {
    // SP-2: a yank exits 0 in both engines. The plan is valid; the warning rides alongside it.
    for engine in ["pip", "uv"] {
        let plan = parse(engine, "yanked", &[]);
        assert_eq!(
            plan.yanked.len(),
            1,
            "{engine} should flag exactly one yank"
        );
        assert_eq!(plan.yanked[0].pkg.as_str(), "urllib3", "{engine}");
        assert_eq!(plan.yanked[0].version.0, "2.0.0", "{engine}");
        assert!(
            plan.yanked[0]
                .reason
                .as_deref()
                .is_some_and(|r| r.contains("Truncated")),
            "{engine} lost the yank reason: {:?}",
            plan.yanked[0].reason
        );
        assert!(
            plan.report.is_clean(),
            "{engine}: a yank must not make the plan dirty"
        );
        assert!(
            !plan.report.changes.is_empty(),
            "{engine}: the install is still planned"
        );
    }
}

#[test]
fn the_encoding_crash_fixture_is_reported_as_a_pipdock_bug() {
    // The fixture captured without the UTF-8 mitigation. Reporting it as "upgrade pip" would send
    // the user chasing the wrong thing, so the parser must tell the two apart.
    let cap = load("pip", "report-encoding-crash").expect("fixture");
    assert_eq!(cap.exit_code, 2);
    let err = pip_report(&cap.stdout, &cap.stderr, &[]).expect_err("crash must not parse");
    assert_eq!(err.code.as_str(), "PD-INT-001");
}

#[test]
fn every_successful_capture_parses() {
    // A blanket sweep: anything an engine emitted on a successful run must be parseable, or the
    // adapter will fail on a real user's machine in a situation we have already seen.
    let scenarios = [
        "clean-upgrade",
        "held-back",
        "held-back-constrained",
        "yanked",
        "requires-python",
    ];
    for engine in ["pip", "uv"] {
        for scenario in scenarios {
            let Some(cap) = load(engine, scenario) else {
                continue;
            };
            if cap.exit_code != 0 {
                continue;
            }
            let result = match engine {
                "pip" => pip_report(&cap.stdout, &cap.stderr, &[]),
                _ => uv_plan(&cap.stdout, &cap.stderr, &[]),
            };
            assert!(result.is_ok(), "{engine}/{scenario}: {:?}", result.err());
        }
    }
}

#[test]
fn uv_scenarios_all_arrive_on_stderr() {
    // SP-1's channel finding, guarded: if a future uv moves the plan to stdout, this fails and the
    // adapter's stdout fallback needs promoting to the primary path.
    let mut checked = 0;
    for scenario in [
        "clean-upgrade",
        "held-back",
        "held-back-constrained",
        "yanked",
    ] {
        let Some(cap) = load("uv", scenario) else {
            continue;
        };
        assert!(
            cap.stdout.trim().is_empty(),
            "uv/{scenario} now writes to stdout"
        );
        assert!(
            !cap.stderr.trim().is_empty(),
            "uv/{scenario} wrote nothing to stderr"
        );
        checked += 1;
    }
    assert!(
        checked >= 4,
        "expected at least four uv fixtures, saw {checked}"
    );
}
