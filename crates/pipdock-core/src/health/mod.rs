//! Code Health: deptry, vulture and ruff run from PipDock's own isolated tools venv.
//!
//! See `docs/CODE-HEALTH-SPEC.md`. Two boundaries are contractual (§1, §7): deptry and vulture are
//! **report-only**, and the sole write path is `ruff --fix` / `ruff format` behind an explicit
//! confirm. PipDock never edits `pyproject.toml` or `requirements.txt` for the user.

pub mod deptry;
pub mod fix;
pub mod project;
pub mod report;
pub mod ruff;
pub mod run;
pub mod vulture;

// `run::run` would read as a stutter and collides with the module name at the call site, so the
// function is re-exported under the name the caller means.
pub use run::run as run_tools;
pub use run::{RunOptions, has_findings, run_steps};

pub use project::{DeclaredSource, declared_source, validate_project};
pub use report::{
    DeptryIssue, FixApplicability, HealthReport, RuffFinding, RuffFindings, SourceLocation,
    ToolProblem, VultureFinding, markdown,
};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::compat::PyVersion;
use crate::engine::ProgressSink;
use crate::errors::{Area, Code, PdError, Result, classify_stderr};
use crate::model::{EnvSource, ExecMode, PinnedSpec, PkgName, StepStatus, Version};

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
/// Kept in the ledger so Dependabot keeps bumping it; excluded here. `pins_for` fails the build's
/// test suite if a name below stops resolving, so a Dependabot rename is a `cargo test` failure
/// rather than a `PD-HLT-001` on a user's machine.
///
/// **P1-1 did not move it in here**, and the reason is the second paragraph above rather than the
/// first. Once the Security tab exists, "a feature they cannot reach" stops being true — but the
/// msgpack risk does not, and `--only-binary=:all:` is load-bearing (see [`install_pins`]). A
/// fourth entry here would mean a CPython with no `msgpack` wheel fails the **whole** sync and
/// takes Code Health down with a feature that has nothing to do with it. So pip-audit gets
/// [`AUDIT_TOOLS`] and a venv of its own, and the blast radius of that wheel stays inside the tab
/// that needs it.
pub const HEALTH_TOOLS: &[&str] = &["deptry", "vulture", "ruff"];

/// The tools installed into the **audit** venv (PRD P1-1).
///
/// Its own set, and its own directory ([`audit_dir`]), for the reason [`HEALTH_TOOLS`] gives: these
/// two venvs fail independently on purpose. Everything else — the manifest, the pin hash, the
/// requirements rendering, `needs_sync`, `sync_tools_venv` — is shared and takes the tool list as
/// an argument, so there is one bootstrap implementation rather than two that drift.
pub const AUDIT_TOOLS: &[&str] = &["pip-audit"];

/// The oldest interpreter that may host the tools venv.
///
/// The binding floor is `deptry`'s and `pip-audit`'s `requires_python` of `>=3.10`, which is
/// already the floor everywhere else in PipDock. **There is deliberately no ceiling** — see
/// [`HEALTH_TOOLS`] for the wheel-tag evidence, and `--only-binary=:all:` for what happens if that
/// ever stops being true.
pub const MIN_TOOLS_PYTHON: (u32, u32) = crate::envs::MIN_PROBE_PYTHON;

/// Did this tool *run and have something to say*, or did it fail?
///
/// **All three exit non-zero on findings, and they disagree about how.** This is the single most
/// likely way to get this module wrong: a plain `!out.ok()` reports every successful run over a
/// real project as `PD-HLT-002`, and a suite that only ever runs the tools over a clean fixture
/// directory agrees with it. Verified by running the pinned versions on 2026-08-12:
///
/// * **deptry 0.25.1** — `0` clean, `1` findings
/// * **vulture 2.16** — `0` none, `1` invalid input, `2` bad arguments, `3` dead code
/// * **ruff 0.16.3** — `0` clean, `1` violations, `2` error (re-checked at 0.16.3 on 2026-08-13;
///   0.16.0 and 0.16.3 produced identical finding sets over two real packages)
///
/// A pin bump has to re-check this, and its real gate is the fixture corpus plus the integration
/// job — not this comment.
#[must_use]
pub fn is_findings_exit(tool: &str, code: Option<i32>) -> bool {
    matches!(
        (tool, code),
        ("deptry" | "ruff", Some(0 | 1))
            | (
                "vulture",
                Some(vulture::EXIT_NO_DEAD_CODE | vulture::EXIT_DEAD_CODE)
            )
    )
}

/// How many steps a sync reports: one venv, one install, one verification per tool.
///
/// Public because the caller builds the [`ProgressSink`], and a sink whose `total` disagrees with
/// what actually runs is a progress bar that stops at four fifths. Takes the tool list rather than
/// reading [`HEALTH_TOOLS`], because the audit venv installs one tool and would otherwise report a
/// five-step sync that runs three.
#[must_use]
pub const fn sync_steps(tools: &[&str]) -> usize {
    2 + tools.len()
}

