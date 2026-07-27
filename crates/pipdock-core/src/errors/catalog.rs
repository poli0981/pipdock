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

/// A stderr pattern and the code it implies. Order in [`CLASSIFIERS`] is priority order.
struct Classifier {
    code: Code,
    /// All of these substrings must be present (case-insensitively) for the rule to fire.
    all_of: &'static [&'static str],
}

/// Priority-ordered classifiers. **First match wins** (`docs/ERROR-CATALOG.md` §preamble).
///
/// Ordering rules that matter, each learned from the SP-2 corpus:
///
/// - Specific build failures precede the generic wheel-build failure, or every MSVC problem would
///   be reported as `PD-BLD-003`.
/// - TLS failures precede generic network failures: a certificate error also prints "Could not
///   fetch URL", and the corporate-proxy guidance is the useful one.
/// - `PD-PKG-001` is **absent by design**. pip reports a Requires-Python mismatch with the same
///   "No matching distribution found" text as an unknown name, and uv does not enforce it at all —
///   so it is raised by [`crate::compat`] before the engine runs, not classified from stderr.
static CLASSIFIERS: &[Classifier] = &[
    // -- environment ---------------------------------------------------------
    Classifier {
        code: Code::EnvExternallyManaged,
        all_of: &["externally-managed-environment"],
    },
    Classifier {
        code: Code::EnvExternallyManaged,
        all_of: &["externally-managed"],
    },
    // -- package identity ----------------------------------------------------
    // Ahead of the resolution rules: uv reports an unknown name *through* its resolver, as
    // "No solution found … because <name> was not found in the package registry". Classified as
    // a resolution failure it would send the user hunting for a version conflict that does not
    // exist.
    Classifier {
        code: Code::PkgNotFound,
        all_of: &["was not found in the package registry"],
    },
    // -- resolution ----------------------------------------------------------
    Classifier {
        code: Code::ResImpossible,
        all_of: &["resolutionimpossible"],
    },
    Classifier {
        code: Code::ResImpossible,
        all_of: &["conflicting dependencies"],
    },
    Classifier {
        code: Code::ResImpossible,
        all_of: &["no solution found when resolving"],
    },
    // -- build ---------------------------------------------------------------
    Classifier {
        code: Code::BldMsvcMissing,
        all_of: &["microsoft visual c++"],
    },
    Classifier {
        code: Code::BldBackendFailed,
        all_of: &["metadata-generation-failed"],
    },
    Classifier {
        code: Code::BldBackendFailed,
        all_of: &["backendunavailable"],
    },
    Classifier {
        code: Code::BldBackendFailed,
        all_of: &["the build backend returned an error"],
    },
    Classifier {
        code: Code::BldBackendFailed,
        all_of: &["error in", "pyproject.toml"],
    },
    Classifier {
        code: Code::BldWheelFailed,
        all_of: &["failed to build"],
    },
    Classifier {
        code: Code::BldWheelFailed,
        all_of: &["failed building wheel"],
    },
    // -- permissions ---------------------------------------------------------
    // Checked before the generic OSError shapes below, which share their prefix.
    Classifier {
        code: Code::PrmFileLocked,
        all_of: &["winerror 32"],
    },
    Classifier {
        code: Code::PrmSitePackagesReadOnly,
        all_of: &["permissionerror"],
    },
    Classifier {
        code: Code::PrmSitePackagesReadOnly,
        all_of: &["winerror 5"],
    },
    // -- system --------------------------------------------------------------
    Classifier {
        code: Code::SysDiskFull,
        all_of: &["no space left on device"],
    },
    Classifier {
        code: Code::SysDiskFull,
        all_of: &["winerror 112"],
    },
    Classifier {
        code: Code::SysPathTooLong,
        all_of: &["path too long"],
    },
    Classifier {
        code: Code::SysPathTooLong,
        all_of: &["filename or extension is too long"],
    },
    // -- network -------------------------------------------------------------
    Classifier {
        code: Code::NetTlsFailure,
        all_of: &["certificate_verify_failed"],
    },
    Classifier {
        code: Code::NetTlsFailure,
        all_of: &["sslerror"],
    },
    Classifier {
        code: Code::NetTlsFailure,
        all_of: &["ssl certificate"],
    },
    Classifier {
        code: Code::NetUnreachable,
        all_of: &["connection aborted"],
    },
    Classifier {
        code: Code::NetUnreachable,
        all_of: &["temporary failure in name resolution"],
    },
    Classifier {
        code: Code::NetUnreachable,
        all_of: &["failed to establish a new connection"],
    },
    Classifier {
        code: Code::NetUnreachable,
        all_of: &["newconnectionerror"],
    },
    Classifier {
        code: Code::NetUnreachable,
        all_of: &["read timed out"],
    },
    // uv's phrasing for the same conditions; it reports through its own HTTP stack.
    Classifier {
        code: Code::NetUnreachable,
        all_of: &["tcp connect error"],
    },
    Classifier {
        code: Code::NetUnreachable,
        all_of: &["error sending request for url"],
    },
    Classifier {
        code: Code::NetUnreachable,
        all_of: &["failed to fetch"],
    },
    Classifier {
        code: Code::NetUnreachable,
        all_of: &["could not fetch url"],
    },
    // -- package / index -----------------------------------------------------
    Classifier {
        code: Code::PkgHashMismatch,
        all_of: &["do not match the hashes"],
    },
    Classifier {
        code: Code::PkgNotFound,
        all_of: &["no matching distribution found"],
    },
    Classifier {
        code: Code::PkgNotFound,
        all_of: &["could not find a version that satisfies"],
    },
    // No `PD-PKG-003` rule. A yank is not a failure: both engines exit 0 and install the release
    // (SP-2), so it is detected structurally by the plan parsers — pip's `is_yanked` field, uv's
    // `warning: … is yanked` line — and shown as a preview warning. A stderr rule here also
    // misfires: pip mentions "Ignored the following yanked versions" while reporting an entirely
    // different failure, which classified the Requires-Python fixture as a yank.
    // -- engine --------------------------------------------------------------
    Classifier {
        code: Code::EngPipTooOld,
        all_of: &["no such option: --report"],
    },
    Classifier {
        code: Code::EngNotFound,
        all_of: &["no module named pip"],
    },
    Classifier {
        code: Code::EnvProbeFailed,
        all_of: &["modulenotfounderror"],
    },
];

