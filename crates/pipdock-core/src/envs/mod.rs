//! Environment discovery and probing. See ARCHITECTURE §4 and DATA-FLOW §2.
//!
//! Discovery sources, in the order the Environments screen shows them (PRD P0-1):
//! PEP 514 registry, the `py` launcher (`py -0p`), `uv python list`, known venv directories,
//! and manual *Browse…*. Spike SP-6 sweeps all of these on a real machine, including the
//! Microsoft Store Python aliasing quirks, before this module is written.

use crate::errors::Result;
use crate::model::PyEnv;

/// The embedded introspection helper, executed as `<env-python> -I probe.py --json`.
///
/// SECURITY §2: `-I` (isolated mode) makes the probe ignore `PYTHONPATH` and user site, so a
/// poisoned environment cannot inject code into it. It is written to a temp file with a random
/// name per invocation and is **never** installed into the environment.
pub const PROBE_PY: &str = include_str!("../probe.py");

/// Lowest Python the probe supports; it uses `importlib.metadata` only (ARCHITECTURE §4).
pub const MIN_PROBE_PYTHON: (u32, u32) = (3, 10);

/// Discover every environment on this machine.
///
/// # Errors
/// Returns a `PD-ENV-*` code when a discovery source fails in a way the user must act on.
pub fn scan() -> Result<Vec<PyEnv>> {
    todo!("M1, after SP-6: PEP 514 registry + py -0p + uv python list + venv scan")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_is_embedded_and_stdlib_only() {
        assert!(
            PROBE_PY.contains("importlib.metadata"),
            "probe must read metadata"
        );
        // ARCHITECTURE §4: no third-party imports. These are the ones a careless edit would add.
        for forbidden in [
            "import requests",
            "import packaging",
            "import setuptools",
            "import pip",
        ] {
            assert!(
                !PROBE_PY.contains(forbidden),
                "probe must stay stdlib-only: {forbidden}"
            );
        }
    }
}
