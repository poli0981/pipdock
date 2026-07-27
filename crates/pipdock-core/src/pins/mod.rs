//! Per-environment pins. See PRD P0-7.
//!
//! DATA-FLOW §9.5: pinned packages never appear in a `PlanRequest.upgrades` unless the user
//! explicitly unpinned them in the same session. *Select all* excludes them and says how many
//! were excluded (UI-SPEC §4).

use crate::errors::Result;
use crate::model::PkgName;

/// A pin with the reason the user gave for it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Pin {
    /// The pinned package.
    pub pkg: PkgName,
    /// Free-text justification shown in the Pins screen.
    #[serde(default)]
    pub reason: Option<String>,
}

/// Pins for one environment, keyed by its `env_hash`.
///
/// # Errors
/// Returns a `PD-INT-*` code when the pin store cannot be read.
pub fn list(_env_hash: &str) -> Result<Vec<Pin>> {
    todo!("M1: read the pins table from index.db")
}
