//! Requires-Python enforcement.
//!
//! **Owner decision 2026-07-27 (spike SP-2).** The engines disagree: given `scipy==1.7.3`, which
//! declares `Requires-Python >=3.7,<3.11`, pip refuses it on Python 3.12 while uv plans to install
//! it. PipDock therefore enforces Requires-Python **itself**, so the preview is the same whichever
//! engine is selected. A candidate this module rejects never reaches an engine command, and is
//! reported as `PD-PKG-001` with the required range shown against the environment's version.
//!
//! This is a deliberately narrow slice of PEP 440: enough to evaluate a `Requires-Python`
//! specifier set against a concrete interpreter version, and nothing more. Resolution itself is
//! still the engine's job (ARCHITECTURE §1.2) — this only filters candidates the engine should
//! never have been offered.

use crate::errors::{Code, PdError, Result};

/// A release version as an ordered list of numeric segments, e.g. `3.12.4` → `[3, 12, 4]`.
///
/// Only the release segment matters here. Pre-release, post-release and local segments are
/// ignored: no interpreter PipDock manages is identified by them, and `Requires-Python` specifiers
/// in the wild do not use them meaningfully.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PyVersion(Vec<u32>);

impl PyVersion {
    /// Parse a dotted numeric version.
    ///
    /// # Errors
    /// Returns `PD-ENV-003` when the string is not a dotted sequence of numbers — that can only
    /// come from a broken `probe.py` reading, which is exactly what that code means.
    pub fn parse(raw: &str) -> Result<Self> {
        // Trim any pre/post/local suffix: "3.14.0rc1" -> "3.14.0".
        let release: &str = raw
            .split(['a', 'b', 'r', 'c', 'd', 'p', '+', '-'])
            .next()
            .unwrap_or("");
        let trimmed = release.trim_end_matches('.');
        if trimmed.is_empty() {
            return Err(PdError::new(
                Code::EnvProbeFailed,
                format!("bad version: {raw:?}"),
            ));
        }
        let mut parts = Vec::new();
        for segment in trimmed.split('.') {
            let n: u32 = segment
                .parse()
                .map_err(|_| PdError::new(Code::EnvProbeFailed, format!("bad version: {raw:?}")))?;
            parts.push(n);
        }
        Ok(Self(parts))
    }

    /// Compare against `other`, treating missing trailing segments as zero so `3.12` == `3.12.0`.
    fn cmp_padded(&self, other: &Self) -> std::cmp::Ordering {
        let len = self.0.len().max(other.0.len());
        for i in 0..len {
            let a = self.0.get(i).copied().unwrap_or(0);
            let b = other.0.get(i).copied().unwrap_or(0);
            match a.cmp(&b) {
                std::cmp::Ordering::Equal => {}
                other_ord => return other_ord,
            }
        }
        std::cmp::Ordering::Equal
    }

    /// True when `self` starts with every segment of `prefix` — the `==3.9.*` / `!=3.9.*` rule.
    fn has_prefix(&self, prefix: &Self) -> bool {
        prefix.0.len() <= self.0.len() && self.0[..prefix.0.len()] == prefix.0[..]
    }
}

impl std::fmt::Display for PyVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parts: Vec<String> = self.0.iter().map(u32::to_string).collect();
        f.write_str(&parts.join("."))
    }
}

/// Why a candidate was rejected, or that it was accepted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Compatibility {
    /// The candidate may be offered to the engine.
    Compatible,
    /// The candidate declares a `Requires-Python` this interpreter does not satisfy.
    ///
    /// Both fields are shown to the user, per `docs/ERROR-CATALOG.md` PD-PKG-001's "shows required
    /// range vs env version".
    RequiresPython {
        /// The specifier exactly as the package declared it.
        required: String,
        /// The interpreter's version.
        found: String,
    },
    /// The specifier could not be parsed.
    ///
    /// Treated as compatible by [`check`] — an unreadable specifier is PipDock's problem, not the
    /// user's, and refusing an installable package would be the worse failure.
    Unparseable {
        /// The specifier that could not be read.
        specifier: String,
    },
}

impl Compatibility {
    /// True when the candidate may proceed.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        !matches!(self, Self::RequiresPython { .. })
    }
}

/// Evaluate a `Requires-Python` specifier set against an interpreter version.
///
/// `requires_python` is the raw metadata value, e.g. `">=3.7,<3.11"`. `None` or empty means the
/// package declares no constraint, which is always compatible.
///
/// Unknown or malformed specifiers yield [`Compatibility::Unparseable`], which
/// [`Compatibility::is_ok`] treats as passing: blocking an install because PipDock could not read
/// a specifier would turn a metadata oddity into a broken feature.
#[must_use]
pub fn check(requires_python: Option<&str>, env_version: &PyVersion) -> Compatibility {
    let Some(spec) = requires_python.map(str::trim).filter(|s| !s.is_empty()) else {
        return Compatibility::Compatible;
    };

    for clause in spec.split(',') {
        let clause = clause.trim();
        if clause.is_empty() {
            continue;
        }
        match satisfies_clause(clause, env_version) {
            Some(true) => {}
            Some(false) => {
                return Compatibility::RequiresPython {
                    required: spec.to_owned(),
                    found: env_version.to_string(),
                };
            }
            None => {
                return Compatibility::Unparseable {
                    specifier: spec.to_owned(),
                };
            }
        }
    }
    Compatibility::Compatible
}

