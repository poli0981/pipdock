//! Code Health: deptry, vulture and ruff run from PipDock's own isolated tools venv.
//!
//! See `docs/CODE-HEALTH-SPEC.md`. Two boundaries are contractual (§1, §7): deptry and vulture are
//! **report-only**, and the sole write path is `ruff --fix` / `ruff format` behind an explicit
//! confirm. PipDock never edits `pyproject.toml` or `requirements.txt` for the user.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::errors::{Code, PdError, Result};
use crate::model::{PinnedSpec, PkgName, Version};

/// CODE-HEALTH-SPEC §4: per-tool watchdog; exceeding it yields a partial report (`PD-HLT-003`).
pub const TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// CODE-HEALTH-SPEC §4: vulture's default confidence floor.
pub const DEFAULT_MIN_CONFIDENCE: u8 = 80;

/// CODE-HEALTH-SPEC §4: directories excluded from every tool, plus user globs from Settings.
pub const DEFAULT_EXCLUDES: &[&str] = &[".venv", "venv", "node_modules", "build", "dist", ".git"];

/// The release-time pin ledger, baked into the binary (CODE-HEALTH-SPEC §2).
///
/// Reached through `CARGO_MANIFEST_DIR` rather than a relative `include_str!` so the path does not
/// silently depend on how deeply this module is nested. The file lives outside the crate, which
/// would break `cargo package` — irrelevant while nothing is published to crates.io, but the
/// reason to keep it a single well-marked constant rather than several.
///
/// Dependabot bumps it in its own `pip` ecosystem at `/tools` (RELEASE-CI §2), so a tool upgrade is
/// never bundled with application dependencies.
pub const TOOLS_REQUIREMENTS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tools/tools-requirements.txt"
));

/// The tools actually installed into the venv, out of the shipped ledger.
///
/// The ledger is the **release-time pin set** and also carries `pip-audit` for the post-1.0
/// Security tab (PRD P1-1). Code Health is three tools (CODE-HEALTH-SPEC §1), and installing a
/// fourth would make every user's first Health click download a feature they cannot reach.
///
/// It is also the one pin that would import a real CPython-ABI risk. Verified 2026-08-12: the
/// three tools here resolve to `deptry-*-cp310-abi3-win_amd64` (stable ABI) and `py3-none-*`
/// wheels, and the only version-tagged member of their closure — `tomli`, required by deptry under
/// `python_full_version < "3.15"` — also publishes a `py3-none-any` fallback, so a CPython with no
/// `cpXXX` wheel degrades to pure Python rather than to an sdist build. `pip-audit`'s closure
/// contains `msgpack`, which publishes **no** universal fallback at all and would therefore hard-
/// fail under `--only-binary=:all:` on a new CPython.
///
/// Kept in the ledger so Dependabot keeps bumping it; excluded here. `pins` fails the build's test
/// suite if a name below stops resolving, so a Dependabot rename is a `cargo test` failure rather
/// than a `PD-HLT-001` on a user's machine.
pub const HEALTH_TOOLS: &[&str] = &["deptry", "vulture", "ruff"];

/// The oldest interpreter that may host the tools venv.
///
/// The binding floor is `deptry`'s and `pip-audit`'s `requires_python` of `>=3.10`, which is
/// already the floor everywhere else in PipDock. **There is deliberately no ceiling** — see
/// [`HEALTH_TOOLS`] for the wheel-tag evidence, and `--only-binary=:all:` for what happens if that
/// ever stops being true.
pub const MIN_TOOLS_PYTHON: (u32, u32) = crate::envs::MIN_PROBE_PYTHON;

