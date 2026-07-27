//! The error-catalog corpus gate.
//!
//! `docs/TESTING.md` §2: *"every catalog code has ≥ 1 stderr fixture; a test enforces 'no code
//! without fixture'"*. This is that test, run against the fixtures spike SP-2 captured plus the
//! labelled synthetic ones for conditions the dev machine cannot produce.
//!
//! It does two jobs a unit test cannot: it proves the classifiers fire on **real engine output**
//! rather than on strings someone typed into a test, and it fails when a new `Code` variant is
//! added without evidence of what it looks like in the wild.

// Integration tests are their own crate, so the lib's test-only allow does not reach them.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use pipdock_core::errors::{Code, classify_stderr};

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// One fixture directory: what it expects, and what the engine actually wrote.
struct Fixture {
    label: String,
    expects: Option<String>,
    stderr: String,
    synthetic: bool,
}

fn load_all() -> Vec<Fixture> {
    let mut out = Vec::new();
    let root = fixtures_root();
    for engine in ["pip", "uv", "synthetic"] {
        let dir = root.join(engine);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let meta_path = path.join("meta.json");
            let Ok(meta_raw) = fs::read_to_string(&meta_path) else {
                continue;
            };
            let meta: serde_json::Value =
                serde_json::from_str(&meta_raw).unwrap_or_else(|e| panic!("{meta_path:?}: {e}"));
            let stderr = fs::read_to_string(path.join("stderr.txt")).unwrap_or_default();
            out.push(Fixture {
                label: format!(
                    "{engine}/{}",
                    path.file_name().unwrap_or_default().display()
                ),
                expects: meta["expects_code"].as_str().map(str::to_owned),
                stderr,
                synthetic: meta["synthetic"].as_bool().unwrap_or(false),
            });
        }
    }
    assert!(
        !out.is_empty(),
        "no fixtures found under {:?}",
        fixtures_root()
    );
    out
}

#[test]
fn every_fixture_with_an_expected_code_classifies_to_it() {
    let mut failures = Vec::new();
    for f in load_all() {
        let Some(expected) = &f.expects else { continue };
        // Yanked releases exit 0 in both engines and are a preview warning, not a failure
        // (SP-2), so the classifier is not the component responsible for them.
        if expected == "PD-PKG-003" {
            continue;
        }
        let got = classify_stderr(&f.stderr);
        if got.as_str() != expected {
            failures.push(format!("{}: expected {expected}, got {got}", f.label));
        }
    }
    assert!(
        failures.is_empty(),
        "classifier mismatches:\n  {}",
        failures.join("\n  ")
    );
}

#[test]
fn no_code_without_a_fixture() {
    // The gate TESTING §2 asks for: adding a Code variant without evidence fails CI.
    let fixtures = load_all();

    // A code is covered when some fixture names it, or when a fixture's stderr classifies to it.
    let mut covered: BTreeSet<&'static str> = BTreeSet::new();
    for f in &fixtures {
        if let Some(code) = f
            .expects
            .as_ref()
            .and_then(|expected| Code::ALL.iter().find(|c| c.as_str() == expected))
        {
            covered.insert(code.as_str());
        }
        let got = classify_stderr(&f.stderr);
        if got != Code::EngUnclassified {
            covered.insert(got.as_str());
        }
    }

    // Codes PipDock raises itself, which by definition have no engine stderr to capture. Each
    // names the module that owns it, so this list stays honest rather than becoming a dumping
    // ground for anything inconvenient.
    let internally_raised: BTreeMap<&str, &str> = BTreeMap::from([
        (
            "PD-ENV-001",
            "envs: interpreter path missing before any command runs",
        ),
        // PEP 668 applies only outside a venv, so it cannot be reproduced without planting the
        // marker in a real system Python. It does not need to be: probe.py reports
        // `externally_managed` and the environment is blocked at step zero (DATA-FLOW §2), before
        // any engine runs. The classifier rule is a backstop for the Settings override path.
        (
            "PD-ENV-002",
            "envs: PEP 668 marker seen by probe.py before any engine command",
        ),
        ("PD-ENV-003", "envs: probe.py output unreadable"),
        ("PD-ENG-001", "exec: the engine binary could not be spawned"),
        (
            "PD-ENG-002",
            "engine: pip version below the 22.2 report floor",
        ),
        (
            "PD-ENG-003",
            "engine::uv: plan text did not match any known shape",
        ),
        (
            "PD-RES-002",
            "plan: report older than PLAN_MAX_AGE or env drifted",
        ),
        (
            "PD-PKG-001",
            "compat: Requires-Python enforced by PipDock, not by the engine",
        ),
        (
            "PD-PKG-003",
            "engine: yanked releases exit 0; surfaced as a preview warning",
        ),
        ("PD-NET-010", "index: PEP 691 refresh failed"),
        (
            "PD-NET-011",
            "health: tools venv bootstrap could not reach PyPI",
        ),
        ("PD-SNP-001", "snapshot: write failed pre-execution"),
        (
            "PD-SNP-002",
            "snapshot: rollback target unavailable upstream",
        ),
        ("PD-HLT-001", "health: tool missing from the tools venv"),
        ("PD-HLT-002", "health: tool exited non-zero"),
        ("PD-HLT-003", "health: tool exceeded its watchdog"),
        ("PD-INT-001", "anywhere: a PipDock bug"),
        ("PD-ENG-999", "the fallback itself"),
    ]);

    let missing: Vec<&str> = Code::ALL
        .iter()
        .map(|c| c.as_str())
        .filter(|c| !covered.contains(c) && !internally_raised.contains_key(c))
        .collect();

    assert!(
        missing.is_empty(),
        "these codes have no fixture and are not listed as internally raised: {missing:?}\n\
         Capture one with `py -3.12 spikes/capture.py`, or add it to the internally_raised table \
         with the module that owns it."
    );
}

