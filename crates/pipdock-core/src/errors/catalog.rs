//! The error catalog: the single source of truth for every user-visible failure.
//!
//! Mirrors `docs/ERROR-CATALOG.md` §2. Two rules from that document are structural:
//!
//! 1. Every user-visible failure carries **exactly one** code.
//! 2. Classifiers run over engine stderr in **priority order, first match wins**, falling back to
//!    [`Code::EngUnclassified`] (`PD-ENG-999`).
//!
//! Codes and stderr are never localized — only the one-liner and action text are, and those live
//! in the i18next catalogs (`docs/I18N.md` §1). The `as_str` values below are the i18next keys'
//! discriminators, so they must stay stable once shipped.

use std::fmt;

/// Catalog area: the `<AREA>` in `PD-<AREA>-<NNN>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Area {
    /// Environment discovery, probing, PEP 668.
    Env,
    /// Engine availability and output shape.
    Eng,
    /// Dependency resolution.
    Res,
    /// Build backends and compiled extensions.
    Bld,
    /// Package identity and the index.
    Pkg,
    /// Network and TLS.
    Net,
    /// Filesystem permissions and locks.
    Prm,
    /// Snapshots and rollback.
    Snp,
    /// Host system limits.
    Sys,
    /// Code Health tooling.
    Hlt,
    /// PipDock's own bugs.
    Int,
}

/// Every code PipDock can surface at v1 launch.
///
/// `docs/TESTING.md` §2 requires at least one captured stderr fixture per variant before ship;
/// the test that enforces "no code without fixture" lives beside the classifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Code {
    // -- PD-ENV: environment ------------------------------------------------
    /// Interpreter path missing or not executable — the env was deleted or moved.
    EnvInterpreterMissing,
    /// PEP 668 `EXTERNALLY-MANAGED` marker present; mutation blocked by default.
    EnvExternallyManaged,
    /// `probe.py` exited non-zero or produced unparseable output.
    EnvProbeFailed,

    // -- PD-ENG: engine -----------------------------------------------------
    /// pip or uv binary not found.
    EngNotFound,
    /// pip older than 22.2, which predates `--dry-run --report`.
    EngPipTooOld,
    /// uv emitted a shape the adapter does not recognize (uv newer than PipDock).
    EngUvShapeUnknown,
    /// Fallback for engine failures no classifier matched. Always last.
    EngUnclassified,

    // -- PD-RES: resolution -------------------------------------------------
    /// `ResolutionImpossible` or the uv equivalent: constraints cannot be satisfied.
    ResImpossible,
    /// Plan older than 10 minutes, or the env's probe hash changed since the preview.
    ResPlanStale,

    // -- PD-BLD: build ------------------------------------------------------
    /// MSVC build tools required but absent.
    BldMsvcMissing,
    /// Build backend failed (`pyproject.toml` error, `metadata-generation-failed`).
    BldBackendFailed,
    /// Generic sdist wheel-build failure.
    BldWheelFailed,

    // -- PD-PKG: package & index -------------------------------------------
    /// No matching distribution, and metadata shows a requires-python mismatch.
    PkgRequiresPython,
    /// No matching distribution: name or version typo, or the release is gone.
    PkgNotFound,
    /// The requested release was yanked.
    PkgYanked,
    /// Downloaded artifact failed hash verification.
    PkgHashMismatch,

    // -- PD-NET: network ----------------------------------------------------
    /// Timeout, connection aborted, or DNS failure.
    NetUnreachable,
    /// TLS/SSL verification failed — typically proxy or AV interception.
    NetTlsFailure,
    /// PEP 691 index refresh failed; the stale index stays searchable.
    NetIndexRefreshFailed,
    /// Code Health tools-venv bootstrap could not reach PyPI.
    NetToolsBootstrapFailed,

    // -- PD-PRM: permissions ------------------------------------------------
    /// `PermissionError` writing site-packages (admin-owned Python).
    PrmSitePackagesReadOnly,
    /// File locked by a running process (`WinError 32`).
    PrmFileLocked,

    // -- PD-SNP: snapshot ---------------------------------------------------
    /// Snapshot write failed before execution. **The plan is aborted; nothing runs.**
    SnpWriteFailed,
    /// A release needed to restore a snapshot is no longer available on PyPI.
    SnpTargetUnavailable,

    // -- PD-SYS: system -----------------------------------------------------
    /// `MAX_PATH` exceeded without the long-path opt-in.
    SysPathTooLong,
    /// Disk full.
    SysDiskFull,

    // -- PD-HLT: code health ------------------------------------------------
    /// A Code Health tool is missing from the tools venv.
    HltToolMissing,
    /// A Code Health tool exited non-zero.
    HltToolFailed,
    /// A Code Health tool exceeded its watchdog timeout; partial report shown.
    HltTimeout,

    // -- PD-INT: internal ---------------------------------------------------
    /// A PipDock bug: panic or unexpected state.
    IntUnexpected,
}

