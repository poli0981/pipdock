//! The payloads the frontend's L3 tests mock `@/ipc` with.
//!
//! TESTING §2 asks for "IPC mocked at the typed wrapper layer with recorded core payloads — the
//! same JSON as L1 fixtures, guaranteeing UI and core agree on shapes". Hand-writing that JSON
//! would defeat the guarantee: rename a field in `model.rs` and the component tests keep passing
//! against a shape the app never sends. So the fixtures are **serialized from the real types**,
//! and a test fails when the committed files no longer match — the same mechanism, and the same
//! reasoning, as [`crate::bindings`].
//!
//! Generated into `ui/` rather than read out of `crates/` behind a Vite alias, for the same reason
//! `generated.ts` is: one directory the frontend owns, no build-time path that has to agree with
//! `vite.config.ts`.
//!
//! The *shapes* come from the types. The *contents* are a deliberate scenario, modelled on the
//! SP-5 dogfood environment, chosen so every rule S2's table has to implement is exercised by at
//! least one row:
//!
//! | Row | Exercises |
//! |---|---|
//! | `numpy` | outdated **and** held at a version — the `UPDATE` badge and a `Hold` 🔒 together |
//! | `scipy` | outdated and `Exclude`-pinned — excluded from *Select all* |
//! | `pandas` | outdated and unpinned — the one *Select all* may actually take |
//! | `requests` | outdated and unpinned, so "N pinned excluded" is 2 of 4 rather than 1 of 2 |
//! | `certifi` | up to date — the dimming rule |
//! | `editable-lib` | up to date and **no `sizeBytes`** — the em-dash cell |
//!
//! That the set is the SP-5 tangle pays off twice: `numpy` has two dependents declaring two
//! *different* specifiers, which is what the uninstall guard's dialog exists to show, so
//! `guard_report.json` is computed from the same rows rather than invented.
//!
//! `snapshot_list.json` carries one of each trigger, with the `Rollback` entry restoring the
//! `Plan` entry above it — the arrangement that makes `latest` move twice across one restore, and
//! the reason the timeline has to label them.

use crate::model::{Dist, OutdatedDist, PkgName, Version};
use crate::pins::{Pin, PinMode};

/// Directory the fixtures are written to, relative to the repository root.
pub const OUTPUT_DIR: &str = "ui/src/test/fixtures";

/// Build a `Dist` the way the probe would.
fn dist(name: &str, version: &str, requires: &[&str], size: Option<u64>) -> Dist {
    Dist {
        // Every name here is a literal in this file, so a parse failure is a typo in the fixture
        // rather than anything a user could cause.
        name: PkgName::parse(name).unwrap_or_else(|e| panic!("fixture name {name:?}: {e:?}")),
        version: Version(version.to_owned()),
        requires_dist: requires.iter().map(|s| (*s).to_owned()).collect(),
        requires_python: Some(">=3.10".to_owned()),
        size_bytes: size,
    }
}

fn outdated(name: &str, current: &str, latest: &str) -> OutdatedDist {
    OutdatedDist {
        name: PkgName::parse(name).unwrap_or_else(|e| panic!("fixture name {name:?}: {e:?}")),
        current: Version(current.to_owned()),
        latest: Version(latest.to_owned()),
    }
}

/// What `pkg_list` returns.
fn pkg_list() -> Vec<Dist> {
    vec![
        dist("certifi", "2024.2.2", &[], Some(163_840)),
        // No size: an editable install's RECORD lists its import shim, not its sources, so the
        // probe reports nothing rather than a few hundred bytes (see `probe.py`).
        dist("editable-lib", "0.3.0", &[], None),
        dist("numpy", "1.26.4", &[], Some(62_914_560)),
        dist(
            "pandas",
            "2.1.4",
            &["numpy<2,>=1.26.0", "python-dateutil>=2.8.2"],
            Some(46_137_344),
        ),
        dist("requests", "2.28.0", &["urllib3<3,>=1.21.1"], Some(133_120)),
        dist(
            "scipy",
            "1.11.4",
            &["numpy<1.28.0,>=1.21.6"],
            Some(58_720_256),
        ),
    ]
}

