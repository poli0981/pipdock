//! The write path — `ruff check --fix`, and everything that gates it.
//!
//! **This is the only place PipDock writes outside site-packages and `%LOCALAPPDATA%`.** Every
//! other mutation in the application changes an environment, which a snapshot describes and a
//! rollback restores. A ruff fix rewrites the user's *source tree*, which no snapshot in this
//! application describes — a snapshot is a freeze, and DATA-FLOW §8's rollback is *uninstall the
//! added, install the removed at snapshot versions*.
//!
//! So DATA-FLOW §9.1 and §9.2 do not reach here: both are scoped to a mutating **engine** call,
//! and ruff is a tool run from PipDock's own venv. Taking a snapshot anyway would produce a
//! document with no consumer that could use it — which is invariant 2's own argument for the pip
//! upkeep exemption, a second time. The safety net CODE-HEALTH-SPEC §5 names is the user's own
//! version control, which is why the dirty-tree finding lives *inside* the consent rather than
//! beside it.

use std::path::Path;

use crate::errors::{Code, PdError, Result};
use crate::exec::Command;

/// How long to wait for `git status`. A repository large enough to exceed this is one where the
/// answer would have arrived too late to gate a dialog on.
const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// What `git status --porcelain` reported before the fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DirtyTree {
    /// How many entries it listed. Named in the dialog so the warning is specific.
    pub entries: usize,
}

/// Whether the project has uncommitted work a fix would mix into.
///
/// `None` means **we cannot say**, and all four ways of getting there mean the same thing to the
/// caller: no repository, git not installed, git failed, or the watchdog fired. Distinguishing
/// them would offer the user a choice they cannot act on.
///
/// **`None` is not a warning.** A folder outside version control is a choice, and a dialog that
/// nagged about it every time would train the user to click through the one that matters. The
/// confirm still says, unconditionally, that PipDock cannot undo this — that sentence is true in
/// every case. The *danger* state is reserved for a tree that really is dirty.
///
/// # Why the two flags are not decoration
///
/// `--no-optional-locks`: plain `git status` refreshes and **writes `.git/index`** on a stat-dirty
/// tree. SECURITY models PipDock's writes as site-packages and `%LOCALAPPDATA%`; writing into the
/// user's repository while checking whether it is safe to write would be a poor first act for the
/// write path.
///
/// `--untracked-files=no`: `git checkout .` restores tracked files whether or not a scratch file
/// is lying around, so an untracked file is not something the user needs warning about — and a
/// warning about nothing is how a warning stops being read.
///
/// # `git` is resolved off `PATH`, and the current directory is not searched
///
/// This is the one place in PipDock that runs a PATH-resolved program with a **user-controlled**
/// working directory, so a `git.exe` planted in a cloned repository would be a real attack if
/// Windows' legacy `CreateProcess` search order applied. Verified by planting one and watching
/// which ran: Rust's `Command` does its own resolution and does not include the current directory.
/// Checked rather than reasoned about, and re-check it if the process layer ever changes.
pub async fn dirty(project: &Path) -> Option<DirtyTree> {
    let out = Command::new("git")
        .arg("--no-optional-locks")
        .arg("status")
        .arg("--porcelain")
        .arg("--untracked-files=no")
        .cwd(project)
        .timeout(GIT_TIMEOUT)
        .run()
        .await
        .ok()?;

    // Exit 128 with "not a git repository" is the ordinary no-repo answer, and every other
    // non-zero exit is equally unusable. `ok()` above already absorbed "no git at all".
    if !out.ok() {
        return None;
    }

    let entries = out.stdout.lines().filter(|l| !l.trim().is_empty()).count();
    (entries > 0).then_some(DirtyTree { entries })
}

/// What the user agreed to, checked against what the server found.
///
/// The third named waiver after `SnapshotProof` and `GuardAck`, and for the same reason: the
/// decision is made in one IPC message and consumed in another, so the only thing that stops
/// "somebody forgot to look" is a value the executing call demands.
///
/// **Not a `SnapshotProof`.** Nothing in PipDock can restore a source tree, so this carries the
/// state of the user's own version control instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixConsent {
    /// Confirmed against a tree with nothing uncommitted, or no repository at all.
    Confirmed { files: usize },
    /// `git status --porcelain` was non-empty, the dialog said so, and the user chose to rewrite
    /// files they have not committed. **The one state with no way back.**
    ConfirmedOverDirtyTree { files: usize, dirty_entries: usize },
}

