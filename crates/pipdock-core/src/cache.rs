//! What PipDock has written to disk, and removing the parts that are safe to remove — PRD P1-4.
//!
//! # This module introduces the first delete-a-tree in the codebase
//!
//! Before it, the only removal anywhere outside test scaffolding was one `remove_file` in
//! [`crate::snapshot::create`]'s failure path. So the safety design *is* the feature, and it is
//! two rules:
//!
//! 1. **Nothing is removed by path.** A caller names a [`Target`], and this module decides where
//!    that target lives. There is no function here that takes a path to delete.
//! 2. **Every resolved path is checked to be inside the data root** before removal, by
//!    canonicalized prefix and not by string comparison — see [`clear`].
//!
//! # Why `index.db` is not a target
//!
//! It holds the package-name index *and* the settings *and* the pin list *and* the legal-consent
//! record, all in one SQLite file (`store::Store`). "Clear the cache" removing a user's pins is
//! exactly the kind of surprise `legal/PRIVACY-POLICY.md` §2 had to be corrected for. Its size is
//! reported so the number is honest about where the space went; refreshing the index is
//! `index refresh`, which replaces rows rather than deleting a file.
//!
//! # There are no log files
//!
//! [`crate::LOG_RETENTION_DAYS`] has never had a reader. The only log is the in-memory ring buffer
//! behind *Report bug*, and `logs_tail` is still owed. When the logging subsystem lands it becomes
//! a fourth entry here; until then, reporting a "logs" line of zero bytes would be inventing a
//! thing that does not exist.

use std::path::{Path, PathBuf};

use crate::errors::{Code, PdError, Result};

/// Something the user can be offered a *Clear* button for.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Target {
    /// Every snapshot of every environment. Rollback loses its history; nothing else changes.
    Snapshots,
    /// The isolated environment Code Health runs deptry, vulture and ruff from. Rebuilt on the
    /// next run, or by `pipdock tools sync`.
    Tools,
    /// The isolated environment the Security tab runs pip-audit from. Rebuilt on the next audit.
    ///
    /// A **separate** target rather than part of [`Self::Tools`], because it is a separate venv:
    /// P1-1 kept pip-audit out of Code Health's so that its `msgpack` dependency cannot fail the
    /// whole sync. Two directories on disk are two rows here, or "clear the cache" would leave one
    /// of them behind while reporting it gone.
    Audit,
}

/// One line of the cache report.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// Bytes on disk, summed recursively.
    pub bytes: u64,
    /// Where it is. Data — shown verbatim, never translated (I18N §2).
    pub path: String,
    /// False when nothing is there yet, so the UI can say "nothing to clear" rather than "0 B".
    pub exists: bool,
}

/// What PipDock is using, by artefact.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// The data root everything below lives in.
    pub root: String,
    /// `index.db` — the package index, settings, pins and the consent record. **Not clearable.**
    pub database: Entry,
    /// Snapshots, across every environment.
    pub snapshots: Entry,
    /// The Code Health tools environment.
    pub tools: Entry,
    /// The Security tab's pip-audit environment (PRD P1-1).
    pub audit: Entry,
    /// How many snapshot documents there are, across every environment.
    pub snapshot_count: usize,
}

impl Usage {
    /// Everything PipDock has written, including the parts that cannot be cleared.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.database.bytes + self.snapshots.bytes + self.tools.bytes + self.audit.bytes
    }
}

