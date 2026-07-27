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
use crate::model::{PinnedSpec, PkgName, Version};
use crate::store::Store;

/// How a pin constrains a package.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// Add or replace a pin.
///
/// # Errors
/// `PD-INT-001` when the write fails.
pub fn add(store: &Store, env_hash: &str, pin: &Pin) -> Result<()> {
    let version = match &pin.mode {
        PinMode::Hold { version } => Some(version.0.as_str()),
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