/// What `pkg_outdated` returns — a strict subset of the names above, which is the join the
/// Installed table performs.
fn pkg_outdated() -> Vec<OutdatedDist> {
    vec![
        outdated("numpy", "1.26.4", "2.5.1"),
        outdated("pandas", "2.1.4", "2.3.0"),
        outdated("requests", "2.28.0", "2.32.3"),
        outdated("scipy", "1.11.4", "1.14.1"),
    ]
}

/// What `pin_list` returns. Two of the four outdated rows are pinned, so *Select all* has a
/// non-trivial number to report and the two `PinMode` variants are both on screen.
fn pin_list() -> Vec<Pin> {
    vec![
        Pin {
            pkg: PkgName::parse("numpy").unwrap_or_else(|_| unreachable!()),
            mode: PinMode::Hold {
                version: Version("1.26.4".to_owned()),
            },
            reason: Some("scipy 1.11.4 needs numpy < 1.28".to_owned()),
        },
        Pin {
            pkg: PkgName::parse("scipy").unwrap_or_else(|_| unreachable!()),
            mode: PinMode::Exclude,
            reason: None,
        },
    ]
}

/// What `plan_resolve` returns for the same scenario: a preview that needs decisions.
///
/// Covers **all four** `ChangeKind` variants, a held-back package with a real blocker, and an
/// impossible one — so the preview's grouping and the conflict control's two states each have a
/// subject. `Downgrade` in particular: it is the variant UI-SPEC had no section for, and a
/// compatible resolve produces it routinely.
fn flow_step() -> crate::flow::FlowStep {
    use crate::plan::{Blocker, Change, ChangeKind, HeldBack, ImpossibleDetail, ResolutionReport};

    let pkg = |n: &str| PkgName::parse(n).unwrap_or_else(|e| panic!("fixture name {n:?}: {e:?}"));
    let change = |name: &str, from: Option<&str>, to: &str, kind: ChangeKind| Change {
        name: pkg(name),
        from: from.map(|v| Version(v.to_owned())),
        to: Version(to.to_owned()),
        kind,
    };

    crate::flow::FlowStep::NeedsDecisions {
        report: ResolutionReport {
            changes: vec![
                change("pandas", Some("2.1.4"), "2.3.0", ChangeKind::Upgrade),
                change("requests", Some("2.28.0"), "2.32.3", ChangeKind::Upgrade),
                // The case that had no home: satisfying pandas 2.3 means moving this *down*.
                change("urllib3", Some("2.2.0"), "1.26.20", ChangeKind::Downgrade),
                change("tzdata", None, "2025.1", ChangeKind::NewDependency),
                change("httpx", None, "0.28.1", ChangeKind::NewInstall),
            ],
            held_back: vec![HeldBack {
                pkg: pkg("numpy"),
                resolved: Version("1.26.4".to_owned()),
                latest: Version("2.5.1".to_owned()),
                blockers: vec![Blocker {
                    by: Some(pkg("scipy")),
                    constraint: "numpy<1.28.0,>=1.21.6".to_owned(),
                }],
            }],
            impossible: Some(ImpossibleDetail {
                explanation: "no version of oldlib is compatible with python 3.12".to_owned(),
                packages: vec![pkg("oldlib")],
            }),
            raw: String::new(),
        },
        round: 0,
        rounds_remaining: 3,
    }
}

/// What `plan_execute` returns after a run that was stopped part-way.
///
/// The cancelled case on purpose: it is the one DATA-FLOW §6 does not work through, the one whose
/// copy was deferred out of Stage 1, and the one where the counts and the rows are most likely to
/// disagree — thirteen applied, one failed, the rest never attempted.
fn execution_summary() -> crate::plan::ExecutionSummary {
    use crate::model::{CheckReport, ExecMode, StepResult, StepStatus};

    let pkg = |n: &str| PkgName::parse(n).unwrap_or_else(|e| panic!("fixture name {n:?}: {e:?}"));
    let step = |name: &str, to: &str, status: StepStatus| StepResult {
        pkg: pkg(name),
        from: None,
        to: Some(Version(to.to_owned())),
        status,
        code: (status == StepStatus::Failed).then_some(crate::errors::Code::BldBackendFailed),
        stderr_tail: (status == StepStatus::Failed)
            .then(|| "metadata-generation-failed".to_owned()),
    };

    let results = vec![
        step("pandas", "2.3.0", StepStatus::Ok),
        step("requests", "2.32.3", StepStatus::Ok),
        step("oldlib", "2.0.0", StepStatus::Failed),
        step("httpx", "0.28.1", StepStatus::Skipped),
    ];
    let counts = crate::plan::ExecutionSummary::tally(&results);

    crate::plan::ExecutionSummary {
        plan_id: "update-abc12345".to_owned(),
        phase: ExecMode::Isolated,
        results,
        check: CheckReport {
            ok: true,
            findings: Vec::new(),
        },
        counts,
        cancelled: true,
    }
}