/// The pin set to install, parsed out of the shipped ledger and sorted by normalized name.
///
/// # Errors
/// `PD-PKG-002` when the ledger is malformed, names a version that is not shaped like one, or does
/// not contain every entry of [`HEALTH_TOOLS`]. All three are release-time mistakes, which is why
/// they surface as a failing test rather than as a runtime error path.
pub fn pins() -> Result<Vec<PinnedSpec>> {
    let ledger = parse_ledger(TOOLS_REQUIREMENTS)?;
    HEALTH_TOOLS
        .iter()
        .map(|tool| {
            let name = PkgName::parse(tool)?;
            let version = ledger.get(&name).cloned().ok_or_else(|| {
                PdError::new(
                    Code::PkgNotFound,
                    format!(
                        "tools-requirements.txt has no pin for {tool:?}; \
                         Code Health cannot be built without one"
                    ),
                )
            })?;
            Ok(PinnedSpec { name, version })
        })
        .collect::<Result<Vec<_>>>()
        .map(sorted_by_name)
}

/// SHA-256, hex, over the normalized pin set.
///
/// Over the **parsed** pins, never over the ledger's bytes. `include_str!` bakes in the working
/// tree, and `.gitattributes`' `* text=auto` makes that `core.autocrlf`-dependent — so a byte hash
/// would differ between two builds of the same commit, and every user of one of them would re-sync
/// on first run. A bug that passes every test.
///
/// Invariant under line endings, comments, blank lines, ordering, a trailing newline and name
/// casing. Changes exactly when a pin's name or version does.
///
/// Same construction as `envs::env_hash`, so there is one hashing idiom in the codebase.
#[must_use]
pub fn pins_hash(pins: &[PinnedSpec]) -> String {
    use std::fmt::Write as _;

    use sha2::{Digest as _, Sha256};

    let digest = Sha256::digest(requirements_body(pins).as_bytes());
    digest
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            // Infallible: writing to a String cannot fail, and `?` here would need a Result return
            // for a function that has no other failure mode.
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

/// The requirements file written into the tools directory, and the bytes the hash covers.
///
/// A **rendering**, not a copy of [`TOOLS_REQUIREMENTS`]. That is what keeps the excluded `pip-audit`
/// pin, the ledger's comments and whatever line endings the build machine checked out from reaching
/// either the hash or the user's disk. Always LF-terminated.
#[must_use]
pub fn requirements_body(pins: &[PinnedSpec]) -> String {
    pins.iter().fold(String::new(), |mut acc, pin| {
        acc.push_str(&pin.to_requirement());
        acc.push('\n');
        acc
    })
}

/// The manifest filename inside the tools directory.
const MANIFEST_FILE: &str = "manifest.json";

/// `<app_data>\tools` — the layout CODE-HEALTH-SPEC §2 draws.
#[must_use]
pub fn tools_dir(app_data: &Path) -> PathBuf {
    app_data.join("tools")
}

/// `<tools_dir>\.venv\Scripts\python.exe`.
#[must_use]
pub fn venv_python(tools_dir: &Path) -> PathBuf {
    tools_dir.join(".venv").join("Scripts").join("python.exe")
}

/// `<tools_dir>\.venv\Scripts\<tool>.exe` — the console script pip installs for each tool.
#[must_use]
pub fn tool_exe(tools_dir: &Path, tool: &str) -> PathBuf {
    tools_dir
        .join(".venv")
        .join("Scripts")
        .join(format!("{tool}.exe"))
}

/// What was installed into the tools venv, and against which pins.
///
/// **On-disk only, for now.** It deliberately does not derive `JsonSchema` and is not in
/// `SCHEMA_TYPES`: P2 has no IPC surface, and a type that crosses the bridge is a contract that has
/// to be kept. The day P3 or P4 returns it from a Tauri command it inherits `snapshot::Meta`'s
/// discipline — `#[serde(alias)]` on every renamed field, because manifests written by earlier
/// builds will be on disk and a manifest that no longer parses silently re-syncs the venv on every
/// launch. The field names below already serialize camelCase, so that rename should never happen.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsManifest {
    /// SHA-256 of the normalized pin set — see [`pins_hash`].
    pub pins_hash: String,
    /// The pins as installed, `name==version`, sorted by name.
    pub pins: Vec<String>,
    /// The version each tool actually reported from `--version`, keyed by tool name.
    ///
    /// Reported, not assumed: a pin says what was requested, this says what answered. P3's
    /// `HealthReport.toolVersions` (CODE-HEALTH-SPEC §5) is this map.
    pub tools: BTreeMap<String, String>,
    /// `exec::canonical_interpreter` of the venv's python.
    ///
    /// **An identity, not a path to execute.** It is case-folded (SP-6), which is fine to run on
    /// Windows and therefore tempting; the path actually invoked is always re-derived from the
    /// tools directory through [`venv_python`], so that a moved app-data folder still works.
    pub python: String,
    /// The interpreter's version, e.g. `"3.14.6"`.
    pub python_version: String,
    /// The build that wrote it, as `snapshot::Meta` records.
    pub app_version: String,
    /// When it was written, RFC 3339.
    pub synced_at: String,
}

