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

pub mod bindings;
pub mod compat;
pub mod engine;
pub mod envs;
pub mod errors;
pub mod exec;
pub mod flow;
pub mod graph;
pub mod health;
pub mod index;
pub mod model;
pub mod pins;
pub mod plan;
pub mod settings;
pub mod snapshot;
pub mod store;

pub use compat::{Compatibility, PyVersion};
pub use errors::{Code, PdError, Result};
pub use model::{Dist, EngineId, OutdatedDist, PkgName, PyEnv, Version};

/// The application data directory under `%LOCALAPPDATA%` (ARCHITECTURE §6).
pub const APP_DATA_DIR_NAME: &str = "PipDock";

/// Log retention in `%LOCALAPPDATA%\PipDock\logs\` (ARCHITECTURE §6).
pub const LOG_RETENTION_DAYS: u32 = 14;

/// Size of the per-plan engine output ring buffer that backs *Report bug*
/// (`docs/ERROR-CATALOG.md` §4).
pub const LOG_RING_BUFFER_BYTES: usize = 64 * 1024;

/// `docs/ERROR-CATALOG.md` §4.3: GitHub rejects very long URLs, so the prefilled `log-excerpt`
/// is truncated (tail-biased) to this many characters. The full log goes to the clipboard.
pub const BUG_REPORT_EXCERPT_CHARS: usize = 6_000;
