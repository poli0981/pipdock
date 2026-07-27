//! The reverse-dependency graph, built from `probe.py`'s `requires_dist` data.
//!
//! Three features read it (ARCHITECTURE §4): held-back blocker attribution, the uninstall guard,
//! and pin auto-suggest. `docs/TESTING.md` §1 lists its correctness as something that must never
//! regress, including cycles and `pkg[extra]` markers.

use crate::model::{Dist, PkgName};

/// Who depends on what, keyed by the depended-upon package.
#[derive(Debug, Default)]
pub struct ReverseDeps;

impl ReverseDeps {
    /// Build the graph from a full environment listing.
    #[must_use]
    pub fn build(_dists: &[Dist]) -> Self {
        todo!("M1: parse requires_dist, honour extras and environment markers")
    }

    /// Packages that would break if `pkg` were removed.
    ///
    /// This is the uninstall guard's whole job: bare `pip uninstall` performs no dependency check
    /// at all (DATA-FLOW §5).
    #[must_use]
    pub fn dependents_of(&self, _pkg: &PkgName) -> Vec<PkgName> {
        todo!("M1: reverse lookup")
    }
}

/// PRD P1-2: default reverse-dependency count at which a pin is suggested. Configurable.
pub const PIN_SUGGEST_THRESHOLD: usize = 5;
