//! Per-environment pins. See PRD P0-7.
//!
//! **DATA-FLOW §9.5 is the reason this module exists:** pinned packages never appear in a
//! `PlanRequest.upgrades` unless the user explicitly unpinned them in the same session. A pin the
//! user set months ago must survive every *Select all*, or the feature is decorative.
//!
//! [`filter_upgrades`] is where that is enforced, and it returns what it excluded rather than
//! quietly dropping it — UI-SPEC §4 requires *Select all* to say "3 pinned excluded", because a
//! selection that silently ignores part of what you selected is worse than one that refuses.

use std::collections::BTreeSet;

use crate::errors::{Code, PdError, Result};
use crate::graph::ReverseDeps;
use crate::model::{Dist, PinnedSpec, PkgName, Version};
use crate::store::Store;

/// How a pin constrains a package.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum PinMode {
    /// Excluded from bulk updates. The user can still update it deliberately, one package at a
    /// time — a pin is "do not sweep this up", not "never touch this".
    Exclude,
    /// Held at an exact version: excluded from bulk updates *and* restated at this version in
    /// every plan, so the resolver treats it as fixed.
    Hold {
        /// The version to hold at.
        version: Version,
    },
}

impl PinMode {
    fn tag(&self) -> &'static str {
        match self {
            Self::Exclude => "exclude",
            Self::Hold { .. } => "hold",
        }
    }
}

/// A pin with the reason the user gave for it.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Pin {
    /// The pinned package.
    pub pkg: PkgName,
    /// How it is constrained.
    pub mode: PinMode,
    /// Free-text justification shown in the Pins screen.
    ///
    /// Worth storing even though nothing reads it programmatically: a pin without a reason
    /// becomes a mystery the user is afraid to remove, and so stays forever.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Read the pin list for an environment, ordered by package name.
///
/// # Errors
/// `PD-INT-001` when the pin table cannot be read.
pub fn list(store: &Store, env_hash: &str) -> Result<Vec<Pin>> {
    let mut stmt = store
        .conn()
        .prepare("SELECT pkg, mode, version, reason FROM pins WHERE env_hash = ?1 ORDER BY pkg")
        .map_err(db_err)?;

    let rows = stmt
        .query_map([env_hash], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(db_err)?;

    let mut out = Vec::new();
    for row in rows {
        let (pkg, mode, version, reason) = row.map_err(db_err)?;
        // A row whose name no longer parses is skipped rather than failing the whole list; it can
        // only come from a hand-edited database, and hiding every other pin would be worse.
        let Ok(pkg) = PkgName::parse(&pkg) else {
            continue;
        };
        let mode = match (mode.as_str(), version) {
            ("hold", Some(v)) => PinMode::Hold {
                version: Version(v),
            },
            _ => PinMode::Exclude,
        };
        out.push(Pin { pkg, mode, reason });
    }
    Ok(out)
}

/// Reject a held version that is not shaped like one.
///
/// SECURITY §2 promises versions are validated before they enter argv, and a `Hold` pin is
/// exactly that path: [`hold_requirements`] turns it into a [`PinnedSpec`] rendered as
/// `name==version` in a mutating command. `PkgName` enforces its own grammar on construction,
/// but `Version` is a transparent newtype over whatever the engine reported — so the one place a
/// *user* supplies a version is the one place that claim needs enforcing.
///
/// Deliberately a character-class check and not a full PEP 440 parse. The job is to refuse
/// whitespace, quotes, path separators and control characters, not to second-guess an epoch or a
/// local-version segment that the resolver understands better than PipDock does. Rejecting a
/// legitimate version would be the worse failure: it would make a package unpinnable.
fn validated_hold(version: &Version) -> Result<&str> {
    let raw = version.0.as_str();
    let invalid = || {
        PdError::new(
            Code::PkgNotFound,
            format!("invalid version for a pin: {raw:?}"),
        )
    };
    let bytes = raw.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphanumeric() {
        return Err(invalid());
    }
    if !bytes
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+' | b'!'))
    {
        return Err(invalid());
    }
    Ok(raw)
}

/// Add or replace a pin.
///
/// # Errors
/// `PD-PKG-002` when a `Hold` names a version that is not well formed; `PD-INT-001` when the
/// write fails.
pub fn add(store: &Store, env_hash: &str, pin: &Pin) -> Result<()> {
    let version = match &pin.mode {
        PinMode::Hold { version } => Some(validated_hold(version)?),
        PinMode::Exclude => None,
    };
    store
        .conn()
        .execute(
            "INSERT INTO pins (env_hash, pkg, mode, version, reason) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (env_hash, pkg) DO UPDATE SET
                mode = excluded.mode, version = excluded.version, reason = excluded.reason",
            rusqlite::params![
                env_hash,
                pin.pkg.as_str(),
                pin.mode.tag(),
                version,
                pin.reason.as_deref()
            ],
        )
        .map_err(db_err)?;
    Ok(())
}

/// Remove a pin. Returns whether one existed.
///
/// # Errors
/// `PD-INT-001` when the delete fails.
pub fn remove(store: &Store, env_hash: &str, pkg: &PkgName) -> Result<bool> {
    let n = store
        .conn()
        .execute(
            "DELETE FROM pins WHERE env_hash = ?1 AND pkg = ?2",
            rusqlite::params![env_hash, pkg.as_str()],
        )
        .map_err(db_err)?;
    Ok(n > 0)
}

/// The result of applying pins to a set of upgrade candidates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filtered {
    /// Candidates that may be upgraded.
    pub allowed: Vec<PkgName>,
    /// Candidates a pin removed, with the pin that did it.
    ///
    /// Surfaced so *Select all* can say how many were excluded (UI-SPEC §4). Dropping these
    /// silently would let the user believe they had selected something they had not.
    pub excluded: Vec<Pin>,
}

