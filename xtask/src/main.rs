//! Repository tooling, run as `cargo run -p xtask -- <task>`.
//!
//! Deliberately not part of the `pipdock` binary: regenerating TypeScript is a contributor's job,
//! not a user's, and CLI-SPEC §3 is a contract about what users can run. Keeping it here also
//! keeps the shipped binary free of codegen it would never execute.

// A task runner's whole output is its console messages, and it exits non-zero on failure rather
// than returning an error to a caller.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("bindings") => match bindings() {
            Ok(changed) => {
                println!(
                    "{} {}",
                    if changed { "wrote" } else { "unchanged" },
                    pipdock_core::bindings::OUTPUT_PATH
                );
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Some("ipc-fixtures") => match ipc_fixtures() {
            Ok(written) => {
                for (name, changed) in written {
                    println!(
                        "{} {}/{name}",
                        if changed { "wrote" } else { "unchanged" },
                        pipdock_core::fixtures::OUTPUT_DIR
                    );
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        other => {
            if let Some(name) = other {
                eprintln!("error: unknown task {name:?}");
            }
            eprintln!("usage: cargo run -p xtask -- <bindings|ipc-fixtures>");
            ExitCode::FAILURE
        }
    }
}

/// Regenerate the L3 mock payloads. Returns which files changed.
fn ipc_fixtures() -> Result<Vec<(&'static str, bool)>, Box<dyn std::error::Error>> {
    let dir = repo_root().join(pipdock_core::fixtures::OUTPUT_DIR);
    std::fs::create_dir_all(&dir)?;

    let mut out = Vec::new();
    for (name, generated) in pipdock_core::fixtures::ipc_fixtures()? {
        let path = dir.join(name);
        // Normalised, so a CRLF checkout does not look like a change on every run.
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current.replace("\r\n", "\n") == generated {
            out.push((name, false));
            continue;
        }
        std::fs::write(&path, generated)?;
        out.push((name, true));
    }
    Ok(out)
}

/// Regenerate the TypeScript bindings. Returns whether the file changed.
fn bindings() -> Result<bool, Box<dyn std::error::Error>> {
    let path = repo_root().join(pipdock_core::bindings::OUTPUT_PATH);
    let generated = pipdock_core::bindings::typescript()?;

    // Compare normalised so a CRLF checkout does not look like a change on every run.
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if current.replace("\r\n", "\n") == generated {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, generated)?;
    Ok(true)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}
