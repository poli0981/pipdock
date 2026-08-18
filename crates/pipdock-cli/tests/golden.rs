//! TESTING L4 — golden output for the CLI surface.
//!
//! Scope note. TESTING §2 asks for "golden-output tests per command against a mocked core". The
//! CLI has no seam to mock at: `run.rs` builds its engine from the global options and drives a
//! real interpreter, and `print_preview` / `print_summary` are private functions that `println!`
//! rather than return. So the preview and summary renderings are covered by L2 against real
//! environments, and what is goldened here is everything that is deterministic without one:
//!
//! * the clap surface — every command's help, which is the flag contract scripts read;
//! * the exhaustive exit-code table (CLI-SPEC §5), which the docs call a public contract;
//! * the `--json` error envelope;
//! * `pipdock schema <T>` for every type in `SCHEMA_TYPES` — the list below, which
//!   `schema_lists_exactly_the_documented_types` holds to the binary's own — which is the JSON
//!   contract CLI-SPEC §6 promises consumers can pin against. No count is written here on
//!   purpose: the one that was ("25 as of Phase 3 P3") was five behind by 1.1.0, and the list is
//!   both authoritative and one screen down.
//!
//! That last one earns its keep beyond regression cover: the M2 bridge changes the wire format to
//! camelCase and makes `Code` serialize as `PD-*` rather than as its Rust variant name. These
//! snapshots are how that change becomes reviewable instead of invisible.
//!
//! When the flow refactor moves rendering out of the CLI (ROADMAP Phase 2 — "the flow never
//! prints"), the preview and summary renderers become pure functions of core types and should
//! gain their own snapshots here.

// Same allowance the core integration tests take: in a test a panic *is* the failure report, and
// the workspace-wide deny exists to keep them out of shipping code.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use assert_cmd::Command;

/// Types `pipdock schema` can emit. Kept in step with `pipdock_core::plan::SCHEMA_TYPES` by
/// `schema_lists_exactly_the_documented_types` below, so this list cannot silently fall behind.
const SCHEMA_TYPES: &[&str] = &[
    "Dist",
    "OutdatedDist",
    "PyEnv",
    "CheckReport",
    "StepResult",
    "PlanRequest",
    "ResolutionReport",
    "ExecutionSummary",
    "Pin",
    "PinSuggestion",
    "ParsedRequirements",
    "CacheUsage",
    "CacheTarget",
    "GuardReport",
    "Diff",
    "SnapshotMeta",
    "EngineInfo",
    "RollbackPlan",
    "RollbackPreview",
    "Hit",
    "PackageMeta",
    "Freshness",
    "RefreshReport",
    "ProgressEvent",
    "FlowStep",
    "Decision",
    "Intent",
    "Code",
    "HealthReport",
    "FixReport",
];

fn pipdock() -> Command {
    let mut cmd = Command::cargo_bin("pipdock").expect("the `pipdock` binary builds");
    // Colour and width leak the terminal into the snapshot; pin both.
    cmd.env("NO_COLOR", "1").env("COLUMNS", "100");
    cmd
}