/// Classify an engine's stderr into a catalog code.
///
/// Patterns are checked in priority order and the **first** match wins, falling back to
/// [`Code::EngUnclassified`] (`PD-ENG-999`) so no failure can reach the user uncoded
/// (DATA-FLOW §9.4).
///
/// Matching is case-insensitive because the same condition is phrased differently by pip, uv and
/// the Windows CRT, and substring-based because engine messages interpolate paths and versions.
///
/// Whitespace is collapsed first, which is not cosmetic: **uv hard-wraps its diagnostics** at a
/// fixed width, so its unknown-package message arrives as `"was not found in the package\n
/// registry"`. Matched literally, that phrase never appears and the failure is misread as a
/// version conflict, sending the user hunting for a constraint problem that does not exist.
#[must_use]
pub fn classify_stderr(stderr: &str) -> Code {
    let haystack = normalize(stderr);
    for rule in CLASSIFIERS {
        if rule.all_of.iter().all(|needle| haystack.contains(needle)) {
            return rule.code;
        }
    }
    Code::EngUnclassified
}

/// Lowercase and collapse every run of whitespace to a single space, so patterns survive the
/// engines' line wrapping and CRLF.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            in_space = true;
        } else {
            if in_space && !out.is_empty() {
                out.push(' ');
            }
            in_space = false;
            out.extend(ch.to_lowercase());
        }
    }
    out
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

    #[test]
    fn quiet_output_is_never_a_failure() {
        // pip prints its upgrade notice on nearly every run and uv writes its whole plan to
        // stderr, so a non-empty stderr must not imply failure.
        for benign in [
            "",
            "   \r\n",
            "[notice] A new release of pip is available: 25.0.1 -> 26.1.2",
            "Resolved 3 packages in 831ms\r\nWould install 2 packages\r\n + httpcore==1.0.9",
        ] {
            assert_eq!(classify_stderr(benign), Code::EngUnclassified, "{benign:?}");
        }
    }

    #[test]
    fn patterns_survive_line_wrapping() {
        // uv hard-wraps diagnostics, so a phrase can be split mid-sentence by a newline and
        // indentation. Matched literally this reads as a version conflict instead of a typo.
        let wrapped = "  × No solution found when resolving dependencies:\r\n  \
                       ╰─▶ Because widget-9f3a was not found in the package\r\n      \
                       registry and you require widget-9f3a, we can conclude\r\n      \
                       that your requirements are unsatisfiable.\r\n";
        assert_eq!(classify_stderr(wrapped), Code::PkgNotFound);
    }

    #[test]
    fn specific_build_failures_beat_the_generic_one() {
        // Both phrases appear together in a real MSVC failure; the specific one must win or the
        // user gets "check the log" instead of "install the Build Tools".
        let msvc = "error: Microsoft Visual C++ 14.0 or greater is required.\r\n\
                    ERROR: Failed building wheel for somepkg\r\n";
        assert_eq!(classify_stderr(msvc), Code::BldMsvcMissing);
    }

    #[test]
    fn tls_failures_beat_generic_network_failures() {
        // A certificate error also prints "Could not fetch URL"; the proxy guidance is the useful
        // message, and SECURITY §4 forbids ever suggesting verification be disabled.
        let tls = "SSLError(SSLCertVerificationError(1, '[SSL: CERTIFICATE_VERIFY_FAILED] ...'))\r\n\
             ERROR: Could not fetch URL https://pypi.org/simple/requests/\r\n";
        assert_eq!(classify_stderr(tls), Code::NetTlsFailure);
    }

    #[test]
    fn a_locked_file_is_not_reported_as_a_permission_problem() {
        // WinError 32 means "close the program using it", not "you lack permission" — different
        // code, different advice.
        let locked = "OSError: [WinError 32] The process cannot access the file because it is \
                      being used by another process: 'C:\\proj\\.venv\\Lib\\site-packages\\x.pyd'";
        assert_eq!(classify_stderr(locked), Code::PrmFileLocked);
    }

    #[test]
    fn incidental_mentions_of_yanking_do_not_hijack_a_different_failure() {
        // pip prefixes an unrelated resolution failure with "Ignored the following yanked
        // versions", which a naive yank rule mis-classified as PD-PKG-003.
        let stderr = "ERROR: Ignored the following yanked versions: 1.11.0, 1.14.0rc1\r\n\
                      ERROR: No matching distribution found for scipy==1.7.3\r\n";
        assert_eq!(classify_stderr(stderr), Code::PkgNotFound);
    }

    #[test]
    fn classifiers_never_yield_a_code_outside_the_catalog() {
        let all: HashSet<&str> = Code::ALL.iter().map(|c| c.as_str()).collect();
        for rule in CLASSIFIERS {
            assert!(
                all.contains(rule.code.as_str()),
                "{} is not in Code::ALL",
                rule.code
            );
        }
    }
}
