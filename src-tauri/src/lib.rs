//! The Tauri shell.
//!
//! ARCHITECTURE §1.1: this crate contains **commands and events only**. Every command is a thin
//! wrapper over a `pipdock_core` function — if a code path here starts making decisions, it
//! belongs in the core so the CLI gets the same behaviour (G5: GUI and CLI never diverge).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod commands;
pub mod state;

/// Run the application.
///
/// # Panics
/// Panics if Tauri cannot build the app context. That means a malformed `tauri.conf.json`, which
/// is a build-time mistake rather than a runtime condition, so there is nothing to recover to.
/// Commands that are **declared** in ARCHITECTURE §7 and `COMMANDS` but not yet implemented, each
/// with the slice that owes it.
///
/// `COMMANDS` is a bare string array the frontend types its wrappers against, and nothing tied it
/// to what is actually registered — so for four stages it listed names that would have failed at
/// runtime, and typechecked the whole time. This list is what makes the gap explicit instead of
/// invisible: a name here is a promise with a date on it, and the test below fails the moment
/// `COMMANDS` and reality disagree in either direction.
pub const NOT_YET: &[(&str, &str)] = &[
    ("env_add_manual", "M3 — Browse… has no surface yet"),
    ("logs_tail", "M3 — needs the logging subsystem"),
];

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(
    clippy::expect_used,
    reason = "no usable app state exists if the context fails to build"
)]
pub fn run() {
    use tauri::Manager as _;

    tauri::Builder::default()
        // **First, before every other plugin.** The single-instance handler has to be registered
        // before anything that could take a lock or open the store, because a second launch runs
        // this far and then hands off — and the reason it must hand off at all is that two windows
        // would be two `AppState`s over one machine: two `Sessions` slots, each certain it owned
        // the mutation in flight, and two `PD-RES-003` guards that cannot see each other.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // Focus the window that already exists. Restoring first matters: a minimized window
            // that is only focused stays minimized, and the user sees their second launch do
            // nothing at all.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
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
            commands::pin_suggestions,
            commands::env_export,
            commands::requirements_read,
            commands::cache_usage,
            commands::cache_clear,
            commands::plan_resolve,
            commands::plan_decide,
            commands::plan_execute,
            commands::plan_cancel,
            commands::uninstall_guard,
            commands::uninstall_execute,
            commands::snapshot_list,
            commands::snapshot_create,
            commands::snapshot_diff,
            commands::snapshot_rollback_preview,
            commands::snapshot_rollback,
            commands::report_bug_url,
            commands::engine_info,
            commands::pip_upgrade,
            commands::health_dirty,
            commands::health_fix,
            commands::health_run,
            commands::audit_run,
            commands::audit_cancel,
            commands::audit_save_report,
            commands::health_save_report,
            commands::index_search,
            commands::index_refresh,
            commands::pkg_metadata,
            commands::settings_get,
            commands::settings_set,
            commands::legal_consent_get,
            commands::legal_consent_set,
        ])
        .run(tauri::generate_context!())
        .expect("error while running PipDock");
}

#[cfg(test)]
mod tests {
    /// The handler list, read out of this file's own source.
    ///
    /// `generate_handler!` is a macro, so there is no runtime list to inspect and no way to ask
    /// Tauri what it registered. Parsing the source is unlovely but it is the only thing that
    /// cannot drift from the macro — a hand-maintained duplicate would be one more list to forget.
    fn registered() -> Vec<String> {
        let src = include_str!("lib.rs");
        let start = match src.find("invoke_handler(tauri::generate_handler![") {
            Some(i) => i,
            None => panic!("the handler list moved; this test parses it out of the source"),
        };
        let end = src[start..].find("])").expect("the handler list is closed") + start;
        src[start..end]
            .split("commands::")
            .skip(1)
            .filter_map(|rest| rest.split(',').next())
            .map(|name| name.trim().to_owned())
            .collect()
    }

    /// The names the frontend types its wrappers against.
    /// The apostrophe that quotes each entry in the TS array.
    const QUOTE: char = 0x27 as char;

    fn declared() -> Vec<String> {
        let src = include_str!("../../ui/src/ipc/index.ts");
        let start = src
            .find("export const COMMANDS = [")
            .expect("COMMANDS exists");
        let end = src[start..].find("] as const").expect("COMMANDS is closed") + start;
        src[start..end]
            .lines()
            .filter_map(|l| l.trim().strip_prefix(QUOTE))
            .filter_map(|l| l.split(QUOTE).next())
            .map(str::to_owned)
            .collect()
    }