fn stdout_of(args: &[&str]) -> String {
    let out = pipdock().args(args).output().expect("the process runs");
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

fn stderr_of(args: &[&str]) -> String {
    let out = pipdock().args(args).output().expect("the process runs");
    String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n")
}

fn code_of(args: &[&str]) -> i32 {
    pipdock()
        .args(args)
        .output()
        .expect("the process runs")
        .status
        .code()
        .expect("the process exits rather than being signalled")
}

/// Two things would otherwise make these snapshots record the machine rather than the contract:
/// clap prints the real executable name in every usage line, which is `pipdock.exe` on Windows
/// and `pipdock` everywhere else; and the crate version appears wherever clap renders it, so a
/// release bump would churn the whole file.
fn settings() -> insta::Settings {
    let mut s = insta::Settings::clone_current();
    s.add_filter(r"\bpipdock\.exe\b", "pipdock");
    s.add_filter(r"\b\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?\b", "[VERSION]");
    s
}

// -- the clap surface ---------------------------------------------------------------------------

#[test]
fn top_level_help_is_stable() {
    settings().bind(|| insta::assert_snapshot!("help", stdout_of(&["--help"])));
}

#[test]
fn every_command_documents_itself() {
    // Drives the same list CLI-SPEC §3 enumerates. A command added without a doc comment, or one
    // whose flags change shape, shows up here as a diff rather than in a user's script.
    for cmd in [
        "env",
        "list",
        "search",
        "info",
        "install",
        "update",
        "uninstall",
        "pin",
        "snapshot",
        "doctor",
        "health",
        "pip-upgrade",
        "engine",
        "index",
        "tools",
        "schema",
        "self",
    ] {
        settings().bind(|| {
            insta::assert_snapshot!(format!("help-{cmd}"), stdout_of(&[cmd, "--help"]));
        });
    }
}

#[test]
fn subcommand_help_is_stable() {
    for (parent, child) in [
        ("env", "list"),
        ("env", "use"),
        ("pin", "add"),
        ("pin", "remove"),
        ("pin", "list"),
        ("snapshot", "list"),
        ("snapshot", "create"),
        ("snapshot", "diff"),
        ("snapshot", "rollback"),
        ("index", "refresh"),
        ("tools", "sync"),
        ("tools", "status"),
        ("self", "report-bug"),
    ] {
        settings().bind(|| {
            insta::assert_snapshot!(
                format!("help-{parent}-{child}"),
                stdout_of(&[parent, child, "--help"])
            );
        });
    }
}

// -- exit codes (CLI-SPEC §5) -------------------------------------------------------------------

#[test]
fn the_exit_code_table_holds() {
    // "Scripts pin against these, so the numbers are a public contract" (main.rs). Every row here
    // is reachable without touching a real interpreter; the rest are exercised by L2.
    let cases: &[(&str, &[&str], i32)] = &[
        ("help exits clean", &["--help"], 0),
        ("version exits clean", &["--version"], 0),
        ("a valid schema type", &["schema", "Dist"], 0),
        // clap's own usage errors are 2, distinct from PlanAborted, which is also 2 but only
        // reachable once a plan exists.
        ("an unknown command", &["no-such-command"], 2),
        ("a missing required argument", &["info"], 2),
        ("an unparseable engine", &["--engine", "cargo", "list"], 2),
        // PD-ENV-001 -> EnvError.
        (
            "a missing environment",
            &["list", "--env", "C:/definitely/not/here"],
            3,
        ),
        (
            "a missing environment, doctor",
            &["doctor", "--env", "C:/definitely/not/here"],
            3,
        ),
    ];

    let mut failures = Vec::new();
    for (what, args, want) in cases {
        let got = code_of(args);
        if got != *want {
            failures.push(format!(
                "{what}: `pipdock {}` exited {got}, want {want}",
                args.join(" ")
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn conflicting_verbosity_flags_are_refused() {
    // `quiet` declares conflicts_with = "verbose"; if that attribute is dropped the two silently
    // both apply and the log level becomes whichever clap saw last.
    assert_eq!(code_of(&["--quiet", "--verbose", "list"]), 2);
}

// -- the error envelope -------------------------------------------------------------------------

#[test]
fn the_json_error_envelope_is_stable() {
    // DATA-FLOW §6 and ERROR-CATALOG §3 fix this shape: a wire code, a developer-facing message,
    // and a stderr tail. `code` must be the catalog code, never the Rust variant name.
    let out = stdout_of(&["list", "--env", "C:/definitely/not/here", "--json"]);
    insta::assert_snapshot!("error-envelope-json", out);
}

#[test]
fn the_human_error_line_carries_its_code() {
    // ERROR-CATALOG §3: every user-visible failure carries exactly one catalog code.
    let err = stderr_of(&["list", "--env", "C:/definitely/not/here"]);
    insta::assert_snapshot!("error-envelope-human", err);
    assert!(
        err.contains("PD-ENV-001"),
        "the code must be in the message: {err}"
    );
}

#[test]
fn an_unknown_schema_type_lists_the_known_ones() {
    insta::assert_snapshot!("schema-unknown-type", stderr_of(&["schema", "NoSuchType"]));
}

// -- the JSON contract (CLI-SPEC §6) ------------------------------------------------------------

#[test]
fn schema_lists_exactly_the_documented_types() {
    // Guards the local copy above against `SCHEMA_TYPES` drifting in core. The error path prints
    // the real list, so it is the cheapest way to read it back out of the binary.
    let listed = stderr_of(&["schema", "NoSuchType"]);
    let (_, tail) = listed
        .split_once("known types: ")
        .unwrap_or_else(|| panic!("the unknown-type message no longer lists the types:\n{listed}"));
    let from_binary: Vec<&str> = tail.trim().split(", ").map(str::trim).collect();

    assert_eq!(
        from_binary, SCHEMA_TYPES,
        "core's SCHEMA_TYPES and this test's copy disagree"
    );
}

#[test]
fn every_schema_is_valid_json_and_stable() {
    // These snapshots are the review surface for the M2 wire-format change: camelCase properties
    // and `Code` serializing as PD-* both land here first.
    for ty in SCHEMA_TYPES {
        let out = stdout_of(&["schema", ty]);
        let parsed: serde_json::Value =
            serde_json::from_str(&out).unwrap_or_else(|e| panic!("`schema {ty}` is not JSON: {e}"));
        assert!(
            parsed.get("$schema").is_some() || parsed.get("title").is_some(),
            "`schema {ty}` does not look like a JSON Schema document"
        );
        insta::assert_snapshot!(format!("schema-{ty}"), out);
    }
}
