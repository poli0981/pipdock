//! Freeze snapshots, diffs and minimal-ops rollback. See DATA-FLOW §8.
//!
//! DATA-FLOW §9.2 is absolute: **no mutating engine call happens without a successful snapshot
//! write.** A failed write aborts the plan and executes nothing (`PD-SNP-001`). That rule is not
//! left to convention: [`crate::plan::execute`] takes a [`Snapshot`], so there is no way to call
//! it without having made one.
//!
//! Layout, per ARCHITECTURE §6:
//!
//! ```text
//! %LOCALAPPDATA%\PipDock\snapshots\<env_hash>\<id>.freeze.txt
//!                                            \<id>.meta.json
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::errors::{Code, PdError, Result};
use crate::model::{EngineId, PinnedSpec, PkgName, Version};

/// Directory name under the app data root (ARCHITECTURE §6).
pub const SNAPSHOT_DIR: &str = "snapshots";

/// What caused a snapshot to be taken. Shown as the trigger label on the timeline (UI-SPEC §4).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Trigger {
    /// Taken automatically before a plan executed.
    Plan {
        /// The plan's id, so a summary and its snapshot can be tied together.
        plan_id: String,
    },
    /// Taken automatically before a rollback, because a rollback is itself reversible
    /// (DATA-FLOW §8).
    Rollback {
        /// The snapshot being restored.
        restoring: String,
    },
    /// Taken because the user asked.
    Manual,
}

/// The `.meta.json` sidecar (ARCHITECTURE §6).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Meta {
    /// Snapshot id, which is also its filename stem.
    pub id: String,
    /// When it was taken, RFC 3339.
    pub created_at: String,
    /// Why it was taken.
    pub trigger: Trigger,
    /// Which engine produced the freeze.
    ///
    /// Load-bearing: pip freezes with `--all` and includes pip/setuptools, uv has no such flag and
    /// omits them (DATA-FLOW §7). Without this, a diff across engines would read as "pip was
    /// uninstalled".
    pub engine: EngineId,
    /// How many distributions the freeze recorded.
    pub package_count: usize,
    /// PipDock's version, so an old snapshot can be interpreted by a newer build.
    pub app_version: String,
}

/// A captured environment state.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// The sidecar.
    pub meta: Meta,
    /// The freeze document, verbatim.
    pub freeze: String,
    /// Where the freeze file lives.
    pub path: PathBuf,
}

impl Snapshot {
    /// The distributions this snapshot recorded.
    #[must_use]
    pub fn entries(&self) -> BTreeMap<PkgName, Version> {
        parse_freeze(&self.freeze)
    }
}

/// Parse a `pip freeze` / `uv pip freeze` document.
///
/// Only `name==version` lines are kept. Editable installs (`-e .`), direct URLs
/// (`name @ file:///…`) and VCS requirements are **skipped**, because PipDock cannot restore them
/// from an index — and silently pretending it could would produce a rollback that reports success
/// while leaving the environment different from the snapshot.
#[must_use]
pub fn parse_freeze(text: &str) -> BTreeMap<PkgName, Version> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }
        let Some((name, version)) = line.split_once("==") else {
            continue;
        };
        if name.contains('@') || version.contains('@') {
            continue;
        }
        let Ok(name) = PkgName::parse(name.trim()) else {
            continue;
        };
        out.insert(name, Version(version.trim().to_owned()));
    }
    out
}

/// Lines a freeze contained that [`parse_freeze`] could not turn into a restorable pin.
///
/// Surfaced so a rollback preview can say what it will not be able to restore, rather than
/// quietly dropping it — the same honesty `PD-SNP-002` exists for.
#[must_use]
pub fn unrestorable_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|l| l.starts_with('-') || l.contains('@') || !l.contains("=="))
        .map(str::to_owned)
        .collect()
}

/// One package whose version differs between two states.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChangedEntry {
    /// The package.
    pub name: PkgName,
    /// What is installed now.
    pub current: Version,
    /// What the snapshot recorded.
    pub snapshot: Version,
}

/// The difference between two environment states.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Diff {
    /// Present now, absent in the snapshot.
    pub added: Vec<PinnedSpec>,
    /// Present in the snapshot, absent now.
    pub removed: Vec<PinnedSpec>,
    /// Present in both at different versions.
    pub changed: Vec<ChangedEntry>,
}

