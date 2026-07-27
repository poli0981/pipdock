//! Turning a raw resolve into the preview the user actually decides on.
//!
//! # The gap this fills
//!
//! Spike SP-1 established that **neither engine reports a held-back package**. Given the installed
//! set restated, pip's report is empty and uv says `Would make no changes`. There is no "blocked
//! by" anywhere in either engine's output — the engines simply do not have the concept.
//!
//! But PRD G2 promises exactly that sentence: *"`requests 2.32` is held back because
//! `apiclient 1.4` requires `requests<2.31`"*. So all three parts are assembled here:
//!
//! | part | source |
//! |---|---|
//! | the version that *was* resolved | the plan, or the absence of a change for that package |
//! | the version that *could* have been | the index, via `list --outdated` |
//! | who is in the way, and why | the reverse-dependency graph from `probe.py` |
//!
//! ARCHITECTURE §3's safety rule governs the third column: **if attribution is ambiguous, show
//! the constraint without a culprit rather than guessing.** An empty blocker list is a valid,
//! honest answer.

use std::collections::{BTreeMap, BTreeSet};

use crate::compat::PyVersion;
use crate::graph::ReverseDeps;
use crate::model::{OutdatedDist, PkgName, Version};

use super::{HeldBack, PlanRequest, ResolutionReport, Strategy};

/// Fill in [`ResolutionReport::held_back`] for the packages the user asked to upgrade.
///
/// A package is held back when the plan does not take it to the newest version the index offers.
/// That covers both shapes the engines produce: a change to an intermediate version, and no change
/// at all — the latter being the common case, and the one that looks like success if nobody checks.
pub fn derive_held_back(
    report: &mut ResolutionReport,
    requested: &[PkgName],
    outdated: &[OutdatedDist],
    graph: &ReverseDeps,
) {
    let latest: BTreeMap<&PkgName, &OutdatedDist> = outdated.iter().map(|o| (&o.name, o)).collect();
    let planned: BTreeMap<&PkgName, &Version> =
        report.changes.iter().map(|c| (&c.name, &c.to)).collect();

    let mut held = Vec::new();
    for pkg in requested {
        let Some(target) = latest.get(pkg) else {
            // Not outdated, so there was nothing to hold back.
            continue;
        };
        // Whatever the plan does with it; absent means it stays where it is.
        let resolved = planned
            .get(pkg)
            .map_or_else(|| target.current.clone(), |v| (*v).clone());

        if resolved == target.latest {
            continue;
        }

        // Attribution asks the graph whether anything actually forbids the *latest* version. A
        // package that permits it is never named — see ARCHITECTURE §3.
        let blockers = PyVersion::parse(&target.latest.0)
            .map(|v| graph.blockers_for(pkg, &v))
            .unwrap_or_default();

        held.push(HeldBack {
            pkg: pkg.clone(),
            resolved,
            latest: target.latest.clone(),
            blockers,
        });
    }

    report.held_back = held;
}

/// What the user chose for one package that needs a decision (DATA-FLOW §3, CLI-SPEC §4).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Decision {
    /// Accept the resolver's compatible version. The safe default, and the one that needs no
    /// extra click in the 4-click budget (UI-SPEC §5).
    KeepCompatible,
    /// Drop the package from this plan entirely.
    Skip,
    /// Take the latest anyway, knowingly breaking whatever declared the constraint
    /// (DISCLAIMER §2).
    ForceLatest,
}

/// The default answer for a package, given the run's policy.
///
/// CLI-SPEC §1.2 and §4: off a TTY, or with `--yes`, **conflicts default to skip, never force**.
/// Held-back packages get `KeepCompatible` because the resolver found something installable;
/// an impossible one has nothing compatible to keep, so the only safe automatic answer is to drop
/// it and let the rest of the batch proceed.
#[must_use]
pub fn default_decision(is_impossible: bool, force_everything: bool) -> Decision {
    if force_everything {
        Decision::ForceLatest
    } else if is_impossible {
        Decision::Skip
    } else {
        Decision::KeepCompatible
    }
}

