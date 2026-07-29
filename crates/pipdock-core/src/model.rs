//! Core domain types shared by every module, the CLI and the GUI.
//!
//! These are the types `specta`/`tauri-specta` will export to TypeScript (ARCHITECTURE §9), so
//! their serde representation is part of the public contract — see `docs/CLI-SPEC.md` §6.

use std::path::PathBuf;

use crate::errors::{Code, PdError, Result};

/// Which resolver PipDock is driving.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum EngineId {
    /// `<python> -m pip …`
    Pip,
    /// `uv pip … --python <python>`
    Uv,
}

impl EngineId {
    /// Wire form, also what the status-line engine badge shows (UI-SPEC §3).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pip => "pip",
            Self::Uv => "uv",
        }
    }
}

/// Availability and version of an engine for a given environment.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct EngineInfo {
    /// Which engine this describes.
    pub id: EngineId,
    /// Reported version, absent when the engine is not available.
    pub version: Option<String>,
    /// False means the binary or module could not be found (`PD-ENG-001`).
    pub available: bool,
}

/// How an environment was discovered. Surfaced as the source chip in the Environments screen.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum EnvSource {
    /// PEP 514 registry entry.
    Registry,
    /// Reported by the `py` launcher (`py -0p`).
    PyLauncher,
    /// Reported by `uv python list`.
    Uv,
    /// Found by scanning known venv directories.
    VenvScan,
    /// Added by the user via *Browse…*.
    Manual,
}

/// A Python environment PipDock can act on.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PyEnv {
    /// Canonicalized path to the interpreter. Never a shell string — see SECURITY §2.
    pub interpreter: PathBuf,
    /// `sys.prefix` as reported by `probe.py`.
    pub prefix: PathBuf,
    /// e.g. `"3.12.4"`.
    pub python_version: String,
    /// PEP 668 marker present. When true, mutation is blocked unless the user enabled the
    /// Settings override (`PD-ENV-002`, SECURITY §3).
    pub externally_managed: bool,
    /// User site-packages directory that `probe.py -I` is hiding, when there is one.
    ///
    /// Owner decision 2026-07-27 (SP-6): the probe keeps running isolated, so on a non-venv
    /// system Python it reports fewer distributions than `pip list` does. `Some` means packages
    /// really are hidden and the Installed screen shows a note naming this path; `None` — always
    /// the case inside a venv — means the listing is complete.
    #[serde(default)]
    pub hidden_user_site: Option<PathBuf>,
    /// Where this env came from.
    pub source: EnvSource,
}

impl PyEnv {
    /// True when the installed listing is known to be incomplete, so the UI must say so.
    #[must_use]
    pub fn listing_is_partial(&self) -> bool {
        self.hidden_user_site.is_some()
    }
}

/// A PEP 503-normalized distribution name.
///
/// Constructing one is the *only* way a package name reaches an argv array, which is what makes
/// SECURITY §2's "validated before it reaches argv" claim structural rather than aspirational.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct PkgName(String);

impl PkgName {
    /// Validate and normalize a distribution name.
    ///
    /// Accepts the PEP 508 name grammar — `^[A-Za-z0-9]([A-Za-z0-9._-]*[A-Za-z0-9])?$` — then
    /// applies PEP 503 normalization: lowercase, and runs of `-`, `_` or `.` collapse to a single
    /// `-`. Anything else is rejected with `PD-PKG-002`.
    ///
    /// # Errors
    /// Returns `PD-PKG-002` when `raw` is not a valid distribution name.
    pub fn parse(raw: &str) -> Result<Self> {
        let invalid = || PdError::new(Code::PkgNotFound, format!("invalid package name: {raw:?}"));

        if raw.is_empty() {
            return Err(invalid());
        }
        let bytes = raw.as_bytes();
        let alnum = |b: u8| b.is_ascii_alphanumeric();
        // First and last characters must be alphanumeric; the middle may also use . _ -
        if !alnum(bytes[0]) || !alnum(bytes[bytes.len() - 1]) {
            return Err(invalid());
        }
        if !bytes
            .iter()
            .all(|&b| alnum(b) || matches!(b, b'.' | b'_' | b'-'))
        {
            return Err(invalid());
        }

        Ok(Self(normalize_name(raw)))
    }

    /// The normalized name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PkgName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// PEP 503 normalization: lowercase, and runs of `-`, `_` or `.` collapse to a single `-`.
///
/// Separate from [`PkgName::parse`] because search must normalize a **query**, which is not yet a
/// valid name and may never become one. Both paths must agree, or a user typing `Zope.Interface`
/// would not find `zope-interface`, so there is exactly one implementation.
#[must_use]
pub fn normalize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_was_separator = false;
    for ch in raw.chars() {
        if matches!(ch, '.' | '_' | '-') {
            if !prev_was_separator {
                out.push('-');
            }
            prev_was_separator = true;
        } else {
            out.push(ch.to_ascii_lowercase());
            prev_was_separator = false;
        }
    }
    out
}

