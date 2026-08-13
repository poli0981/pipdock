//! Environment discovery and probing. See ARCHITECTURE §4 and DATA-FLOW §2.
//!
//! Discovery sources, in the order the Environments screen shows them (PRD P0-1): PEP 514
//! registry, the `py` launcher, `uv python list`, known venv directories, and manual *Browse…*.
//!
//! # What spike SP-6 changed here
//!
//! - **`uv python list` returns interpreters that are not installed** (`<download available>`) and
//!   **shims that duplicate a real interpreter** — a Chocolatey shim and the direct install both
//!   resolved to the same `python.exe`. De-duplication cannot work on the discovery path alone.
//! - **The same interpreter reports different path casing** depending on how it was launched, so
//!   [`env_hash`] case-folds. Without that, one environment silently splits its pins and snapshot
//!   history in two (ARCHITECTURE §6).
//! - **`probe.py -I` hides user-site packages** on a non-venv system Python. Owner decision: keep
//!   `-I`, and report [`crate::model::PyEnv::hidden_user_site`] so the UI can say so accurately.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::compat::PyVersion;
use crate::errors::{Code, PdError, Result};
use crate::exec::{Command, canonical_interpreter, write_temp};
use crate::model::{Dist, EnvSource, PkgName, PyEnv, Version};

/// The embedded introspection helper, executed as `<env-python> -I probe.py --json`.
///
/// SECURITY §2: `-I` (isolated mode) makes the probe ignore `PYTHONPATH` and user site, so a
/// poisoned environment cannot inject code into it. It is written to a temp file with a random
/// name per invocation and is **never** installed into the environment.
pub const PROBE_PY: &str = include_str!("../probe.py");

/// Lowest Python the probe supports; it uses `importlib.metadata` only (ARCHITECTURE §4).
pub const MIN_PROBE_PYTHON: (u32, u32) = (3, 10);

/// Everything `probe.py` reports about one environment.
#[derive(Debug, Clone)]
pub struct ProbeResult {
    /// The environment itself.
    pub env: PyEnv,
    /// Installed distributions, sorted by normalized name.
    pub dists: Vec<Dist>,
}

/// Identity of an environment: SHA-256 of the canonicalized interpreter path (ARCHITECTURE §6).
///
/// Case-folded on Windows — see the SP-6 note above. This value keys the pin store, the snapshot
/// directory and the recents list, so a change here orphans a user's data.
#[must_use]
pub fn env_hash(interpreter: &Path) -> String {
    use std::fmt::Write as _;

    use sha2::{Digest as _, Sha256};
    let canonical = canonical_interpreter(interpreter);
    let digest = Sha256::digest(canonical.as_bytes());
    digest
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            // Infallible: writing to a String cannot fail, and `?` here would need a Result return
            // for a function that has no other failure mode.
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

/// Run `probe.py` against an interpreter.
///
/// # Errors
/// `PD-ENV-001` when the interpreter is missing, `PD-ENV-003` when the probe fails or its output
/// cannot be read.
pub async fn probe(interpreter: &Path, source: EnvSource) -> Result<ProbeResult> {
    if !interpreter.is_file() {
        return Err(PdError::new(
            Code::EnvInterpreterMissing,
            format!("no interpreter at {}", interpreter.display()),
        ));
    }

    let script = write_temp("pipdock-probe", "py", PROBE_PY)?;
    // The probe is removed whatever happens; a leaked temp file is harmless but untidy, and
    // leaving one behind after every scan is not.
    let result = Command::python(interpreter)
        .arg("-I")
        .arg(script.display().to_string())
        .arg("--json")
        .run()
        .await;
    let _ = std::fs::remove_file(&script);
    let out = result?;

    if !out.ok() {
        return Err(
            PdError::new(Code::EnvProbeFailed, format!("probe exited {:?}", out.code))
                .with_stderr(&out.stderr),
        );
    }

    parse_probe(&out.stdout, interpreter, source)
}

/// Parse the probe's JSON document.
///
/// # Errors
/// `PD-ENV-003` when the document is missing or malformed.
pub fn parse_probe(stdout: &str, interpreter: &Path, source: EnvSource) -> Result<ProbeResult> {
    let bad = |detail: &str| {
        PdError::new(
            Code::EnvProbeFailed,
            format!("unreadable probe output: {detail}"),
        )
    };

    let doc: serde_json::Value =
        serde_json::from_str(stdout.trim()).map_err(|e| bad(&e.to_string()))?;
    if let Some(err) = doc.get("error").and_then(serde_json::Value::as_str) {
        return Err(bad(err));
    }

    let python_version = doc
        .get("python")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| bad("no `python` field"))?
        .to_owned();

    let mut dists = Vec::new();
    for entry in doc
        .get("dists")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        // The probe reports unreadable distributions as `{"name": null, "error": ...}` rather
        // than failing the whole scan; skip them here for the same reason.
        let Some(raw_name) = entry.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Ok(name) = PkgName::parse(raw_name) else {
            continue;
        };
        dists.push(Dist {
            name,
            version: Version(
                entry
                    .get("version")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            ),
            requires_dist: entry
                .get("requires_dist")
                .and_then(serde_json::Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            requires_python: entry
                .get("requires_python")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            // Absent on a schema-1 probe and null whenever the probe judged the number
            // unknowable; both arrive here as `None`, which is the honest answer either way.
            size_bytes: entry.get("size_bytes").and_then(serde_json::Value::as_u64),
        });
    }

    let env = PyEnv {
        interpreter: interpreter.to_path_buf(),
        prefix: doc
            .get("prefix")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .into(),
        python_version,
        externally_managed: doc
            .get("externally_managed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        hidden_user_site: doc
            .get("hidden_user_site")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from),
        source,
    };

    Ok(ProbeResult { env, dists })
}

