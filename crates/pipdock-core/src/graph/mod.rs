//! The reverse-dependency graph, built from `probe.py`'s `requires_dist` data.
//!
//! Three features read it (ARCHITECTURE §4): held-back blocker attribution, the uninstall guard,
//! and pin auto-suggest. `docs/TESTING.md` §1 lists its correctness as something that must never
//! regress, including cycles and `pkg[extra]` markers.
//!
//! # Why this exists at all
//!
//! Spike SP-1 established that **no engine reports held-back items**. With the installed set
//! restated, pip's report is simply empty and uv says `Would make no changes` — there is no
//! "blocked by" information anywhere in either engine's output. So the sentence the preview shows
//! ("`requests 2.32` is held back because `apiclient 1.4` requires `requests<2.31`", PRD G2) is
//! assembled here, from metadata PipDock read itself.
//!
//! ARCHITECTURE §3 sets the safety rule for that: **if attribution is ambiguous, show the
//! constraint without a culprit rather than guessing.** A confidently wrong culprit is worse than
//! an honest "something requires this", because the user will go and change the wrong package.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Dist, PkgName};
use crate::plan::Blocker;

pub mod markers;

pub use markers::MarkerEnv;

/// PRD P1-2: default reverse-dependency count at which a pin is suggested. Configurable.
pub const PIN_SUGGEST_THRESHOLD: usize = 5;

/// One parsed `Requires-Dist` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    /// The required distribution, normalized.
    pub name: PkgName,
    /// The version specifier, e.g. `">=1.21.1,<3"`. Empty when unconstrained.
    pub constraint: String,
    /// The PEP 508 environment marker, if any, e.g. `extra == "socks"`.
    pub marker: Option<String>,
}

impl Requirement {
    /// True when this requirement only applies to an optional extra.
    ///
    /// Extras are **not** installed unless requested, so a dependency that exists only under
    /// `extra == "trio"` must not make the uninstall guard claim something would break. Getting
    /// this wrong turns the guard into noise, and a guard users learn to dismiss protects nobody.
    #[must_use]
    pub fn is_extra_only(&self) -> bool {
        self.marker
            .as_deref()
            .is_some_and(|m| m.contains("extra =="))
    }

    /// True when this requirement is actually in force in `env`.
    ///
    /// Supersedes [`Self::is_extra_only`] for callers that know the interpreter: a requirement
    /// gated on `python_version == "3.10"` is no more in force on 3.12 than an extra-gated one
    /// is. With `env` as `None` only the extra rule applies, which is the behaviour every caller
    /// had before markers were understood.
    #[must_use]
    pub fn applies_in(&self, env: Option<&MarkerEnv>) -> bool {
        markers::applies(self.marker.as_deref(), env)
    }

    /// Parse one `Requires-Dist` value.
    ///
    /// Returns `None` when the name is not a valid distribution name, which is the only failure
    /// mode worth acting on — a malformed requirement should not sink the whole graph.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let (head, marker) = match raw.split_once(';') {
            Some((h, m)) => (h, Some(m.trim().to_owned())),
            None => (raw, None),
        };
        let head = head.trim();

        // Split the name from its extras and constraint: `httpx[http2] (>=0.23,<1)`.
        let name_end = head
            .find(|c: char| c == '[' || c == '(' || c == ' ' || is_specifier_start(c))
            .unwrap_or(head.len());
        let (name, rest) = head.split_at(name_end);

        let name = PkgName::parse(name.trim()).ok()?;

        // Drop the requester's own extras group, keep the version specifier.
        let rest = rest.trim();
        let constraint = match rest.find(']') {
            Some(i) => rest[i + 1..].trim(),
            None => rest,
        };
        let constraint = constraint
            .trim_matches(|c| c == '(' || c == ')')
            .trim()
            .to_owned();

        Some(Self {
            name,
            constraint,
            marker,
        })
    }
}

fn is_specifier_start(c: char) -> bool {
    matches!(c, '<' | '>' | '=' | '!' | '~')
}

/// Who depends on what, keyed by the depended-upon package.
#[derive(Debug, Default, Clone)]
pub struct ReverseDeps {
    /// `dependency -> [(dependent, the requirement it declared)]`.
    edges: BTreeMap<PkgName, Vec<(PkgName, Requirement)>>,
    /// Installed versions, so attribution can name `apiclient 1.4` rather than bare `apiclient`.
    versions: BTreeMap<PkgName, String>,
    /// The interpreter these requirements are read against, when known. `None` disables marker
    /// evaluation beyond the extra rule.
    env: Option<MarkerEnv>,
}

