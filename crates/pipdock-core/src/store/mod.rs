//! The local SQLite store, `%LOCALAPPDATA%\PipDock\data\index.db` (ARCHITECTURE §6).
//!
//! One database holds everything that outlives a session: the PyPI name index, the metadata
//! cache, the recent-environments list and the pin store. They share a file because they share a
//! lifetime — deleting the app data folder is documented as a complete reset (SECURITY §8), and
//! that promise is easier to keep with one file than four.
//!
//! Migrations are forward-only and idempotent, and run only when the database's `user_version` is
//! older than this build's. There is no downgrade path, but an older PipDock opening a newer
//! database must not *lower* the version either — it used to set it unconditionally, so the next
//! newer build re-ran every migration.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension as _};

use crate::errors::{Code, PdError, Result};

/// The database filename under the app data root.
pub const DB_FILE: &str = "index.db";

/// Schema version this build expects. Bump when adding a migration.
pub const SCHEMA_VERSION: i64 = 5;

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

        // Per connection, so every open; everything below is per database, and the database
        // remembers it. Every open used to run the whole batch — a `journal_mode` write and eight
        // `CREATE TABLE`s — on a store some paths open twice per metadata lookup (M38).
        self.conn
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(sql)?;
        let current: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(sql)?;
        if current >= SCHEMA_VERSION {
            return Ok(());
        }

        self.conn
            .execute_batch(
                "PRAGMA journal_mode = WAL;

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
                 );

                 -- v4. `Requires-Python` of one release, as PyPI publishes it (crate::release).
                 -- An uploaded release's metadata never changes, so a row is never refreshed;
                 -- NULL is the answer \"declares none\", not \"unknown\", which is never stored.
                 CREATE TABLE IF NOT EXISTS release_meta (
                     normalized       TEXT NOT NULL,
                     version          TEXT NOT NULL,
                     requires_python  TEXT,
                     PRIMARY KEY (normalized, version)
                 );

                 -- v5. Interpreters a person added - Browse... in the GUI, `pipdock env add` in the
                 -- CLI - for discovery to list beside its own (PRD P0-1). Not the `envs` table:
                 -- that is the CLI's recents and its starred default, and adding an interpreter to
                 -- the list is not choosing it. Same env_hash as everywhere, case-folded (SP-6).
                 CREATE TABLE IF NOT EXISTS manual_envs (
                     env_hash     TEXT PRIMARY KEY,
                     interpreter  TEXT NOT NULL,
                     added_at     TEXT NOT NULL
                 );",
            )
            .map_err(sql)?;

        self.conn
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(sql)?;
        Ok(())
    }
}