impl Diff {
    /// True when the environment already matches the snapshot.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// Total number of packages that differ.
    #[must_use]
    pub fn len(&self) -> usize {
        self.added.len() + self.removed.len() + self.changed.len()
    }
}

/// Compare the current environment against a snapshot.
///
/// `current` is what is installed now; `snapshot` is what we want to return to. Swapping them
/// inverts every classification, so the names are deliberately not `a` and `b`.
#[must_use]
pub fn diff(current: &BTreeMap<PkgName, Version>, snapshot: &BTreeMap<PkgName, Version>) -> Diff {
    let mut out = Diff::default();

    for (name, version) in current {
        match snapshot.get(name) {
            None => out.added.push(PinnedSpec {
                name: name.clone(),
                version: version.clone(),
            }),
            Some(old) if old != version => out.changed.push(ChangedEntry {
                name: name.clone(),
                current: version.clone(),
                snapshot: old.clone(),
            }),
            Some(_) => {}
        }
    }
    for (name, version) in snapshot {
        if !current.contains_key(name) {
            out.removed.push(PinnedSpec {
                name: name.clone(),
                version: version.clone(),
            });
        }
    }
    out
}

/// The operations that would restore a snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RollbackPlan {
    /// Packages to remove: present now, absent in the snapshot.
    pub uninstall: Vec<PkgName>,
    /// Packages to install at the snapshot's versions.
    pub install: Vec<PinnedSpec>,
}

impl RollbackPlan {
    /// True when there is nothing to do.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.uninstall.is_empty() && self.install.is_empty()
    }
}

/// Turn a diff into the minimal set of operations that restores the snapshot (DATA-FLOW §8).
///
/// Minimal in the sense that matters: packages already at the snapshot's version are **not**
/// reinstalled. On a 200-package environment where two things changed, reinstalling everything
/// would take minutes and risk failures on packages that were fine.
#[must_use]
pub fn rollback_plan(diff: &Diff) -> RollbackPlan {
    RollbackPlan {
        uninstall: diff.added.iter().map(|s| s.name.clone()).collect(),
        install: diff
            .removed
            .iter()
            .cloned()
            .chain(diff.changed.iter().map(|c| PinnedSpec {
                name: c.name.clone(),
                version: c.snapshot.clone(),
            }))
            .collect(),
    }
}

/// Where snapshots for `env_hash` live.
#[must_use]
pub fn dir_for(app_data: &Path, env_hash: &str) -> PathBuf {
    app_data.join(SNAPSHOT_DIR).join(env_hash)
}

/// Write a snapshot.
///
/// # Errors
/// `PD-SNP-001` on any write failure. **Callers must abort the plan** — this is the one failure
/// mode that is not skip-and-continue, because executing without a snapshot removes the user's
/// only way back.
pub fn create(
    app_data: &Path,
    env_hash: &str,
    freeze: String,
    trigger: Trigger,
    engine: EngineId,
    now: jiff::Timestamp,
) -> Result<Snapshot> {
    let dir = dir_for(app_data, env_hash);
    let fail = |what: &str, e: &dyn std::fmt::Display| {
        PdError::new(
            Code::SnpWriteFailed,
            format!("could not {what}: {e} (plan aborted, nothing was executed)"),
        )
    };

    std::fs::create_dir_all(&dir).map_err(|e| fail("create the snapshot directory", &e))?;

    let id = snapshot_id(now);
    let meta = Meta {
        id: id.clone(),
        created_at: now.to_string(),
        trigger,
        engine,
        package_count: parse_freeze(&freeze).len(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
    };

    let freeze_path = dir.join(format!("{id}.freeze.txt"));
    let meta_path = dir.join(format!("{id}.meta.json"));

    std::fs::write(&freeze_path, &freeze).map_err(|e| fail("write the freeze file", &e))?;

    let meta_json =
        serde_json::to_string_pretty(&meta).map_err(|e| fail("serialize the metadata", &e))?;
    // The sidecar is written second and the freeze is cleaned up if it fails, so a snapshot is
    // never half present. A freeze without metadata would not appear in the timeline at all,
    // which is worse than no snapshot: the user would believe they had one.
    if let Err(e) = std::fs::write(&meta_path, meta_json) {
        let _ = std::fs::remove_file(&freeze_path);
        return Err(fail("write the snapshot metadata", &e));
    }

    Ok(Snapshot {
        meta,
        freeze,
        path: freeze_path,
    })
}

/// Snapshot ids sort lexicographically in time order, which is what makes `latest` cheap and the
/// timeline correct without reading every sidecar. Also filename-safe: no colons.
fn snapshot_id(now: jiff::Timestamp) -> String {
    now.to_string()
        .replace([':', '-'], "")
        .replace(['.', '+'], "-")
}

/// List snapshots for an environment, newest first.
///
/// # Errors
/// Never fails for a missing directory — an environment with no snapshots yet is normal, not an
/// error. A single unreadable sidecar is skipped rather than hiding the rest.
pub fn list(app_data: &Path, env_hash: &str) -> Result<Vec<Meta>> {
    let dir = dir_for(app_data, env_hash);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };

    let mut out: Vec<Meta> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".meta.json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|raw| serde_json::from_str(&raw).ok())
        .collect();

    out.sort_by(|a: &Meta, b: &Meta| b.id.cmp(&a.id));
    Ok(out)
}