impl FixConsent {
    /// The file count the user was shown.
    #[must_use]
    pub const fn files(&self) -> usize {
        match *self {
            Self::Confirmed { files } | Self::ConfirmedOverDirtyTree { files, .. } => files,
        }
    }
}

/// Build the consent the *server's* observation supports, refusing one the frontend invented.
///
/// The wire carries two scalars — how many files the dialog named, and whether the user
/// acknowledged a dirty tree — and this turns them into the enum. A frontend claiming a clean
/// tree against one the server just found dirty is the case that must not pass, and it is the
/// same shape `ack_ok` checks a `GuardAck` against the guard that produced it.
///
/// # Errors
/// `PD-INT-001` when `acknowledged_dirty` is false and the tree is dirty. Only a broken or
/// out-of-date frontend can produce that, which is what that code means.
pub fn consent_ok(
    files: usize,
    acknowledged_dirty: bool,
    found: Option<DirtyTree>,
) -> Result<FixConsent> {
    match (found, acknowledged_dirty) {
        (Some(tree), true) => Ok(FixConsent::ConfirmedOverDirtyTree {
            files,
            dirty_entries: tree.entries,
        }),
        (Some(tree), false) => Err(PdError::new(
            Code::IntUnexpected,
            format!(
                "the working tree has {} uncommitted change(s) and the confirmation did not \
                 acknowledge them",
                tree.entries
            ),
        )),
        // A tree that was dirty when the dialog opened and is clean now needs no acknowledgement:
        // the thing the user waived no longer exists, and refusing here would be pedantry that
        // costs them the fix.
        (None, _) => Ok(FixConsent::Confirmed { files }),
    }
}

