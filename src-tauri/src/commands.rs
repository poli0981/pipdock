//! Tauri commands.
//!
//! ARCHITECTURE §1.1: each command is a **thin** wrapper over a `pipdock_core` function. If a
//! wrapper starts making decisions, that logic belongs in the core instead, so the CLI inherits
//! it too (G5: GUI and CLI never diverge).
//!
//! The full surface is fixed by ARCHITECTURE §7 (`env_scan`, `pkg_list`, `plan_resolve`,
//! `plan_execute`, …) and mirrored in `ui/src/ipc`. Those wrappers land in M2 with the screens;
//! only the bridge smoke test exists in Phase 0.

/// What the shell can report before any core wiring exists.
#[derive(serde::Serialize)]
pub struct AppInfo {
    /// PipDock's own version, from Cargo.
    pub version: &'static str,
    /// The milestone this build belongs to.
    pub phase: &'static str,
}

/// Smoke-test command: proves the IPC bridge is wired before any real command exists.
#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        phase: "phase-0",
    }
}
