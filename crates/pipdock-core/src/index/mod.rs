//! PyPI name index and metadata cache. See ARCHITECTURE §5.
//!
//! The PEP 691 JSON Simple Index is ingested into SQLite (`index.db`, table `names`), and fuzzy
//! search runs in Rust over the normalized column. Spike SP-3 measures whether the documented
//! budgets hold on ~600 k project names before this module is written.

use crate::errors::Result;
use crate::model::PkgName;

/// UI-SPEC §4 and PRD §6: keystroke-to-results budget for the local index.
pub const SEARCH_LATENCY_BUDGET: std::time::Duration = std::time::Duration::from_millis(50);

/// ROADMAP SP-3: cold refresh of the full name index must finish inside this.
pub const COLD_REFRESH_BUDGET: std::time::Duration = std::time::Duration::from_secs(60);

/// ARCHITECTURE §5: the name index is refreshed manually or on this cadence.
pub const INDEX_REFRESH_INTERVAL_DAYS: u32 = 7;

/// ARCHITECTURE §5: per-package PyPI JSON metadata is cached this long.
pub const METADATA_TTL_HOURS: u32 = 24;

/// Fuzzy-search the cached name index.
///
/// # Errors
/// Returns `PD-NET-010` when the index has never been populated.
pub fn search(_query: &str, _limit: usize) -> Result<Vec<PkgName>> {
    todo!("M1, after SP-3: nucleo matcher over the normalized names column")
}