/// What `uninstall_guard` returns for removing `numpy` from the same scenario.
///
/// **Computed, not written.** Running the real graph over the real `pkg_list()` is what makes this
/// fixture worth having: hand-writing the map would let the guard's own rules — marker evaluation,
/// the extra-only exclusion, dedup by constraint — drift away from what the dialog is tested
/// against. `numpy` is the removal because the fixture set was built as the SP-5 tangle, so it has
/// two dependents with two different specifiers, which is exactly the case the dialog exists for.
fn guard_report() -> crate::graph::GuardReport {
    crate::graph::ReverseDeps::build_for(&pkg_list(), "3.12.4").guard(&[
        PkgName::parse("numpy").unwrap_or_else(|e| panic!("fixture name \"numpy\": {e:?}"))
    ])
}

/// What `snapshot_list` returns: one of each trigger, newest first.
///
/// All three on purpose. A `Rollback` entry *restoring* the `Plan` entry above it is the shape
/// that makes `latest` move twice across a single restore — the trap that made TESTING L2's first
/// run report every package as changed when the rollback had worked — so the timeline has to be
/// able to tell them apart, and a fixture with one trigger could not prove it does.
fn snapshot_list() -> Vec<crate::snapshot::Meta> {
    use crate::snapshot::{Meta, Trigger};
    let meta = |id: &str, at: &str, trigger: Trigger, count: usize| Meta {
        id: id.to_owned(),
        created_at: at.to_owned(),
        trigger,
        engine: crate::model::EngineId::Pip,
        package_count: count,
        app_version: "0.1.0".to_owned(),
    };
    vec![
        meta(
            "20260809T140000-0000000Z",
            "2026-08-09T14:00:00Z",
            Trigger::Rollback {
                restoring: "20260809T120000-0000000Z".to_owned(),
            },
            6,
        ),
        meta(
            "20260809T130000-0000000Z",
            "2026-08-09T13:00:00Z",
            Trigger::Plan {
                plan_id: "update-abc12345".to_owned(),
            },
            6,
        ),
        meta(
            "20260809T120000-0000000Z",
            "2026-08-09T12:00:00Z",
            Trigger::Manual,
            5,
        ),
    ]
}

/// What `snapshot_rollback_preview` returns, with all three sections populated.
///
/// The `unrestorable` entry is the reason this fixture exists rather than an empty preview:
/// DATA-FLOW §8 requires those lines to be listed explicitly as `PD-SNP-002` before the confirm,
/// and a component test against a preview that has none would pass whether or not they render.
fn rollback_preview() -> crate::flow::RollbackPreview {
    let pkg = |n: &str| PkgName::parse(n).unwrap_or_else(|e| panic!("fixture name {n:?}: {e:?}"));
    crate::flow::RollbackPreview {
        target: snapshot_list()
            .into_iter()
            .next_back()
            .unwrap_or_else(|| unreachable!("the list is not empty")),
        restore: crate::snapshot::RollbackPlan {
            uninstall: vec![pkg("httpx")],
            install: vec![
                crate::model::PinnedSpec {
                    name: pkg("numpy"),
                    version: Version("1.26.4".to_owned()),
                },
                crate::model::PinnedSpec {
                    name: pkg("requests"),
                    version: Version("2.28.0".to_owned()),
                },
            ],
        },
        unrestorable: vec![
            "-e C:\\src\\editable-lib".to_owned(),
            "local-wheel @ file:///C:/wheels/local_wheel-1.0-py3-none-any.whl".to_owned(),
        ],
    }
}

/// Every catalog code, in `Code::ALL` order.
///
/// Not a payload any command returns — a *contract*. `i18n.test.ts` asserts that every code has a
/// one-liner in both locales, and it has to read the list from somewhere: a hand-written array in
/// TypeScript would drift the moment a code is added, and the test would go on passing over a
/// catalog that is no longer complete. Generated here, so `cargo run -p xtask -- ipc-fixtures`
/// and its staleness test carry it the same way they carry everything else.
fn codes() -> Vec<crate::errors::Code> {
    crate::errors::Code::ALL.to_vec()
}

