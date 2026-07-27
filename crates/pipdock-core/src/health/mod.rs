//! Code Health: deptry, vulture and ruff run from PipDock's own isolated tools venv.
//!
//! See `docs/CODE-HEALTH-SPEC.md`. Two boundaries are contractual (§1, §7): deptry and vulture are
//! **report-only**, and the sole write path is `ruff --fix` / `ruff format` behind an explicit
//! confirm. PipDock never edits `pyproject.toml` or `requirements.txt` for the user.

use crate::errors::Result;

/// CODE-HEALTH-SPEC §4: per-tool watchdog; exceeding it yields a partial report (`PD-HLT-003`).
pub const TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// CODE-HEALTH-SPEC §4: vulture's default confidence floor.
pub const DEFAULT_MIN_CONFIDENCE: u8 = 80;

/// CODE-HEALTH-SPEC §4: directories excluded from every tool, plus user globs from Settings.
pub const DEFAULT_EXCLUDES: &[&str] = &[".venv", "venv", "node_modules", "build", "dist", ".git"];

/// Create or re-sync the tools venv from the shipped `tools-requirements.txt`.
///
/// # Errors
/// Returns `PD-NET-011` when bootstrap cannot reach PyPI; Health stays disabled and every other
/// tab is unaffected (CODE-HEALTH-SPEC §2).
pub fn sync_tools_venv() -> Result<()> {
    todo!(r"M3: create %LOCALAPPDATA%\PipDock\tools\.venv from exact pins")
}