impl ReverseDeps {
    /// Build the graph from a full environment listing.
    ///
    /// Requirements gated behind an extra are recorded but flagged, so callers can decide: the
    /// uninstall guard ignores them, while pin auto-suggest may still count them.
    ///
    /// Prefer [`Self::build_for`] wherever the interpreter is known: without it, requirements
    /// gated on a different `python_version` are treated as if they were in force.
    #[must_use]
    pub fn build(dists: &[Dist]) -> Self {
        let mut edges: BTreeMap<PkgName, Vec<(PkgName, Requirement)>> = BTreeMap::new();
        let mut versions = BTreeMap::new();

        for dist in dists {
            versions.insert(dist.name.clone(), dist.version.0.clone());
            for raw in &dist.requires_dist {
                let Some(req) = Requirement::parse(raw) else {
                    continue;
                };
                // Self-referential entries appear in the wild via `pkg[extra]` recursion; they
                // would make a package look like its own dependent.
                if req.name == dist.name {
                    continue;
                }
                edges
                    .entry(req.name.clone())
                    .or_default()
                    .push((dist.name.clone(), req));
            }
        }

        Self {
            edges,
            versions,
            env: None,
        }
    }

    /// Build the graph and evaluate markers against `python_version`, e.g. `"3.12.10"`.
    ///
    /// An unparseable version silently leaves evaluation off rather than guessing — the same
    /// direction the rest of this module takes when it cannot be sure.
    #[must_use]
    pub fn build_for(dists: &[Dist], python_version: &str) -> Self {
        Self {
            env: MarkerEnv::from_python_version(python_version),
            ..Self::build(dists)
        }
    }

    /// Packages that would break if `pkg` were removed.
    ///
    /// This is the uninstall guard's whole job: bare `pip uninstall` performs no dependency check
    /// at all (DATA-FLOW §5). Extra-gated requirements are excluded — those dependents are not
    /// actually relying on `pkg` in this environment.
    #[must_use]
    pub fn dependents_of(&self, pkg: &PkgName) -> Vec<PkgName> {
        let mut out: BTreeSet<PkgName> = BTreeSet::new();
        if let Some(list) = self.edges.get(pkg) {
            for (dependent, req) in list {
                if req.applies_in(self.env.as_ref()) {
                    out.insert(dependent.clone());
                }
            }
        }
        out.into_iter().collect()
    }

    /// Dependents of the whole removal set, excluding members of the set itself.
    ///
    /// DATA-FLOW §5 evaluates the guard **once, against the full set**: removing A and B together
    /// is fine when only B depends on A, and asking per package would raise a warning the user
    /// cannot act on.
    #[must_use]
    pub fn dependents_of_set(&self, removing: &[PkgName]) -> BTreeMap<PkgName, Vec<PkgName>> {
        let set: BTreeSet<&PkgName> = removing.iter().collect();
        let mut out = BTreeMap::new();
        for pkg in removing {
            let breaks: Vec<PkgName> = self
                .dependents_of(pkg)
                .into_iter()
                .filter(|d| !set.contains(d))
                .collect();
            if !breaks.is_empty() {
                out.insert(pkg.clone(), breaks);
            }
        }
        out
    }

    /// How many packages depend on `pkg`, for pin auto-suggest (PRD P1-2).
    #[must_use]
    pub fn dependent_count(&self, pkg: &PkgName) -> usize {
        self.dependents_of(pkg).len()
    }

    /// Attribute why `pkg` could not move past `target`.
    ///
    /// Returns every installed dependent whose declared constraint actually excludes the target
    /// version. **Constraints that the target satisfies are omitted**, because naming a package
    /// that is not in the way is exactly the confident-but-wrong attribution ARCHITECTURE §3
    /// warns against.
    ///
    /// An empty result means "nothing here explains it" and the caller must present the situation
    /// without a culprit — not invent one.
    #[must_use]
    pub fn blockers_for(&self, pkg: &PkgName, target: &crate::compat::PyVersion) -> Vec<Blocker> {
        let Some(list) = self.edges.get(pkg) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (dependent, req) in list {
            if req.constraint.is_empty() || !req.applies_in(self.env.as_ref()) {
                continue;
            }
            if !satisfies(&req.constraint, target) {
                let named = match self.versions.get(dependent) {
                    Some(v) => format!("{dependent} {v}"),
                    None => dependent.to_string(),
                };
                out.push(Blocker {
                    by: Some(dependent.clone()),
                    constraint: format!("{named} requires {} {}", pkg, req.constraint),
                });
            }
        }
        out
    }

