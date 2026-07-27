//! `pipdock` — the command-line head over `pipdock-core`.
//!
//! CLI-SPEC §1.1: every command maps 1:1 onto the core functions the GUI uses; **the CLI adds no
//! logic of its own**. CLI-SPEC §1.2: safe by default — off a TTY, or with `--yes`, conflicts
//! default to *skip*, never *force*.
//!
//! English-only in v1 (I18N §1); messages reuse catalog codes so they stay greppable.

// The CLI is the one place that legitimately writes to stdout; the core never does.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Exit codes, exhaustively specified in CLI-SPEC §5.
///
/// Scripts pin against these, so the numbers are a public contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Exit {
    /// All steps succeeded.
    Success = 0,
    /// Completed with per-package failures; see `counts.failed` in `--json`.
    PartialFailure = 1,
    /// Plan aborted — resolution impossible and the skip policy removed everything.
    PlanAborted = 2,
    /// Environment error, including a PEP 668 block (`PD-ENV-*`).
    EnvError = 3,
    /// Engine unavailable or too old (`PD-ENG-*`).
    EngineError = 4,
    /// Snapshot failure — **nothing was executed** (`PD-SNP-001`).
    SnapshotError = 5,
    /// Network or index error (`PD-NET-*`).
    NetworkError = 6,
    /// Internal error (`PD-INT-*`); the log path is printed.
    Internal = 10,
    /// User cancelled; child processes reaped and a partial summary printed.
    Cancelled = 130,
}

impl From<Exit> for ExitCode {
    fn from(e: Exit) -> Self {
        Self::from(e as u8)
    }
}

/// Conflict-handling strategy for `update` and `install`.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum StrategyArg {
    /// Accept the resolver's compatible version. The safe default.
    Compatible,
    /// Force the latest version. Off a TTY this requires an explicit acknowledgement.
    Latest,
}

/// Which engine to drive.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum EngineArg {
    /// `<python> -m pip`
    Pip,
    /// `uv pip --python <python>`
    Uv,
}

/// Options accepted by every command (CLI-SPEC §2).
#[derive(Debug, Args)]
pub struct GlobalOpts {
    /// Interpreter or environment directory. Defaults to the last used env, else an
    /// auto-detected `.venv` in the working directory.
    #[arg(long, global = true, value_name = "PATH")]
    pub env: Option<PathBuf>,

    /// Override the configured engine for this invocation.
    #[arg(long, global = true, value_enum)]
    pub engine: Option<EngineArg>,

    /// Machine-readable output; NDJSON for streaming commands.
    #[arg(long, global = true)]
    pub json: bool,

    /// Assume defaults on all prompts. Conflicts resolve to *skip*, never *force*.
    #[arg(long, short = 'y', global = true)]
    pub yes: bool,

    /// Reduce log output.
    #[arg(long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Increase log output.
    #[arg(long, global = true)]
    pub verbose: bool,

    /// Tee logs to a file.
    #[arg(long, global = true, value_name = "PATH")]
    pub log_file: Option<PathBuf>,

    /// DANGEROUS: skip the pre-batch snapshot. CI images only; prints a warning.
    ///
    /// This is the sole documented escape from DATA-FLOW §9.2, and it exists because a
    /// throwaway CI container has nothing to roll back to.
    #[arg(long, global = true)]
    pub no_snapshot: bool,
}

#[derive(Debug, Parser)]
#[command(
    name = "pipdock",
    version,
    about = "A friendly dock for your Python environments.",
    long_about = "Inspect, install, update and clean up Python packages in bulk.\n\
                  Every mutating operation is previewed and snapshotted first."
)]
struct Cli {
    #[command(flatten)]
    global: GlobalOpts,

    #[command(subcommand)]
    command: Command,
}

/// The command surface, per CLI-SPEC §3.
#[derive(Debug, Subcommand)]
enum Command {
    /// Manage environments.
    #[command(subcommand)]
    Env(EnvCommand),

    /// List installed packages.
    List {
        /// Show only packages with a newer release available.
        #[arg(long)]
        outdated: bool,
    },

