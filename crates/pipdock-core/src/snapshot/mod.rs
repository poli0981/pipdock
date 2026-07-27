//! Freeze snapshots, diffs and minimal-ops rollback. See DATA-FLOW §8.
//!
//! DATA-FLOW §9.2 is absolute: no mutating engine call happens without a successful snapshot
//! write. A failed write aborts the plan and executes nothing (`PD-SNP-001`).

use crate::errors::Result;

/// A captured environment state: a freeze file plus its metadata sidecar.
#[derive(Debug)]
pub struct Snapshot;

/// Write a snapshot for `env_hash` before a batch runs.
///
/// # Errors
/// Returns `PD-SNP-001` on any write failure. **Callers must abort the plan on error** — this is
/// the one failure mode that is not skip-and-continue.
pub fn create(_env_hash: &str) -> Result<Snapshot> {
    todo!("M1: engine freeze + .meta.json (trigger, engine, package count, app version)")
}

/// Snapshot retention: `%LOCALAPPDATA%\PipDock\snapshots\<envhash>\<iso-ts>.freeze.txt`
/// alongside `.meta.json` (ARCHITECTURE §6).
pub const SNAPSHOT_DIR: &str = "snapshots";