/// A finished Code Health run, with every tab in its populated state.
///
/// **Two documents, not one.** A populated ruff tab and a ruff `ToolProblem` are contradictory
/// states of the same tool, so one fixture would leave either the ruff tab or the partial-report
/// branch with no subject. [`health_partial`] is the other half.
///
/// The deptry rows are chosen against [`pkg_list`], because the *Review in Uninstall…* button has
/// two gates and both need a case:
///
/// | Row | Exercises |
/// |---|---|
/// | `DEP002` `requests` | unused **and** an installed distribution — the one row that offers the button |
/// | `DEP002` `httpx` | unused but not installed here — the same code, no button |
/// | `DEP001` `yaml` | a module name that is not a distribution (`PyYAML`), **and** the wrong code for the button twice over |
/// | `DEP003` `numpy` | two locations, so the location list is not always a singleton |
fn health_report() -> crate::health::HealthReport {
    use crate::health::{
        DeptryIssue, FixApplicability, HealthReport, RuffFinding, RuffFindings, SourceLocation,
        VultureFinding,
    };

    HealthReport {
        project: r"C:\proj\bot".to_owned(),
        env: r"c:\proj\.venv\scripts\python.exe".to_owned(),
        ran_at: "2026-08-13T09:00:00Z".to_owned(),
        tool_versions: [
            ("deptry".to_owned(), "0.25.1".to_owned()),
            ("ruff".to_owned(), "0.16.2".to_owned()),
            ("vulture".to_owned(), "2.16".to_owned()),
        ]
        .into_iter()
        .collect(),
        // Two files, so the requirements case is not silently tested as if it were always one.
        declared: crate::health::DeclaredSource::Requirements {
            files: vec![
                "requirements-dev.txt".to_owned(),
                "requirements.txt".to_owned(),
            ],
        },
        ran: crate::health::HEALTH_TOOLS
            .iter()
            .map(|t| (*t).to_owned())
            .collect(),
        deptry: vec![
            DeptryIssue {
                code: "DEP002".to_owned(),
                dep: "requests".to_owned(),
                message: "'requests' defined as a dependency but not used in the codebase"
                    .to_owned(),
                // deptry names no position for an unused declaration: the finding is about the
                // manifest, not a line of code. The renderer must not assume `line` is present.
                locations: vec![SourceLocation {
                    file: "requirements.txt".to_owned(),
                    line: None,
                    column: None,
                }],
            },
            DeptryIssue {
                code: "DEP002".to_owned(),
                dep: "httpx".to_owned(),
                message: "'httpx' defined as a dependency but not used in the codebase".to_owned(),
                locations: Vec::new(),
            },
            DeptryIssue {
                code: "DEP001".to_owned(),
                dep: "yaml".to_owned(),
                message: "'yaml' imported but missing from the dependency definitions".to_owned(),
                locations: vec![SourceLocation {
                    file: r"bot\config.py".to_owned(),
                    line: Some(4),
                    column: Some(1),
                }],
            },
            DeptryIssue {
                code: "DEP003".to_owned(),
                dep: "numpy".to_owned(),
                message: "'numpy' imported but it is a transitive dependency".to_owned(),
                locations: vec![
                    SourceLocation {
                        file: r"bot\stats.py".to_owned(),
                        line: Some(2),
                        column: Some(1),
                    },
                    SourceLocation {
                        file: r"bot\plot.py".to_owned(),
                        line: Some(9),
                        column: Some(1),
                    },
                ],
            },
        ],
        vulture: vec![
            VultureFinding {
                path: r"bot\util.py".to_owned(),
                line: 88,
                message: "unused function 'old_parse'".to_owned(),
                name: Some("old_parse".to_owned()),
                confidence: 60,
            },
            VultureFinding {
                path: r"bot\util.py".to_owned(),
                line: 91,
                message: "unused import 'os'".to_owned(),
                name: Some("os".to_owned()),
                confidence: 90,
            },
            VultureFinding {
                path: r"bot\worker.py".to_owned(),
                line: 12,
                // The eighth message shape, and the reason `name` is optional: it quotes the
                // token the dead code follows, not an identifier there is anything to name.
                message: "unreachable code after 'return'".to_owned(),
                name: None,
                confidence: 100,
            },
        ],
        // Built then recounted, so `fixable` and `fixableFiles` cannot disagree with the list they
        // describe — the same reason `recount` exists for the real runner. Two safe fixes in one
        // file and one in another makes the two numbers differ (3 and 2), which is what stops a
        // UI that confuses them from passing.
        ruff: RuffFindings {
            findings: vec![
                RuffFinding {
                    code: Some("F401".to_owned()),
                    name: "unused-import".to_owned(),
                    message: "`os` imported but unused".to_owned(),
                    filename: r"C:\proj\bot\bot\util.py".to_owned(),
                    row: 91,
                    column: 8,
                    url: Some("https://docs.astral.sh/ruff/rules/unused-import".to_owned()),
                    fix: Some(FixApplicability::Safe),
                },
                RuffFinding {
                    code: Some("I001".to_owned()),
                    name: "unsorted-imports".to_owned(),
                    message: "Import block is un-sorted or un-formatted".to_owned(),
                    filename: r"C:\proj\bot\bot\util.py".to_owned(),
                    row: 1,
                    column: 1,
                    url: Some("https://docs.astral.sh/ruff/rules/unsorted-imports".to_owned()),
                    fix: Some(FixApplicability::Safe),
                },
                RuffFinding {
                    code: Some("F841".to_owned()),
                    name: "unused-variable".to_owned(),
                    message: "Local variable `tmp` is assigned to but never used".to_owned(),
                    filename: r"C:\proj\bot\bot\worker.py".to_owned(),
                    row: 30,
                    column: 5,
                    url: Some("https://docs.astral.sh/ruff/rules/unused-variable".to_owned()),
                    fix: Some(FixApplicability::Safe),
                },
                RuffFinding {
                    code: Some("B006".to_owned()),
                    name: "mutable-argument-default".to_owned(),
                    message: "Do not use mutable data structures for argument defaults".to_owned(),
                    filename: r"C:\proj\bot\bot\worker.py".to_owned(),
                    row: 44,
                    column: 18,
                    url: Some(
                        "https://docs.astral.sh/ruff/rules/mutable-argument-default".to_owned(),
                    ),
                    // Offered but never applied: P5 runs plain `ruff check --fix`. A badge that
                    // implied otherwise would promise a fix the fix will not deliver.
                    fix: Some(FixApplicability::Unsafe),
                },
                RuffFinding {
                    code: Some("E722".to_owned()),
                    name: "bare-except".to_owned(),
                    message: "Do not use bare `except`".to_owned(),
                    filename: r"C:\proj\bot\bot\worker.py".to_owned(),
                    row: 52,
                    column: 5,
                    url: Some("https://docs.astral.sh/ruff/rules/bare-except".to_owned()),
                    fix: None,
                },
                RuffFinding {
                    // A syntax error carries `invalid-syntax` rather than a null code — the
                    // opposite of what CODE-HEALTH-SPEC §6 assumed — and is the one shape with
                    // **no rule page**, so it is the subject for the absent-link branch.
                    code: Some("invalid-syntax".to_owned()),
                    name: "invalid-syntax".to_owned(),
                    message: "Expected `)`, found newline".to_owned(),
                    filename: r"C:\proj\bot\bot\broken.py".to_owned(),
                    row: 1,
                    column: 11,
                    url: None,
                    fix: None,
                },
            ],
            fixable: 0,
            fixable_files: 0,
        }
        .recount(),
        problems: Vec::new(),
    }
}