/// Load a snapshot by id, or the newest when `id` is `"latest"` (CLI-SPEC §3).
///
/// # Errors
/// `PD-SNP-002` when no such snapshot exists.
pub fn load(app_data: &Path, env_hash: &str, id: &str) -> Result<Snapshot> {
    let dir = dir_for(app_data, env_hash);
    let all = list(app_data, env_hash)?;
    let meta = if id == "latest" {
        all.into_iter().next()
    } else {
        all.into_iter().find(|m| m.id == id)
    };
    let meta = meta.ok_or_else(|| {
        PdError::new(
            Code::SnpTargetUnavailable,
            format!("no snapshot {id:?} for this environment"),
        )
    })?;

    let path = dir.join(format!("{}.freeze.txt", meta.id));
    let freeze = std::fs::read_to_string(&path).map_err(|e| {
        PdError::new(
            Code::SnpTargetUnavailable,
            format!("snapshot {} has no freeze file: {e}", meta.id),
        )
    })?;

    Ok(Snapshot { meta, freeze, path })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(n: &str) -> PkgName {
        PkgName::parse(n).unwrap()
    }

    fn state(pairs: &[(&str, &str)]) -> BTreeMap<PkgName, Version> {
        pairs
            .iter()
            .map(|(n, v)| (pkg(n), Version((*v).to_owned())))
            .collect()
    }

    fn now() -> jiff::Timestamp {
        "2026-07-27T12:34:56Z".parse().unwrap()
    }

    #[test]
    fn parses_a_real_freeze_document() {
        let text = "certifi==2025.1.1\r\nidna==3.4\r\n\r\n# a comment\r\nurllib3==2.2.1\r\n";
        let got = parse_freeze(text);
        assert_eq!(got.len(), 3);
        assert_eq!(got.get(&pkg("idna")).map(|v| v.0.as_str()), Some("3.4"));
    }

    #[test]
    fn unrestorable_entries_are_skipped_and_reported() {
        // A rollback that silently dropped these would report success while leaving the
        // environment different from the snapshot.
        let text = "idna==3.4\n-e git+https://example/x#egg=x\nmypkg @ file:///C:/src/mypkg\n\
                    other @ https://example/other.whl\n";
        assert_eq!(parse_freeze(text).len(), 1, "only idna is restorable");

        let skipped = unrestorable_lines(text);
        assert_eq!(
            skipped.len(),
            3,
            "the other three must be reported: {skipped:?}"
        );
    }

    #[test]
    fn diff_classifies_added_removed_and_changed() {
        let current = state(&[("idna", "3.18"), ("httpx", "0.28.1")]);
        let snapshot = state(&[("idna", "3.4"), ("certifi", "2025.1.1")]);
        let d = diff(&current, &snapshot);

        assert_eq!(
            d.added,
            [PinnedSpec {
                name: pkg("httpx"),
                version: Version("0.28.1".into())
            }]
        );
        assert_eq!(
            d.removed,
            [PinnedSpec {
                name: pkg("certifi"),
                version: Version("2025.1.1".into())
            }]
        );
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].name, pkg("idna"));
        assert_eq!(d.changed[0].current.0, "3.18");
        assert_eq!(d.changed[0].snapshot.0, "3.4");
        assert_eq!(d.len(), 3);
    }

    #[test]
    fn an_unchanged_environment_diffs_to_nothing() {
        let s = state(&[("idna", "3.4")]);
        assert!(diff(&s, &s).is_empty());
        assert!(rollback_plan(&diff(&s, &s)).is_empty());
    }

    #[test]
    fn rollback_touches_only_what_differs() {
        // Reinstalling the untouched 198 packages of a 200-package environment would take minutes
        // and risk failures on packages that were fine.
        let current = state(&[("a", "2.0"), ("b", "1.0"), ("c", "1.0")]);
        let snapshot = state(&[("a", "1.0"), ("b", "1.0"), ("d", "1.0")]);
        let plan = rollback_plan(&diff(&current, &snapshot));

        assert_eq!(
            plan.uninstall,
            [pkg("c")],
            "only the added package is removed"
        );
        let installs: Vec<String> = plan
            .install
            .iter()
            .map(PinnedSpec::to_requirement)
            .collect();
        assert!(
            installs.contains(&"d==1.0".to_owned()),
            "the removed package comes back"
        );
        assert!(
            installs.contains(&"a==1.0".to_owned()),
            "the changed package is pinned back"
        );
        assert!(
            !installs.iter().any(|i| i.starts_with("b==")),
            "b was identical and must not be touched"
        );
    }

    #[test]
    fn applying_a_rollback_plan_reproduces_the_snapshot() {
        // TESTING §1's property: apply(plan(diff(current, snapshot)), current) == snapshot.
        let current = state(&[("a", "2.0"), ("c", "1.0")]);
        let snapshot = state(&[("a", "1.0"), ("d", "1.0")]);
        let plan = rollback_plan(&diff(&current, &snapshot));

        let mut applied = current.clone();
        for name in &plan.uninstall {
            applied.remove(name);
        }
        for spec in &plan.install {
            applied.insert(spec.name.clone(), spec.version.clone());
        }
        assert_eq!(applied, snapshot);
    }

    #[test]
    fn writing_and_loading_a_snapshot_round_trips() {
        let tmp = std::env::temp_dir().join(format!("pd-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        let snap = create(
            &tmp,
            "abc123",
            "idna==3.4\ncertifi==2025.1.1\n".to_owned(),
            Trigger::Manual,
            EngineId::Pip,
            now(),
        )
        .expect("write");

        assert_eq!(snap.meta.package_count, 2);
        assert_eq!(snap.meta.engine, EngineId::Pip);
        assert!(snap.path.is_file());

        assert_eq!(list(&tmp, "abc123").expect("list").len(), 1);

        let loaded = load(&tmp, "abc123", "latest").expect("load latest");
        assert_eq!(loaded.meta.id, snap.meta.id);
        assert_eq!(loaded.entries().len(), 2);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn a_missing_snapshot_directory_is_not_an_error() {
        // A fresh environment has no snapshots; that is normal, not a failure to report.
        assert!(
            list(Path::new("no-such-app-data"), "deadbeef")
                .expect("list")
                .is_empty()
        );
    }

    #[test]
    fn loading_an_unknown_snapshot_uses_the_documented_code() {
        let err = load(Path::new("no-such-app-data"), "deadbeef", "latest").expect_err("must fail");
        assert_eq!(err.code, Code::SnpTargetUnavailable);
    }

    #[test]
    fn snapshot_ids_sort_in_time_order_and_are_filename_safe() {
        // `latest` and the timeline both rely on the ordering, and neither reads sidecars to sort.
        let earlier = snapshot_id("2026-07-27T12:00:00Z".parse().unwrap());
        let later = snapshot_id("2026-07-27T12:34:56Z".parse().unwrap());
        assert!(earlier < later, "{earlier} should sort before {later}");
        assert!(
            !earlier.contains(':'),
            "ids must be filename-safe: {earlier}"
        );
    }
}
