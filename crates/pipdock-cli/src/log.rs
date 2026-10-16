//! The CLI's subscriber: which log lines go where.
//!
//! - **Always** the daily file, `data\logs\pipdock-cli.<date>.log`, at `PIPDOCK_LOG` or INFO.
//! - `--verbose` adds **stderr**, at DEBUG — every engine line as it was printed.
//! - `--log-file <path>` adds a copy of the file's lines there, for a CI job's artefacts.
//!
//! **Never stdout**, which is the CLI's contract (CLI-SPEC §6): a script reading `--json` gets a
//! document and nothing else, verbose or not. `--quiet` adds nothing, and turns nothing off that
//! is not already off — the file is for after the fact, not for the terminal.
//!
//! Both flags were parsed and ignored before 1.4.0.

use std::path::PathBuf;

use pipdock_core::logging::{self, DailyFile};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer as _, fmt};

use crate::GlobalOpts;

/// Set up logging for one invocation. Returns today's log file, for the line an internal error
/// prints (CLI-SPEC §5: exit 10 says where the log is).
pub fn init(opts: &GlobalOpts) -> PathBuf {
    let dir = logging::log_dir(&pipdock_core::store::default_app_data());
    // The first reader `LOG_RETENTION_DAYS` has had since it was written.
    logging::sweep(
        &dir,
        pipdock_core::LOG_RETENTION_DAYS,
        jiff::Timestamp::now(),
    );
    let file = DailyFile::new(&dir, "pipdock-cli");
    let path = file.current_path();

    let level =
        || EnvFilter::try_from_env("PIPDOCK_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let to_file = fmt::layer()
        .with_ansi(false)
        .with_writer(move || file.clone())
        .with_filter(level());
    let to_stderr = opts.verbose.then(|| {
        fmt::layer()
            .with_ansi(false)
            .with_writer(std::io::stderr)
            .with_filter(EnvFilter::new("debug"))
    });
    let to_copy = opts
        .log_file
        .as_ref()
        .and_then(|p| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .map_err(|e| eprintln!("warning: --log-file {}: {e}", p.display()))
                .ok()
        })
        .map(|f| {
            fmt::layer()
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(f))
                .with_filter(level())
        });

    // A second init (a test harness, say) keeps the first; logging is never worth failing over.
    let _ = tracing_subscriber::registry()
        .with(to_file)
        .with(to_stderr)
        .with(to_copy)
        .try_init();
    logging::install_panic_hook();

    tracing::info!(
        target: "pipdock::cli",
        version = env!("CARGO_PKG_VERSION"),
        args = ?std::env::args().skip(1).collect::<Vec<_>>(),
        "start"
    );
    path
}