    /// Fuzzy-search the local PyPI name index.
    Search {
        /// Query string.
        query: String,
        /// Maximum results.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Show cached PyPI metadata for a package.
    Info {
        /// Package name.
        pkg: String,
    },

    /// Install packages.
    Install {
        /// `name` or `name==version`.
        #[arg(required = true)]
        specs: Vec<String>,
        /// Print the resolution report and exit without changing anything.
        #[arg(long)]
        dry_run: bool,
    },

    /// Update packages.
    Update {
        /// Packages to update; omit and pass `--all` for everything.
        pkgs: Vec<String>,
        /// Update every outdated package (pins still apply).
        #[arg(long, conflicts_with = "pkgs")]
        all: bool,
        /// Conflict strategy.
        #[arg(long, value_enum, default_value = "compatible")]
        strategy: StrategyArg,
        /// Ad-hoc exclusions on top of the pin list.
        #[arg(long, value_delimiter = ',', value_name = "PKG,...")]
        except: Vec<String>,
        /// Print the resolution report and exit without changing anything.
        #[arg(long)]
        dry_run: bool,
    },

    /// Remove packages, guarded by the reverse-dependency check.
    Uninstall {
        /// Packages to remove.
        #[arg(required = true)]
        pkgs: Vec<String>,
        /// Proceed even when the guard reports dependents that would break.
        #[arg(long)]
        force: bool,
    },

    /// Manage the per-environment pin list.
    #[command(subcommand)]
    Pin(PinCommand),

    /// Manage snapshots.
    #[command(subcommand)]
    Snapshot(SnapshotCommand),

    /// Engine check and environment sanity report.
    Doctor,

    /// Run Code Health (deptry / vulture / ruff) over a project folder.
    Health {
        /// Project folder; defaults to the folder remembered for this environment.
        #[arg(long, value_name = "DIR")]
        path: Option<PathBuf>,
        /// Run a single tool instead of all three.
        #[arg(long)]
        tool: Option<String>,
        /// Apply ruff's safe fixes. Prompts before writing.
        #[arg(long)]
        fix: bool,
    },

    /// Upgrade pip inside the selected environment.
    PipUpgrade,

    /// Show or set the configured engine.
    Engine {
        /// Engine to switch to; omit to print the current one.
        engine: Option<EngineArg>,
    },

    /// Index maintenance.
    #[command(subcommand)]
    Index(IndexCommand),

    /// Print the JSON Schema for a core type so scripts can pin against it.
    Schema {
        /// Type name, e.g. `ResolutionReport`.
        type_name: String,
    },

    /// Self-service helpers.
    #[command(subcommand, name = "self")]
    Selfx(SelfCommand),
}

#[derive(Debug, Subcommand)]
enum EnvCommand {
    /// List discovered environments, their sources and PEP 668 flags.
    List,
    /// Set the default environment.
    Use {
        /// Interpreter or environment directory.
        path: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum PinCommand {
    /// Pin a package so bulk updates skip it.
    Add {
        /// Package name.
        pkg: String,
        /// Why it is pinned.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Remove a pin.
    Remove {
        /// Package name.
        pkg: String,
    },
    /// Show the pin list.
    List,
}

#[derive(Debug, Subcommand)]
enum SnapshotCommand {
    /// List snapshots for the selected environment.
    List,
    /// Take a snapshot now.
    Create,
    /// Diff a snapshot against the current environment.
    Diff {
        /// Snapshot id.
        id: String,
    },
    /// Restore a snapshot.
    Rollback {
        /// Snapshot id, or `latest`.
        id: String,
    },
}

#[derive(Debug, Subcommand)]
enum IndexCommand {
    /// Re-pull the PEP 691 name index.
    Refresh,
}

#[derive(Debug, Subcommand)]
enum SelfCommand {
    /// Print a prefilled GitHub issue URL.
    ReportBug,
}

mod run;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match &cli.command {
        Command::Env(EnvCommand::List) => run::env_list(&cli.global).await,
        Command::List { outdated } => run::list(&cli.global, *outdated).await,
        Command::Doctor => run::doctor(&cli.global).await,
        Command::Engine { engine: None } => run::engine_status(&cli.global).await,
        Command::Snapshot(SnapshotCommand::List) => run::snapshot_list(&cli.global).await,
        Command::Snapshot(SnapshotCommand::Create) => run::snapshot_create(&cli.global).await,
        Command::Snapshot(SnapshotCommand::Diff { id }) => {
            run::snapshot_diff(&cli.global, id).await
        }

        // Everything below needs the planner, snapshots or the index, none of which are wired
        // yet. Reporting that honestly beats a stub that appears to work — this is a tool whose
        // whole promise is that it tells you what it is about to do.
        other => {
            eprintln!(
                "error[PD-INT-001]: `{}` is not implemented yet.\n\
                 Working today: env list, list [--outdated], doctor, engine,
                 snapshot list|create|diff.",
                command_name(other)
            );
            Ok(Exit::Internal)
        }
    };

    match result {
        Ok(exit) => exit.into(),
        Err(e) => {
            // ERROR-CATALOG §3: `error[PD-XXX-NNN]: <one-liner>` on stderr, or the JSON envelope.
            if cli.global.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "code": e.code.as_str(),
                        "message": e.message,
                        "stderrTail": e.stderr_tail,
                    })
                );
            } else {
                eprintln!("error[{}]: {}", e.code, e.message);
                if let Some(tail) = &e.stderr_tail {
                    eprintln!("{tail}");
                }
            }
            run::exit_for(e.code).into()
        }
    }
}