/// `Some(true|false)` when the clause was understood, `None` when it was not.
fn satisfies_clause(clause: &str, env: &PyVersion) -> Option<bool> {
    use std::cmp::Ordering::{Equal, Greater, Less};

    // Longest operators first: `>=` must not be read as `>`.
    let (op, rest) = ["===", "==", "!=", ">=", "<=", "~=", ">", "<"]
        .iter()
        .find_map(|op| clause.strip_prefix(*op).map(|rest| (*op, rest.trim())))?;

    // Wildcards are only meaningful for == and !=.
    let is_wildcard = rest.ends_with(".*");
    let bare = if is_wildcard {
        rest.trim_end_matches(".*")
    } else {
        rest
    };
    let target = PyVersion::parse(bare).ok()?;

    Some(match op {
        "==" | "===" if is_wildcard => env.has_prefix(&target),
        "!=" if is_wildcard => !env.has_prefix(&target),
        "==" | "===" => env.cmp_padded(&target) == Equal,
        "!=" => env.cmp_padded(&target) != Equal,
        ">=" => matches!(env.cmp_padded(&target), Greater | Equal),
        "<=" => matches!(env.cmp_padded(&target), Less | Equal),
        ">" => env.cmp_padded(&target) == Greater,
        "<" => env.cmp_padded(&target) == Less,
        "~=" => {
            // PEP 440 compatible release: >= target, and equal on all but the last segment.
            if target.0.len() < 2 {
                return None; // `~=3` is invalid per PEP 440.
            }
            let prefix = PyVersion(target.0[..target.0.len() - 1].to_vec());
            matches!(env.cmp_padded(&target), Greater | Equal) && env.has_prefix(&prefix)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> PyVersion {
        PyVersion::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn the_scipy_case_from_sp2() {
        // The exact divergence that prompted this module: scipy 1.7.3 declares >=3.7,<3.11.
        // pip refuses it on Python 3.12; uv planned to install it. PipDock refuses, both engines.
        let got = check(Some(">=3.7,<3.11"), &v("3.12.10"));
        assert_eq!(
            got,
            Compatibility::RequiresPython {
                required: ">=3.7,<3.11".into(),
                found: "3.12.10".into()
            }
        );
        assert!(!got.is_ok());

        // ...and accepts it on an interpreter that does satisfy the range.
        assert_eq!(
            check(Some(">=3.7,<3.11"), &v("3.10.13")),
            Compatibility::Compatible
        );
    }

    #[test]
    fn no_constraint_is_always_compatible() {
        for spec in [None, Some(""), Some("   ")] {
            assert_eq!(
                check(spec, &v("3.12")),
                Compatibility::Compatible,
                "{spec:?}"
            );
        }
    }

    #[test]
    fn boundary_comparisons_pad_missing_segments() {
        // ">=3.8" must accept exactly 3.8.0 — an off-by-one here would reject a supported Python.
        assert!(check(Some(">=3.8"), &v("3.8.0")).is_ok());
        assert!(check(Some(">=3.8"), &v("3.8")).is_ok());
        assert!(!check(Some(">=3.8"), &v("3.7.17")).is_ok());

        // "<3.11" must reject exactly 3.11.0 and accept 3.10.x.
        assert!(!check(Some("<3.11"), &v("3.11.0")).is_ok());
        assert!(check(Some("<3.11"), &v("3.10.99")).is_ok());
    }

    #[test]
    fn wildcard_clauses() {
        assert!(!check(Some("!=3.9.*"), &v("3.9.7")).is_ok());
        assert!(check(Some("!=3.9.*"), &v("3.10.0")).is_ok());
        assert!(check(Some("==3.12.*"), &v("3.12.4")).is_ok());
        assert!(!check(Some("==3.12.*"), &v("3.13.0")).is_ok());
    }

    #[test]
    fn compatible_release_operator() {
        // ~=3.7 means >=3.7, ==3.*
        assert!(check(Some("~=3.7"), &v("3.12")).is_ok());
        assert!(!check(Some("~=3.7"), &v("4.0")).is_ok());
        assert!(!check(Some("~=3.7"), &v("3.6")).is_ok());
        // ~=3.7.2 means >=3.7.2, ==3.7.*
        assert!(check(Some("~=3.7.2"), &v("3.7.9")).is_ok());
        assert!(!check(Some("~=3.7.2"), &v("3.8.0")).is_ok());
        assert!(!check(Some("~=3.7.2"), &v("3.7.1")).is_ok());
    }

    #[test]
    fn multiple_clauses_must_all_hold() {
        let spec = Some(">=3.8,!=3.9.*,<4");
        assert!(check(spec, &v("3.8.1")).is_ok());
        assert!(!check(spec, &v("3.9.18")).is_ok());
        assert!(check(spec, &v("3.12.0")).is_ok());
        assert!(!check(spec, &v("4.0.0")).is_ok());
    }

    #[test]
    fn unreadable_specifiers_do_not_block_the_user() {
        // PipDock failing to read metadata must never be the reason an installable package is
        // refused — that would turn a metadata oddity into a broken feature.
        let got = check(Some(">=3.8 or maybe 3.9"), &v("3.12"));
        assert!(matches!(got, Compatibility::Unparseable { .. }));
        assert!(got.is_ok());
    }

    #[test]
    fn prerelease_interpreter_versions_parse_to_their_release() {
        assert_eq!(v("3.14.0rc1"), v("3.14.0"));
        assert_eq!(v("3.15.0b4"), v("3.15.0"));
        assert!(check(Some(">=3.14"), &v("3.14.0rc1")).is_ok());
    }

    #[test]
    fn malformed_versions_are_rejected_with_the_probe_code() {
        for bad in ["", "abc", "3..4", "3.x", "."] {
            let err = PyVersion::parse(bad).expect_err(&format!("{bad:?} must be rejected"));
            assert_eq!(err.code, Code::EnvProbeFailed, "{bad:?}");
        }
    }
}