/// A discovered interpreter, before it has been probed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Path to the interpreter.
    pub path: PathBuf,
    /// Where it was found.
    pub source: EnvSource,
}

/// Discover every interpreter on this machine.
///
/// Sources are queried independently and merged by canonical identity, so a Chocolatey shim and
/// the installation it points at collapse into one entry (SP-6). The **first** source to report a
/// path wins, and sources are ordered so the more authoritative one is asked first.
pub async fn scan() -> Vec<Candidate> {
    scan_reporting(&|_| {}).await
}

/// Where a discovery sweep currently is.
///
/// ARCHITECTURE §7 names a `scan-progress` event and never says what it carries; this is the
/// payload. Discovery is the slowest thing PipDock does before it can show anything — a registry
/// walk, `py -0p`, `uv python list` and a venv scan, each spawning processes — so a first screen
/// that sits blank through all of it is the worst possible first impression.
///
/// `label` is a path and is **never localized** (I18N §2). The UI localizes `phase`.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    /// Which source is being read.
    pub phase: ScanPhase,
    /// Sources finished so far.
    pub done: usize,
    /// How many sources there are, so a caller can render a determinate bar.
    pub total: usize,
    /// The interpreter or directory in hand, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The discovery sources, in the order [`scan`] reads them.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ScanPhase {
    /// PEP 514 registry entries.
    Registry,
    /// `py -0p`.
    Launcher,
    /// `uv python list`.
    Uv,
    /// `.venv` directories below the working directory.
    VenvScan,
    /// Deduplicating and canonicalizing what was found.
    Collating,
}

/// How many sources a sweep reads, for the `total` a caller renders against.
const SCAN_SOURCES: usize = 5;