/// Why a re-sync is owed, or that it is not.
///
/// An enum rather than a `bool` because `doctor` and P4's Health screen have to *say* why, and
/// because `ToolMissing` is the state `PD-HLT-001`'s shipped copy is about ("Re-sync the tools
/// environment"). A `bool` would make "antivirus quarantined `ruff.exe`" indistinguishable from
/// "up to date".
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SyncNeed {
    /// The venv matches the shipped pins and every tool is on disk.
    Fresh,
    /// No manifest — never synced, or a previous sync was interrupted before it finished.
    NeverSynced,
    /// The shipped pin set has moved since the venv was built, typically a Dependabot bump.
    #[serde(rename_all = "camelCase")]
    PinsChanged {
        /// The hash the manifest recorded.
        from: String,
        /// The hash this build ships.
        to: String,
    },
    /// The manifest is current but the venv's interpreter is gone.
    InterpreterGone,
    /// The manifest is current but a tool's console script is not on disk.
    ToolMissing(String),
}

impl SyncNeed {
    /// Whether a sync has to run.
    #[must_use]
    pub fn is_needed(&self) -> bool {
        !matches!(self, Self::Fresh)
    }
}

/// Read `manifest.json`, if there is a readable one.
///
/// `None` for absent, unreadable and unparseable alike — all three mean the same thing to every
/// caller, which is "re-sync". Distinguishing them would offer the user a choice they cannot act on.
#[must_use]
pub fn read_manifest(tools_dir: &Path) -> Option<ToolsManifest> {
    let raw = std::fs::read_to_string(tools_dir.join(MANIFEST_FILE)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Whether the tools venv matches the shipped pins **and** its tools are actually on disk.
///
/// Both halves matter. Comparing hashes alone would report `Fresh` for a venv whose `ruff.exe` an
/// antivirus quarantined, which is precisely the case `PD-HLT-001` exists for.
///
/// # Errors
/// `PD-PKG-002` when the shipped ledger is malformed — a build-time mistake, surfaced here because
/// this is the first thing every caller does.
pub fn needs_sync(tools_dir: &Path) -> Result<SyncNeed> {
    let want = pins_hash(&pins()?);

    let Some(manifest) = read_manifest(tools_dir) else {
        return Ok(SyncNeed::NeverSynced);
    };
    if manifest.pins_hash != want {
        return Ok(SyncNeed::PinsChanged {
            from: manifest.pins_hash,
            to: want,
        });
    }
    if !venv_python(tools_dir).is_file() {
        return Ok(SyncNeed::InterpreterGone);
    }
    for tool in HEALTH_TOOLS {
        if !tool_exe(tools_dir, tool).is_file() {
            return Ok(SyncNeed::ToolMissing((*tool).to_owned()));
        }
    }
    Ok(SyncNeed::Fresh)
}

/// Sort by normalized name, so a reordered ledger is not a re-sync.
fn sorted_by_name(mut pins: Vec<PinnedSpec>) -> Vec<PinnedSpec> {
    pins.sort_by(|a, b| a.name.cmp(&b.name));
    pins
}

/// Parse every `name==version` line of a pin ledger.
///
/// A `BTreeMap` rather than a `Vec` so a duplicated entry collapses instead of being installed
/// twice, and so ordering cannot reach the hash.
///
/// **Only `==` is accepted.** CODE-HEALTH-SPEC §2 promises "no floating versions at runtime"; a
/// `>=` line is a release-time mistake and must fail here rather than pin to a moving target on a
/// user's machine.
fn parse_ledger(text: &str) -> Result<BTreeMap<PkgName, Version>> {
    let mut out = BTreeMap::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, version)) = line.split_once("==") else {
            return Err(PdError::new(
                Code::PkgNotFound,
                format!(
                    "tools-requirements.txt line {line:?} is not an exact `name==version` pin; \
                     CODE-HEALTH-SPEC §2 forbids floating versions at runtime"
                ),
            ));
        };
        out.insert(
            PkgName::parse(name.trim())?,
            validated_version(version.trim())?,
        );
    }
    Ok(out)
}

