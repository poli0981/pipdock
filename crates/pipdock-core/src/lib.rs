//! PipDock's domain logic — **this crate is the product**.
//!
//! The Tauri GUI (`src-tauri`) and the clap CLI (`pipdock-cli`) are thin adapters over these
//! functions; neither contains business logic (ARCHITECTURE §1.1). Read `docs/ARCHITECTURE.md`
//! before changing anything here.
//!
//! # Invariants this crate enforces
//!
//! From `docs/DATA-FLOW.md` §9, tested rather than assumed:
//!
//! 1. No mutating engine call without a [`plan::ResolutionReport`] accepted in this session.
//! 2. No mutating engine call without a successful snapshot write.
//! 3. `plan_execute` refuses a report older than [`plan::PLAN_MAX_AGE`], or one whose environment
//!    probe hash has changed.
//! 4. Every failure surfaced to the UI or CLI carries an [`errors::Code`].
//! 5. Pinned packages never enter `PlanRequest::upgrades` unless explicitly unpinned this session.
//!
//! # Status
//!
//! Phase 0. Types and command surfaces are defined; behaviour lands in M1, gated on the spikes in
//! `docs/ROADMAP.md`. In particular [`engine::uv`] is gated on SP-1, which decides whether v1.0
//! ships both engines.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod audit;
pub mod bindings;
pub mod cache;
pub mod compat;
pub mod engine;
pub mod envs;
pub mod errors;
pub mod exec;
pub mod fixtures;
pub mod flow;
pub mod graph;
pub mod health;
pub mod index;
pub mod model;
pub mod pins;
pub mod plan;
pub mod report;
pub mod requirements;
pub mod settings;
pub mod snapshot;
pub mod store;

pub use compat::{Compatibility, PyVersion};
pub use errors::{Code, PdError, Result};
pub use model::{Dist, EngineId, OutdatedDist, PkgName, PyEnv, Version};

/// The application data directory under `%LOCALAPPDATA%` (ARCHITECTURE §6).
///
/// **`PipDock\data`, not `PipDock`, and the nesting is the point.** Tauri's NSIS per-user
/// installer writes the program to `$LOCALAPPDATA\{productName}` — the same folder — so the flat
/// layout put `pipdock-app.exe` and `uninstall.exe` beside `index.db`, `snapshots\` and
/// `tools\`. SECURITY §8 tells users that deleting the app data folder is a complete reset;
/// with the two collided, following that advice also uninstalled the application.
///
/// Found by installing the first bundle and reading the uninstall entry, and changed while it was
/// still free to change: no release has ever been published, so there is no on-disk layout in the
/// world to migrate and **no migration code belongs here**.
pub const APP_DATA_DIR_NAME: &str = "PipDock";

/// The subdirectory holding everything PipDock writes. See [`APP_DATA_DIR_NAME`].
///
/// A separate constant rather than `"PipDock/data"` in one join: a path built from a literal
/// containing a separator is a path that reads differently depending on the platform's idea of
/// one, and every comparison against it inherits that.
pub const APP_DATA_SUBDIR: &str = "data";

/// Log retention in `%LOCALAPPDATA%\PipDock\data\logs\` (ARCHITECTURE §6).
pub const LOG_RETENTION_DAYS: u32 = 14;

/// Size of the per-plan engine output ring buffer that backs *Report bug*
/// (`docs/ERROR-CATALOG.md` §4).
pub const LOG_RING_BUFFER_BYTES: usize = 64 * 1024;

/// `docs/ERROR-CATALOG.md` §4.3: GitHub rejects very long URLs, so the prefilled `log-excerpt`
/// is truncated (tail-biased) to this many characters. The full log goes to the clipboard.
pub const BUG_REPORT_EXCERPT_CHARS: usize = 6_000;
