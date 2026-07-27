//! The local SQLite store, `%LOCALAPPDATA%\PipDock\index.db` (ARCHITECTURE §6).
//!
//! One database holds everything that outlives a session: the PyPI name index, the metadata
//! cache, the recent-environments list and the pin store. They share a file because they share a
//! lifetime — deleting the app data folder is documented as a complete reset (SECURITY §8), and
//! that promise is easier to keep with one file than four.
//!
//! Migrations are forward-only and idempotent, applied on every open. There is no downgrade path:
//! an older PipDock opening a newer database is a situation the updater does not create, and
//! pretending to support it would mean writing migrations nobody can test.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::errors::{Code, PdError, Result};

/// The database filename under the app data root.
pub const DB_FILE: &str = "index.db";

/// Schema version this build expects. Bump when adding a migration.
pub const SCHEMA_VERSION: i64 = 2;

/// An open store.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if needed) the store under `app_data`.
    ///
    /// # Errors
    /// `PD-INT-001` when the database cannot be opened or migrated — the user cannot act on a
    /// corrupt local cache beyond deleting the folder, which the error text says.
    pub fn open(app_data: &Path) -> Result<Self> {
        std::fs::create_dir_all(app_data).map_err(|e| {
            PdError::new(
                Code::IntUnexpected,
                format!("could not create {}: {e}", app_data.display()),
            )
        })?;
        Self::open_at(&app_data.join(DB_FILE))
    }

    /// Open a store at an exact path. Tests use `:memory:`.
    ///
    /// # Errors
    /// `PD-INT-001` on any SQLite failure.
    pub fn open_at(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(|e| {
            PdError::new(
                Code::IntUnexpected,
                format!(
                    "could not open {}: {e} — deleting the PipDock app data folder resets it",
                    path.display()
                ),
            )
        })?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// An in-memory store, for tests.
    ///
    /// # Errors
    /// `PD-INT-001` on any SQLite failure.
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| PdError::new(Code::IntUnexpected, format!("in-memory store: {e}")))?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// The underlying connection, for the modules that own a table.
    #[must_use]
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Read a scalar from the `kv` table.
    ///
    /// # Errors
    /// `PD-INT-001` when the read fails.
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0))
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(PdError::new(
                    Code::IntUnexpected,
                    format!("store read {key}: {other}"),
                )),
            })
    }

    /// Write a scalar to the `kv` table.
    ///
    /// # Errors
    /// `PD-INT-001` when the write fails.
    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO kv (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                rusqlite::params![key, value],
            )
            .map_err(|e| PdError::new(Code::IntUnexpected, format!("store write {key}: {e}")))?;
        Ok(())
    }

    fn migrate(&self) -> Result<()> {
        let sql = |e: rusqlite::Error| {
            PdError::new(Code::IntUnexpected, format!("store migration failed: {e}"))
        };

        self.conn
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA foreign_keys = ON;

                 CREATE TABLE IF NOT EXISTS names (
                     name        TEXT NOT NULL,
                     normalized  TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_names_normalized ON names (normalized);

                 CREATE TABLE IF NOT EXISTS meta_cache (
                     normalized  TEXT PRIMARY KEY,
                     payload     TEXT NOT NULL,
                     fetched_at  TEXT NOT NULL
                 );

                 CREATE TABLE IF NOT EXISTS envs (
                     env_hash    TEXT PRIMARY KEY,
                     interpreter TEXT NOT NULL,
                     last_used   TEXT NOT NULL,
                     is_default  INTEGER NOT NULL DEFAULT 0
                 );

                 -- Pins are per environment, so the same package can be pinned in one venv and
                 -- free in another. env_hash is case-folded upstream (SP-6), so a shim and the
                 -- interpreter it points at share one pin list rather than silently diverging.
                 CREATE TABLE IF NOT EXISTS pins (
                     env_hash    TEXT NOT NULL,
                     pkg         TEXT NOT NULL,
                     mode        TEXT NOT NULL,
                     version     TEXT,
                     reason      TEXT,
                     PRIMARY KEY (env_hash, pkg)
                 );

                 -- Small scalars that do not deserve a table of their own: when the name index
                 -- was last refreshed, how many projects it holds.
                 CREATE TABLE IF NOT EXISTS kv (
                     key   TEXT PRIMARY KEY,
                     value TEXT NOT NULL
                 );",
            )
            .map_err(sql)?;

        self.conn
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(sql)?;
        Ok(())
    }
}

/// Default app data root, `%LOCALAPPDATA%\PipDock` (ARCHITECTURE §6).
#[must_use]
pub fn default_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(crate::APP_DATA_DIR_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_store_has_every_table() {
        let store = Store::in_memory().expect("open");
        let mut stmt = store
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .expect("query");
        let tables: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .expect("map")
            .filter_map(std::result::Result::ok)
            .collect();

        for expected in ["envs", "meta_cache", "names", "pins"] {
            assert!(
                tables.contains(&expected.to_owned()),
                "missing {expected}: {tables:?}"
            );
        }
    }

    #[test]
    fn migrating_twice_is_harmless() {
        // Migrations run on every open, so they must be idempotent or the second launch fails.
        let dir = std::env::temp_dir().join(format!("pd-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let first = Store::open(&dir).expect("first open");
        drop(first);
        let second = Store::open(&dir).expect("second open");

        let version: i64 = second
            .conn()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .expect("version");
        assert_eq!(version, SCHEMA_VERSION);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