/// Refuse a ledger version that is not shaped like one.
///
/// The same character-class check `pins::validated_hold` applies, and for the same reason: these
/// strings reach argv as `name==version`, so SECURITY §2's "validated before argv" claim has to
/// hold here too. Deliberately not a full PEP 440 parse — the job is to refuse whitespace, quotes,
/// path separators and control characters, not to second-guess an epoch or a local-version segment.
fn validated_version(raw: &str) -> Result<Version> {
    let invalid = || {
        PdError::new(
            Code::PkgNotFound,
            format!("invalid version in tools-requirements.txt: {raw:?}"),
        )
    };
    let bytes = raw.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphanumeric() {
        return Err(invalid());
    }
    if !bytes
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+' | b'!'))
    {
        return Err(invalid());
    }
    Ok(Version(raw.to_owned()))
}

/// Create or re-sync the tools venv from the shipped `tools-requirements.txt`.
///
/// # Errors
/// Returns `PD-NET-011` when bootstrap cannot reach PyPI; Health stays disabled and every other
/// tab is unaffected (CODE-HEALTH-SPEC §2).
pub fn sync_tools_venv() -> Result<()> {
    todo!(r"M3: create %LOCALAPPDATA%\PipDock\tools\.venv from exact pins")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate that turns a Dependabot rename into a `cargo test` failure rather than a
    /// `PD-HLT-001` on a user's machine.
    #[test]
    fn the_ledger_parses_to_exactly_the_three_health_tools() {
        let pins = pins().expect("the shipped ledger must parse");

        let names: Vec<_> = pins.iter().map(|p| p.name.to_string()).collect();
        assert_eq!(names, ["deptry", "ruff", "vulture"], "sorted by name");

        // The ledger carries pip-audit for the post-1.0 Security tab. It must not be installed.
        assert!(
            !names.iter().any(|n| n == "pip-audit"),
            "pip-audit is in the ledger but must not reach the tools venv — see HEALTH_TOOLS"
        );
        assert!(
            parse_ledger(TOOLS_REQUIREMENTS)
                .expect("ledger parses")
                .contains_key(&PkgName::parse("pip-audit").expect("valid name")),
            "pip-audit must stay in the ledger so Dependabot keeps bumping it"
        );
    }

    /// The CRLF trap, made into a gate.
    ///
    /// `include_str!` bakes in the build machine's working tree, so with `core.autocrlf=true` this
    /// constant arrives with CRLF. A hash over bytes would then differ between two builds of the
    /// same commit, and every user of one of them would re-sync on first run.
    #[test]
    fn line_endings_do_not_change_the_hash() {
        let lf = "deptry==0.25.1\nvulture==2.16\nruff==0.16.0\n";
        let crlf = lf.replace('\n', "\r\n");

        assert_eq!(hash_of(lf), hash_of(&crlf));
    }

    #[test]
    fn reordering_the_file_does_not_change_the_hash() {
        let a = "deptry==0.25.1\nvulture==2.16\nruff==0.16.0\n";
        let b = "ruff==0.16.0\ndeptry==0.25.1\nvulture==2.16\n";

        assert_eq!(hash_of(a), hash_of(b));
    }

    #[test]
    fn comments_and_blank_lines_do_not_change_the_hash() {
        let bare = "deptry==0.25.1\nvulture==2.16\nruff==0.16.0\n";
        let noisy = "# a comment\n\ndeptry==0.25.1  # inline\n\nvulture==2.16\nruff==0.16.0\n\n";

        assert_eq!(hash_of(bare), hash_of(noisy));
    }

    #[test]
    fn changing_a_version_does_change_the_hash() {
        let before = "deptry==0.25.1\nvulture==2.16\nruff==0.16.0\n";
        let after = "deptry==0.25.1\nvulture==2.16\nruff==0.16.2\n";

        assert_ne!(hash_of(before), hash_of(after));
    }

    #[test]
    fn a_floating_version_is_refused() {
        let err = parse_ledger("ruff>=0.16.0\n").expect_err("`>=` must not parse");
        assert_eq!(err.code, Code::PkgNotFound);
    }

    #[test]
    fn a_version_that_could_reach_argv_as_something_else_is_refused() {
        for bad in ["0.16.0 --break-system-packages", "../evil", "\"1.0\""] {
            assert!(
                parse_ledger(&format!("ruff=={bad}\n")).is_err(),
                "{bad:?} must not parse"
            );
        }
    }

    #[test]
    fn the_written_requirements_are_lf_and_carry_only_the_health_tools() {
        let body = requirements_body(&pins().expect("ledger parses"));

        assert!(!body.contains('\r'), "always LF on disk");
        assert!(!body.contains('#'), "a rendering, not a copy");
        assert!(!body.contains("pip-audit"));
        assert_eq!(body.lines().count(), HEALTH_TOOLS.len());
        assert!(body.ends_with('\n'));
    }

    // -- the manifest and the re-sync predicate ---------------------------------

    #[test]
    fn a_directory_with_no_manifest_has_never_been_synced() {
        let dir = scratch("never");

        assert_eq!(
            needs_sync(&dir).expect("ledger parses"),
            SyncNeed::NeverSynced
        );
        assert!(needs_sync(&dir).expect("ledger parses").is_needed());
    }

    #[test]
    fn an_unparseable_manifest_reads_as_never_synced() {
        // Not its own state: absent, unreadable and corrupt all mean "re-sync" to every caller.
        let dir = scratch("corrupt");
        write_venv(&dir, HEALTH_TOOLS);
        std::fs::write(dir.join(MANIFEST_FILE), "{ not json").expect("write");

        assert_eq!(
            needs_sync(&dir).expect("ledger parses"),
            SyncNeed::NeverSynced
        );
    }

    #[test]
    fn a_matching_manifest_with_every_tool_present_is_fresh() {
        let dir = scratch("fresh");
        write_venv(&dir, HEALTH_TOOLS);
        write_manifest(&dir, &pins_hash(&pins().expect("ledger parses")));

        assert_eq!(needs_sync(&dir).expect("ledger parses"), SyncNeed::Fresh);
        assert!(!needs_sync(&dir).expect("ledger parses").is_needed());
    }

    /// The case a hash comparison alone would call `Fresh`, and the one PD-HLT-001 is about.
    #[test]
    fn a_quarantined_tool_is_detected_even_though_the_hash_still_matches() {
        let dir = scratch("quarantined");
        write_venv(&dir, &["deptry", "vulture"]); // ruff.exe never written
        write_manifest(&dir, &pins_hash(&pins().expect("ledger parses")));

        assert_eq!(
            needs_sync(&dir).expect("ledger parses"),
            SyncNeed::ToolMissing("ruff".to_owned())
        );
    }

    #[test]
    fn a_stale_hash_reports_both_sides_so_the_user_can_see_what_moved() {
        let dir = scratch("stale");
        write_venv(&dir, HEALTH_TOOLS);
        write_manifest(&dir, &"0".repeat(64));

        let want = pins_hash(&pins().expect("ledger parses"));
        assert_eq!(
            needs_sync(&dir).expect("ledger parses"),
            SyncNeed::PinsChanged {
                from: "0".repeat(64),
                to: want,
            }
        );
    }

    #[test]
    fn a_missing_interpreter_is_its_own_state() {
        let dir = scratch("gone");
        write_venv(&dir, HEALTH_TOOLS);
        std::fs::remove_file(venv_python(&dir)).expect("remove python");
        write_manifest(&dir, &pins_hash(&pins().expect("ledger parses")));

        assert_eq!(
            needs_sync(&dir).expect("ledger parses"),
            SyncNeed::InterpreterGone
        );
    }

    #[test]
    fn a_manifest_round_trips_through_its_on_disk_form() {
        let dir = scratch("roundtrip");
        write_venv(&dir, HEALTH_TOOLS);
        write_manifest(&dir, "abc");

        let read = read_manifest(&dir).expect("written manifest is readable");
        assert_eq!(read.pins_hash, "abc");
        assert_eq!(read.tools.get("ruff").map(String::as_str), Some("0.16.0"));
    }

    /// The camelCase spelling is the shape a future IPC surface would inherit, so pin it now —
    /// a later rename is what would need `#[serde(alias)]`.
    #[test]
    fn the_manifest_serializes_camel_case() {
        let dir = scratch("camel");
        write_venv(&dir, HEALTH_TOOLS);
        write_manifest(&dir, "abc");

        let raw = std::fs::read_to_string(dir.join(MANIFEST_FILE)).expect("read");
        for key in ["pinsHash", "pythonVersion", "appVersion", "syncedAt"] {
            assert!(raw.contains(key), "expected {key} in {raw}");
        }
        assert!(!raw.contains('_'), "no snake_case key may survive: {raw}");
    }

    /// A scratch directory of our own, cleaned on entry. Matches `store`'s idiom rather than
    /// adding a `tempfile` dev-dependency that would have to clear `cargo audit` forever.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pd-health-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    /// Lay out a plausible venv: an interpreter plus a console script per named tool.
    fn write_venv(tools_dir: &Path, tools: &[&str]) {
        let scripts = tools_dir.join(".venv").join("Scripts");
        std::fs::create_dir_all(&scripts).expect("create Scripts");
        std::fs::write(venv_python(tools_dir), "").expect("write python.exe");
        for tool in tools {
            std::fs::write(tool_exe(tools_dir, tool), "").expect("write tool");
        }
    }

    fn write_manifest(tools_dir: &Path, hash: &str) {
        let installed = pins().expect("ledger parses");
        let manifest = ToolsManifest {
            pins_hash: hash.to_owned(),
            pins: installed.iter().map(PinnedSpec::to_requirement).collect(),
            tools: installed
                .iter()
                .map(|p| (p.name.to_string(), p.version.to_string()))
                .collect(),
            python: "c:\\x\\.venv\\scripts\\python.exe".to_owned(),
            python_version: "3.14.6".to_owned(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            synced_at: "2026-08-12T00:00:00Z".to_owned(),
        };
        let json = serde_json::to_string_pretty(&manifest).expect("serialize");
        std::fs::write(tools_dir.join(MANIFEST_FILE), json).expect("write manifest");
    }

    /// Parse a ledger fragment and hash it, the way `pins`/`pins_hash` do for the real one.
    fn hash_of(ledger: &str) -> String {
        let parsed = parse_ledger(ledger).expect("fragment parses");
        let pins = sorted_by_name(
            parsed
                .into_iter()
                .map(|(name, version)| PinnedSpec { name, version })
                .collect(),
        );
        pins_hash(&pins)
    }
}