/// Rebuild a plan request from the user's decisions, for the next resolve round.
///
/// - `KeepCompatible` leaves the package in the request unchanged.
/// - `Skip` removes it, so the rest of the batch is unaffected by one awkward package.
/// - `ForceLatest` moves it into the strategy's override list, which
///   [`forced_requirements`] turns into an exact pin and which frees the resolver from the
///   constraint that was holding it back.
#[must_use]
pub fn apply_decisions(req: &PlanRequest, decisions: &BTreeMap<PkgName, Decision>) -> PlanRequest {
    let mut forced: Vec<PkgName> = match &req.strategy {
        Strategy::ForceLatest { overrides } => overrides.clone(),
        Strategy::Compatible => Vec::new(),
    };
    let mut upgrades = Vec::new();

    for pkg in &req.upgrades {
        match decisions.get(pkg) {
            Some(Decision::Skip) => {}
            Some(Decision::ForceLatest) => {
                if !forced.contains(pkg) {
                    forced.push(pkg.clone());
                }
                upgrades.push(pkg.clone());
            }
            _ => upgrades.push(pkg.clone()),
        }
    }

    let installs = req
        .installs
        .iter()
        .filter(|s| decisions.get(&s.name) != Some(&Decision::Skip))
        .cloned()
        .collect();

    PlanRequest {
        upgrades,
        installs,
        strategy: if forced.is_empty() {
            Strategy::Compatible
        } else {
            Strategy::ForceLatest { overrides: forced }
        },
    }
}

/// The exact requirements a forced package contributes, and the guards it must be freed from.
///
/// Forcing is not just "ask for the newest": whatever declared the constraint is still installed
/// and still pinned in the guard set, so the resolver would refuse. To force, PipDock pins the
/// forced package to `latest` **and drops its blockers from the guard set** — which is precisely
/// why the UI must name what breaks first (UI-SPEC §4), and why DISCLAIMER §2 says the resulting
/// state is one the environment's own metadata declares broken.
#[must_use]
pub fn forced_requirements(
    forced: &[PkgName],
    outdated: &[OutdatedDist],
    graph: &ReverseDeps,
) -> ForcedPlan {
    let latest: BTreeMap<&PkgName, &Version> =
        outdated.iter().map(|o| (&o.name, &o.latest)).collect();

    let mut pins = Vec::new();
    let mut release = BTreeSet::new();

    for pkg in forced {
        let Some(version) = latest.get(pkg) else {
            continue;
        };
        pins.push(crate::model::PinnedSpec {
            name: pkg.clone(),
            version: (*version).clone(),
        });
        if let Ok(v) = PyVersion::parse(&version.0) {
            for blocker in graph.blockers_for(pkg, &v) {
                if let Some(by) = blocker.by {
                    release.insert(by);
                }
            }
        }
    }

    ForcedPlan {
        pins,
        release_from_guards: release.into_iter().collect(),
    }
}