    /// Every package in `all` that nothing else depends on.
    #[must_use]
    pub fn leaves(&self, all: &[PkgName]) -> Vec<PkgName> {
        all.iter()
            .filter(|p| self.dependents_of(p).is_empty())
            .cloned()
            .collect()
    }

    /// Everything that must also go if `removing` is removed and nothing may be left broken.
    ///
    /// This is the *transitive* closure, which DATA-FLOW §5's "[Remove dependents too] (adds Y,Z
    /// to set, re-guard)" implies: pulling Y in can break Z, and stopping after one level would
    /// hand the user a set that still breaks something. Terminates on cycles because packages are
    /// only ever added to the frontier once.
    #[must_use]
    pub fn removal_closure(&self, removing: &[PkgName]) -> Vec<PkgName> {
        let mut seen: BTreeSet<PkgName> = removing.iter().cloned().collect();
        let mut frontier: Vec<PkgName> = removing.to_vec();

        while let Some(pkg) = frontier.pop() {
            for dependent in self.dependents_of(&pkg) {
                if seen.insert(dependent.clone()) {
                    frontier.push(dependent);
                }
            }
        }
        seen.into_iter().collect()
    }

    /// Evaluate the uninstall guard against a removal set (DATA-FLOW §5).
    #[must_use]
    pub fn guard(&self, removing: &[PkgName]) -> GuardReport {
        GuardReport {
            removing: removing.to_vec(),
            breaks: self.dependents_of_set(removing),
            with_dependents: self.removal_closure(removing),
        }
    }
}

/// What the uninstall guard found.
///
/// Rationale from DATA-FLOW §5: bare `pip uninstall` performs **no** dependency check at all, so
/// this is a core value-add rather than a nicety. It is computed once against the full set,
/// because removing A and B together is fine when only B depends on A — asking per package would
/// raise a warning the user cannot act on.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct GuardReport {
    /// The set the user asked to remove.
    pub removing: Vec<PkgName>,
    /// For each package that has them, the installed packages that would break.
    pub breaks: BTreeMap<PkgName, Vec<PkgName>>,
    /// The removal set expanded to include everything that depends on it, transitively — the
    /// "remove dependents too" option.
    pub with_dependents: Vec<PkgName>,
}

impl GuardReport {
    /// True when nothing would break and the flow can go straight to a plain confirm.
    #[must_use]
    pub fn is_clear(&self) -> bool {
        self.breaks.is_empty()
    }

    /// Every package that would break, de-duplicated across the removal set.
    #[must_use]
    pub fn all_broken(&self) -> Vec<PkgName> {
        let set: BTreeSet<&PkgName> = self.breaks.values().flatten().collect();
        set.into_iter().cloned().collect()
    }

    /// The extra packages "remove dependents too" would add beyond what the user selected.
    #[must_use]
    pub fn extra_removals(&self) -> Vec<PkgName> {
        let asked: BTreeSet<&PkgName> = self.removing.iter().collect();
        self.with_dependents
            .iter()
            .filter(|p| !asked.contains(p))
            .cloned()
            .collect()
    }
}