/// The pin set to install, parsed out of the shipped ledger and sorted by normalized name.
///
/// Takes the tool list rather than assuming [`HEALTH_TOOLS`]: the two venvs render, hash and
/// install different subsets of one ledger, and a function that guessed which would be the single
/// place they could silently swap.
///
/// # Errors
/// `PD-PKG-002` when the ledger is malformed, names a version that is not shaped like one, or does
/// not contain every entry of `tools`. All three are release-time mistakes, which is why they
/// surface as a failing test rather than as a runtime error path.
pub fn pins_for(tools: &[&str]) -> Result<Vec<PinnedSpec>> {
    let ledger = parse_ledger(TOOLS_REQUIREMENTS)?;
    tools
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
/// A **rendering**, not a copy of [`TOOLS_REQUIREMENTS`]. That is what keeps the *other* venv's
/// pins, the ledger's comments and whatever line endings the build machine checked out from
/// reaching either the hash or the user's disk. Always LF-terminated. Rendering per set is also
/// what makes the two hashes independent, so a `pip-audit` bump does not re-sync Code Health.
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

/// The resolved requirements written beside the venv, for the user to read.
const REQUIREMENTS_FILE: &str = "tools-requirements.txt";

/// `ERROR_FILENAME_EXCED_RANGE` — Windows' "the path is too long".
const OS_PATH_TOO_LONG: i32 = 206;

/// `ERROR_DISK_FULL`.
const OS_DISK_FULL: i32 = 112;

/// `ERROR_SHARING_VIOLATION` — the file is open in another process.
const OS_SHARING_VIOLATION: i32 = 32;

/// `<app_data>\tools` — the layout CODE-HEALTH-SPEC §2 draws.
#[must_use]
pub fn tools_dir(app_data: &Path) -> PathBuf {
    app_data.join("tools")
}

/// `<app_data>\audit` — the Security tab's venv (PRD P1-1).
///
/// A sibling of [`tools_dir`] rather than a directory inside it, so that clearing one is not
/// silently clearing the other: `cache::Target` resolves each separately, and CODE-HEALTH-SPEC
/// §2's layout describes `tools\` as Code Health's own.
#[must_use]
pub fn audit_dir(app_data: &Path) -> PathBuf {
    app_data.join("audit")
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
    ///
    /// A struct variant, not a newtype: `#[serde(tag = "state")]` is internal tagging, and serde
    /// cannot represent a tagged newtype wrapping a string. As a newtype this serialized fine in
    /// every unit test and panicked the first time `tools status --json` reached it.
    #[serde(rename_all = "camelCase")]
    ToolMissing {
        /// Which tool is absent.
        tool: String,
    },
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
pub fn needs_sync(tools_dir: &Path, tools: &[&str]) -> Result<SyncNeed> {
    let want = pins_hash(&pins_for(tools)?);

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
    for tool in tools {
        if !tool_exe(tools_dir, tool).is_file() {
            return Ok(SyncNeed::ToolMissing {
                tool: (*tool).to_owned(),
            });
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

/// Create or re-sync the tools venv from the shipped pin ledger.
///
/// Three arguments rather than none, each for a reason that has already bitten:
///
/// * `tools_dir` is passed, not derived from `store::default_app_data()` inside — the same rule
///   `index::metadata`/`refresh` follow. Without it there is no way to run a sync in a test or in
///   CI without clobbering the developer's real `%LOCALAPPDATA%\PipDock\data\tools`.
/// * `base_python` is taken, not discovered. Discovery is [`choose_tools_python`] over
///   `envs::scan()`, which spawns four subprocesses; a sync is not the place to hide a
///   machine-wide sweep, and passing it lets `--python` force a specific interpreter.
/// * `sink` carries the [`CancellationToken`](tokio_util::sync::CancellationToken), so a bootstrap
///   stops through the same path as everything else. Build it with [`sync_steps`].
/// * `tool_set` is [`HEALTH_TOOLS`] or [`AUDIT_TOOLS`], and has to match the directory: the two
///   venvs are separate so that one tool's wheel availability cannot break the other's feature.
///
/// Returns the manifest it wrote, because the caller prints the tool versions and P3's
/// `HealthReport.toolVersions` (CODE-HEALTH-SPEC §5) is exactly this map.
///
/// # Errors
/// `PD-HLT-004` when the venv cannot be created; `PD-NET-011` when the pin set cannot be
/// installed, which is CODE-HEALTH-SPEC §2's single "could not bootstrap" code; `PD-HLT-001` and
/// `PD-HLT-002` when a tool is absent or unrunnable after a successful install; `PD-HLT-003` on a
/// tool's watchdog; `PD-SYS-001`/`PD-SYS-002` for long paths and full disks; `PD-PKG-002` when the
/// shipped ledger is malformed.
pub async fn sync_tools_venv(
    tools_dir: &Path,
    base_python: &Path,
    tool_set: &[&str],
    sink: &ProgressSink,
) -> Result<ToolsManifest> {
    let pins = pins_for(tool_set)?;
    let venv = tools_dir.join(".venv");
    let requirements = tools_dir.join(REQUIREMENTS_FILE);
    let manifest_path = tools_dir.join(MANIFEST_FILE);

    std::fs::create_dir_all(tools_dir).map_err(|e| fs_failure("create the tools directory", &e))?;

    // Deleted **before** anything is touched, which inverts `snapshot::create`'s data-then-metadata
    // ordering — and for the opposite reason. A snapshot writes a sidecar that did not exist; a
    // re-sync replaces a manifest that already describes the *old* pin set. If the sync then dies
    // half-way, a surviving manifest would claim tools that have just been part-replaced. With it
    // gone, a torn sync reads as `NeverSynced` and the next run redoes the whole thing.
    let _ = std::fs::remove_file(&manifest_path);

    // A rendering of the ledger, not a copy: this is where the *other* venv's pins, the comments,
    // and whatever line endings the build machine checked out stop.
    std::fs::write(&requirements, requirements_body(&pins))
        .map_err(|e| fs_failure("write the tools requirements", &e))?;

    // The venv is rebuilt, not repaired. `pip install -r` over an existing venv is a no-op when
    // the pins are already satisfied — so a sync run to replace a deleted `ruff.exe` installed
    // nothing, and then failed its own verification with `PD-HLT-001`. That is the exact state
    // PD-HLT-001's copy tells the user to re-sync out of, and re-syncing could not.
    //
    // Worse, it wedged: the manifest is deleted above, so every later run took the same path and
    // failed identically, with no way out but deleting the folder by hand.
    //
    // A sync means "make this directory match the pins", and the only way to guarantee that from
    // an arbitrary broken state is to start from nothing. The cost lands only where a sync was
    // needed anyway — `needs_sync` short-circuits the common case in ~30 ms.
    remove_venv(&venv)?;
    create_venv(base_python, &venv, &sink.at(0)).await?;
    install_pins(&venv, &requirements, &sink.at(1)).await?;
    let tools = verify_tools(tools_dir, tool_set, sink).await?;

    let python = venv_python(tools_dir);
    let manifest = ToolsManifest {
        pins_hash: pins_hash(&pins),
        pins: pins.iter().map(PinnedSpec::to_requirement).collect(),
        tools,
        python: crate::exec::canonical_interpreter(&python),
        python_version: reported_version(&python)
            .await
            .unwrap_or_else(|| "unknown".to_owned()),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        synced_at: jiff::Timestamp::now().to_string(),
    };

    let json = serde_json::to_string_pretty(&manifest).map_err(|e| {
        PdError::new(
            Code::HltVenvCreateFailed,
            format!("serialize manifest: {e}"),
        )
    })?;
    // Written last: until it exists, the venv is not claimed to be anything.
    if let Err(e) = std::fs::write(&manifest_path, json) {
        let _ = std::fs::remove_file(&manifest_path);
        return Err(fs_failure("write the tools manifest", &e));
    }
    Ok(manifest)
}

/// Clear the way for a fresh venv.
///
/// A locked file here is the common Windows failure — something is running out of the venv — and
/// `PD-PRM-002` says exactly that, rather than leaving it as a generic "could not build".
fn remove_venv(venv: &Path) -> Result<()> {
    match std::fs::remove_dir_all(venv) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) if e.raw_os_error() == Some(OS_SHARING_VIOLATION) => Err(PdError::new(
            Code::PrmFileLocked,
            format!(
                "could not replace {}: a file in it is in use. Close anything running from the \
                 tools environment and retry: {e}",
                venv.display()
            ),
        )),
        Err(e) => Err(fs_failure("replace the tools venv", &e)),
    }
}

/// Step 0 — `python -m venv <tools_dir>\.venv`.
///
/// `--upgrade-deps` is deliberately **not** passed. It would make this step reach PyPI, so an
/// offline machine would fail here with a venv-creation code instead of at the install step with
/// `PD-NET-011`, which is the honest one and the one CODE-HEALTH-SPEC §2 names.
async fn create_venv(base_python: &Path, venv: &Path, sink: &ProgressSink) -> Result<()> {
    sink.started(None, ExecMode::Batch);
    let out = crate::exec::Command::python(base_python)
        .args(["-m", "venv"])
        .arg(venv.display().to_string())
        .cancel(sink.cancel.clone())
        .run_streaming(sink, None, ExecMode::Batch)
        .await
        .map_err(|e| venv_failure(e, base_python));

    match out {
        Ok(out) if out.ok() => {
            sink.finished(None, ExecMode::Batch, StepStatus::Ok);
            Ok(())
        }
        Ok(out) => {
            sink.finished(None, ExecMode::Batch, StepStatus::Failed);
            Err(PdError::new(
                Code::HltVenvCreateFailed,
                format!(
                    "`{} -m venv` exited {}",
                    base_python.display(),
                    out.code.unwrap_or(-1)
                ),
            )
            .with_stderr(&out.stderr))
        }
        Err(e) => {
            sink.finished(None, ExecMode::Batch, StepStatus::Failed);
            Err(e)
        }
    }
}

/// Step 1 — install the pin set with the venv's own pip.
///
/// pip unconditionally, never the configured engine. **This amends CODE-HEALTH-SPEC §2**, which
/// says "using the configured engine": uv is a preference about the *user's* environments, and
/// Health must not be unavailable with `PD-ENG-001` because uv is not on PATH. pip is present
/// wherever Python is.
///
/// `--only-binary=:all:` is load-bearing rather than defensive. Without it a pin with no wheel
/// falls back to an sdist build, which ends at `PD-BLD-001` — telling someone who clicked *Health*
/// to install Visual Studio Build Tools. With it, the same situation is a clean resolution failure
/// this maps to `PD-NET-011`. As of 2026-08-12 it never fires, which is exactly when to add it.
async fn install_pins(venv: &Path, requirements: &Path, sink: &ProgressSink) -> Result<()> {
    let python = venv.join("Scripts").join("python.exe");
    sink.started(None, ExecMode::Batch);

    let out = crate::exec::Command::python(&python)
        .args([
            "-m",
            "pip",
            "install",
            "--only-binary=:all:",
            "--disable-pip-version-check",
            "--no-input",
            "-r",
        ])
        .arg(requirements.display().to_string())
        .cancel(sink.cancel.clone())
        .run_streaming(sink, None, ExecMode::Batch)
        .await
        .map_err(bootstrap_failure);

    match out {
        Ok(out) if out.ok() => {
            sink.finished(None, ExecMode::Batch, StepStatus::Ok);
            Ok(())
        }
        Ok(out) => {
            sink.finished(None, ExecMode::Batch, StepStatus::Failed);
            // Classify first, then fold everything that is not a local-machine problem into
            // PD-NET-011. CODE-HEALTH-SPEC §2 makes that the single "Health could not bootstrap"
            // code, and the distinctions the classifier draws — "no matching distribution" versus
            // "connection timed out" — are not ones the user can act on differently here.
            let classified = classify_stderr(&out.stderr);
            let code = match classified.area() {
                Area::Sys | Area::Prm => classified,
                _ => Code::NetToolsBootstrapFailed,
            };
            Err(PdError::new(
                code,
                format!(
                    "tools venv bootstrap failed: pip exited {}",
                    out.code.unwrap_or(-1)
                ),
            )
            .with_stderr(&out.stderr))
        }
        Err(e) => {
            sink.finished(None, ExecMode::Batch, StepStatus::Failed);
            Err(e)
        }
    }
}

/// Step 2..n — every tool is on disk and answers `--version`.
///
/// Per-tool and sequential, so `ExecMode::Isolated` is the honest phase. The install exiting zero
/// is not proof the console scripts exist: this is the check `PD-HLT-001`'s shipped copy assumes
/// somebody performs.
async fn verify_tools(
    tools_dir: &Path,
    tool_set: &[&str],
    sink: &ProgressSink,
) -> Result<BTreeMap<String, String>> {
    let mut versions = BTreeMap::new();

    for (i, tool) in tool_set.iter().enumerate() {
        let step = sink.at(2 + i);
        let name = PkgName::parse(tool)?;
        step.started(Some(name.clone()), ExecMode::Isolated);

        let exe = tool_exe(tools_dir, tool);
        if !exe.is_file() {
            step.finished(Some(name), ExecMode::Isolated, StepStatus::Failed);
            return Err(PdError::new(
                Code::HltToolMissing,
                format!("{tool} is not in the tools venv after a successful install"),
            ));
        }

        let out = crate::exec::Command::new(&exe)
            .arg("--version")
            .timeout(TOOL_TIMEOUT)
            .cancel(step.cancel.clone())
            .run()
            .await
            .map_err(|e| tool_failure(e, tool));

        match out {
            Ok(out) if out.ok() => {
                versions.insert((*tool).to_owned(), parse_tool_version(&out.stdout, tool));
                step.finished(Some(name), ExecMode::Isolated, StepStatus::Ok);
            }
            Ok(out) => {
                step.finished(Some(name), ExecMode::Isolated, StepStatus::Failed);
                return Err(PdError::new(
                    Code::HltToolFailed,
                    format!("{tool} --version exited {}", out.code.unwrap_or(-1)),
                )
                .with_stderr(&out.stderr));
            }
            Err(e) => {
                step.finished(Some(name), ExecMode::Isolated, StepStatus::Failed);
                return Err(e);
            }
        }
    }
    Ok(versions)
}

/// The version out of `<tool> --version`, which all three print as `<name> <version>`.
///
/// Falls back to the whole trimmed line rather than failing: a tool that answered is working, and
/// refusing to record its version would turn a cosmetic surprise into a failed bootstrap.
fn parse_tool_version(stdout: &str, tool: &str) -> String {
    let line = stdout.lines().next().unwrap_or("").trim();
    line.strip_prefix(tool)
        .map_or(line, str::trim)
        .trim_start_matches('v')
        .to_owned()
}

/// The newest discovered interpreter that can host the tools venv.
///
/// Candidates are supplied rather than discovered here: `envs::scan()` spawns four subprocesses,
/// and a sync is not the place to hide a machine-wide sweep. It also lets `--python` skip discovery
/// entirely, which is what makes CI deterministic.
///
/// # Errors
/// `PD-ENV-001` when nothing at or above [`MIN_TOOLS_PYTHON`] can be run.
pub async fn choose_tools_python(
    candidates: &[crate::envs::Candidate],
) -> Result<(PathBuf, String)> {
    let floor = PyVersion::parse(&format!("{}.{}", MIN_TOOLS_PYTHON.0, MIN_TOOLS_PYTHON.1))?;
    let mut best: Option<(PathBuf, String, PyVersion)> = None;
    let mut seen = std::collections::HashSet::new();

    for candidate in candidates {
        // A project's `.venv` is the user's, and they may delete it tomorrow. Building PipDock's
        // permanent tools environment on top of one would break Health later for a reason nobody
        // could connect to the cause — and `venv_scan` finds whatever is below the *current
        // working directory*, which for the CLI is wherever the user happened to be standing.
        if candidate.source == EnvSource::VenvScan {
            continue;
        }
        let Some(raw) = reported_version(&candidate.path).await else {
            continue;
        };
        let Ok(version) = PyVersion::parse(&raw) else {
            continue;
        };
        // SP-6: dedupe on what the probe reported, not on the discovery path — a shim and its
        // target are one interpreter, and a shim that could not run has already dropped out above.
        if !seen.insert(crate::exec::canonical_interpreter(&candidate.path)) {
            continue;
        }
        if version < floor {
            continue;
        }
        // Strictly greater, so ties keep the earlier candidate and `scan`'s source ordering
        // (registry first) decides. Otherwise the choice would depend on iteration luck.
        if best.as_ref().is_none_or(|(_, _, b)| version > *b) {
            best = Some((candidate.path.clone(), raw, version));
        }
    }

    best.map(|(path, raw, _)| (path, raw)).ok_or_else(|| {
        PdError::new(
            Code::EnvInterpreterMissing,
            format!(
                "no Python {}.{} or newer was discovered; Code Health needs one to build its tools environment",
                MIN_TOOLS_PYTHON.0, MIN_TOOLS_PYTHON.1
            ),
        )
    })
}

/// What an interpreter says its version is, e.g. `"3.14.6"`, or `None` if it could not be run.
///
/// Deliberately **not** `envs::probe`: that writes a temp file and enumerates every installed
/// distribution, which CLAUDE.md flags as the hot path costing 200+ metadata reads. All that is
/// wanted here is three numbers, and this costs one process.
///
/// Returns the string rather than a [`PyVersion`] because both callers need it printable — the
/// manifest records it and the CLI shows it — and `PyVersion` is an ordering type with no
/// `Display`. Callers that need to compare parse it themselves.
async fn reported_version(path: &Path) -> Option<String> {
    let out = crate::exec::Command::python(path)
        .args([
            "-c",
            "import sys;print('.'.join(map(str,sys.version_info[:3])))",
        ])
        .timeout(std::time::Duration::from_secs(15))
        .run()
        .await
        .ok()?;
    if !out.ok() {
        return None;
    }
    let raw = out.stdout.trim();
    (!raw.is_empty()).then(|| raw.to_owned())
}

/// Re-read a failure out of `exec` before it reaches the user.
///
/// `exec::Command::stopped` raises `PD-INT-001` for **both** its watchdog and cancellation, so
/// every timeout anywhere in PipDock currently says "PipDock hit an internal error". That is a
/// pre-existing defect and bigger than this module, but it must not be inherited here: a bootstrap
/// that stalled for ten minutes is a reachability failure, not a bug report.
fn is_timeout(e: &PdError) -> bool {
    e.code == Code::IntUnexpected && e.message.contains("timed out")
}

/// Remap an `exec` failure from the install step.
fn bootstrap_failure(e: PdError) -> PdError {
    if is_timeout(&e) {
        return PdError::new(
            Code::NetToolsBootstrapFailed,
            format!("tools venv bootstrap timed out ({})", e.message),
        );
    }
    e
}

/// Remap an `exec` failure from the venv step.
fn venv_failure(e: PdError, base_python: &Path) -> PdError {
    if is_timeout(&e) {
        return PdError::new(
            Code::HltVenvCreateFailed,
            format!("`{} -m venv` timed out", base_python.display()),
        );
    }
    e
}

/// Remap an `exec` failure from a tool's `--version`.
fn tool_failure(e: PdError, tool: &str) -> PdError {
    if is_timeout(&e) {
        return PdError::new(
            Code::HltTimeout,
            format!("{tool} --version exceeded its {TOOL_TIMEOUT:?} watchdog"),
        );
    }
    e
}

/// Map a filesystem failure onto the code whose user action actually differs.
///
/// Everything that is not a long path or a full disk is "could not build the tools environment",
/// which is what `PD-HLT-004` says.
fn fs_failure(what: &str, e: &std::io::Error) -> PdError {
    let code = match e.raw_os_error() {
        Some(OS_PATH_TOO_LONG) => Code::SysPathTooLong,
        Some(OS_DISK_FULL) => Code::SysDiskFull,
        _ => Code::HltVenvCreateFailed,
    };
    PdError::new(code, format!("could not {what}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{CancellationToken, ProgressEvent};

    /// The gate that turns a Dependabot rename into a `cargo test` failure rather than a
    /// `PD-HLT-001` on a user's machine.
    #[test]
    fn the_ledger_parses_into_two_disjoint_venvs() {
        let health: Vec<_> = pins_for(HEALTH_TOOLS)
            .expect("the shipped ledger must parse")
            .iter()
            .map(|p| p.name.to_string())
            .collect();
        assert_eq!(health, ["deptry", "ruff", "vulture"], "sorted by name");

        // Until P1-1 this asserted that pip-audit reached **no** venv. It has one of its own now,
        // so the invariant that actually matters is that the two sets are *disjoint*: a CPython
        // with no `msgpack` wheel has to be able to fail the Security tab without taking Code
        // Health down with it, and that holds only while nothing is installed into both.
        let audit: Vec<_> = pins_for(AUDIT_TOOLS)
            .expect("the shipped ledger must parse")
            .iter()
            .map(|p| p.name.to_string())
            .collect();
        assert_eq!(audit, ["pip-audit"]);
        assert!(
            !health.iter().any(|n| audit.contains(n)),
            "the two venvs must install disjoint sets — see HEALTH_TOOLS"
        );

        // Both halves stay in one ledger, which is what keeps Dependabot bumping them together
        // and what makes a rename a `cargo test` failure rather than a `PD-HLT-001` on a user's
        // machine.
        let ledger = parse_ledger(TOOLS_REQUIREMENTS).expect("ledger parses");
        for tool in HEALTH_TOOLS.iter().chain(AUDIT_TOOLS) {
            assert!(
                ledger.contains_key(&PkgName::parse(tool).expect("valid name")),
                "{tool} must stay in the ledger so Dependabot keeps bumping it"
            );
        }
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
        let after = "deptry==0.25.1\nvulture==2.16\nruff==0.16.3\n";

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
        let body = requirements_body(&pins_for(HEALTH_TOOLS).expect("ledger parses"));

        assert!(!body.contains('\r'), "always LF on disk");
        assert!(!body.contains('#'), "a rendering, not a copy");
        assert!(
            !body.contains("pip-audit"),
            "a rendering of this set, not the ledger"
        );
        assert_eq!(body.lines().count(), HEALTH_TOOLS.len());

        // And the other direction, which is the half that would have installed nothing at all:
        // rendering the audit set must produce pip-audit and none of Code Health's.
        let audit = requirements_body(&pins_for(AUDIT_TOOLS).expect("ledger parses"));
        assert!(audit.starts_with("pip-audit=="), "{audit:?}");
        assert_eq!(audit.lines().count(), AUDIT_TOOLS.len());
        for tool in HEALTH_TOOLS {
            assert!(
                !audit.contains(tool),
                "{tool} must not reach the audit venv"
            );
        }
        assert_ne!(
            pins_hash(&pins_for(HEALTH_TOOLS).expect("ledger parses")),
            pins_hash(&pins_for(AUDIT_TOOLS).expect("ledger parses")),
            "independent hashes are what stop a pip-audit bump re-syncing Code Health"
        );
        assert!(body.ends_with('\n'));
    }

    // -- the manifest and the re-sync predicate ---------------------------------

    #[test]
    fn a_directory_with_no_manifest_has_never_been_synced() {
        let dir = scratch("never");

        assert_eq!(
            needs_sync(&dir, HEALTH_TOOLS).expect("ledger parses"),
            SyncNeed::NeverSynced
        );
        assert!(
            needs_sync(&dir, HEALTH_TOOLS)
                .expect("ledger parses")
                .is_needed()
        );
    }

    #[test]
    fn an_unparseable_manifest_reads_as_never_synced() {
        // Not its own state: absent, unreadable and corrupt all mean "re-sync" to every caller.
        let dir = scratch("corrupt");
        write_venv(&dir, HEALTH_TOOLS);
        std::fs::write(dir.join(MANIFEST_FILE), "{ not json").expect("write");

        assert_eq!(
            needs_sync(&dir, HEALTH_TOOLS).expect("ledger parses"),
            SyncNeed::NeverSynced
        );
    }

    #[test]
    fn a_matching_manifest_with_every_tool_present_is_fresh() {
        let dir = scratch("fresh");
        write_venv(&dir, HEALTH_TOOLS);
        write_manifest(
            &dir,
            &pins_hash(&pins_for(HEALTH_TOOLS).expect("ledger parses")),
        );

        assert_eq!(
            needs_sync(&dir, HEALTH_TOOLS).expect("ledger parses"),
            SyncNeed::Fresh
        );
        assert!(
            !needs_sync(&dir, HEALTH_TOOLS)
                .expect("ledger parses")
                .is_needed()
        );
    }

    /// The case a hash comparison alone would call `Fresh`, and the one PD-HLT-001 is about.
    #[test]
    fn a_quarantined_tool_is_detected_even_though_the_hash_still_matches() {
        let dir = scratch("quarantined");
        write_venv(&dir, &["deptry", "vulture"]); // ruff.exe never written
        write_manifest(
            &dir,
            &pins_hash(&pins_for(HEALTH_TOOLS).expect("ledger parses")),
        );

        assert_eq!(
            needs_sync(&dir, HEALTH_TOOLS).expect("ledger parses"),
            SyncNeed::ToolMissing {
                tool: "ruff".to_owned()
            }
        );
    }

    /// `#[serde(tag = "state")]` is internal tagging, which cannot represent a newtype variant
    /// wrapping a string. As `ToolMissing(String)` every unit test passed and `tools status --json`
    /// panicked the first time a tool was actually missing — so serialize every variant here.
    #[test]
    fn every_sync_state_survives_serialization() {
        for need in [
            SyncNeed::Fresh,
            SyncNeed::NeverSynced,
            SyncNeed::PinsChanged {
                from: "a".to_owned(),
                to: "b".to_owned(),
            },
            SyncNeed::InterpreterGone,
            SyncNeed::ToolMissing {
                tool: "ruff".to_owned(),
            },
        ] {
            let json = serde_json::to_value(&need)
                .unwrap_or_else(|e| panic!("{need:?} must serialize: {e}"));
            assert!(
                json.get("state").is_some(),
                "{need:?} must carry its tag: {json}"
            );
        }
    }

    #[test]
    fn a_stale_hash_reports_both_sides_so_the_user_can_see_what_moved() {
        let dir = scratch("stale");
        write_venv(&dir, HEALTH_TOOLS);
        write_manifest(&dir, &"0".repeat(64));

        let want = pins_hash(&pins_for(HEALTH_TOOLS).expect("ledger parses"));
        assert_eq!(
            needs_sync(&dir, HEALTH_TOOLS).expect("ledger parses"),
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
        write_manifest(
            &dir,
            &pins_hash(&pins_for(HEALTH_TOOLS).expect("ledger parses")),
        );

        assert_eq!(
            needs_sync(&dir, HEALTH_TOOLS).expect("ledger parses"),
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
        // Compared against the ledger rather than a literal. This assertion used to name
        // `0.16.0`, which made a Dependabot bump of a pin Dependabot exists to bump fail a test
        // about serialization — and it failed on `main`, because that PR's CI predates the health
        // module. What is being pinned here is that the tools map survives the round trip, not
        // which version is in it.
        let pinned = pins_for(HEALTH_TOOLS)
            .expect("ledger parses")
            .into_iter()
            .find(|p| p.name.to_string() == "ruff")
            .map(|p| p.version.to_string())
            .expect("the ledger pins ruff");
        assert_eq!(read.tools.get("ruff"), Some(&pinned));
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

    // -- the sync ---------------------------------------------------------------

    // -- the exit-code table ----------------------------------------------------

    #[test]
    fn a_tool_that_found_something_is_not_a_tool_that_failed() {
        // The whole point. Every one of these is a *successful* run over a project with problems,
        // and reading them as failures is how this module ships broken while its tests pass.
        assert!(is_findings_exit("deptry", Some(1)));
        assert!(is_findings_exit("ruff", Some(1)));
        assert!(
            is_findings_exit("vulture", Some(3)),
            "vulture uses 3, not 1"
        );
    }

    #[test]
    fn a_clean_run_is_still_a_run() {
        for tool in HEALTH_TOOLS {
            assert!(is_findings_exit(tool, Some(0)), "{tool} exit 0");
        }
    }

    #[test]
    fn vultures_other_codes_are_real_failures() {
        // 1 is invalid input and 2 is bad arguments — the two that would otherwise be swallowed as
        // "found dead code" by a table copied from deptry's.
        assert!(!is_findings_exit("vulture", Some(1)));
        assert!(!is_findings_exit("vulture", Some(2)));
    }

    #[test]
    fn a_crash_or_a_signal_is_never_findings() {
        for tool in HEALTH_TOOLS {
            // `None` is the watchdog or a signal: `Output.code` is None when the process did not
            // exit on its own, and reading that as clean would report a killed tool as passing.
            assert!(!is_findings_exit(tool, None), "{tool} killed");
            assert!(!is_findings_exit(tool, Some(101)), "{tool} panicked");
            // 2 is a real failure for all three — ruff's "error", vulture's "bad arguments", and
            // nothing deptry documents.
            assert!(!is_findings_exit(tool, Some(2)), "{tool} exit 2");
        }
    }

    #[test]
    fn the_table_covers_exactly_the_tools_that_run() {
        // A fourth tool added to HEALTH_TOOLS without a row here would fall through to `false` and
        // report every clean run as a failure.
        for tool in HEALTH_TOOLS {
            assert!(
                is_findings_exit(tool, Some(0)),
                "{tool} has no row in the exit table"
            );
        }
        assert!(!is_findings_exit("pip-audit", Some(0)), "not a health tool");
    }

    #[test]
    fn the_step_count_matches_what_the_sync_actually_reports() {
        // A sink whose total disagrees with what runs is a progress bar that stops at four fifths.
        assert_eq!(sync_steps(HEALTH_TOOLS), 2 + HEALTH_TOOLS.len());
        assert_eq!(sync_steps(HEALTH_TOOLS), 5);
        // The audit venv installs one tool, so a shared constant would have promised five
        // steps and run three.
        assert_eq!(sync_steps(AUDIT_TOOLS), 3);
    }

    #[test]
    fn a_tool_version_line_loses_its_tool_name() {
        // All three print `<name> <version>`; ruff has also shipped a `ruff 0.16.0` / `v0.16.0` mix.
        assert_eq!(parse_tool_version("ruff 0.16.0\n", "ruff"), "0.16.0");
        assert_eq!(parse_tool_version("deptry 0.25.1", "deptry"), "0.25.1");
        assert_eq!(parse_tool_version("vulture 2.16\n", "vulture"), "2.16");
        assert_eq!(parse_tool_version("ruff v0.16.0", "ruff"), "0.16.0");
    }

    #[test]
    fn an_unrecognised_version_line_is_kept_whole_rather_than_failing() {
        // A tool that answered is working. Refusing to record its version would turn a cosmetic
        // surprise into a failed bootstrap.
        assert_eq!(
            parse_tool_version("something else\n", "ruff"),
            "something else"
        );
    }

    /// `exec::Command::stopped` raises PD-INT-001 for both its watchdog and cancellation, so the
    /// remaps below are the only thing standing between a stalled download and a bug report.
    #[test]
    fn a_watchdog_never_reaches_the_user_as_an_internal_error() {
        let timed_out = || PdError::new(Code::IntUnexpected, "timed out after 600s: pip.exe");

        assert_eq!(
            bootstrap_failure(timed_out()).code,
            Code::NetToolsBootstrapFailed
        );
        assert_eq!(
            venv_failure(timed_out(), Path::new("py.exe")).code,
            Code::HltVenvCreateFailed
        );
        assert_eq!(tool_failure(timed_out(), "ruff").code, Code::HltTimeout);
    }

    #[test]
    fn a_cancellation_is_not_mistaken_for_a_watchdog() {
        // `stopped` builds both from the same constructor; only the message tells them apart, and
        // a cancel must not be reported as a bootstrap failure.
        let cancelled = PdError::new(Code::IntUnexpected, "cancelled: pip.exe");

        assert_eq!(bootstrap_failure(cancelled).code, Code::IntUnexpected);
    }

    #[test]
    fn the_filesystem_codes_are_the_ones_whose_user_action_differs() {
        use std::io::Error;

        assert_eq!(
            fs_failure("x", &Error::from_raw_os_error(OS_PATH_TOO_LONG)).code,
            Code::SysPathTooLong
        );
        assert_eq!(
            fs_failure("x", &Error::from_raw_os_error(OS_DISK_FULL)).code,
            Code::SysDiskFull
        );
        assert_eq!(
            fs_failure("x", &Error::other("whatever")).code,
            Code::HltVenvCreateFailed
        );
    }

    #[tokio::test]
    async fn a_project_venv_is_never_chosen_to_host_the_tools() {
        // `venv_scan` finds whatever is below the *current working directory*. Building PipDock's
        // permanent tools env on a venv the user may delete tomorrow breaks Health later for a
        // reason nobody could connect to the cause.
        let only_a_project_venv = [crate::envs::Candidate {
            path: PathBuf::from(r"C:\proj\.venv\Scripts\python.exe"),
            source: EnvSource::VenvScan,
        }];

        let err = choose_tools_python(&only_a_project_venv)
            .await
            .expect_err("a project venv is not a candidate");
        assert_eq!(err.code, Code::EnvInterpreterMissing);
    }

    #[tokio::test]
    async fn nothing_discoverable_is_a_catalog_code_not_a_panic() {
        let err = choose_tools_python(&[])
            .await
            .expect_err("no candidates means no interpreter");
        assert_eq!(err.code, Code::EnvInterpreterMissing);
    }

    /// The whole bootstrap, against real PyPI, into a scratch directory.
    ///
    /// `#[ignore]`d because `cargo test --workspace` must not need a network. CI runs it explicitly
    /// (`ci-integration.yml`), and so should anyone touching this module:
    /// `cargo test -p pipdock-core -- --ignored the_bootstrap`.
    #[tokio::test]
    #[ignore = "hits real PyPI; run with --ignored"]
    async fn the_bootstrap_produces_a_venv_that_reports_fresh() {
        let dir = scratch("bootstrap");
        let candidates = crate::envs::scan().await;
        let (python, _) = choose_tools_python(&candidates)
            .await
            .expect("this machine has a Python 3.10+");

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let events = tokio::spawn(async move {
            let mut seen = Vec::new();
            while let Some(e) = rx.recv().await {
                seen.push(e);
            }
            seen
        });
        let sink = ProgressSink::new(tx, sync_steps(HEALTH_TOOLS), Default::default());

        let manifest = sync_tools_venv(&dir, &python, HEALTH_TOOLS, &sink)
            .await
            .expect("bootstrap succeeds");
        drop(sink);

        assert_eq!(manifest.tools.len(), HEALTH_TOOLS.len());
        assert_eq!(
            needs_sync(&dir, HEALTH_TOOLS).expect("ledger parses"),
            SyncNeed::Fresh
        );
        // The exclusion, checked where it actually matters rather than only in the rendering.
        // Against a real bootstrap rather than a constructed manifest: this is the venv Code
        // Health runs out of, and pip-audit belongs to the other one.
        assert!(!manifest.pins.iter().any(|p| p.starts_with("pip-audit")));

        let seen = events.await.expect("collector");
        assert!(
            seen.iter().any(|e| matches!(e, ProgressEvent::Line { .. })),
            "the console drawer needs streamed lines, not one burst at the end"
        );
    }

    /// The bug a green suite hid: a sync that could not repair the one state it exists to repair.
    ///
    /// `pip install -r` over a venv whose pins are already satisfied is a no-op, so deleting
    /// `ruff.exe` and re-syncing installed nothing and then failed its own verification with
    /// `PD-HLT-001` — permanently, because the manifest had already been removed.
    #[tokio::test]
    #[ignore = "hits real PyPI; run with --ignored"]
    async fn a_sync_repairs_a_venv_that_lost_a_tool() {
        let dir = scratch("repair");
        let candidates = crate::envs::scan().await;
        let (python, _) = choose_tools_python(&candidates)
            .await
            .expect("this machine has a Python 3.10+");
        let sink = || {
            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
            ProgressSink::new(tx, sync_steps(HEALTH_TOOLS), CancellationToken::new())
        };

        sync_tools_venv(&dir, &python, HEALTH_TOOLS, &sink())
            .await
            .expect("first bootstrap succeeds");
        std::fs::remove_file(tool_exe(&dir, "ruff")).expect("quarantine ruff");
        assert!(
            needs_sync(&dir, HEALTH_TOOLS)
                .expect("ledger parses")
                .is_needed()
        );

        sync_tools_venv(&dir, &python, HEALTH_TOOLS, &sink())
            .await
            .expect("a re-sync must repair it, not fail on it");

        assert_eq!(
            needs_sync(&dir, HEALTH_TOOLS).expect("ledger parses"),
            SyncNeed::Fresh
        );
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
        let installed = pins_for(HEALTH_TOOLS).expect("ledger parses");
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
