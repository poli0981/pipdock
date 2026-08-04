//! The Tauri shell.
//!
//! ARCHITECTURE §1.1: this crate contains **commands and events only**. Every command is a thin
//! wrapper over a `pipdock_core` function — if a code path here starts making decisions, it
//! belongs in the core so the CLI gets the same behaviour (G5: GUI and CLI never diverge).

pub mod commands;
pub mod state;

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
    use tauri::Manager as _;

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            // Opening the store is the one startup step that can fail for a reason the user can
            // act on (a data directory that is not writable). Failing here, before a window is
            // shown, beats every command failing later with the same cause.
            app.manage(state::AppState::new()?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::env_scan,
            commands::env_probe,
            commands::pkg_list,
            commands::pkg_outdated,
            commands::pin_list,
            commands::pin_add,
            commands::pin_remove,
            commands::settings_get,
            commands::settings_set,
            commands::legal_consent_get,
            commands::legal_consent_set,
        ])
        .run(tauri::generate_context!())
        .expect("error while running PipDock");
}