fn command_name(cmd: &Command) -> &'static str {
    match cmd {
        Command::Env(_) => "env",
        Command::List { .. } => "list",
        Command::Search { .. } => "search",
        Command::Info { .. } => "info",
        Command::Install { .. } => "install",
        Command::Update { .. } => "update",
        Command::Uninstall { .. } => "uninstall",
        Command::Pin(_) => "pin",
        Command::Snapshot(_) => "snapshot",
        Command::Doctor => "doctor",
        Command::Health { .. } => "health",
        Command::PipUpgrade => "pip-upgrade",
        Command::Engine { .. } => "engine",
        Command::Index(_) => "index",
        Command::Schema { .. } => "schema",
        Command::Selfx(_) => "self",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        // Catches conflicting flags, duplicate shorts and bad defaults at test time rather than
        // at the user's first invocation.
        Cli::command().debug_assert();
    }

    #[test]
    fn exit_codes_match_the_specification() {
        // CLI-SPEC §5 is a public contract; scripts branch on these numbers.
        assert_eq!(Exit::Success as u8, 0);
        assert_eq!(Exit::PartialFailure as u8, 1);
        assert_eq!(Exit::PlanAborted as u8, 2);
        assert_eq!(Exit::EnvError as u8, 3);
        assert_eq!(Exit::EngineError as u8, 4);
        assert_eq!(Exit::SnapshotError as u8, 5);
        assert_eq!(Exit::NetworkError as u8, 6);
        assert_eq!(Exit::Internal as u8, 10);
        assert_eq!(Exit::Cancelled as u8, 130);
    }

    #[test]
    fn update_defaults_to_the_safe_strategy() {
        let cli = Cli::try_parse_from(["pipdock", "update", "--all"]).unwrap();
        match cli.command {
            Command::Update { strategy, all, .. } => {
                assert!(all);
                assert!(matches!(strategy, StrategyArg::Compatible));
            }
            other => panic!("expected update, got {other:?}"),
        }
    }

    #[test]
    fn documented_examples_parse() {
        // The three examples in CLI-SPEC §7, verbatim.
        Cli::try_parse_from([
            "pipdock",
            "update",
            "--all",
            "--env",
            r"C:\bots\scraper\.venv",
            "--yes",
            "--json",
            "--log-file",
            r"C:\logs\pd.json",
        ])
        .expect("nightly maintenance example");

        Cli::try_parse_from([
            "pipdock",
            "update",
            "pandas",
            "numpy",
            "--dry-run",
            "--json",
        ])
        .expect("audit example");

        Cli::try_parse_from(["pipdock", "uninstall", "legacylib", "--json"])
            .expect("refuse-to-break example");
    }

    #[test]
    fn all_and_explicit_packages_are_mutually_exclusive() {
        assert!(Cli::try_parse_from(["pipdock", "update", "--all", "numpy"]).is_err());
    }
}