/// The same run with ruff unrunnable — `PD-HLT-003`'s "partial report shown", as a shape.
///
/// One document covering all three states a tab can be in, which is the distinction `ran` exists
/// for and the one a screen keyed on `ran` alone gets wrong: **`ran` is filled before the loop**,
/// so a tool that failed is in it. Here deptry reported, ruff is in `ran` *and* in `problems`, and
/// vulture was never asked. A tab that renders ruff as "no lint findings" is the bug.
fn health_partial() -> crate::health::HealthReport {
    let full = health_report();
    crate::health::HealthReport {
        ran: vec!["deptry".to_owned(), "ruff".to_owned()],
        vulture: Vec::new(),
        ruff: crate::health::RuffFindings::default(),
        problems: vec![crate::health::ToolProblem {
            tool: "ruff".to_owned(),
            code: crate::errors::Code::HltToolMissing,
            message: "ruff is in the tools environment but could not be run".to_owned(),
            stderr_tail: Some("%1 is not a valid Win32 application. (os error 193)".to_owned()),
        }],
        ..full
    }
}

/// Every fixture, as `(file name, contents)`.
///
/// # Errors
/// Propagates serialization failures, which can only mean a type in this module stopped being
/// `Serialize` — a compile-time mistake surfacing at runtime.
pub fn ipc_fixtures() -> serde_json::Result<Vec<(&'static str, String)>> {
    Ok(vec![
        ("pkg_list.json", render(&pkg_list())?),
        ("pkg_outdated.json", render(&pkg_outdated())?),
        ("pin_list.json", render(&pin_list())?),
        ("flow_step.json", render(&flow_step())?),
        ("execution_summary.json", render(&execution_summary())?),
        ("guard_report.json", render(&guard_report())?),
        ("snapshot_list.json", render(&snapshot_list())?),
        ("rollback_preview.json", render(&rollback_preview())?),
        ("health_report.json", render(&health_report())?),
        ("health_partial.json", render(&health_partial())?),
        ("codes.json", render(&codes())?),
    ])
}