/// An interpreter a person added with *Browse…* or `pipdock env add`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualEnv {
    /// Identity, case-folded (SP-6).
    pub env_hash: String,
    /// The canonical interpreter path, as stored.
    pub interpreter: String,
    /// RFC 3339, which orders the list.
    pub added_at: String,
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

    /// Put an interpreter a person added on the list (PRD P0-1's manual browse).
    ///
    /// Adding one that is already there keeps its place and its date — the list is in the order
    /// things were added, and adding again is not a new addition — but takes the path as given
    /// this time, which is the absolute one [`crate::envs::validate_interpreter`] returned, links
    /// left alone.
    ///
    /// Keyed by `env_hash`, which follows links, so a `--symlinks` venv and its base interpreter
    /// are one entry here: added after the base, the venv replaces it. Only an added base and an
    /// added venv of it meet this; the backlog records it with the rest of that identity.
    ///
    /// # Errors
    /// `PD-INT-001` when the write fails.
    pub fn add_manual_env(&self, env_hash: &str, interpreter: &str, now: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO manual_envs (env_hash, interpreter, added_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (env_hash) DO UPDATE SET interpreter = excluded.interpreter",
                rusqlite::params![env_hash, interpreter, now],
            )
            .map_err(|e| PdError::new(Code::IntUnexpected, format!("manual env store: {e}")))?;
        Ok(())
    }

    /// The interpreters a person added, oldest first.
    ///
    /// # Errors
    /// `PD-INT-001` when the read fails.
    pub fn manual_envs(&self) -> Result<Vec<ManualEnv>> {
        let err = |e: rusqlite::Error| {
            PdError::new(Code::IntUnexpected, format!("manual env store: {e}"))
        };
        let mut stmt = self
            .conn
            .prepare(
                "SELECT env_hash, interpreter, added_at FROM manual_envs
                 ORDER BY added_at, rowid",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ManualEnv {
                    env_hash: r.get(0)?,
                    interpreter: r.get(1)?,
                    added_at: r.get(2)?,
                })
            })
            .map_err(err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)
    }

    /// Take an interpreter off the list. Only the entry goes: nothing on disk is touched, and the
    /// pins, snapshots and Health folder keyed to the same `env_hash` stay, ready for the day it
    /// is added — or discovered — again.
    ///
    /// **Matched by identity or by the stored path**, case-insensitively for the path. An
    /// interpreter whose file is gone can no longer be canonicalized into the identity it was added
    /// under — on a network share the two spellings differ (`\\server\share` against
    /// `UNC\server\share`) — and that entry, listed with PD-ENV-001, is exactly the one a person
    /// wants removed. By hash alone, removing it reported success and the next scan brought it back.
    ///
    /// # Errors
    /// `PD-INT-001` when the write fails. `Ok(false)` when it was not on the list.
    pub fn remove_manual_env(&self, env_hash: &str, interpreter: &str) -> Result<bool> {
        let removed = self
            .conn
            .execute(
                "DELETE FROM manual_envs WHERE env_hash = ?1 OR lower(interpreter) = lower(?2)",
                rusqlite::params![env_hash, interpreter],
            )
            .map_err(|e| PdError::new(Code::IntUnexpected, format!("manual env store: {e}")))?;
        Ok(removed > 0)
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

/// Default app data root, `%LOCALAPPDATA%\PipDock\data` (ARCHITECTURE §6).
///
/// Nested under the install directory rather than beside the binaries — see `APP_DATA_DIR_NAME`.
#[must_use]
pub fn default_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(crate::APP_DATA_DIR_NAME)
        .join(crate::APP_DATA_SUBDIR)
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

        for expected in [
            "envs",
            "health_projects",
            "manual_envs",
            "meta_cache",
            "names",
            "pins",
        ] {
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

    #[test]
    fn an_older_build_never_lowers_the_version() {
        let dir = std::env::temp_dir().join(format!("pd-store-newer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).expect("open");
        store
            .conn()
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .expect("a newer build was here");
        drop(store);

        let reopened = Store::open(&dir).expect("this build opens it");
        let version: i64 = reopened
            .conn()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .expect("version");
        assert_eq!(version, SCHEMA_VERSION + 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_v3_store_gains_the_release_cache() {
        let store = Store::in_memory().expect("open");
        store
            .conn()
            .execute_batch("DROP TABLE release_meta; PRAGMA user_version = 3;")
            .expect("rewind to v3");
        store.migrate().expect("migrate");
        store
            .conn()
            .execute(
                "INSERT INTO release_meta (normalized, version, requires_python) VALUES ('a', '1', NULL)",
                [],
            )
            .expect("the v4 table exists");
    }

    #[test]
    fn a_v4_store_gains_the_manual_list_and_keeps_its_data() {
        let store = Store::in_memory().expect("open");
        store
            .remember_env("h1", r"C:\a\python.exe", "2026-11-01T00:00:00Z", true)
            .expect("a CLI default from 1.5.0");
        store
            .conn()
            .execute_batch("DROP TABLE manual_envs; PRAGMA user_version = 4;")
            .expect("rewind to v4");
        store.migrate().expect("migrate");
        assert!(store.manual_envs().expect("the v5 table exists").is_empty());
        assert_eq!(
            store.default_env().expect("read").map(|e| e.env_hash),
            Some("h1".to_owned()),
            "the migration does not touch what was there"
        );
    }

    #[test]
    fn the_manual_list_keeps_its_order_and_takes_no_duplicates() {
        let store = Store::in_memory().expect("open");
        store
            .add_manual_env("b", r"D:\b\python.exe", "2026-11-02T00:00:00Z")
            .expect("add");
        store
            .add_manual_env("a", r"D:\a\python.exe", "2026-11-03T00:00:00Z")
            .expect("add");
        // Added again, later and spelled differently: one entry, first place, the new spelling.
        store
            .add_manual_env("b", r"D:\B\python.exe", "2026-11-04T00:00:00Z")
            .expect("add again");

        let listed = store.manual_envs().expect("read");
        assert_eq!(
            listed
                .iter()
                .map(|m| (
                    m.env_hash.as_str(),
                    m.interpreter.as_str(),
                    m.added_at.as_str()
                ))
                .collect::<Vec<_>>(),
            [
                ("b", r"D:\B\python.exe", "2026-11-02T00:00:00Z"),
                ("a", r"D:\a\python.exe", "2026-11-03T00:00:00Z"),
            ]
        );
    }

    #[test]
    fn removing_from_the_manual_list_touches_nothing_else() {
        let store = Store::in_memory().expect("open");
        store
            .add_manual_env("h", r"D:\p\python.exe", "2026-11-02T00:00:00Z")
            .expect("add");
        store
            .remember_env("h", r"D:\p\python.exe", "2026-11-02T00:00:00Z", true)
            .expect("also the CLI default");
        store
            .set_health_project("h", r"D:\p", "2026-11-02T00:00:00Z")
            .expect("and Health's folder");

        assert!(
            !store
                .remove_manual_env("other", r"D:\elsewhere\python.exe")
                .expect("remove"),
            "neither the identity nor the path matches"
        );
        assert!(
            store.remove_manual_env("h", "unused").expect("remove"),
            "it was there"
        );
        assert!(
            !store.remove_manual_env("h", "unused").expect("again"),
            "and is not now"
        );
        assert!(store.manual_envs().expect("read").is_empty());
        assert!(
            store.default_env().expect("read").is_some(),
            "the CLI default is another list"
        );
        assert_eq!(
            store.health_project("h").expect("read").as_deref(),
            Some(r"D:\p"),
            "keyed data outlives the entry"
        );
    }

    #[test]
    fn a_gone_interpreter_is_removed_by_its_stored_path_when_its_identity_moved() {
        // Added while the file was there, under the identity its canonical path hashed to; once the
        // file is gone the row's identity is the hash of the stored text, which on a network share
        // is spelled differently. The stored path, in any case, still matches.
        let store = Store::in_memory().expect("open");
        store
            .add_manual_env(
                "hash-of-unc-spelling",
                r"\\server\share\proj\Scripts\python.exe",
                "2026-11-02T00:00:00Z",
            )
            .expect("add");
        assert!(
            store
                .remove_manual_env(
                    "hash-of-the-stored-text",
                    r"\\SERVER\share\proj\Scripts\python.exe"
                )
                .expect("remove"),
            "matched by the path"
        );
        assert!(store.manual_envs().expect("read").is_empty());
    }
}