/// Refuse to write anything unless **every** target can be written.
///
/// # Why this is a pre-flight rather than a classification of the failure afterwards
///
/// ruff can fail to write a file and still exit 1, which [`super::is_findings_exit`] accepts as a
/// clean run — so the failure is not merely mis-coded, it can be **silent**, with the fix
/// reporting success over a file it never touched. And a partial rewrite of a source tree is the
/// worst outcome this design can produce, because nothing in PipDock can undo half of one.
///
/// Two checks, because they catch different things: `readonly()` is the `attrib +R` case, and
/// opening for append is the ACL case, which the metadata bit does not show. Append rather than
/// write, so the probe cannot truncate the file it is asking about.
///
/// # Errors
/// `PD-PRM-003`, naming the first file that cannot be written.
pub fn ensure_writable(files: &[String]) -> Result<()> {
    for file in files {
        let path = Path::new(file);
        let refuse = |detail: &str| {
            PdError::new(
                Code::PrmSourceReadOnly,
                format!("{file} cannot be written ({detail}); nothing was changed"),
            )
        };

        match std::fs::metadata(path) {
            Ok(meta) if meta.permissions().readonly() => return Err(refuse("read-only")),
            Ok(_) => {}
            Err(e) => return Err(refuse(&e.to_string())),
        }
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|e| refuse(&e.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frontend_claiming_a_clean_tree_against_a_dirty_one_is_refused() {
        let found = Some(DirtyTree { entries: 3 });
        let err = consent_ok(2, false, found).expect_err("must refuse");
        assert_eq!(err.code, Code::IntUnexpected);
        assert!(
            err.message.contains('3'),
            "the count belongs in the message"
        );
    }

    #[test]
    fn acknowledging_a_dirty_tree_records_what_was_waived() {
        let consent = consent_ok(2, true, Some(DirtyTree { entries: 3 })).expect("accepted");
        assert_eq!(
            consent,
            FixConsent::ConfirmedOverDirtyTree {
                files: 2,
                dirty_entries: 3
            }
        );
        assert_eq!(consent.files(), 2);
    }

    #[test]
    fn a_clean_tree_needs_no_acknowledgement_either_way() {
        // Including the case where the dialog warned and the user committed before confirming:
        // the thing they waived no longer exists, and refusing would cost them the fix for being
        // tidy.
        assert_eq!(
            consent_ok(2, false, None).expect("accepted"),
            FixConsent::Confirmed { files: 2 }
        );
        assert_eq!(
            consent_ok(2, true, None).expect("accepted"),
            FixConsent::Confirmed { files: 2 }
        );
    }

    #[test]
    fn a_read_only_target_refuses_the_whole_fix_before_anything_is_written() {
        let dir = std::env::temp_dir().join(format!("pipdock-fix-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch");
        let ok = dir.join("ok.py");
        let locked = dir.join("locked.py");
        std::fs::write(&ok, "x = 1\n").expect("write");
        std::fs::write(&locked, "y = 2\n").expect("write");

        let mut perms = std::fs::metadata(&locked).expect("meta").permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&locked, perms).expect("chmod");

        let files = vec![ok.display().to_string(), locked.display().to_string()];
        let err = ensure_writable(&files).expect_err("must refuse");
        assert_eq!(err.code, Code::PrmSourceReadOnly);
        assert!(
            err.message.contains("locked.py"),
            "the message must name the file: {}",
            err.message
        );

        // The writable one is untouched: this refuses, it does not fix what it can.
        assert_eq!(std::fs::read_to_string(&ok).expect("read"), "x = 1\n");

        // Cleared so the directory can be removed: Windows refuses to delete a read-only file.
        // The lint's concern is Unix world-writability, which cannot arise for a temp file this
        // test created moments ago and deletes on the next line.
        #[allow(
            clippy::permissions_set_readonly_false,
            reason = "test cleanup: Windows will not delete a read-only file"
        )]
        {
            let mut perms = std::fs::metadata(&locked).expect("meta").permissions();
            perms.set_readonly(false);
            std::fs::set_permissions(&locked, perms).expect("chmod");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_target_is_the_same_refusal() {
        // ruff reported it moments ago, so its absence means something changed underneath — which
        // is exactly when not writing is the right answer.
        let err = ensure_writable(&[r"C:\nope\gone.py".to_owned()]).expect_err("must refuse");
        assert_eq!(err.code, Code::PrmSourceReadOnly);
    }

    #[tokio::test]
    async fn a_folder_with_no_repository_is_not_a_warning() {
        let dir = std::env::temp_dir().join(format!("pipdock-norepo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch");
        assert_eq!(dirty(&dir).await, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Against a real repository, because the three states differ only in what git prints.
    ///
    /// Skipped when git is absent rather than failing: `dirty` answers `None` in that case by
    /// design, so a machine without git would otherwise fail a test about repositories.
    #[tokio::test]
    async fn a_real_repository_reports_only_tracked_changes() {
        let dir = std::env::temp_dir().join(format!("pipdock-repo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");

        let git = |args: &[&str]| {
            let mut c = std::process::Command::new("git");
            c.current_dir(&dir).args(args);
            c.output()
        };
        if git(&["init", "-q"]).is_err() {
            return; // no git on this machine; `dirty` is documented to answer None there
        }
        // Committing needs an identity, and the machine's may not be set in CI.
        let _ = git(&["config", "user.email", "t@example.com"]);
        let _ = git(&["config", "user.name", "t"]);
        std::fs::write(dir.join("a.py"), "x = 1\n").expect("write");
        let _ = git(&["add", "-A"]);
        let _ = git(&["commit", "-qm", "one"]);

        assert_eq!(dirty(&dir).await, None, "a committed tree is clean");

        // Untracked is deliberately invisible: `git checkout .` restores tracked files whether or
        // not a scratch file is lying around, so warning here would be warning about nothing.
        std::fs::write(dir.join("scratch.txt"), "notes\n").expect("write");
        assert_eq!(dirty(&dir).await, None, "an untracked file is not dirt");

        std::fs::write(dir.join("a.py"), "x = 2\n").expect("write");
        assert_eq!(
            dirty(&dir).await,
            Some(DirtyTree { entries: 1 }),
            "a modified tracked file is the case the dialog exists for"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