/// Discover environments, reporting progress as each source is read.
///
/// The callback is synchronous and must not block: it is called between sources, on the task
/// driving the scan.
pub async fn scan_reporting(report: &(dyn Fn(ScanProgress) + Sync)) -> Vec<Candidate> {
    let step = |phase: ScanPhase, done: usize| ScanProgress {
        phase,
        done,
        total: SCAN_SOURCES,
        label: None,
    };

    report(step(ScanPhase::Registry, 0));
    let registry = registry_interpreters();
    report(step(ScanPhase::Launcher, 1));
    let launcher = py_launcher_interpreters().await;
    report(step(ScanPhase::Uv, 2));
    let uv = uv_interpreters().await;
    report(step(ScanPhase::VenvScan, 3));
    let venvs = venv_scan(&std::env::current_dir().unwrap_or_default());
    report(step(ScanPhase::Collating, 4));

    let mut found: BTreeMap<String, Candidate> = BTreeMap::new();
    let sources = [
        (registry, EnvSource::Registry),
        (launcher, EnvSource::PyLauncher),
        (uv, EnvSource::Uv),
        (venvs, EnvSource::VenvScan),
    ];
    for (paths, source) in sources {
        for path in paths {
            if !path.is_file() {
                continue;
            }
            found
                .entry(canonical_interpreter(&path))
                .or_insert(Candidate { path, source });
        }
    }

    found.into_values().collect()
}