impl Filtered {
    /// How many candidates a pin removed.
    #[must_use]
    pub fn excluded_count(&self) -> usize {
        self.excluded.len()
    }
}

/// Apply pins to upgrade candidates — the enforcement point for DATA-FLOW §9.5.
///
/// `unpinned_this_session` is the escape hatch the invariant names: a package the user unpinned
/// deliberately during this session may be upgraded even though the stored pin still exists,
/// because the store is only written when the change is committed. Nothing else overrides a pin.
#[must_use]
pub fn filter_upgrades(
    candidates: &[PkgName],
    pins: &[Pin],
    unpinned_this_session: &BTreeSet<PkgName>,
) -> Filtered {
    let mut out = Filtered::default();
    for candidate in candidates {
        let pin = pins
            .iter()
            .find(|p| &p.pkg == candidate && !unpinned_this_session.contains(candidate));
        match pin {
            Some(p) => out.excluded.push(p.clone()),
            None => out.allowed.push(candidate.clone()),
        }
    }
    out
}

/// A package worth pinning, and why — PRD P1-2, UI-SPEC §4.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct PinSuggestion {
    /// The package being suggested.
    pub pkg: PkgName,
    /// How many installed packages depend on it.
    ///
    /// The number is the whole argument, so it is carried rather than recomputed for display:
    /// "12 packages depend on it" is what makes the suggestion actionable, and a count the screen
    /// derived separately would eventually disagree with the one that qualified it.
    pub dependents: usize,
}