/// A version string as the engine reported it. Never reshaped or localized (I18N §2).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct Version(pub String);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What the user asked for: a name, optionally constrained.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Spec {
    /// Normalized distribution name.
    pub name: PkgName,
    /// PEP 440 specifier such as `">=2,<3"`, or `None` for "latest".
    pub version_req: Option<String>,
}

/// A resolved, exact `name==version` pair. Only these reach a mutating engine command.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct PinnedSpec {
    /// Normalized distribution name.
    pub name: PkgName,
    /// The exact version the resolver chose.
    pub version: Version,
}

impl PinnedSpec {
    /// Render as the `name==version` argv token.
    #[must_use]
    pub fn to_requirement(&self) -> String {
        format!("{}=={}", self.name, self.version)
    }
}

/// An installed distribution, as read by `probe.py` or `<engine> list`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Dist {
    /// Normalized distribution name.
    pub name: PkgName,
    /// Installed version.
    pub version: Version,
    /// Raw `Requires-Dist` entries. The reverse-dependency graph is built from these.
    #[serde(default)]
    pub requires_dist: Vec<String>,
    /// Raw `Requires-Python` specifier, when declared.
    #[serde(default)]
    pub requires_python: Option<String>,
}

/// An installed distribution with a newer release available.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct OutdatedDist {
    /// Normalized distribution name.
    pub name: PkgName,
    /// Currently installed version.
    pub current: Version,
    /// Newest version the index offers.
    pub latest: Version,
}

/// Which of the two execution phases produced a result (ARCHITECTURE §8).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ExecMode {
    /// Phase A: one engine invocation for the whole pinned set.
    Batch,
    /// Phase B: per-package sequential retry after Phase A failed. Skip-and-continue.
    Isolated,
}

/// Outcome of a single package's step.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    /// Applied successfully.
    Ok,
    /// Failed; carries a catalog code. Does **not** abort the batch.
    Failed,
    /// Not attempted — user cancelled, or dropped by a Skip decision.
    Skipped,
}

/// One row of the summary report (DATA-FLOW §6).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StepResult {
    /// Normalized distribution name.
    pub pkg: PkgName,
    /// Version before the step, absent for a fresh install.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Version>,
    /// Version the step targeted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Version>,
    /// Whether it applied.
    pub status: StepStatus,
    /// Catalog code, present exactly when `status` is `Failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
    /// Tail of the engine's stderr for the failure detail pane.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_tail: Option<String>,
}

/// One unsatisfied requirement reported by `pip check` / `uv pip check`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct CheckFinding {
    /// The distribution whose requirement is unsatisfied.
    pub pkg: PkgName,
    /// Human-readable requirement text as the engine printed it.
    pub requirement: String,
}

/// Post-execution environment verification.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct CheckReport {
    /// True when the engine reported no broken requirements.
    pub ok: bool,
    /// Details when `ok` is false.
    #[serde(default)]
    pub findings: Vec<CheckFinding>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_per_pep_503() {
        let cases = [
            ("Requests", "requests"),
            ("zope.interface", "zope-interface"),
            ("typing_extensions", "typing-extensions"),
            ("ruamel.yaml.clib", "ruamel-yaml-clib"),
            // Runs of mixed separators collapse to a single dash.
            ("foo._-.bar", "foo-bar"),
            ("A", "a"),
        ];
        for (raw, want) in cases {
            let got = PkgName::parse(raw).unwrap_or_else(|e| panic!("{raw:?} rejected: {e}"));
            assert_eq!(got.as_str(), want, "normalizing {raw:?}");
        }
    }

    #[test]
    fn rejects_names_outside_the_pep_508_grammar() {
        // Every one of these would be a way to smuggle something past argv validation.
        let bad = [
            "",
            "-leading",
            "trailing-",
            ".dotstart",
            "has space",
            "semi;colon",
            "amp&and",
            "pipe|d",
            "quote\"d",
            "new\nline",
            "back\\slash",
            "sla/sh",
            "--upgrade",
            "requests>=2",
            "naïve",
        ];
        for raw in bad {
            assert!(PkgName::parse(raw).is_err(), "{raw:?} should be rejected");
        }
    }

    #[test]
    fn rejected_names_use_the_documented_code() {
        let err = PkgName::parse("--upgrade").expect_err("flag-like name must be rejected");
        assert_eq!(err.code, Code::PkgNotFound);
        assert_eq!(err.code.as_str(), "PD-PKG-002");
    }

    #[test]
    fn pinned_spec_renders_the_argv_token() {
        let spec = PinnedSpec {
            name: PkgName::parse("Typing_Extensions").unwrap_or_else(|_| unreachable!()),
            version: Version("4.12.2".into()),
        };
        assert_eq!(spec.to_requirement(), "typing-extensions==4.12.2");
    }
}
