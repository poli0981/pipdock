//! The Tauri shell.
//!
//! ARCHITECTURE §1.1: this crate contains **commands and events only**. Every command is a thin
//! wrapper over a `pipdock_core` function — if a code path here starts making decisions, it
//! belongs in the core so the CLI gets the same behaviour (G5: GUI and CLI never diverge).

pub mod commands;

/// Run the application.
///
/// # Panics
/// Panics if Tauri cannot build the app context. That means a malformed `tauri.conf.json`, which
/// is a build-time mistake rather than a runtime condition, so there is nothing to recover to.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(
    clippy::expect_used,
    reason = "no usable app state exists if the context fails to build"
)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![commands::app_info])
        .run(tauri::generate_context!())
        .expect("error while running PipDock");
}