/// Packages many others depend on, which a bulk update should probably leave alone.
///
/// **The advisory half of this module.** [`filter_upgrades`] is enforcement — what a pin *does*;
/// this is what the shape of an environment suggests pinning. PRD P1-2: at or above `threshold`
/// reverse dependencies, offer a pin.
///
/// Counts come from [`ReverseDeps::dependent_count`], which counts **in-force** edges only —
/// extra-gated and marker-excluded requirements do not qualify a package. That is the same set the
/// uninstall guard warns about, and keeping the two identical is the entire reason this is
/// computed here rather than in the frontend, which already holds `requires_dist` and could
/// plausibly loop over it. Two implementations of one edge rule drift, and the half that drifts
/// silently is this one.
///
/// Takes the pins and the threshold as values rather than a `&Store`: `Store` is not `Sync`, so a
/// future holding one is not `Send` and cannot be returned from a Tauri command. [`crate::flow`]
/// takes pins for the same reason.
///
/// Already-pinned packages are dropped — suggesting what the user has already done is noise.
/// Nothing records a *rejected* suggestion, so unpinning something re-offers it; that is a known
/// rough edge rather than an oversight, and both are on the same screen so it is visible.
#[must_use]
pub fn suggest(
    dists: &[Dist],
    python_version: &str,
    existing: &[Pin],
    threshold: usize,
) -> Vec<PinSuggestion> {
    // A threshold of zero would suggest every package in the environment, including the leaves
    // nothing depends on. Treated as "off" rather than as "suggest everything".
    if threshold == 0 {
        return Vec::new();
    }

    let pinned: BTreeSet<&PkgName> = existing.iter().map(|p| &p.pkg).collect();
    // `build_for`, not `build`: without the interpreter, a requirement gated on some other
    // `python_version` counts as if it were in force, and the suggestion inherits the same wrong
    // answer the preview used to give (SP-5).
    let graph = ReverseDeps::build_for(dists, python_version);

    let mut out: Vec<PinSuggestion> = dists
        .iter()
        .filter(|d| !pinned.contains(&d.name))
        .filter_map(|d| {
            let dependents = graph.dependent_count(&d.name);
            (dependents >= threshold).then(|| PinSuggestion {
                pkg: d.name.clone(),
                dependents,
            })
        })
        .collect();

    // Most-depended-upon first, then by name. Stable ordering matters more than it looks: this
    // list is re-fetched on every visit to the screen, and one that reorders between two identical
    // reads reads as though the environment changed.
    out.sort_by(|a, b| b.dependents.cmp(&a.dependents).then(a.pkg.cmp(&b.pkg)));
    out
}

/// The exact-version requirements `Hold` pins contribute to every plan.
///
/// These join the guard set [`crate::engine::plan_requirements`] builds, so the resolver treats a
/// held package as fixed rather than merely un-selected.
#[must_use]
pub fn hold_requirements(pins: &[Pin]) -> Vec<PinnedSpec> {
    pins.iter()
        .filter_map(|p| match &p.mode {
            PinMode::Hold { version } => Some(PinnedSpec {
                name: p.pkg.clone(),
                version: version.clone(),
            }),
            PinMode::Exclude => None,
        })
        .collect()
}