/// Pretty JSON with a trailing newline, so the files read as source and diff cleanly.
fn render<T: serde::Serialize>(value: &T) -> serde_json::Result<String> {
    let mut out = serde_json::to_string_pretty(value)?;
    out.push('\n');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_ipc_fixtures_are_current() {
        // The same guarantee as `bindings::the_committed_bindings_are_current`, for the same
        // reason: without it the L3 tests drift silently into asserting against a shape the app
        // does not send, and stay green while doing it.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        for (name, expected) in ipc_fixtures().expect("fixtures serialize") {
            let path = root.join(OUTPUT_DIR).join(name);
            let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                panic!(
                    "{} is missing ({e}); run `cargo run -p xtask -- ipc-fixtures`",
                    path.display()
                )
            });
            assert_eq!(
                committed.replace("\r\n", "\n"),
                expected,
                "{name} is stale — run `cargo run -p xtask -- ipc-fixtures`"
            );
        }
    }

    #[test]
    fn the_scenario_still_exercises_every_rule_the_table_implements() {
        // Guards the fixture *contents*, not just their shape. Editing the scenario without
        // reading this is how a component test quietly stops covering the case it names.
        let (dists, outdated, pins) = (pkg_list(), pkg_outdated(), pin_list());

        let outdated_names: Vec<&str> = outdated.iter().map(|o| o.name.as_str()).collect();
        let pinned_names: Vec<&str> = pins.iter().map(|p| p.pkg.as_str()).collect();

        // Every outdated row must have an installed row, or the join drops it.
        for name in &outdated_names {
            assert!(
                dists.iter().any(|d| &d.name.as_str() == name),
                "{name} is outdated but not installed — the join would lose it"
            );
        }
        // Something up to date, so the dimming rule has a subject.
        assert!(
            dists
                .iter()
                .any(|d| !outdated_names.contains(&d.name.as_str())),
            "no up-to-date row, so nothing exercises dimming"
        );
        // Something with no size, so the em-dash cell has a subject.
        assert!(
            dists.iter().any(|d| d.size_bytes.is_none()),
            "no row without a size, so nothing exercises the unknown-size cell"
        );
        // Both pin modes on screen, so the 🔒 chip has to tell them apart.
        assert!(pins.iter().any(|p| matches!(p.mode, PinMode::Exclude)));
        assert!(pins.iter().any(|p| matches!(p.mode, PinMode::Hold { .. })));
        // Pinned *and* outdated, so "N pinned excluded" is a real subtraction rather than 0.
        let excluded = outdated_names
            .iter()
            .filter(|n| pinned_names.contains(n))
            .count();
        assert_eq!(
            excluded, 2,
            "Select all should exclude exactly 2 of the outdated rows"
        );
        assert!(
            outdated_names.len() > excluded,
            "Select all would select nothing"
        );
    }

    #[test]
    fn the_guard_fixture_still_has_two_dependents_with_different_specifiers() {
        // The dialog's whole job is naming the constraint, so one dependent — or two that happen
        // to declare the same specifier — would let it pass a test it does not deserve.
        let report = guard_report();
        let broken = report
            .breaks
            .get(&PkgName::parse("numpy").unwrap_or_else(|_| unreachable!()))
            .expect("removing numpy breaks something in this scenario");

        assert_eq!(broken.len(), 2, "pandas and scipy both depend on numpy");
        assert!(
            broken.iter().all(|b| !b.constraint.is_empty()),
            "a dependent with no specifier cannot exercise the parenthetical"
        );
        assert_ne!(
            broken[0].constraint, broken[1].constraint,
            "two identical specifiers would not prove the constraint is per-dependent"
        );
        assert!(
            broken.iter().all(|b| b.version.is_some()),
            "the dialog names `pandas 2.1.4`, not bare `pandas`"
        );
    }

    #[test]
    fn the_health_fixtures_still_cover_every_branch_the_tabs_implement() {
        use crate::health::{DeptryIssue, FixApplicability};

        let report = health_report();
        let dists = pkg_list();
        let installed: Vec<&str> = dists.iter().map(|d| d.name.as_str()).collect();

        // -- deptry: the Uninstall button has two gates and each needs both answers ------------
        let unused: Vec<&DeptryIssue> = report
            .deptry
            .iter()
            .filter(|i| i.code == "DEP002")
            .collect();
        assert!(
            unused.iter().any(|i| installed.contains(&i.dep.as_str())),
            "no DEP002 naming an installed distribution, so nothing renders the button"
        );
        assert!(
            unused.iter().any(|i| !installed.contains(&i.dep.as_str())),
            "no DEP002 naming something absent, so nothing renders the *no* button case"
        );
        assert!(
            report
                .deptry
                .iter()
                .any(|i| i.code != "DEP002" && !installed.contains(&i.dep.as_str())),
            "no non-DEP002 row, so the code gate is never exercised"
        );
        assert!(
            report.deptry.iter().any(|i| i.locations.is_empty()),
            "every deptry row has a location, so the no-location branch is untested"
        );
        assert!(
            report.deptry.iter().any(|i| i.locations.len() > 1),
            "no row with two locations, so the list is only ever a singleton"
        );
        assert!(
            report
                .deptry
                .iter()
                .flat_map(|i| &i.locations)
                .any(|l| l.line.is_none()),
            "no whole-file location, so `file:line` is never rendered as bare `file`"
        );

        // -- vulture: confidence tones, and the message that names nothing ---------------------
        assert!(report.vulture.iter().any(|f| f.confidence == 100));
        assert!(report.vulture.iter().any(|f| f.confidence < 100));
        assert!(
            report.vulture.iter().any(|f| f.name.is_none()),
            "no unreachable-code finding, so the optional identifier is never absent"
        );

        // -- ruff: the fix badges, the absent link, and two counts that must differ ------------
        let ruff = &report.ruff;
        assert!(
            ruff.findings
                .iter()
                .any(|f| f.fix == Some(FixApplicability::Safe))
        );
        assert!(
            ruff.findings
                .iter()
                .any(|f| f.fix == Some(FixApplicability::Unsafe))
        );
        assert!(ruff.findings.iter().any(|f| f.fix.is_none()));
        assert!(
            ruff.findings.iter().any(|f| f.url.is_none()),
            "every finding links somewhere, so the absent-link branch is untested"
        );
        assert_ne!(
            ruff.fixable, ruff.fixable_files,
            "equal counts let a UI confuse findings with files and still pass"
        );
        assert_eq!(
            ruff.clone().recount().fixable,
            ruff.fixable,
            "the committed counts disagree with the findings they describe"
        );

        // -- the partial report: all three tab states in one document --------------------------
        let partial = health_partial();
        let failed = &partial.problems[0].tool;
        assert!(
            partial.ran.contains(failed),
            "a failed tool must still be in `ran` — that is exactly why `ran` alone cannot \
             distinguish 'clean' from 'did not finish', and why this fixture exists"
        );
        assert!(
            partial.ruff.findings.is_empty(),
            "the failed tool must report nothing, or the tab is not in the state being tested"
        );
        assert!(
            !partial.ran.contains(&"vulture".to_owned()) && partial.vulture.is_empty(),
            "no never-run tool, so 'not run' and 'clean' are not both on screen at once"
        );
        assert!(
            !partial.deptry.is_empty(),
            "nothing reported, so the partial report is indistinguishable from a failed run"
        );
    }
}