/// What forcing a set of packages requires of the plan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForcedPlan {
    /// Exact `name==latest` requirements for the forced packages.
    pub pins: Vec<crate::model::PinnedSpec>,
    /// Packages that must be dropped from the guard set, because their constraints are what was
    /// holding the forced packages back. **These are the packages that will end up broken.**
    pub release_from_guards: Vec<PkgName>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Dist, Spec};
    use crate::plan::{Change, ChangeKind};

    fn pkg(n: &str) -> PkgName {
        PkgName::parse(n).unwrap()
    }

    fn dist(name: &str, version: &str, requires: &[&str]) -> Dist {
        Dist {
            name: pkg(name),
            version: Version(version.into()),
            requires_dist: requires.iter().map(|s| (*s).to_owned()).collect(),
            requires_python: None,
        }
    }

    fn outdated(name: &str, current: &str, latest: &str) -> OutdatedDist {
        OutdatedDist {
            name: pkg(name),
            current: Version(current.into()),
            latest: Version(latest.into()),
        }
    }

    fn empty_report() -> ResolutionReport {
        ResolutionReport {
            changes: Vec::new(),
            held_back: Vec::new(),
            impossible: None,
            raw: String::new(),
        }
    }

    #[test]
    fn a_package_that_did_not_move_at_all_is_held_back() {
        // The SP-1 case, and the one that looks like success if nobody checks: with httpx
        // restated, the plan is empty. "No changes" is not the same as "nothing to do".
        let graph = ReverseDeps::build(&[
            dist("httpcore", "0.15.0", &[]),
            dist("httpx", "0.23.0", &["httpcore>=0.15.0,<0.16.0"]),
        ]);
        let mut report = empty_report();

        derive_held_back(
            &mut report,
            &[pkg("httpcore")],
            &[outdated("httpcore", "0.15.0", "1.0.9")],
            &graph,
        );

        assert_eq!(report.held_back.len(), 1);
        let held = &report.held_back[0];
        assert_eq!(held.resolved.0, "0.15.0", "it stayed where it was");
        assert_eq!(held.latest.0, "1.0.9");
        assert_eq!(held.blockers.len(), 1);
        assert_eq!(held.blockers[0].by.as_ref(), Some(&pkg("httpx")));
        assert!(
            held.blockers[0]
                .constraint
                .contains("httpcore >=0.15.0,<0.16.0")
        );
        assert!(!report.is_clean(), "a held-back package needs a decision");
    }

    #[test]
    fn a_package_resolved_to_an_intermediate_version_is_held_back() {
        let graph = ReverseDeps::build(&[
            dist("requests", "2.28.0", &[]),
            dist("apiclient", "1.4", &["requests<2.31"]),
        ]);
        let mut report = empty_report();
        report.changes.push(Change {
            name: pkg("requests"),
            from: Some(Version("2.28.0".into())),
            to: Version("2.30.0".into()),
            kind: ChangeKind::Upgrade,
        });

        derive_held_back(
            &mut report,
            &[pkg("requests")],
            &[outdated("requests", "2.28.0", "2.32.3")],
            &graph,
        );

        assert_eq!(report.held_back.len(), 1);
        assert_eq!(report.held_back[0].resolved.0, "2.30.0");
        assert_eq!(report.held_back[0].latest.0, "2.32.3");
        assert_eq!(
            report.held_back[0].blockers[0].by.as_ref(),
            Some(&pkg("apiclient"))
        );
    }

    #[test]
    fn a_package_that_reached_latest_is_not_held_back() {
        let graph = ReverseDeps::build(&[dist("idna", "3.4", &[])]);
        let mut report = empty_report();
        report.changes.push(Change {
            name: pkg("idna"),
            from: Some(Version("3.4".into())),
            to: Version("3.18".into()),
            kind: ChangeKind::Upgrade,
        });

        derive_held_back(
            &mut report,
            &[pkg("idna")],
            &[outdated("idna", "3.4", "3.18")],
            &graph,
        );

        assert!(report.held_back.is_empty());
        assert!(report.is_clean(), "a clean plan needs no decisions at all");
    }

    #[test]
    fn a_held_back_package_with_no_explanation_still_appears() {
        // ARCHITECTURE §3: show the situation without a culprit rather than inventing one. The
        // row must still be there — silently omitting it would hide the fact that the user did
        // not get what they asked for.
        let graph = ReverseDeps::build(&[dist("thing", "1.0", &[])]);
        let mut report = empty_report();

        derive_held_back(
            &mut report,
            &[pkg("thing")],
            &[outdated("thing", "1.0", "2.0")],
            &graph,
        );

        assert_eq!(report.held_back.len(), 1);
        assert!(
            report.held_back[0].blockers.is_empty(),
            "no culprit is invented"
        );
    }

    #[test]
    fn packages_that_were_not_outdated_are_ignored() {
        let graph = ReverseDeps::build(&[dist("idna", "3.18", &[])]);
        let mut report = empty_report();
        derive_held_back(&mut report, &[pkg("idna")], &[], &graph);
        assert!(report.held_back.is_empty());
    }

    #[test]
    fn automatic_answers_never_force() {
        // CLI-SPEC §1.2: safe by default, scriptable on request. A script that silently forced
        // would break environments unattended.
        assert_eq!(default_decision(false, false), Decision::KeepCompatible);
        assert_eq!(default_decision(true, false), Decision::Skip);
        // ...unless the user asked for it explicitly.
        assert_eq!(default_decision(false, true), Decision::ForceLatest);
    }

    #[test]
    fn skipping_removes_only_the_awkward_package() {
        let req = PlanRequest {
            upgrades: vec![pkg("a"), pkg("b"), pkg("c")],
            installs: Vec::new(),
            strategy: Strategy::Compatible,
        };
        let decisions = BTreeMap::from([(pkg("b"), Decision::Skip)]);

        let next = apply_decisions(&req, &decisions);
        assert_eq!(next.upgrades, [pkg("a"), pkg("c")]);
        assert_eq!(next.strategy, Strategy::Compatible);
    }

    #[test]
    fn forcing_moves_the_package_into_the_override_list() {
        let req = PlanRequest {
            upgrades: vec![pkg("a"), pkg("b")],
            installs: Vec::new(),
            strategy: Strategy::Compatible,
        };
        let decisions = BTreeMap::from([(pkg("b"), Decision::ForceLatest)]);

        let next = apply_decisions(&req, &decisions);
        assert_eq!(
            next.upgrades,
            [pkg("a"), pkg("b")],
            "it is still being upgraded"
        );
        assert_eq!(
            next.strategy,
            Strategy::ForceLatest {
                overrides: vec![pkg("b")]
            }
        );
    }

    #[test]
    fn skipping_an_install_drops_it_too() {
        let req = PlanRequest {
            upgrades: Vec::new(),
            installs: vec![
                Spec {
                    name: pkg("wanted"),
                    version_req: None,
                },
                Spec {
                    name: pkg("awkward"),
                    version_req: None,
                },
            ],
            strategy: Strategy::Compatible,
        };
        let next = apply_decisions(&req, &BTreeMap::from([(pkg("awkward"), Decision::Skip)]));
        assert_eq!(next.installs.len(), 1);
        assert_eq!(next.installs[0].name, pkg("wanted"));
    }

    #[test]
    fn forcing_frees_the_blocker_from_the_guard_set() {
        // The part that makes forcing actually work. Pinning httpcore==1.0.9 alone would still
        // fail, because httpx is pinned in the guard set and forbids it. httpx has to be released
        // — and that is exactly the package that ends up broken, which is what the warning names.
        let graph = ReverseDeps::build(&[
            dist("httpcore", "0.15.0", &[]),
            dist("httpx", "0.23.0", &["httpcore>=0.15.0,<0.16.0"]),
        ]);
        let forced = forced_requirements(
            &[pkg("httpcore")],
            &[outdated("httpcore", "0.15.0", "1.0.9")],
            &graph,
        );

        assert_eq!(forced.pins.len(), 1);
        assert_eq!(forced.pins[0].to_requirement(), "httpcore==1.0.9");
        assert_eq!(
            forced.release_from_guards,
            [pkg("httpx")],
            "the blocker must be released, and it is what breaks"
        );
    }

    #[test]
    fn forcing_something_nothing_blocks_releases_nobody() {
        let graph = ReverseDeps::build(&[dist("idna", "3.4", &[])]);
        let forced =
            forced_requirements(&[pkg("idna")], &[outdated("idna", "3.4", "3.18")], &graph);
        assert_eq!(forced.pins[0].to_requirement(), "idna==3.18");
        assert!(forced.release_from_guards.is_empty());
    }
}
