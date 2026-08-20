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
    /// `dependent -> [the requirements it declared]` — the same edges the other way up.
    ///
    /// Held rather than derived because deriving it means scanning every value of `edges` for
    /// one package, which is O(edges) per lookup where this is O(1); the whole-graph view asks
    /// once per installed package. Built in the same pass, from the same parse, filtered by the
    /// same rule — a second traversal is how the two directions would come to disagree.
    forward: BTreeMap<PkgName, Vec<Requirement>>,
    /// Installed versions, so attribution can name `apiclient 1.4` rather than bare `apiclient`.
    versions: BTreeMap<PkgName, String>,
    /// The interpreter these requirements are read against, when known. `None` disables marker
    /// evaluation beyond the extra rule.
    env: Option<MarkerEnv>,
}

impl ReverseDeps {
    /// Build the graph from a full environment listing.
    ///
    /// Requirements gated behind an extra are recorded but flagged, and **every reader filters
    /// them out** via [`Requirement::applies_in`]. This comment used to say auto-suggest "may
    /// still count them", describing an intent nothing implemented — [`Self::dependent_count`]
    /// delegates to [`Self::dependents_of`] and always has. Post-1.0 P1-A settled it in favour of
    /// what shipped: a suggestion counts what actually depends on a package *in this environment*,
    /// which is the same set the uninstall guard warns about. Two features disagreeing about one
    /// edge rule is the failure mode worth avoiding, and it is the whole reason this lives in Rust
    /// rather than being recomputed in the frontend.
    ///
    /// Prefer [`Self::build_for`] wherever the interpreter is known: without it, requirements
    /// gated on a different `python_version` are treated as if they were in force.
    #[must_use]
    pub fn build(dists: &[Dist]) -> Self {
        let mut edges: BTreeMap<PkgName, Vec<(PkgName, Requirement)>> = BTreeMap::new();
        let mut forward: BTreeMap<PkgName, Vec<Requirement>> = BTreeMap::new();
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
                forward
                    .entry(dist.name.clone())
                    .or_default()
                    .push(req.clone());
                edges
                    .entry(req.name.clone())
                    .or_default()
                    .push((dist.name.clone(), req));
            }
        }

        Self {
            edges,
            forward,
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

    /// Packages that would break if `pkg` were removed, each with the requirement it declared.
    ///
    /// The same set as [`Self::dependents_of`], carrying the reason. DATA-FLOW §5's dialog says
    /// *"Removing X breaks Y (requires X>=1)"* — a bare list of names tells the user what will
    /// break but not whether they can live with it, and the specifier is the only thing that
    /// distinguishes "needs any version of this" from "needs exactly this one".
    ///
    /// A dependent appears once per **distinct** in-force constraint. Two marker-gated branches
    /// that both apply are two genuine constraints, but an exact duplicate is metadata noise.
    #[must_use]
    pub fn breaking_dependents(&self, pkg: &PkgName) -> Vec<BrokenDependent> {
        let mut out: BTreeSet<BrokenDependent> = BTreeSet::new();
        if let Some(list) = self.edges.get(pkg) {
            for (dependent, req) in list {
                if req.applies_in(self.env.as_ref()) {
                    out.insert(BrokenDependent {
                        pkg: dependent.clone(),
                        version: self.versions.get(dependent).cloned(),
                        constraint: req.constraint.clone(),
                    });
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
    pub fn dependents_of_set(
        &self,
        removing: &[PkgName],
    ) -> BTreeMap<PkgName, Vec<BrokenDependent>> {
        let set: BTreeSet<&PkgName> = removing.iter().collect();
        let mut out = BTreeMap::new();
        for pkg in removing {
            let breaks: Vec<BrokenDependent> = self
                .breaking_dependents(pkg)
                .into_iter()
                .filter(|d| !set.contains(&d.pkg))
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
                // Three fields, no phrasing: `by` and `version` name the culprit and
                // `constraint` is the requirement it declared, exactly as `breaking_dependents`
                // fills `BrokenDependent`. Each head writes its own sentence.
                out.push(Blocker {
                    by: Some(dependent.clone()),
                    version: self.versions.get(dependent).cloned(),
                    constraint: format!("{pkg}{}", req.constraint),
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

    /// What `pkg` requires: in force, and installed.
    ///
    /// The forward mirror of [`Self::dependents_of`]. The two must filter by one rule or a focus
    /// view's two columns describe the same edge differently, so both go through
    /// [`Requirement::applies_in`] — which is also why neither can be recomputed in the frontend.
    /// That restriction is not about phrasing (I18N §1); it is that PEP 508 marker evaluation and
    /// the extra-gating rule live in [`markers`], and a second implementation would drift from the
    /// guard the user is warned by.
    ///
    /// Requirements that are in force but **not installed** are excluded here and reported by
    /// [`Self::unsatisfied`] instead. Two disjoint sets whose union is the in-force requirement
    /// list, so no row has to carry an "is it actually there?" flag the reader must then interpret.
    #[must_use]
    pub fn requires_of(&self, pkg: &PkgName) -> Vec<DepEdge> {
        let mut out: BTreeSet<DepEdge> = BTreeSet::new();
        for req in self.forward.get(pkg).into_iter().flatten() {
            if !req.applies_in(self.env.as_ref()) {
                continue;
            }
            if let Some(version) = self.versions.get(&req.name) {
                out.insert(DepEdge {
                    pkg: req.name.clone(),
                    version: Some(version.clone()),
                    constraint: req.constraint.clone(),
                });
            }
        }
        out.into_iter().collect()
    }

    /// Who requires `pkg`, each with the constraint it declared.
    ///
    /// Deliberately a projection of [`Self::breaking_dependents`] rather than a second walk of
    /// `edges`: the dependents column and the uninstall guard are then the same set by
    /// construction, and cannot come to disagree about which edges count. That is the failure
    /// this module exists to prevent — `ReverseDeps::build`'s doc records the last time two
    /// features nearly applied two edge rules.
    #[must_use]
    pub fn edges_to(&self, pkg: &PkgName) -> Vec<DepEdge> {
        self.breaking_dependents(pkg)
            .into_iter()
            .map(DepEdge::from)
            .collect()
    }

    /// In-force requirements of `pkg` that no installed distribution satisfies.
    ///
    /// Nothing else in PipDock reports this. It is the one thing a dependency view can say that
    /// the package list cannot, and it is only trustworthy because the markers are evaluated:
    /// counting raw `Requires-Dist` names would report every `python_version < "3.11"` backport
    /// as missing on 3.12 and make a healthy environment look broken.
    #[must_use]
    pub fn unsatisfied(&self, pkg: &PkgName) -> Vec<PkgName> {
        let mut out: BTreeSet<PkgName> = BTreeSet::new();
        for req in self.forward.get(pkg).into_iter().flatten() {
            if req.applies_in(self.env.as_ref()) && !self.versions.contains_key(&req.name) {
                out.insert(req.name.clone());
            }
        }
        out.into_iter().collect()
    }

    /// How many installed packages `pkg` pulls in, transitively, excluding itself.
    ///
    /// The forward mirror of [`Self::removal_closure`], and cycle-safe for its reason: a package
    /// enters the frontier at most once. `pkg` itself is never counted, so a dependency cycle
    /// through it reports the same number an acyclic graph would.
    ///
    /// Goes through [`Self::requires_of`] rather than walking `forward` itself, and that was
    /// measured rather than assumed. Walking `forward` directly takes the whole-graph
    /// [`Self::view`] from **42.4 ms to 31.4 ms** on the 352-package fixture in `--release` — 11 ms
    /// against a probe that costs 605 ms, so **1.8% of an operation the user waits for**, bought
    /// with a second copy of the in-force-and-installed rule. This module exists because two
    /// features applying two edge rules is the failure worth designing against; a 1.8% saving does
    /// not buy that risk back.
    #[must_use]
    pub fn reach(&self, pkg: &PkgName) -> usize {
        let mut seen: BTreeSet<PkgName> = BTreeSet::new();
        let mut frontier = vec![pkg.clone()];
        while let Some(current) = frontier.pop() {
            for edge in self.requires_of(&current) {
                if edge.pkg != *pkg && seen.insert(edge.pkg.clone()) {
                    frontier.push(edge.pkg);
                }
            }
        }
        seen.len()
    }

    /// The whole graph, precomputed once per environment (PRD P1-6).
    ///
    /// **One value rather than one call per package**, because the alternative pays a 605 ms
    /// probe on every re-centring click. Everything a focus view needs is decided here, in Rust,
    /// with markers evaluated; the frontend indexes this map by name and computes nothing about
    /// which edges are in force.
    ///
    /// Uncapped on purpose. `setuptools` has 150 dependents in the 352-package fixture, and the
    /// view shows a bounded window — but the "+ N more" count has to come from the full set or a
    /// capped view misreports a total, which is the rule `SUGGESTIONS_SHOWN` and `RUFF_ROWS_SHOWN`
    /// already follow. Capping here would put that count out of reach.
    ///
    /// Every installed package gets a node, including the leaves: a package with no edges in
    /// either direction is a fact worth rendering, not an absence to fall through on.
    #[must_use]
    pub fn view(&self) -> DepsGraph {
        let nodes = self
            .versions
            .iter()
            .map(|(pkg, version)| {
                let node = DepsNode {
                    version: Some(version.clone()),
                    dependents: self.edges_to(pkg),
                    dependencies: self.requires_of(pkg),
                    impact: self
                        .removal_closure(std::slice::from_ref(pkg))
                        .len()
                        .saturating_sub(1),
                    reach: self.reach(pkg),
                    unsatisfied: self.unsatisfied(pkg),
                };
                (pkg.clone(), node)
            })
            .collect();
        DepsGraph { nodes }
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

/// One installed package that a removal would break, and the requirement that says so.
///
/// `constraint` is the **bare specifier tail** — `"<2,>=1.26.0"`, not `"numpy<2,>=1.26.0"` —
/// because [`Requirement::parse`] splits the distribution name off and the name is already the key
/// of [`GuardReport::breaks`]. The head that goes in front of it is the package being removed, so
/// the caller has both halves and can join them the way its own language does. Rust does not
/// assemble the sentence: I18N §1 keeps every word of phrasing in the frontend catalogs, and a
/// specifier is data, like a version or a path.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct BrokenDependent {
    /// The installed package that would be left with a missing dependency.
    pub pkg: PkgName,
    /// Its installed version, when the graph knows it — so the dialog can say `pandas 2.1.4`
    /// rather than bare `pandas`, which is what makes the constraint checkable by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The version specifier it declared, e.g. `"<2,>=1.26.0"`. Empty when unconstrained.
    pub constraint: String,
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
    /// For each package that has them, the installed packages that would break and why.
    pub breaks: BTreeMap<PkgName, Vec<BrokenDependent>>,
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
    ///
    /// Names only: a package can appear under two removals with two different constraints, and
    /// callers that want a count or a set want it counted once.
    #[must_use]
    pub fn all_broken(&self) -> Vec<PkgName> {
        let set: BTreeSet<&PkgName> = self.breaks.values().flatten().map(|b| &b.pkg).collect();
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

/// One edge of the dependency graph, seen from a focused package.
///
/// `pkg` is always the **other end** of the edge and `constraint` always the specifier written on
/// it, so one type serves both directions: in a dependents column `pkg` is who requires the focus,
/// in a dependencies column it is what the focus requires. `version` is that package's installed
/// version.
///
/// Three fields identical to [`BrokenDependent`], and deliberately a separate type. That one is
/// about *breakage* — its doc is written for the uninstall dialog and its `Ord` exists for the
/// dedup in [`ReverseDeps::breaking_dependents`] — so reusing it here would have a dependencies
/// column claim every dependency is a broken dependent. The `From` impl below is the bridge, and
/// [`ReverseDeps::edges_to`] goes through it precisely so the two cannot drift.
///
/// `constraint` is the **bare specifier tail** — `"<2,>=1.26.0"`, never `"numpy<2,>=1.26.0"` — for
/// [`BrokenDependent`]'s reason. The other half of the sentence is the focused package, which the
/// caller already has; Rust emits data and each head writes its own sentence (I18N §1). An empty
/// string means unconstrained, which is a different statement from "no edge" and the view says so.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct DepEdge {
    /// The package at the other end of the edge.
    pub pkg: PkgName,
    /// Its installed version, when the graph knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The version specifier written on the edge. Empty when unconstrained.
    pub constraint: String,
}

impl From<BrokenDependent> for DepEdge {
    fn from(d: BrokenDependent) -> Self {
        Self {
            pkg: d.pkg,
            version: d.version,
            constraint: d.constraint,
        }
    }
}

/// Everything a focus view needs about one installed package.
///
/// `impact` and `reach` are the pair the view exists for. Every other question PipDock answers
/// about the graph is single-hop — the uninstall guard, blocker attribution and pin auto-suggest
/// all look one edge out — so the only thing a dependency view adds over shipped behaviour is
/// **transitive** reach, and it is a number rather than a picture because the picture does not
/// survive the scale: measured on the 352-package fixture, a depth-2 neighbourhood is a median of
/// 172 nodes and exceeds 60 for 212 of 352 packages.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct DepsNode {
    /// The package's own installed version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Installed packages that require this one, with the constraint each declared.
    pub dependents: Vec<DepEdge>,
    /// Installed packages this one requires, in force.
    pub dependencies: Vec<DepEdge>,
    /// How many packages would be left broken if this one went, transitively. The size of
    /// [`ReverseDeps::removal_closure`] without the package itself.
    pub impact: usize,
    /// How many installed packages this one pulls in, transitively.
    pub reach: usize,
    /// In-force requirements with nothing installed to satisfy them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsatisfied: Vec<PkgName>,
}

/// The in-force dependency graph of one environment (PRD P1-6).
///
/// Keyed by package name, one entry per installed distribution. Produced by
/// [`ReverseDeps::view`]; see its doc for why this crosses the bridge whole rather than a package
/// at a time, and why nothing here is capped.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct DepsGraph {
    /// Every installed package, including those with no edges in either direction.
    pub nodes: BTreeMap<PkgName, DepsNode>,
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
        assert_eq!(
            alone.get(&pkg("x")),
            Some(&vec![BrokenDependent {
                pkg: pkg("y"),
                version: Some("2.0".to_owned()),
                constraint: ">=1".to_owned(),
            }])
        );

        let together = g.dependents_of_set(&[pkg("x"), pkg("y")]);
        assert!(together.is_empty(), "removing both together breaks nothing");
    }

    #[test]
    fn the_guard_carries_the_requirement_that_justifies_it() {
        // DATA-FLOW §5's dialog is "Removing X breaks Y (requires X>=1), Z" — the parenthetical is
        // the point, and an unconstrained dependent must not invent one.
        let g = ReverseDeps::build(&[
            dist("x", "1.0", &[]),
            dist("y", "2.0", &["x>=1"]),
            dist("z", "3.0", &["x"]),
        ]);
        let broken = g.breaking_dependents(&pkg("x"));

        assert_eq!(broken.len(), 2);
        assert_eq!(broken[0].pkg, pkg("y"));
        assert_eq!(broken[0].version.as_deref(), Some("2.0"));
        assert_eq!(broken[0].constraint, ">=1");
        assert_eq!(broken[1].pkg, pkg("z"));
        assert_eq!(broken[1].constraint, "", "z declares no specifier");
    }

    #[test]
    fn a_dependent_appears_once_per_distinct_constraint() {
        // The SP-5 bug in miniature: pandas declares numpy under four marker-gated branches. Only
        // the ones in force count, and two identical in-force entries are one constraint.
        let g = ReverseDeps::build_for(
            &[
                dist("numpy", "1.26.4", &[]),
                dist(
                    "pandas",
                    "2.1.4",
                    &[
                        "numpy<2,>=1.22.4; python_version < \"3.11\"",
                        "numpy<2,>=1.26.0; python_version >= \"3.12\"",
                        "numpy<2,>=1.26.0",
                    ],
                ),
            ],
            "3.12.4",
        );

        let broken = g.breaking_dependents(&pkg("numpy"));
        assert_eq!(
            broken,
            [BrokenDependent {
                pkg: pkg("pandas"),
                version: Some("2.1.4".to_owned()),
                constraint: "<2,>=1.26.0".to_owned(),
            }],
            "the 3.11-gated branch does not apply, and the two that do are one constraint"
        );
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
        // Three fields, no phrasing (hard invariant 4). The CLI joins them with "requires" and
        // the GUI interpolates `plan.blocker`; a sentence built here is one both heads then have
        // to un-build, which is how the preview came to name the dependent twice.
        assert_eq!(blockers[0].by.as_ref(), Some(&pkg("httpx")));
        assert_eq!(blockers[0].version.as_deref(), Some("0.23.0"));
        assert_eq!(blockers[0].constraint, "httpcore>=0.15.0,<0.16.0");
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
        let named: Vec<(Option<&str>, &str)> = found
            .iter()
            .map(|b| (b.version.as_deref(), b.constraint.trim()))
            .collect();
        assert_eq!(
            named,
            [
                (Some("2.1.4"), "numpy<2,>=1.26.0"),
                (Some("0.14.1"), "numpy<2,>=1.18"),
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
    // --- P1-6: the dependency view -------------------------------------------------------

    /// The environment the view tests share: a diamond, an extra-gated edge, a marker for
    /// another Python, a requirement nothing satisfies, and a leaf.
    fn env() -> Vec<Dist> {
        vec![
            dist(
                "app",
                "1.0",
                &["lib>=2", "plot; extra == \"charts\"", "gone>=9"],
            ),
            dist("also", "1.0", &["lib<3"]),
            dist("lib", "2.5", &["core", "old; python_version < \"3.0\""]),
            dist("core", "0.9", &[]),
            dist("plot", "4.0", &[]),
            dist("leaf", "1.0", &[]),
        ]
    }

    #[test]
    fn the_two_directions_agree_edge_for_edge() {
        // One rule, two directions. If these ever disagree, a focus view describes the same edge
        // twice and differently, and its dependents column stops matching the guard the user is
        // actually warned by.
        let g = ReverseDeps::build_for(&env(), "3.12.10");
        let all: Vec<PkgName> = env().iter().map(|d| d.name.clone()).collect();

        for from in &all {
            for edge in g.requires_of(from) {
                let back = g.edges_to(&edge.pkg);
                assert!(
                    back.iter()
                        .any(|e| e.pkg == *from && e.constraint == edge.constraint),
                    "{from} -> {} is missing from edges_to({})",
                    edge.pkg,
                    edge.pkg
                );
            }
            for edge in g.edges_to(from) {
                let fwd = g.requires_of(&edge.pkg);
                assert!(
                    fwd.iter()
                        .any(|e| e.pkg == *from && e.constraint == edge.constraint),
                    "{} -> {from} is missing from requires_of({})",
                    edge.pkg,
                    edge.pkg
                );
            }
        }
    }

    #[test]
    fn an_extra_gated_edge_appears_in_neither_direction() {
        // `plot` is installed, but `app` wants it only under `extra == "charts"`. Counting it
        // would have the view claim a dependency the environment does not rely on — the rule
        // `dependents_of` already applies, which is why both directions go through `applies_in`.
        let g = ReverseDeps::build_for(&env(), "3.12.10");
        assert!(
            !g.requires_of(&pkg("app"))
                .iter()
                .any(|e| e.pkg == pkg("plot"))
        );
        assert!(g.edges_to(&pkg("plot")).is_empty());
    }

    #[test]
    fn a_marker_for_another_python_is_not_a_missing_dependency() {
        // `lib` wants `old` only below Python 3.0, and `old` is not installed. Reporting it as
        // unsatisfied on 3.12 would make a healthy environment look broken, which is exactly why
        // this cannot be recomputed in the frontend from the raw strings.
        let g = ReverseDeps::build_for(&env(), "3.12.10");
        assert!(g.unsatisfied(&pkg("lib")).is_empty());
        // ...while a genuinely absent requirement still is one.
        assert_eq!(g.unsatisfied(&pkg("app")), [pkg("gone")]);
    }

    #[test]
    fn an_unsatisfied_requirement_is_never_also_a_dependency_row() {
        // The two sets are disjoint by construction, so no row carries an "is it installed?"
        // flag for the reader to interpret.
        let g = ReverseDeps::build_for(&env(), "3.12.10");
        let deps: Vec<PkgName> = g
            .requires_of(&pkg("app"))
            .into_iter()
            .map(|e| e.pkg)
            .collect();
        for missing in g.unsatisfied(&pkg("app")) {
            assert!(!deps.contains(&missing), "{missing} is in both sets");
        }
    }

    #[test]
    fn reach_and_impact_count_transitively_and_exclude_the_package() {
        let g = ReverseDeps::build_for(&env(), "3.12.10");
        // app -> lib -> core: two, not one, and `plot` is extra-gated so not three.
        assert_eq!(g.reach(&pkg("app")), 2);
        // core <- lib <- {app, also}: three.
        assert_eq!(g.removal_closure(&[pkg("core")]).len() - 1, 3);
        // A leaf is zero in both directions, which is a fact to render, not an absence.
        assert_eq!(g.reach(&pkg("leaf")), 0);
        assert_eq!(g.removal_closure(&[pkg("leaf")]).len() - 1, 0);
    }

    #[test]
    fn reach_terminates_on_a_cycle_and_does_not_count_the_package_itself() {
        // `removal_closure` has had this test since S3. `reach` is the same walk the other way
        // and needs its own: a cycle through the focus would otherwise count it as its own
        // transitive dependency.
        let g = ReverseDeps::build(&[
            dist("a", "1.0", &["b"]),
            dist("b", "1.0", &["c"]),
            dist("c", "1.0", &["a"]),
        ]);
        assert_eq!(g.reach(&pkg("a")), 2);
        assert_eq!(g.removal_closure(&[pkg("a")]).len() - 1, 2);
    }

    #[test]
    fn the_view_has_a_node_for_every_installed_package() {
        // Including `leaf`, which has no edge in either direction. A view that fell through on
        // those would leave 32 of the 352-package fixture with nothing to render.
        let g = ReverseDeps::build_for(&env(), "3.12.10");
        let view = g.view();
        assert_eq!(view.nodes.len(), env().len());
        for d in env() {
            assert!(view.nodes.contains_key(&d.name), "{} has no node", d.name);
        }
        let leaf = &view.nodes[&pkg("leaf")];
        assert!(leaf.dependents.is_empty() && leaf.dependencies.is_empty());
        assert_eq!(leaf.version.as_deref(), Some("1.0"));
    }

    #[test]
    fn the_view_agrees_with_the_methods_it_is_built_from() {
        // The view is what crosses the bridge; the methods are what every test above pins. If
        // the two can differ, all of that is proved about something the user never sees — which
        // is the shape of the fixture bug Slice 0 found.
        let g = ReverseDeps::build_for(&env(), "3.12.10");
        for (name, node) in &g.view().nodes {
            assert_eq!(node.dependents, g.edges_to(name), "{name} dependents");
            assert_eq!(
                node.dependencies,
                g.requires_of(name),
                "{name} dependencies"
            );
            assert_eq!(node.reach, g.reach(name), "{name} reach");
            assert_eq!(node.unsatisfied, g.unsatisfied(name), "{name} unsatisfied");
            assert_eq!(
                node.impact,
                g.removal_closure(std::slice::from_ref(name)).len() - 1,
                "{name} impact"
            );
        }
    }

    #[test]
    fn the_dependents_column_is_the_set_the_guard_warns_about() {
        // `edges_to` projects `breaking_dependents` rather than walking `edges` again, so this
        // holds by construction. Asserted anyway: it is the property that makes the projection
        // worth having, and an "optimisation" that inlined the walk would break it silently.
        let g = ReverseDeps::build_for(&env(), "3.12.10");
        // Compared whole, not by name. Comparing names alone passed a mutation that dropped the
        // version and the constraint from every row — the two halves of the sentence the column
        // exists to show.
        for d in env() {
            let guarded: Vec<DepEdge> = g
                .breaking_dependents(&d.name)
                .into_iter()
                .map(DepEdge::from)
                .collect();
            assert_eq!(g.edges_to(&d.name), guarded, "{} disagrees", d.name);
        }
    }
    /// The shape of a real environment, which no synthetic fixture can supply.
    ///
    /// Everything above is built from six hand-written dists. This one runs the probe against a
    /// live interpreter and reports what the graph actually looks like — the numbers that decided
    /// the view is drawn at depth 1 and navigated beyond it. Opt-in, like `audit`'s live tests:
    ///
    /// ```text
    /// PIPDOCK_GRAPH_PYTHON=C:\Python314\python.exe \
    ///   cargo test -p pipdock-core --release --lib graph::tests::the_real_environment -- --ignored --nocapture
    /// ```
    ///
    /// `--release` is not optional. A debug build measures bounds checks; the same slice measured
    /// an index load at 572 ms in debug and 140 ms in release, and a design was justified on the
    /// larger number before anyone noticed.
    #[tokio::test]
    #[ignore = "needs a real interpreter; run with --ignored and PIPDOCK_GRAPH_PYTHON"]
    async fn the_real_environment() {
        let Ok(exe) = std::env::var("PIPDOCK_GRAPH_PYTHON") else {
            panic!("set PIPDOCK_GRAPH_PYTHON to an interpreter path");
        };
        let probed =
            crate::envs::probe(std::path::Path::new(&exe), crate::model::EnvSource::Manual)
                .await
                .expect("probe succeeds");

        let built = std::time::Instant::now();
        let g = ReverseDeps::build_for(&probed.dists, &probed.env.python_version);
        let build_ms = built.elapsed();

        let viewed = std::time::Instant::now();
        let view = g.view();
        let view_ms = viewed.elapsed();

        let json = serde_json::to_string(&view).expect("serializes");

        let edges: usize = view.nodes.values().map(|n| n.dependencies.len()).sum();
        let mut fan_in: Vec<usize> = view.nodes.values().map(|n| n.dependents.len()).collect();
        let mut impacts: Vec<usize> = view.nodes.values().map(|n| n.impact).collect();
        fan_in.sort_unstable();
        impacts.sort_unstable();
        let p = |v: &[usize], q: f64| v[((v.len() as f64 * q) as usize).min(v.len() - 1)];
        let leaves = view
            .nodes
            .values()
            .filter(|n| n.dependents.is_empty())
            .count();
        let missing: usize = view.nodes.values().map(|n| n.unsatisfied.len()).sum();

        eprintln!("packages          {}", view.nodes.len());
        eprintln!("in-force edges    {edges}");
        eprintln!("build             {build_ms:?}");
        eprintln!("view              {view_ms:?}");
        eprintln!("payload           {} KB", json.len() / 1024);
        eprintln!(
            "fan-in            median {} p90 {} max {}",
            p(&fan_in, 0.5),
            p(&fan_in, 0.9),
            fan_in.last().copied().unwrap_or(0)
        );
        eprintln!(
            "impact            median {} p90 {} max {}",
            p(&impacts, 0.5),
            p(&impacts, 0.9),
            impacts.last().copied().unwrap_or(0)
        );
        eprintln!("leaves            {leaves}");
        eprintln!("unsatisfied       {missing}");

        // Not a benchmark assertion — those belong in `tests/`, and this number is a report. What
        // is asserted is the invariant a benchmark cannot state: every installed package has a
        // node, and no edge points anywhere but at another node.
        assert_eq!(view.nodes.len(), probed.dists.len());
        for (name, node) in &view.nodes {
            for edge in node.dependencies.iter().chain(&node.dependents) {
                assert!(
                    view.nodes.contains_key(&edge.pkg),
                    "{name} has an edge to {}, which has no node",
                    edge.pkg
                );
            }
        }
    }
}