/// Bytes under `path`, following directories and ignoring what cannot be read.
///
/// An unreadable entry contributes zero rather than failing the report: a size is advisory, and a
/// number that is slightly low is more useful than a screen that will not open. Symlinks are not
/// followed — `fs::metadata` on the entry would report the target's size and could recurse
/// outside the data root, which is the one thing this module is careful about.
fn bytes_of(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_symlink() {
        return 0;
    }
    if meta.is_file() {
        return meta.len();
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(std::result::Result::ok)
        .map(|e| bytes_of(&e.path()))
        .sum()
}

fn entry_at(path: PathBuf) -> Entry {
    Entry {
        bytes: bytes_of(&path),
        exists: path.exists(),
        path: path.display().to_string(),
    }
}

/// How many `*.meta.json` sidecars there are under the snapshots root.
fn count_snapshots(root: &Path) -> usize {
    let Ok(envs) = std::fs::read_dir(root) else {
        return 0;
    };
    envs.filter_map(std::result::Result::ok)
        .filter_map(|env| std::fs::read_dir(env.path()).ok())
        .flat_map(|files| {
            files.filter_map(std::result::Result::ok).filter(|f| {
                f.file_name()
                    .to_str()
                    .is_some_and(|n| n.ends_with(".meta.json"))
            })
        })
        .count()
}

/// Where a target lives under `app_data`.
///
/// The only place a `Target` becomes a path, so [`clear`] cannot be handed one from outside.
fn path_of(app_data: &Path, target: Target) -> PathBuf {
    match target {
        Target::Snapshots => app_data.join(crate::snapshot::SNAPSHOT_DIR),
        Target::Tools => crate::health::tools_dir(app_data),
        Target::Audit => crate::health::audit_dir(app_data),
    }
}

/// Read what is on disk.
///
/// # Errors
/// Never — an unreadable path reports zero. Returns `Result` so a future variant that must fail
/// does not change every caller.
pub fn usage(app_data: &Path) -> Result<Usage> {
    let snapshots = app_data.join(crate::snapshot::SNAPSHOT_DIR);
    Ok(Usage {
        root: app_data.display().to_string(),
        database: entry_at(app_data.join(crate::store::DB_FILE)),
        snapshot_count: count_snapshots(&snapshots),
        snapshots: entry_at(snapshots),
        tools: entry_at(crate::health::tools_dir(app_data)),
        audit: entry_at(crate::health::audit_dir(app_data)),
    })
}

/// Remove a target, returning how many bytes went.
///
/// **The containment check is the point of this function.** `app_data` arrives from
/// [`crate::store::default_app_data`] or from the Tauri app handle, and `target` is an enum — so
/// on paper the path cannot escape. The check is here anyway, because the cost of being wrong is
/// deleting something outside the data root and the cost of being right is one `canonicalize`.
///
/// Canonicalized on **both** sides before comparing: a prefix test on unresolved paths is a string
/// comparison wearing a `Path`, and on Windows it would pass `data\..\..\Users` and fail a
/// perfectly good path that reached here through a junction or a short name.
///
/// # Errors
/// `PD-INT-001` when the resolved path is not inside `app_data` — which is a bug, not a user
/// error, and must be loud. `PD-PRM-002` when something has the files open, which on Windows is
/// the ordinary case for a tools venv whose Python is still running.
pub fn clear(app_data: &Path, target: Target) -> Result<u64> {
    let path = path_of(app_data, target);
    if !path.exists() {
        return Ok(0);
    }

    let root = app_data.canonicalize().map_err(|e| {
        PdError::new(
            Code::IntUnexpected,
            format!("resolve data root {}: {e}", app_data.display()),
        )
    })?;
    let resolved = path.canonicalize().map_err(|e| {
        PdError::new(
            Code::IntUnexpected,
            format!("resolve {}: {e}", path.display()),
        )
    })?;
    if !resolved.starts_with(&root) {
        return Err(PdError::new(
            Code::IntUnexpected,
            format!(
                "refusing to remove {} — outside the data root {}",
                resolved.display(),
                root.display()
            ),
        ));
    }

    let bytes = bytes_of(&resolved);
    std::fs::remove_dir_all(&resolved).map_err(|e| {
        // Windows holds files open while a process is using them, and a tools venv whose Python is
        // still running is the ordinary way to hit this — which is a thing the user can act on.
        PdError::new(
            Code::PrmFileLocked,
            format!("remove {}: {e}", resolved.display()),
        )
    })?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    /// Every `Entry` on a `Usage` must reach `total_bytes`.
    ///
    /// Written because it did not. `audit` was added as a fourth row, reported on screen, and left
    /// out of the sum — so the total silently under-reported by a whole venv while every existing
    /// test passed. The assertion is on distinct powers of two so a *missing* term is visible in
    /// the failure rather than merely a wrong number.
    #[test]
    fn every_entry_reaches_the_total() {
        use super::{Entry, Usage};

        let at = |bytes: u64| Entry {
            bytes,
            path: String::new(),
            exists: true,
        };
        let usage = Usage {
            root: String::new(),
            database: at(1),
            snapshots: at(2),
            tools: at(4),
            audit: at(8),
            snapshot_count: 0,
        };

        assert_eq!(
            usage.total_bytes(),
            15,
            "a missing term shows up as its own bit"
        );
    }

    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pipdock-cache-{}",
            jiff::Timestamp::now().as_nanosecond()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, vec![b'x'; bytes]).expect("write");
    }

    #[test]
    fn a_fresh_install_reports_nothing_rather_than_failing() {
        let root = temp();
        let got = usage(&root).expect("usage");
        assert_eq!(got.total_bytes(), 0);
        assert!(!got.snapshots.exists);
        assert!(!got.tools.exists);
        assert_eq!(got.snapshot_count, 0);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn sizes_are_summed_recursively_and_snapshots_counted_by_sidecar() {
        let root = temp();
        write(&root.join("index.db"), 100);
        write(&root.join("snapshots/envA/2026.freeze.txt"), 30);
        write(&root.join("snapshots/envA/2026.meta.json"), 20);
        write(&root.join("snapshots/envB/2027.meta.json"), 10);
        write(&root.join("tools/.venv/Scripts/python.exe"), 40);

        let got = usage(&root).expect("usage");
        assert_eq!(got.database.bytes, 100);
        assert_eq!(got.snapshots.bytes, 60);
        assert_eq!(got.tools.bytes, 40);
        assert_eq!(got.total_bytes(), 200);
        // Counted by sidecar, not by file: a freeze and its meta are one snapshot.
        assert_eq!(got.snapshot_count, 2);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn clearing_snapshots_leaves_the_database_alone() {
        // The whole reason `index.db` is not a target: it holds settings, pins and the consent
        // record as well as the index, and "clear the cache" must never take a user's pins.
        let root = temp();
        write(&root.join("index.db"), 100);
        write(&root.join("snapshots/envA/2026.meta.json"), 20);

        let freed = clear(&root, Target::Snapshots).expect("clear");
        assert_eq!(freed, 20);
        assert!(!root.join("snapshots").exists());
        assert!(root.join("index.db").exists());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn clearing_something_that_is_not_there_is_not_an_error() {
        // A fresh install has no tools venv, and offering a button that errors is worse than one
        // that does nothing.
        let root = temp();
        assert_eq!(clear(&root, Target::Tools).expect("clear"), 0);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_symlink_contributes_nothing_and_is_not_followed_out_of_the_root() {
        // `fs::metadata` follows links and would report the target's size — and a recursive walk
        // that follows one can leave the data root entirely, which is the single thing this
        // module exists to prevent.
        let root = temp();
        write(&root.join("snapshots/envA/2026.meta.json"), 20);
        let outside = temp();
        write(&outside.join("big.bin"), 5000);

        #[cfg(windows)]
        let linked =
            std::os::windows::fs::symlink_dir(&outside, root.join("snapshots/link")).is_ok();
        #[cfg(not(windows))]
        let linked = std::os::unix::fs::symlink(&outside, root.join("snapshots/link")).is_ok();

        let got = usage(&root).expect("usage");
        if linked {
            assert_eq!(
                got.snapshots.bytes, 20,
                "a symlink must contribute nothing, not 5020"
            );
        }
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }
}
