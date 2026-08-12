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

use rusqlite::{Connection, OptionalExtension as _};

use crate::errors::{Code, PdError, Result};

/// The database filename under the app data root.
pub const DB_FILE: &str = "index.db";

/// Schema version this build expects. Bump when adding a migration.
pub const SCHEMA_VERSION: i64 = 3;

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

                 -- The project folder Code Health runs its tools in, per environment
                 -- (CODE-HEALTH-SPEC §3). Its own table rather than a `kv` row because it is
                 -- keyed and carries a second column; `kv` is for scalars. Same env_hash the
                 -- pins use, case-folded upstream (SP-6).
                 CREATE TABLE IF NOT EXISTS health_projects (
                     env_hash  TEXT PRIMARY KEY,
                     folder    TEXT NOT NULL,
                     last_run  TEXT
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

/// A remembered environment (ARCHITECTURE §6, PRD P0-1 "recents persisted").
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecentEnv {
    /// Identity, case-folded (SP-6).
    pub env_hash: String,
    /// Where its interpreter lives.
    pub interpreter: String,
    /// RFC 3339, for ordering the recents list.
    pub last_used: String,
    /// Whether this is the starred default.
    pub is_default: bool,
}

impl Store {
    /// Record that an environment was used, and optionally make it the default.
    ///
    /// # Errors
    /// `PD-INT-001` when the write fails.
    pub fn remember_env(
        &self,
        env_hash: &str,
        interpreter: &str,
        now: &str,
        make_default: bool,
    ) -> Result<()> {
        let err = |e: rusqlite::Error| PdError::new(Code::IntUnexpected, format!("env store: {e}"));

        if make_default {
            // Exactly one default, so clear the previous star before setting the new one.
            self.conn
                .execute("UPDATE envs SET is_default = 0", [])
                .map_err(err)?;
        }
        self.conn
            .execute(
                "INSERT INTO envs (env_hash, interpreter, last_used, is_default)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (env_hash) DO UPDATE SET
                    interpreter = excluded.interpreter,
                    last_used = excluded.last_used,
                    is_default = MAX(envs.is_default, excluded.is_default)",
                rusqlite::params![env_hash, interpreter, now, i32::from(make_default)],
            )
            .map_err(err)?;
        Ok(())
    }

    /// Recently used environments, newest first.
    ///
    /// # Errors
    /// `PD-INT-001` when the read fails.
    pub fn recent_envs(&self) -> Result<Vec<RecentEnv>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT env_hash, interpreter, last_used, is_default
                 FROM envs ORDER BY is_default DESC, last_used DESC",
            )
            .map_err(|e| PdError::new(Code::IntUnexpected, format!("env store: {e}")))?;

        let rows = stmt
            .query_map([], |r| {
                Ok(RecentEnv {
                    env_hash: r.get(0)?,
                    interpreter: r.get(1)?,
                    last_used: r.get(2)?,
                    is_default: r.get::<_, i32>(3)? != 0,
                })
            })
            .map_err(|e| PdError::new(Code::IntUnexpected, format!("env store: {e}")))?;

        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    /// The starred default environment, if one has been set.
    ///
    /// # Errors
    /// `PD-INT-001` when the read fails.
    pub fn default_env(&self) -> Result<Option<RecentEnv>> {
        Ok(self.recent_envs()?.into_iter().find(|e| e.is_default))
    }

    /// The project folder Code Health last ran in for this environment (CODE-HEALTH-SPEC §3).
    ///
    /// Per environment, because deptry compares declared dependencies against what is *installed*
    /// — the same folder against a different env is a different question.
    ///
    /// # Errors
    /// `PD-INT-001` when the read fails.
    pub fn health_project(&self, env_hash: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT folder FROM health_projects WHERE env_hash = ?1",
                [env_hash],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| PdError::new(Code::IntUnexpected, format!("read health project: {e}")))
    }

    /// Remember the folder, and when it was last run in.
    ///
    /// `last_run` is written on every save rather than only on success: the question it answers is
    /// "when did we last do this here", and a run that failed still happened.
    ///
    /// # Errors
    /// `PD-INT-001` when the write fails.
    pub fn set_health_project(&self, env_hash: &str, folder: &str, now: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO health_projects (env_hash, folder, last_run) VALUES (?1, ?2, ?3)
                 ON CONFLICT(env_hash) DO UPDATE SET folder = ?2, last_run = ?3",
                [env_hash, folder, now],
            )
            .map(|_| ())
            .map_err(|e| PdError::new(Code::IntUnexpected, format!("save health project: {e}")))
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

        for expected in ["envs", "health_projects", "meta_cache", "names", "pins"] {
            assert!(
                tables.contains(&expected.to_owned()),
                "missing {expected}: {tables:?}"
            );
        }
    }

    #[test]
    fn a_project_folder_is_remembered_per_environment() {
        // Per environment, not per folder: deptry compares declared dependencies against what is
        // *installed*, so the same folder against a different env is a different question.
        let store = Store::in_memory().expect("open");

        assert_eq!(store.health_project("aaa").expect("read"), None);

        store
            .set_health_project("aaa", r"C:\proj\one", "2026-08-12T00:00:00Z")
            .expect("write");
        store
            .set_health_project("bbb", r"C:\proj\two", "2026-08-12T00:00:00Z")
            .expect("write");

        assert_eq!(
            store.health_project("aaa").expect("read").as_deref(),
            Some(r"C:\proj\one")
        );
        assert_eq!(
            store.health_project("bbb").expect("read").as_deref(),
            Some(r"C:\proj\two")
        );
    }

    #[test]
    fn choosing_a_different_folder_replaces_rather_than_duplicates() {
        // env_hash is the primary key, so the upsert is the whole mechanism — a plain INSERT would
        // fail on the second save and the user's new choice would silently not stick.
        let store = Store::in_memory().expect("open");

        store
            .set_health_project("aaa", r"C:\proj\one", "2026-08-12T00:00:00Z")
            .expect("first");
        store
            .set_health_project("aaa", r"C:\proj\two", "2026-08-12T01:00:00Z")
            .expect("second");

        assert_eq!(
            store.health_project("aaa").expect("read").as_deref(),
            Some(r"C:\proj\two")
        );
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