    /// The commands ARCHITECTURE §7's table documents.
    ///
    /// §7 opens with *"A command that is not listed here does not exist; adding one means amending
    /// this section in the same commit"*, and the whole 1.1.0 P1 wave broke that rule — five
    /// commands landed that the table never gained. The three tests below police `lib.rs`,
    /// `COMMANDS` and `NOT_YET`; not one of them read the document making the promise, and the
    /// assertion message of the second even claimed it did. Parsed rather than duplicated, for the
    /// reason [`registered`] gives.
    fn documented() -> Vec<String> {
        let src = include_str!("../../docs/ARCHITECTURE.md");
        let start = src
            .find("| Command | Returns | Purpose |")
            .expect("ARCHITECTURE §7's command table exists");
        src[start..]
            .lines()
            .skip(2)
            .take_while(|l| l.starts_with("| `"))
            .filter_map(|l| l.strip_prefix("| `"))
            .filter_map(|l| l.split(0x60 as char).next())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn every_command_is_in_the_architecture_table() {
        // Both directions, because a table row for a command nobody registered is the same lie as
        // a command nobody documented — it just fails later, when someone builds against it.
        let documented = documented();
        let mut expected = registered();
        expected.extend(super::NOT_YET.iter().map(|(n, _)| (*n).to_owned()));

        let undocumented: Vec<&String> = expected
            .iter()
            .filter(|c| !documented.contains(c))
            .collect();
        assert!(
            undocumented.is_empty(),
            "registered or owed but absent from ARCHITECTURE §7: {undocumented:?}"
        );

        let ghosts: Vec<&String> = documented
            .iter()
            .filter(|c| !expected.contains(c))
            .collect();
        assert!(
            ghosts.is_empty(),
            "ARCHITECTURE §7 documents commands that do not exist: {ghosts:?}"
        );
    }

    #[test]
    fn every_declared_command_is_registered_or_owed() {
        // The failure this catches: `COMMANDS` listed 32 names while 19 were registered, so a
        // wrapper for any of the other 13 typechecked and then failed at runtime — in front of a
        // user, on a command that looked implemented.
        let registered = registered();
        let owed: Vec<&str> = super::NOT_YET.iter().map(|(n, _)| *n).collect();

        let ghosts: Vec<&String> = declared()
            .iter()
            .filter(|c| !registered.contains(c) && !owed.contains(&c.as_str()))
            .cloned()
            .collect::<Vec<_>>()
            .leak()
            .iter()
            .collect();
        assert!(
            ghosts.is_empty(),
            "declared in COMMANDS but neither registered nor listed in NOT_YET: {ghosts:?}"
        );
    }

    #[test]
    fn every_registered_command_is_declared() {
        // The other direction: a command Tauri answers that the frontend has no wrapper for is a
        // surface nobody documented, and ARCHITECTURE §7 says a command not in its table does not
        // exist.
        let declared = declared();
        let undeclared: Vec<String> = registered()
            .into_iter()
            .filter(|c| !declared.contains(c))
            .collect();
        assert!(
            undeclared.is_empty(),
            "registered but missing from COMMANDS: {undeclared:?}"
        );
    }

    #[test]
    fn nothing_is_owed_that_already_exists() {
        // A stale `NOT_YET` entry is worse than none: it says a command is missing while the
        // frontend is already calling it.
        let registered = registered();
        let stale: Vec<&str> = super::NOT_YET
            .iter()
            .map(|(n, _)| *n)
            .filter(|n| registered.iter().any(|r| r == n))
            .collect();
        assert!(
            stale.is_empty(),
            "NOT_YET lists commands that exist: {stale:?}"
        );
    }

    #[test]
    fn the_architecture_table_is_well_formed() {
        // [`documented`] strips only the *leading* pipe, so a row missing its trailing one
        // parses fine and renders wrong. `audit_run` shipped that way in 1.2.0 and no gate
        // could see it: the command was registered, declared and documented, and the only
        // thing broken was the markdown. Assert the shape as well as the contents.
        let src = include_str!("../../docs/ARCHITECTURE.md");
        let start = src
            .find("| Command | Returns | Purpose |")
            .expect("ARCHITECTURE §7's command table exists");
        let ragged: Vec<&str> = src[start..]
            .lines()
            .skip(2)
            .take_while(|l| l.starts_with("| `"))
            .filter(|l| !l.trim_end().ends_with('|'))
            .collect();
        assert!(
            ragged.is_empty(),
            "ARCHITECTURE §7 rows missing a trailing pipe: {ragged:?}"
        );
    }
}