fn db_err(e: rusqlite::Error) -> PdError {
    PdError::new(Code::IntUnexpected, format!("pin store: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(n: &str) -> PkgName {
        PkgName::parse(n).unwrap()
    }

    fn exclude(name: &str, reason: Option<&str>) -> Pin {
        Pin {
            pkg: pkg(name),
            mode: PinMode::Exclude,
            reason: reason.map(str::to_owned),
        }
    }

    fn dist(name: &str, requires: &[&str]) -> Dist {
        Dist {
            name: pkg(name),
            version: Version("1.0".into()),
            requires_dist: requires.iter().map(|s| (*s).to_owned()).collect(),
            requires_python: None,
            size_bytes: None,
        }
    }

    /// `n` packages that all depend on `on`, plus `on` itself.
    fn depended_on_by(on: &str, n: usize) -> Vec<Dist> {
        std::iter::once(dist(on, &[]))
            .chain((0..n).map(|i| {
                let name = format!("app{i}");
                dist(&name, &[on])
            }))
            .collect()
    }

    #[test]
    fn a_package_below_the_threshold_is_not_suggested() {
        let dists = depended_on_by("urllib3", 4);
        assert_eq!(suggest(&dists, "3.12.4", &[], 5), Vec::new());
    }

    #[test]
    fn the_threshold_is_inclusive() {
        // PRD P1-2 says "count >= threshold". Off by one here means the default of 5 fires at 6,
        // and nobody would notice because both are plausible.
        let dists = depended_on_by("urllib3", 5);
        let got = suggest(&dists, "3.12.4", &[], 5);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].pkg, pkg("urllib3"));
        assert_eq!(got[0].dependents, 5);
    }

    #[test]
    fn an_already_pinned_package_is_never_suggested() {
        // Suggesting what the user has already done is noise, however high the count.
        let dists = depended_on_by("urllib3", 50);
        let pins = vec![exclude("urllib3", None)];
        assert_eq!(suggest(&dists, "3.12.4", &pins, 5), Vec::new());
    }

    #[test]
    fn extra_gated_dependents_do_not_qualify_a_package() {
        // The same rule the uninstall guard follows. An extra is not installed unless it was
        // asked for, so a dependency that exists only under `extra == "socks"` is not a reason to
        // pin anything — and the guard and the suggestion must agree about which edges are real
        // or the user is pinning on one rule and being warned on another.
        let dists: Vec<Dist> = std::iter::once(dist("urllib3", &[]))
            .chain((0..6).map(|i| {
                let name = format!("app{i}");
                dist(&name, &["urllib3; extra == \"socks\""])
            }))
            .collect();
        assert_eq!(suggest(&dists, "3.12.4", &[], 5), Vec::new());
    }

    #[test]
    fn requirements_gated_on_another_python_do_not_qualify_a_package() {
        // `build_for`, not `build`. With markers unevaluated these six count and urllib3 is
        // suggested on 3.12 because of constraints that only apply below 3.11.
        let dists: Vec<Dist> = std::iter::once(dist("urllib3", &[]))
            .chain((0..6).map(|i| {
                let name = format!("app{i}");
                dist(&name, &["urllib3; python_version < \"3.11\""])
            }))
            .collect();
        assert_eq!(suggest(&dists, "3.12.4", &[], 5), Vec::new());
    }

    #[test]
    fn suggestions_are_ordered_by_count_then_name() {
        // The list is re-read on every visit to the screen. One that reorders between two
        // identical reads reads as though the environment changed under the user.
        let mut dists = vec![
            dist("urllib3", &[]),
            dist("certifi", &[]),
            dist("idna", &[]),
        ];
        for i in 0..8 {
            dists.push(dist(&format!("app{i}"), &["urllib3", "certifi", "idna"]));
        }
        // Two more that only reach urllib3, so it leads; certifi and idna tie at 8 and sort by
        // name.
        for i in 8..10 {
            dists.push(dist(&format!("extra{i}"), &["urllib3"]));
        }

        let got = suggest(&dists, "3.12.4", &[], 5);
        let names: Vec<&str> = got.iter().map(|s| s.pkg.as_str()).collect();
        assert_eq!(names, ["urllib3", "certifi", "idna"]);
        assert_eq!(got[0].dependents, 10);
        assert_eq!(got[1].dependents, 8);
        assert_eq!(got[2].dependents, 8);
    }

    #[test]
    fn a_threshold_of_zero_suggests_nothing_rather_than_everything() {
        // The degenerate setting. Every package trivially has >= 0 dependents, so without the
        // guard this offers to pin the whole environment, leaves included.
        let dists = depended_on_by("urllib3", 6);
        assert_eq!(suggest(&dists, "3.12.4", &[], 0), Vec::new());
    }

    #[test]
    fn a_held_version_is_shape_checked_before_it_can_reach_argv() {
        let store = Store::in_memory().expect("store");
        let hold = |v: &str| Pin {
            pkg: pkg("numpy"),
            mode: PinMode::Hold {
                version: Version(v.into()),
            },
            reason: None,
        };

        // Real versions, including the awkward corners of PEP 440, must stay pinnable: an
        // over-strict check would make a package impossible to hold, which is worse than lax.
        for good in [
            "1.26.4",
            "2.0.0rc1",
            "1!2.0",
            "1.0+local.7",
            "v1.2",
            "0.10.12",
        ] {
            add(&store, "envA", &hold(good)).unwrap_or_else(|e| panic!("{good:?}: {e:?}"));
        }

        for bad in [
            "",
            " 1.0",
            "1.0 ",
            "-1.0",
            "1.0;rm",
            "1.0/../x",
            "--upgrade",
            "1.0\n2.0",
        ] {
            let err = add(&store, "envA", &hold(bad)).expect_err("{bad:?} should be rejected");
            assert_eq!(err.code.as_str(), "PD-PKG-002", "for {bad:?}");
        }

        // An Exclude pin has no version, so it is unaffected either way.
        add(&store, "envA", &exclude("requests", None)).expect("exclude still adds");
    }

    #[test]
    fn pins_round_trip_through_the_store() {
        let store = Store::in_memory().expect("store");
        add(
            &store,
            "envA",
            &exclude("urllib3", Some("12 packages depend on it")),
        )
        .expect("add");
        add(
            &store,
            "envA",
            &Pin {
                pkg: pkg("numpy"),
                mode: PinMode::Hold {
                    version: Version("1.26.4".into()),
                },
                reason: None,
            },
        )
        .expect("add");

        let pins = list(&store, "envA").expect("list");
        assert_eq!(pins.len(), 2);
        assert_eq!(pins[0].pkg, pkg("numpy"), "ordered by name");
        assert_eq!(
            pins[0].mode,
            PinMode::Hold {
                version: Version("1.26.4".into())
            }
        );
        assert_eq!(pins[1].reason.as_deref(), Some("12 packages depend on it"));
    }

    #[test]
    fn pins_are_per_environment() {
        // The same package pinned in one venv must stay free in another, or a pin set for one
        // project silently changes behaviour in every other.
        let store = Store::in_memory().expect("store");
        add(&store, "envA", &exclude("requests", None)).expect("add");

        assert_eq!(list(&store, "envA").expect("list").len(), 1);
        assert!(list(&store, "envB").expect("list").is_empty());
    }

    #[test]
    fn adding_the_same_package_twice_updates_rather_than_duplicates() {
        let store = Store::in_memory().expect("store");
        add(&store, "envA", &exclude("requests", Some("first"))).expect("add");
        add(&store, "envA", &exclude("requests", Some("second"))).expect("add");

        let pins = list(&store, "envA").expect("list");
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].reason.as_deref(), Some("second"));
    }

    #[test]
    fn removing_reports_whether_a_pin_existed() {
        let store = Store::in_memory().expect("store");
        add(&store, "envA", &exclude("requests", None)).expect("add");

        assert!(remove(&store, "envA", &pkg("requests")).expect("remove"));
        assert!(!remove(&store, "envA", &pkg("requests")).expect("remove again"));
        assert!(list(&store, "envA").expect("list").is_empty());
    }

    #[test]
    fn pinned_packages_are_kept_out_of_upgrades() {
        // DATA-FLOW §9.5, the whole point of the module.
        let candidates = [pkg("requests"), pkg("urllib3"), pkg("httpx")];
        let pins = [exclude("urllib3", Some("holds the stack together"))];

        let got = filter_upgrades(&candidates, &pins, &BTreeSet::new());

        assert_eq!(got.allowed, [pkg("requests"), pkg("httpx")]);
        assert_eq!(got.excluded_count(), 1);
        assert_eq!(got.excluded[0].pkg, pkg("urllib3"));
    }

    #[test]
    fn unpinning_in_this_session_releases_the_package() {
        // The one documented override. Without it, unpinning would not take effect until the
        // change was committed and reloaded.
        let candidates = [pkg("urllib3")];
        let pins = [exclude("urllib3", None)];
        let unpinned = BTreeSet::from([pkg("urllib3")]);

        let got = filter_upgrades(&candidates, &pins, &unpinned);
        assert_eq!(got.allowed, [pkg("urllib3")]);
        assert!(got.excluded.is_empty());
    }

    #[test]
    fn a_pin_for_a_package_that_is_not_a_candidate_changes_nothing() {
        let got = filter_upgrades(
            &[pkg("requests")],
            &[exclude("numpy", None)],
            &BTreeSet::new(),
        );
        assert_eq!(got.allowed, [pkg("requests")]);
        assert!(got.excluded.is_empty());
    }

    #[test]
    fn hold_pins_become_exact_requirements_but_exclude_pins_do_not() {
        // An Exclude pin means "do not sweep this up"; it must not also freeze the resolver's hand
        // when the package moves as someone else's dependency.
        let pins = [
            exclude("requests", None),
            Pin {
                pkg: pkg("numpy"),
                mode: PinMode::Hold {
                    version: Version("1.26.4".into()),
                },
                reason: None,
            },
        ];
        let reqs = hold_requirements(&pins);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].to_requirement(), "numpy==1.26.4");
    }

    #[test]
    fn nothing_pinned_means_nothing_filtered() {
        let candidates = [pkg("a"), pkg("b")];
        let got = filter_upgrades(&candidates, &[], &BTreeSet::new());
        assert_eq!(got.allowed, candidates);
        assert_eq!(got.excluded_count(), 0);
    }
}