/// PEP 514 registry entries, from both `HKCU` and `HKLM`.
///
/// SP-6 found `PythonCore` present under **both** hives on the reference machine, which is one of
/// the two ways the same interpreter reaches discovery twice.
#[must_use]
pub fn registry_interpreters() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};

        let mut out = Vec::new();
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            let root = RegKey::predef(hive);
            let Ok(software) = root.open_subkey_with_flags(r"SOFTWARE\Python", KEY_READ) else {
                continue;
            };
            for company in software.enum_keys().flatten() {
                let Ok(company_key) = software.open_subkey_with_flags(&company, KEY_READ) else {
                    continue;
                };
                for tag in company_key.enum_keys().flatten() {
                    let Ok(install) =
                        company_key.open_subkey_with_flags(format!(r"{tag}\InstallPath"), KEY_READ)
                    else {
                        continue;
                    };
                    // PEP 514: `ExecutablePath` is authoritative; the default value is the
                    // directory, which older installers set instead.
                    if let Ok(exe) = install.get_value::<String, _>("ExecutablePath") {
                        out.push(PathBuf::from(exe));
                    } else if let Ok(dir) = install.get_value::<String, _>("") {
                        out.push(Path::new(&dir).join("python.exe"));
                    }
                }
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Interpreters reported by the `py` launcher (`py -0p`).
pub async fn py_launcher_interpreters() -> Vec<PathBuf> {
    let Ok(out) = Command::new("py").arg("-0p").run().await else {
        return Vec::new();
    };
    if !out.ok() {
        return Vec::new();
    }
    out.stdout.lines().filter_map(parse_listed_path).collect()
}

/// Interpreters reported by `uv python list`.
///
/// SP-6: uv lists **downloadable** interpreters alongside installed ones. Those rows carry
/// `<download available>` where a path would be, and must be dropped — offering to manage a
/// Python that is not on the machine would be nonsense.
pub async fn uv_interpreters() -> Vec<PathBuf> {
    let Ok(out) = Command::new("uv").args(["python", "list"]).run().await else {
        return Vec::new();
    };
    if !out.ok() {
        return Vec::new();
    }
    out.stdout.lines().filter_map(parse_listed_path).collect()
}

/// Take the trailing path from a listing line, rejecting `<download available>` placeholders.
fn parse_listed_path(line: &str) -> Option<PathBuf> {
    let last = line.split_whitespace().next_back()?;
    if last.starts_with('<') || !last.contains(std::path::MAIN_SEPARATOR) {
        return None;
    }
    Some(PathBuf::from(last))
}

/// Virtual environments in `dir`, one level deep.
#[must_use]
pub fn venv_scan(dir: &Path) -> Vec<PathBuf> {
    const NAMES: &[&str] = &[".venv", "venv", "env", ".env"];
    let exe: PathBuf = if cfg!(windows) {
        ["Scripts", "python.exe"].iter().collect()
    } else {
        ["bin", "python"].iter().collect()
    };

    NAMES
        .iter()
        .map(|name| dir.join(name).join(&exe))
        .filter(|p| p.is_file())
        .collect()
}

/// Is this interpreter new enough to run the probe?
#[must_use]
pub fn probe_supported(version: &str) -> bool {
    let Ok(v) = PyVersion::parse(version) else {
        return false;
    };
    let floor = format!("{}.{}", MIN_PROBE_PYTHON.0, MIN_PROBE_PYTHON.1);
    PyVersion::parse(&floor).is_ok_and(|min| v >= min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_is_embedded_and_stdlib_only() {
        assert!(
            PROBE_PY.contains("importlib.metadata"),
            "probe must read metadata"
        );
        // ARCHITECTURE §4: no third-party imports. These are the ones a careless edit would add.
        for forbidden in [
            "import requests",
            "import packaging",
            "import setuptools",
            "import pip",
        ] {
            assert!(
                !PROBE_PY.contains(forbidden),
                "probe must stay stdlib-only: {forbidden}"
            );
        }
    }

    #[test]
    fn env_hash_is_stable_and_case_insensitive_on_windows() {
        // SP-6: the Chocolatey shim and the direct install differ only in casing. Two hashes for
        // one environment would split its pins and snapshot history without any visible cause.
        let a = env_hash(Path::new(r"C:\Python314\python.exe"));
        let b = env_hash(Path::new(r"c:\python314\python.exe"));
        assert_eq!(a.len(), 64, "SHA-256 hex");
        if cfg!(windows) {
            assert_eq!(a, b);
        }
        assert_ne!(a, env_hash(Path::new(r"C:\Python312\python.exe")));
    }

    #[test]
    fn parses_a_real_probe_document() {
        let doc = r#"{
            "schema": 2, "python": "3.12.4", "prefix": "C:\\proj\\.venv",
            "is_venv": true, "externally_managed": false, "hidden_user_site": null,
            "dists": [
              {"name":"requests","version":"2.32.3",
               "requires_dist":["urllib3<3,>=1.21.1"],"requires_python":">=3.8",
               "size_bytes":131072},
              {"name":null,"error":"KeyError: broken"}
            ]}"#;
        let got = parse_probe(doc, Path::new("python.exe"), EnvSource::Manual).expect("parse");

        assert_eq!(got.env.python_version, "3.12.4");
        assert!(!got.env.externally_managed);
        assert!(!got.env.listing_is_partial());
        // The unreadable distribution is skipped rather than sinking the whole scan.
        assert_eq!(got.dists.len(), 1);
        assert_eq!(got.dists[0].name.as_str(), "requests");
        assert_eq!(got.dists[0].requires_dist, ["urllib3<3,>=1.21.1"]);
        assert_eq!(got.dists[0].size_bytes, Some(131_072));
    }

    #[test]
    fn an_unknowable_size_arrives_as_none_not_zero() {
        // Three ways the probe declines to guess, and one older probe that never knew to.
        // `Some(0)` would render as "0 B" in the Installed table, which is a claim; `None`
        // renders as "—", which is the truth.
        let doc = r#"{
            "schema": 2, "python": "3.12.4", "prefix": "C:\\proj\\.venv",
            "externally_managed": false, "hidden_user_site": null,
            "dists": [
              {"name":"editable-one","version":"1.0.0","size_bytes":null},
              {"name":"legacy-egg","version":"0.9.0"},
              {"name":"schema-one-probe","version":"2.0.0"}
            ]}"#;
        let got = parse_probe(doc, Path::new("python.exe"), EnvSource::Manual).expect("parse");
        assert_eq!(got.dists.len(), 3);
        assert!(got.dists.iter().all(|d| d.size_bytes.is_none()));
    }

    /// The join P4's remembered project folder rests on.
    ///
    /// `env_scan` keys the store lookup off the **discovered candidate path**, before any probe has
    /// run; `health_run` keys the store *write* off `PyEnv.interpreter`, which comes back through
    /// the frontend afterwards. If those two ever hash differently the folder is written under one
    /// key and read under another, and the only symptom is a Health screen that keeps asking where
    /// the project is — no error, nothing in a log.
    ///
    /// They agree because `parse_probe` carries the path it was given through untouched, and
    /// `env_hash` canonicalizes whatever it is handed. Both halves are load-bearing, so both are
    /// asserted here rather than reasoned about at the call site.
    #[test]
    fn a_probed_env_hashes_the_same_as_the_path_it_was_discovered_at() {
        let doc = r#"{"python":"3.12.4","prefix":"C:\\proj\\.venv","externally_managed":false,
            "hidden_user_site":null,"dists":[]}"#;
        let discovered = Path::new(r"C:\Proj\.venv\Scripts\Python.exe");
        let got = parse_probe(doc, discovered, EnvSource::VenvScan).expect("parse");

        assert_eq!(
            got.env.interpreter, discovered,
            "the path is carried, not rewritten"
        );
        assert_eq!(
            env_hash(discovered),
            env_hash(&got.env.interpreter),
            "the write key and the read key must be the same string"
        );
    }

    #[test]
    fn a_hidden_user_site_marks_the_listing_partial() {
        let doc = r#"{"python":"3.14.6","prefix":"C:\\Python314","externally_managed":false,
            "hidden_user_site":"C:\\Users\\a\\AppData\\Roaming\\Python\\Python314\\site-packages",
            "dists":[]}"#;
        let got = parse_probe(doc, Path::new("python.exe"), EnvSource::Registry).expect("parse");
        assert!(got.env.listing_is_partial());
        assert!(got.env.hidden_user_site.is_some());
    }

    #[test]
    fn probe_errors_use_the_documented_code() {
        for bad in [
            "",
            "not json",
            r#"{"error":"ModuleNotFoundError"}"#,
            r#"{"prefix":"x"}"#,
        ] {
            let err = parse_probe(bad, Path::new("python.exe"), EnvSource::Manual)
                .unwrap_err_or_panic(bad);
            assert_eq!(err.code, Code::EnvProbeFailed, "{bad:?}");
        }
    }

    /// Small helper so the loop above reads cleanly.
    trait UnwrapErrOrPanic {
        fn unwrap_err_or_panic(self, ctx: &str) -> PdError;
    }
    impl UnwrapErrOrPanic for Result<ProbeResult> {
        fn unwrap_err_or_panic(self, ctx: &str) -> PdError {
            match self {
                Ok(_) => panic!("{ctx:?} should not have parsed"),
                Err(e) => e,
            }
        }
    }

    #[test]
    fn downloadable_uv_entries_are_rejected() {
        // SP-6: `uv python list` mixes installed interpreters with ones it offers to fetch.
        assert!(
            parse_listed_path("cpython-3.15.0b4-windows-x86_64-none    <download available>")
                .is_none()
        );
        assert_eq!(
            parse_listed_path(r"cpython-3.14.6-windows-x86_64-none   C:\Python314\python.exe"),
            Some(PathBuf::from(r"C:\Python314\python.exe"))
        );
        // `py -0p` rows have the same trailing-path shape.
        assert_eq!(
            parse_listed_path(r" -V:3.14 *        C:\Python314\python.exe"),
            Some(PathBuf::from(r"C:\Python314\python.exe"))
        );
        assert!(parse_listed_path("").is_none());
    }

    #[test]
    fn the_probe_python_floor_matches_the_document() {
        assert_eq!(MIN_PROBE_PYTHON, (3, 10));
        assert!(probe_supported("3.10.0"));
        assert!(probe_supported("3.14.6"));
        assert!(!probe_supported("3.9.18"));
        assert!(!probe_supported("nonsense"));
    }

    #[tokio::test]
    async fn probing_a_missing_interpreter_is_an_env_error() {
        let err = probe(Path::new("no-such-python.exe"), EnvSource::Manual)
            .await
            .expect_err("missing interpreter must fail");
        assert_eq!(err.code, Code::EnvInterpreterMissing);
    }
}