#[test]
fn synthetic_fixtures_are_labelled_and_justified() {
    // Synthetic fixtures are a stopgap for conditions this machine cannot produce (SP-2). They
    // must stay obviously distinguishable from captures so nobody treats them as evidence of real
    // engine behaviour, and each must say what would replace it.
    let root = fixtures_root().join("synthetic");
    let entries = fs::read_dir(&root).unwrap_or_else(|e| panic!("{root:?}: {e}"));
    let mut count = 0;
    for entry in entries.flatten() {
        let meta_path = entry.path().join("meta.json");
        let Ok(raw) = fs::read_to_string(&meta_path) else {
            continue;
        };
        let meta: serde_json::Value = serde_json::from_str(&raw).expect("valid meta.json");
        assert_eq!(
            meta["synthetic"].as_bool(),
            Some(true),
            "{meta_path:?} must be flagged"
        );
        assert!(
            meta["why"].is_string(),
            "{meta_path:?} must say why it is synthetic"
        );
        assert!(
            meta["replace_with"].is_string(),
            "{meta_path:?} must say what replaces it"
        );
        count += 1;
    }
    assert_eq!(
        count, 8,
        "expected exactly the eight documented unreproducible conditions"
    );
}

#[test]
fn real_captures_are_not_silently_synthetic() {
    for f in load_all() {
        if f.label.starts_with("synthetic/") {
            assert!(f.synthetic, "{} must be flagged synthetic", f.label);
        } else {
            assert!(
                !f.synthetic,
                "{} is a capture and must not be flagged synthetic",
                f.label
            );
        }
    }
}

#[test]
fn successful_runs_are_never_classified_as_failures() {
    // pip prints its own upgrade notice to stderr on almost every run, and uv writes its entire
    // plan there (SP-1). Treating a non-empty stderr as failure would break both engines, so the
    // classifier must stay quiet on output from commands that exited zero.
    for f in load_all() {
        if f.expects.is_some() || f.label.starts_with("synthetic/") {
            continue;
        }
        let got = classify_stderr(&f.stderr);
        assert_eq!(
            got,
            Code::EngUnclassified,
            "{}: successful run classified as {got}\n--- stderr ---\n{}",
            f.label,
            f.stderr
        );
    }
}

#[test]
fn crlf_in_fixtures_is_preserved() {
    // .gitattributes marks the fixture tree -text so Windows line endings survive. If that
    // regressed, the parsers would be tested against endings that never occur in production.
    let with_crlf = load_all()
        .iter()
        .filter(|f| f.stderr.contains("\r\n"))
        .count();
    assert!(
        with_crlf > 0,
        "no fixture retained CRLF — check .gitattributes"
    );
}
