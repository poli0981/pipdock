//! Process-wide state the commands share.
//!
//! Two things force this to exist rather than opening what each command needs:
//!
//! * `Store` wraps a `rusqlite::Connection`, which is `Send` but **not `Sync`**, so
//!   `tauri::State<Store>` does not compile. It needs the mutex.
//! * `NameIndex` **must** be held across calls. Spike SP-3 measured scanning SQLite at 218 ms per
//!   keystroke against a 50 ms budget; loading it per `index_search` would reintroduce exactly the
//!   failure that spike ruled out.

use std::path::PathBuf;

use pipdock_core::store::Store;

/// Everything a command may need that outlives one call.
pub struct AppState {
    /// `%LOCALAPPDATA%\PipDock`.
    pub app_data: PathBuf,
    /// Settings, pins, recents and the package index.
    pub store: tokio::sync::Mutex<Store>,
}

impl AppState {
    /// Open the store under the app-data directory.
    ///
    /// # Errors
    /// Propagates store failures, which at startup means the data directory is unusable.
    pub fn new() -> pipdock_core::Result<Self> {
        let app_data = pipdock_core::store::default_app_data();
        let store = Store::open(&app_data)?;
        Ok(Self {
            app_data,
            store: tokio::sync::Mutex::new(store),
        })
    }
}