/// Does `version` satisfy the PEP 440 specifier set `constraint`?
///
/// Reuses [`crate::compat`]'s evaluator: a `Requires-Dist` specifier and a `Requires-Python`
/// specifier are the same grammar, so there is no reason to have two of them drifting apart.
/// An unparseable constraint is treated as satisfied, so a metadata oddity cannot manufacture a
/// blocker out of nothing.
fn satisfies(constraint: &str, version: &crate::compat::PyVersion) -> bool {
    crate::compat::check(Some(constraint), version).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compat::PyVersion;
    use crate::model::Version;

    fn dist(name: &str, version: &str, requires: &[&str]) -> Dist {
        Dist {
            name: PkgName::parse(name).unwrap(),
            version: Version(version.into()),
            requires_dist: requires.iter().map(|s| (*s).to_owned()).collect(),
            requires_python: None,
            size_bytes: None,
        }
    }

    fn pkg(name: &str) -> PkgName {
        PkgName::parse(name).unwrap()
    }

    #[test]
    fn parses_the_shapes_that_appear_in_real_metadata() {
        // Every one of these came out of the SP-1/SP-2 fixture corpus.
        let cases = [
            ("certifi", "certifi", "", None),
            ("h11>=0.16", "h11", ">=0.16", None),
            ("urllib3<3,>=1.21.1", "urllib3", "<3,>=1.21.1", None),
            (
                "anyio<5.0,>=4.0; extra == \"asyncio\"",
                "anyio",
                "<5.0,>=4.0",
                Some("extra == \"asyncio\""),
            ),
            (
                "socksio==1.*; extra == \"socks\"",
                "socksio",
                "==1.*",
                Some("extra == \"socks\""),
            ),
            (
                "typing-extensions>=4.2; python_version < \"3.13\"",
                "typing-extensions",
                ">=4.2",
                Some("python_version < \"3.13\""),
            ),
            (
                "Brotli>=1.2; platform_python_implementation == \"CPython\"",
                "brotli",
                ">=1.2",
                Some("platform_python_implementation == \"CPython\""),
            ),
            ("httpx[http2] (>=0.23,<1)", "httpx", ">=0.23,<1", None),
            ("zope.interface", "zope-interface", "", None),
        ];
        for (raw, name, constraint, marker) in cases {
            let got = Requirement::parse(raw).unwrap_or_else(|| panic!("failed to parse {raw:?}"));
            assert_eq!(got.name.as_str(), name, "name of {raw:?}");
            assert_eq!(got.constraint, constraint, "constraint of {raw:?}");
            assert_eq!(got.marker.as_deref(), marker, "marker of {raw:?}");
        }
    }

    #[test]
    fn a_malformed_requirement_does_not_sink_the_graph() {
        assert!(Requirement::parse("").is_none());
        assert!(Requirement::parse(">=1.0").is_none());
        // ...and a package carrying one still contributes its valid entries.
        let g = ReverseDeps::build(&[dist("app", "1.0", &["", "requests>=2"])]);
        assert_eq!(g.dependents_of(&pkg("requests")), [pkg("app")]);
    }

    #[test]
    fn the_uninstall_guard_lists_dependents() {
        // DATA-FLOW §5's example: removing X breaks Y and Z.
        let g = ReverseDeps::build(&[
            dist("x", "1.0", &[]),
            dist("y", "2.0", &["x>=1"]),
            dist("z", "3.0", &["x"]),
            dist("unrelated", "1.0", &["requests"]),
        ]);
        assert_eq!(g.dependents_of(&pkg("x")), [pkg("y"), pkg("z")]);
        assert!(g.dependents_of(&pkg("y")).is_empty());
    }

    #[test]
    fn extra_only_dependents_do_not_trip_the_guard() {
        // aiohttp declares Brotli only under `extra == "speedups"`. Warning that removing Brotli
        // breaks aiohttp would be false, and a guard that cries wolf protects nobody.
        let g = ReverseDeps::build(&[
            dist("brotli", "1.2", &[]),
            dist("aiohttp", "3.13.3", &["Brotli>=1.2; extra == \"speedups\""]),
        ]);
        assert!(g.dependents_of(&pkg("brotli")).is_empty());
    }

    #[test]
    fn the_guard_is_evaluated_against_the_whole_removal_set() {
        // Removing y alone breaks nothing; removing x alone breaks y; removing both is fine.
        let g = ReverseDeps::build(&[dist("x", "1.0", &[]), dist("y", "2.0", &["x>=1"])]);

        let alone = g.dependents_of_set(&[pkg("x")]);
        assert_eq!(alone.get(&pkg("x")), Some(&vec![pkg("y")]));

        let together = g.dependents_of_set(&[pkg("x"), pkg("y")]);
        assert!(together.is_empty(), "removing both together breaks nothing");
    }

    #[test]
    fn cycles_do_not_hang_or_duplicate() {
        // TESTING §1 calls out cycles explicitly.
        let g = ReverseDeps::build(&[dist("a", "1.0", &["b"]), dist("b", "1.0", &["a"])]);
        assert_eq!(g.dependents_of(&pkg("a")), [pkg("b")]);
        assert_eq!(g.dependents_of(&pkg("b")), [pkg("a")]);
    }

    #[test]
    fn self_references_are_ignored() {
        // `pkg[extra]` recursion makes a package list itself; it must not become its own dependent.
        let g = ReverseDeps::build(&[dist(
            "celery",
            "5.0",
            &["celery[redis]; extra == \"redis\""],
        )]);
        assert!(g.dependents_of(&pkg("celery")).is_empty());
    }

    #[test]
    fn attribution_names_only_constraints_that_actually_block() {
        // The httpx/httpcore case from SP-1: httpx 0.23.0 requires httpcore<0.16, so httpcore
        // cannot reach 1.0.9. anyio also depends on httpcore but permits it.
        let g = ReverseDeps::build(&[
            dist("httpcore", "0.15.0", &[]),
            dist("httpx", "0.23.0", &["httpcore>=0.15.0,<0.16.0"]),
            dist("anyio", "4.0.0", &["httpcore>=0.15"]),
        ]);
        let blockers = g.blockers_for(&pkg("httpcore"), &PyVersion::parse("1.0.9").unwrap());

        assert_eq!(
            blockers.len(),
            1,
            "anyio permits 1.0.9 and must not be named"
        );
        assert_eq!(blockers[0].by.as_ref(), Some(&pkg("httpx")));
        assert_eq!(
            blockers[0].constraint,
            "httpx 0.23.0 requires httpcore >=0.15.0,<0.16.0"
        );
    }

    #[test]
    fn attribution_ignores_constraints_gated_on_another_python() {
        // The SP-5 dogfood case, verbatim from the installed metadata. pandas declares one numpy
        // bound per interpreter and only the 3.12 branch is in force; naming the other two tells
        // the user something false about their own environment.
        let dists = [
            dist("numpy", "1.26.4", &[]),
            dist(
                "pandas",
                "2.1.4",
                &[
                    "numpy<2,>=1.22.4; python_version < \"3.11\"",
                    "numpy<2,>=1.23.2; python_version == \"3.11\"",
                    "numpy<2,>=1.26.0; python_version >= \"3.12\"",
                ],
            ),
            dist(
                "statsmodels",
                "0.14.1",
                &[
                    "numpy <2,>=1.18",
                    "numpy <2,>=1.22.3 ; python_version == \"3.10\" and platform_system == \"Windows\"",
                ],
            ),
        ];
        let latest = PyVersion::parse("2.5.1").unwrap();

        let g = ReverseDeps::build_for(&dists, "3.12.10");
        let found = g.blockers_for(&pkg("numpy"), &latest);
        let named: Vec<&str> = found.iter().map(|b| b.constraint.trim()).collect();
        assert_eq!(
            named,
            [
                "pandas 2.1.4 requires numpy <2,>=1.26.0",
                "statsmodels 0.14.1 requires numpy <2,>=1.18",
            ],
            "only the branches in force on 3.12 may be named"
        );

        // Without an interpreter nothing can be ruled out, so every branch is still reported —
        // noisy, but never hiding the real reason.
        assert_eq!(g.env.as_ref().map(|_| ()), Some(()));
        assert_eq!(
            ReverseDeps::build(&dists)
                .blockers_for(&pkg("numpy"), &latest)
                .len(),
            5
        );
    }

    #[test]
    fn the_guard_ignores_dependents_gated_on_another_python() {
        // Same bug, other consumer: a dependent that only needs `pkg` on 3.10 would otherwise
        // make the uninstall guard refuse a removal that breaks nothing on 3.12.
        let dists = [
            dist("tomli", "2.0.1", &[]),
            dist(
                "build",
                "1.0.0",
                &["tomli>=1.1.0; python_version < \"3.11\""],
            ),
        ];
        assert!(
            ReverseDeps::build_for(&dists, "3.12.10")
                .dependents_of(&pkg("tomli"))
                .is_empty()
        );
        assert_eq!(
            ReverseDeps::build_for(&dists, "3.10.13").dependents_of(&pkg("tomli")),
            [pkg("build")]
        );
    }

    #[test]
    fn no_blocker_is_invented_when_nothing_explains_it() {
        // ARCHITECTURE §3: show the constraint without a culprit rather than guessing. An empty
        // result is the signal for that, so it must stay empty rather than degrade to a guess.
        let g = ReverseDeps::build(&[
            dist("httpcore", "0.15.0", &[]),
            dist("anyio", "4.0.0", &["httpcore>=0.15"]),
        ]);
        assert!(
            g.blockers_for(&pkg("httpcore"), &PyVersion::parse("1.0.9").unwrap())
                .is_empty()
        );
        // ...and a package nothing depends on has no blockers either.
        assert!(
            g.blockers_for(&pkg("nobody"), &PyVersion::parse("1.0").unwrap())
                .is_empty()
        );
    }

    #[test]
    fn an_unreadable_constraint_does_not_manufacture_a_blocker() {
        let g = ReverseDeps::build(&[
            dist("thing", "1.0", &[]),
            dist("app", "1.0", &["thing >= maybe two"]),
        ]);
        assert!(
            g.blockers_for(&pkg("thing"), &PyVersion::parse("2.0").unwrap())
                .is_empty()
        );
    }

    #[test]
    fn dependent_count_feeds_pin_auto_suggest() {
        let dists: Vec<Dist> = std::iter::once(dist("urllib3", "2.0", &[]))
            .chain((0..6).map(|i| {
                let name = format!("app{i}");
                Dist {
                    name: PkgName::parse(&name).unwrap(),
                    version: Version("1.0".into()),
                    requires_dist: vec!["urllib3".into()],
                    requires_python: None,
                    size_bytes: None,
                }
            }))
            .collect();
        let g = ReverseDeps::build(&dists);
        assert_eq!(g.dependent_count(&pkg("urllib3")), 6);
        assert!(g.dependent_count(&pkg("urllib3")) >= PIN_SUGGEST_THRESHOLD);
    }

    #[test]
    fn leaves_are_packages_nothing_depends_on() {
        let g = ReverseDeps::build(&[dist("x", "1.0", &[]), dist("y", "2.0", &["x>=1"])]);
        assert_eq!(g.leaves(&[pkg("x"), pkg("y")]), [pkg("y")]);
    }

    #[test]
    fn the_guard_reports_the_documented_example() {
        // DATA-FLOW §5: "Removing X breaks Y (requires X>=1), Z".
        let g = ReverseDeps::build(&[
            dist("x", "1.0", &[]),
            dist("y", "2.0", &["x>=1"]),
            dist("z", "3.0", &["x"]),
        ]);
        let report = g.guard(&[pkg("x")]);

        assert!(!report.is_clear());
        assert_eq!(report.all_broken(), [pkg("y"), pkg("z")]);
        assert_eq!(report.extra_removals(), [pkg("y"), pkg("z")]);
    }

    #[test]
    fn a_removal_that_breaks_nothing_is_clear() {
        let g = ReverseDeps::build(&[dist("x", "1.0", &[]), dist("y", "2.0", &["x>=1"])]);
        let report = g.guard(&[pkg("y")]);

        assert!(report.is_clear());
        assert!(report.all_broken().is_empty());
        assert!(report.extra_removals().is_empty());
    }

    #[test]
    fn remove_dependents_too_is_transitive() {
        // a <- b <- c. Removing `a` and stopping after one level would hand the user {a, b},
        // which still breaks c — a set that is guaranteed to fail its own re-guard.
        let g = ReverseDeps::build(&[
            dist("a", "1.0", &[]),
            dist("b", "1.0", &["a"]),
            dist("c", "1.0", &["b"]),
        ]);
        let report = g.guard(&[pkg("a")]);

        assert_eq!(report.with_dependents, [pkg("a"), pkg("b"), pkg("c")]);
        // ...and the expanded set is genuinely safe: re-guarding it finds nothing left to break.
        assert!(g.guard(&report.with_dependents).is_clear());
    }

    #[test]
    fn the_closure_terminates_on_a_cycle() {
        // Mutually dependent packages are rare but real; a naive walk would not stop.
        let g = ReverseDeps::build(&[dist("a", "1.0", &["b"]), dist("b", "1.0", &["a"])]);
        assert_eq!(g.removal_closure(&[pkg("a")]), [pkg("a"), pkg("b")]);
    }

    #[test]
    fn removing_a_package_together_with_its_only_dependent_is_clear() {
        // The reason the guard runs once against the whole set rather than per package.
        let g = ReverseDeps::build(&[dist("x", "1.0", &[]), dist("y", "2.0", &["x>=1"])]);
        assert!(g.guard(&[pkg("x"), pkg("y")]).is_clear());
    }
}