impl Code {
    /// The wire form, e.g. `"PD-BLD-002"`. Stable once shipped: it appears in logs, in `--json`
    /// output, and in prefilled bug-report URLs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EnvInterpreterMissing => "PD-ENV-001",
            Self::EnvExternallyManaged => "PD-ENV-002",
            Self::EnvProbeFailed => "PD-ENV-003",
            Self::EngNotFound => "PD-ENG-001",
            Self::EngPipTooOld => "PD-ENG-002",
            Self::EngUvShapeUnknown => "PD-ENG-003",
            Self::EngUnclassified => "PD-ENG-999",
            Self::ResImpossible => "PD-RES-001",
            Self::ResPlanStale => "PD-RES-002",
            Self::BldMsvcMissing => "PD-BLD-001",
            Self::BldBackendFailed => "PD-BLD-002",
            Self::BldWheelFailed => "PD-BLD-003",
            Self::PkgRequiresPython => "PD-PKG-001",
            Self::PkgNotFound => "PD-PKG-002",
            Self::PkgYanked => "PD-PKG-003",
            Self::PkgHashMismatch => "PD-PKG-004",
            Self::NetUnreachable => "PD-NET-001",
            Self::NetTlsFailure => "PD-NET-002",
            Self::NetIndexRefreshFailed => "PD-NET-010",
            Self::NetToolsBootstrapFailed => "PD-NET-011",
            Self::PrmSitePackagesReadOnly => "PD-PRM-001",
            Self::PrmFileLocked => "PD-PRM-002",
            Self::SnpWriteFailed => "PD-SNP-001",
            Self::SnpTargetUnavailable => "PD-SNP-002",
            Self::SysPathTooLong => "PD-SYS-001",
            Self::SysDiskFull => "PD-SYS-002",
            Self::HltToolMissing => "PD-HLT-001",
            Self::HltToolFailed => "PD-HLT-002",
            Self::HltTimeout => "PD-HLT-003",
            Self::IntUnexpected => "PD-INT-001",
        }
    }

    /// The catalog area this code belongs to.
    #[must_use]
    pub const fn area(self) -> Area {
        match self {
            Self::EnvInterpreterMissing | Self::EnvExternallyManaged | Self::EnvProbeFailed => {
                Area::Env
            }
            Self::EngNotFound
            | Self::EngPipTooOld
            | Self::EngUvShapeUnknown
            | Self::EngUnclassified => Area::Eng,
            Self::ResImpossible | Self::ResPlanStale => Area::Res,
            Self::BldMsvcMissing | Self::BldBackendFailed | Self::BldWheelFailed => Area::Bld,
            Self::PkgRequiresPython
            | Self::PkgNotFound
            | Self::PkgYanked
            | Self::PkgHashMismatch => Area::Pkg,
            Self::NetUnreachable
            | Self::NetTlsFailure
            | Self::NetIndexRefreshFailed
            | Self::NetToolsBootstrapFailed => Area::Net,
            Self::PrmSitePackagesReadOnly | Self::PrmFileLocked => Area::Prm,
            Self::SnpWriteFailed | Self::SnpTargetUnavailable => Area::Snp,
            Self::SysPathTooLong | Self::SysDiskFull => Area::Sys,
            Self::HltToolMissing | Self::HltToolFailed | Self::HltTimeout => Area::Hlt,
            Self::IntUnexpected => Area::Int,
        }
    }

    /// Every code, in catalog order. The fixture-coverage test in `docs/TESTING.md` §2 iterates
    /// this, so a new variant fails CI until it has a captured stderr fixture.
    pub const ALL: &'static [Self] = &[
        Self::EnvInterpreterMissing,
        Self::EnvExternallyManaged,
        Self::EnvProbeFailed,
        Self::EngNotFound,
        Self::EngPipTooOld,
        Self::EngUvShapeUnknown,
        Self::ResImpossible,
        Self::ResPlanStale,
        Self::BldMsvcMissing,
        Self::BldBackendFailed,
        Self::BldWheelFailed,
        Self::PkgRequiresPython,
        Self::PkgNotFound,
        Self::PkgYanked,
        Self::PkgHashMismatch,
        Self::NetUnreachable,
        Self::NetTlsFailure,
        Self::NetIndexRefreshFailed,
        Self::NetToolsBootstrapFailed,
        Self::PrmSitePackagesReadOnly,
        Self::PrmFileLocked,
        Self::SnpWriteFailed,
        Self::SnpTargetUnavailable,
        Self::SysPathTooLong,
        Self::SysDiskFull,
        Self::HltToolMissing,
        Self::HltToolFailed,
        Self::HltTimeout,
        Self::IntUnexpected,
        Self::EngUnclassified,
    ];
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Classify an engine's stderr into a catalog code.
///
/// Filled during spike SP-2, which captures real stderr for each case. Until then this returns
/// the documented fallback so no call site can accidentally surface an uncoded failure.
///
/// Contract when implemented: patterns are checked in priority order and the **first** match wins.
#[must_use]
pub fn classify_stderr(_stderr: &str) -> Code {
    Code::EngUnclassified
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_variant_is_listed_in_all() {
        // `ALL` drives the fixture-coverage gate, so a variant missing from it would silently
        // escape that gate. There is no derive for "enumerate variants", hence this count check.
        assert_eq!(Code::ALL.len(), 30, "add the new variant to Code::ALL");
    }

    #[test]
    fn wire_codes_are_unique() {
        let seen: HashSet<&str> = Code::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            seen.len(),
            Code::ALL.len(),
            "duplicate wire code in catalog"
        );
    }

    #[test]
    fn wire_codes_match_the_documented_grammar() {
        for code in Code::ALL {
            let s = code.as_str();
            let mut parts = s.split('-');
            assert_eq!(parts.next(), Some("PD"), "{s} must start with PD");
            let area = parts.next().unwrap_or_default();
            let num = parts.next().unwrap_or_default();
            assert_eq!(parts.next(), None, "{s} has too many segments");
            assert_eq!(area.len(), 3, "{s} area must be 3 letters");
            assert!(
                area.chars().all(|c| c.is_ascii_uppercase()),
                "{s} area case"
            );
            assert_eq!(num.len(), 3, "{s} number must be 3 digits");
            assert!(num.chars().all(|c| c.is_ascii_digit()), "{s} number digits");
        }
    }

    #[test]
    fn unclassified_is_the_documented_fallback() {
        assert_eq!(
            classify_stderr("something nobody has seen before"),
            Code::EngUnclassified
        );
        assert_eq!(Code::EngUnclassified.as_str(), "PD-ENG-999");
    }
}
