//! `env add`, `env remove` and `env list` over the list the GUI's *Add interpreter…* shares (1.6.0).
//!
//! The paths that need no Python, which are the ones a refactor is likeliest to break unseen: a
//! file that is not an interpreter is refused before anything runs it and nothing is stored; an
//! added interpreter whose file has since gone is still listed — with `PD-ENV-001`, because that is
//! the row a person needs in order to remove it — and is removed by the path it was added under.
//! The success path of `env add` probes a real interpreter, which `ci-integration.yml` has.
//!
//! Each test has an app-data root of its own: the store is a file, and tests run in parallel.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use pipdock_core::store::Store;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pd-manual-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// `%LOCALAPPDATA%\PipDock\data`, as the binary will compute it from `LOCALAPPDATA`.
fn app_data(local: &Path) -> PathBuf {
    local
        .join(pipdock_core::APP_DATA_DIR_NAME)
        .join(pipdock_core::APP_DATA_SUBDIR)
}

fn pipdock(local: &Path) -> Command {
    let mut cmd = Command::cargo_bin("pipdock").expect("the `pipdock` binary builds");
    cmd.env("NO_COLOR", "1")
        .env("COLUMNS", "100")
        .env("LOCALAPPDATA", local);
    cmd
}

fn manual_rows(local: &Path) -> Vec<serde_json::Value> {
    let out = pipdock(local)
        .args(["env", "list", "--json"])
        .output()
        .expect("runs");
    assert!(out.status.success(), "env list exits 0");
    let all: Vec<serde_json::Value> =
        serde_json::from_slice(&out.stdout).expect("env list --json is one JSON document");
    all.into_iter()
        .filter(|row| row["source"] == "manual")
        .collect()
}

#[test]
fn another_program_is_refused_before_it_runs_and_nothing_is_stored() {
    let local = scratch("refused");
    let other = local.join(if cfg!(windows) {
        "notepad.exe"
    } else {
        "notepad"
    });
    std::fs::write(&other, "").expect("an empty file that is not Python");

    let out = pipdock(&local)
        .args(["env", "add"])
        .arg(&other)
        .output()
        .expect("runs");
    assert_eq!(
        out.status.code(),
        Some(3),
        "PD-ENV-001 is an environment error"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error[PD-ENV-001]"), "{stderr}");
    assert!(stderr.contains("is not a Python interpreter"), "{stderr}");
    let store = Store::open(&app_data(&local)).expect("the store opens");
    assert!(
        store.manual_envs().expect("reads").is_empty(),
        "a refused path is never listed"
    );
}

#[test]
fn an_entry_whose_file_is_gone_is_listed_with_its_error_and_removed_by_its_path() {
    let local = scratch("gone");
    let gone = local
        .join("deleted-venv")
        .join("Scripts")
        .join("python.exe");
    {
        let store = Store::open(&app_data(&local)).expect("the store opens");
        store
            .add_manual_env(
                &pipdock_core::envs::env_hash(&gone),
                &gone.display().to_string(),
                "2026-11-21T00:00:00Z",
            )
            .expect("added, as a venv later deleted would have been");
    }

    let rows = manual_rows(&local);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["interpreter"], gone.display().to_string());
    assert_eq!(rows[0]["error"]["code"], "PD-ENV-001");

    // By the environment directory, as a person would type it: the file cannot be canonicalized
    // any more, so the stored path is what matches.
    let venv = local.join("deleted-venv");
    let out = pipdock(&local)
        .args(["env", "remove"])
        .arg(&venv)
        .output()
        .expect("runs");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("removed from the list"), "{stdout}");
    assert!(manual_rows(&local).is_empty(), "gone from the listing");

    // Removing it again is not an error: the list is right.
    let out = pipdock(&local)
        .args(["env", "remove", "--json"])
        .arg(&gone)
        .output()
        .expect("runs");
    assert!(out.status.success());
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("one JSON document");
    assert_eq!(doc["removed"], false);
}
