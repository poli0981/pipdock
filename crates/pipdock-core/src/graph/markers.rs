//! Just enough PEP 508 environment-marker evaluation to stop the preview lying.
//!
//! Found by the SP-5 dogfood. On Python 3.12, `pipdock update --all` explained a held-back numpy
//! with eight constraints, four of which do not apply to that interpreter:
//!
//! ```text
//!   numpy 1.26.4 (latest 2.5.1)
//!       pandas 2.1.4 requires numpy <2,>=1.22.4     <- python_version < "3.11"
//!       pandas 2.1.4 requires numpy <2,>=1.23.2     <- python_version == "3.11"
//!       pandas 2.1.4 requires numpy <2,>=1.26.0     <- the only one that applies
//!       statsmodels 0.14.1 requires numpy <2,>=1.22.3   <- python_version == "3.10"
//! ```
//!
//! Requirements carry a marker, and until now only `extra ==` was honoured, so every
//! marker-gated branch of a dependency was reported as if it were in force. That is not merely
//! noisy: telling someone on 3.12 that "pandas requires numpy >=1.22.4" is false, and
//! ERROR-CATALOG's whole premise is that what PipDock says about a failure can be trusted.
//!
//! Scope is deliberately narrow. Only `python_version` and `python_full_version` are evaluated,
//! because those are the markers that actually gate version constraints in the wild, and the
//! environment already knows its interpreter version. The platform markers are constants for a
//! Windows-only v1 (PRD non-goals) and reading them would mean plumbing more out of `probe.py`
//! for no present gain.
//!
//! **An unrecognised marker keeps the requirement.** Over-reporting a constraint is noise;
//! dropping one hides the reason a package is stuck, and ARCHITECTURE §3 would rather show
//! constraints without a culprit than quietly omit them.

use crate::compat::PyVersion;

/// The interpreter facts a marker can be evaluated against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerEnv {
    /// `sys.version_info[:2]` joined with a dot — what PEP 508 calls `python_version`.
    python_version: PyVersion,
    /// The full interpreter version — PEP 508's `python_full_version`.
    python_full_version: PyVersion,
}

impl MarkerEnv {
    /// Build from `PyEnv::python_version`, e.g. `"3.12.10"`.
    ///
    /// Returns `None` for anything unparseable, which leaves marker evaluation switched off
    /// rather than guessing — the conservative direction.
    #[must_use]
    pub fn from_python_version(raw: &str) -> Option<Self> {
        let full = PyVersion::parse(raw).ok()?;
        // `python_version` is major.minor only. Keeping the patch would make the common
        // `python_version == "3.12"` false on 3.12.10, which is backwards.
        let mut parts = raw.split('.');
        let major = parts.next()?;
        let minor = parts.next().unwrap_or("0");
        let short = PyVersion::parse(&format!("{major}.{minor}")).ok()?;
        Some(Self {
            python_version: short,
            python_full_version: full,
        })
    }
}

/// Whether a requirement carrying `marker` applies in `env`.
///
/// `None` for `env` means "no interpreter known", which switches evaluation off entirely except
/// for `extra`, preserving the behaviour every caller had before markers were understood.
#[must_use]
pub fn applies(marker: Option<&str>, env: Option<&MarkerEnv>) -> bool {
    let Some(marker) = marker else {
        return true;
    };
    // Unparsed or unrecognised evaluates to "applies": see the module note.
    eval(&mut Lexer::new(marker), env)
        .unwrap_or(Some(true))
        .unwrap_or(true)
}

// -- evaluation ---------------------------------------------------------------------------------

/// `Some(v)` when the expression was understood, `None` when it was not.
type Verdict = Option<bool>;

/// `and` binds tighter than `or`; parentheses override both.
fn eval(lx: &mut Lexer<'_>, env: Option<&MarkerEnv>) -> Result<Verdict, ()> {
    let mut acc = eval_and(lx, env)?;
    while lx.eat_word("or") {
        let rhs = eval_and(lx, env)?;
        acc = match (acc, rhs) {
            // One true arm settles an `or` even if the other is unreadable.
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        };
    }
    Ok(acc)
}

fn eval_and(lx: &mut Lexer<'_>, env: Option<&MarkerEnv>) -> Result<Verdict, ()> {
    let mut acc = eval_atom(lx, env)?;
    while lx.eat_word("and") {
        let rhs = eval_atom(lx, env)?;
        acc = match (acc, rhs) {
            // One false arm settles an `and`. This is what rules out
            // `python_version == "3.10" and platform_system == "Windows"` on 3.12 without
            // needing to know anything about the platform.
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        };
    }
    Ok(acc)
}

fn eval_atom(lx: &mut Lexer<'_>, env: Option<&MarkerEnv>) -> Result<Verdict, ()> {
    if lx.eat_char('(') {
        let inner = eval(lx, env)?;
        if !lx.eat_char(')') {
            return Err(());
        }
        return Ok(inner);
    }

    let lhs = lx.operand().ok_or(())?;
    let op = lx.operator().ok_or(())?;
    let rhs = lx.operand().ok_or(())?;
    Ok(compare(&lhs, op, &rhs, env))
}

fn compare(lhs: &Operand, op: &str, rhs: &Operand, env: Option<&MarkerEnv>) -> Verdict {
    // An extra-gated requirement is not in force: the extra is not installed unless it was asked
    // for, and this is the rule the uninstall guard has always relied on.
    if lhs.is_var("extra") || rhs.is_var("extra") {
        return Some(false);
    }

    let env = env?;
    let (var, literal, flipped) = match (lhs, rhs) {
        (Operand::Var(v), Operand::Str(s)) => (v.as_str(), s.as_str(), false),
        (Operand::Str(s), Operand::Var(v)) => (v.as_str(), s.as_str(), true),
        _ => return None,
    };

    let version = match var {
        "python_version" => &env.python_version,
        "python_full_version" => &env.python_full_version,
        _ => return None,
    };

    // `"3.11" > python_version` means the same as `python_version < "3.11"`.
    let op = if flipped { mirror(op) } else { op };
    crate::compat::satisfies_clause(&format!("{op}{literal}"), version)
}

fn mirror(op: &str) -> &str {
    match op {
        "<" => ">",
        "<=" => ">=",
        ">" => "<",
        ">=" => "<=",
        other => other,
    }
}

// -- lexing -------------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Operand {
    Var(String),
    Str(String),
}

impl Operand {
    fn is_var(&self, name: &str) -> bool {
        matches!(self, Self::Var(v) if v == name)
    }
}

struct Lexer<'a> {
    src: &'a str,
    at: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, at: 0 }
    }

    fn skip_ws(&mut self) {
        while self.src[self.at..].starts_with(char::is_whitespace) {
            self.at += 1;
        }
    }

    fn eat_char(&mut self, c: char) -> bool {
        self.skip_ws();
        if self.src[self.at..].starts_with(c) {
            self.at += c.len_utf8();
            return true;
        }
        false
    }

    fn eat_word(&mut self, word: &str) -> bool {
        self.skip_ws();
        let rest = &self.src[self.at..];
        // Must be a whole word: `android_api` must not be read as the operator `and`.
        let ends_cleanly = rest[word.len().min(rest.len())..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_');
        if rest.starts_with(word) && ends_cleanly {
            self.at += word.len();
            return true;
        }
        false
    }

    fn operand(&mut self) -> Option<Operand> {
        self.skip_ws();
        let rest = &self.src[self.at..];
        if let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') {
            let end = rest[1..].find(quote)? + 1;
            self.at += end + 1;
            return Some(Operand::Str(rest[1..end].to_owned()));
        }
        let end = rest
            .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
            .unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        self.at += end;
        Some(Operand::Var(rest[..end].to_owned()))
    }

    fn operator(&mut self) -> Option<&'static str> {
        self.skip_ws();
        let rest = &self.src[self.at..];
        // Longest first: `>=` must not be read as `>`.
        for op in ["===", "==", "!=", ">=", "<=", "~=", ">", "<"] {
            if rest.starts_with(op) {
                self.at += op.len();
                return Some(op);
            }
        }
        // `in` / `not in` are string-containment operators PipDock does not evaluate; letting
        // them fall through to "unrecognised" keeps the requirement.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(v: &str) -> MarkerEnv {
        MarkerEnv::from_python_version(v).expect("parses")
    }

    #[test]
    fn the_sp5_dogfood_case_is_resolved() {
        // The four lines that made the held-back explanation false on Python 3.12.
        let e = env("3.12.10");
        assert!(!applies(Some(r#"python_version < "3.11""#), Some(&e)));
        assert!(!applies(Some(r#"python_version == "3.11""#), Some(&e)));
        assert!(applies(Some(r#"python_version >= "3.12""#), Some(&e)));
        assert!(!applies(
            Some(
                r#"python_version == "3.10" and platform_system == "Windows" and platform_python_implementation != "PyPy""#
            ),
            Some(&e)
        ));
    }

    #[test]
    fn a_dotted_release_still_matches_its_minor() {
        // PEP 508's python_version is major.minor, so this must hold on 3.12.10. Comparing the
        // full version instead would make it false, which is the trap this exists to avoid.
        assert!(applies(
            Some(r#"python_version == "3.12""#),
            Some(&env("3.12.10"))
        ));
        assert!(applies(
            Some(r#"python_full_version == "3.12.10""#),
            Some(&env("3.12.10"))
        ));
        assert!(!applies(
            Some(r#"python_full_version == "3.12""#),
            Some(&env("3.12.10"))
        ));
    }

    #[test]
    fn extras_are_never_in_force() {
        let e = env("3.12.10");
        assert!(!applies(Some(r#"extra == "docs""#), Some(&e)));
        assert!(!applies(Some(r#"extra == "socks""#), None));
        // Even combined with a marker that does apply.
        assert!(!applies(
            Some(r#"python_version >= "3.8" and extra == "test""#),
            Some(&e)
        ));
    }

    #[test]
    fn an_unreadable_marker_keeps_the_requirement() {
        // Dropping a constraint hides the reason a package is stuck; showing a spare one is only
        // noise. The module note argues this direction.
        let e = env("3.12.10");
        assert!(applies(Some("platform_system == \"Windows\""), Some(&e)));
        assert!(applies(Some("sys_platform != \"win32\""), Some(&e)));
        assert!(applies(Some("os_name in \"posix\""), Some(&e)));
        assert!(applies(Some("this is not a marker at all"), Some(&e)));
        assert!(applies(Some(r#"python_version < "3.11""#), None));
        assert!(applies(None, Some(&e)));
    }

    #[test]
    fn or_and_parentheses_bind_as_python_does() {
        let e = env("3.12.10");
        assert!(applies(
            Some(r#"python_version < "3.9" or python_version >= "3.12""#),
            Some(&e)
        ));
        assert!(!applies(
            Some(r#"python_version < "3.9" or python_version == "3.10""#),
            Some(&e)
        ));
        // `and` binds tighter, so this is `false or (true and true)`.
        assert!(applies(
            Some(
                r#"python_version == "3.10" or python_version >= "3.12" and python_version < "4""#
            ),
            Some(&e)
        ));
        assert!(!applies(
            Some(
                r#"(python_version == "3.10" or python_version >= "3.12") and python_version < "3.11""#
            ),
            Some(&e)
        ));
    }

    #[test]
    fn a_reversed_comparison_reads_the_same_as_its_mirror() {
        let e = env("3.12.10");
        assert!(applies(Some(r#""3.11" < python_version"#), Some(&e)));
        assert!(!applies(Some(r#""3.11" > python_version"#), Some(&e)));
    }

    #[test]
    fn a_variable_starting_with_and_is_not_the_operator() {
        // `eat_word` must not split `android_api`; if it does the parse desyncs and the marker
        // silently becomes unreadable.
        let e = env("3.12.10");
        let mut lx = Lexer::new("android_api == \"21\"");
        assert!(!lx.eat_word("and"));
        assert!(applies(Some("android_api == \"21\""), Some(&e)));
    }

    #[test]
    fn an_unparseable_interpreter_version_disables_evaluation() {
        assert_eq!(MarkerEnv::from_python_version("not-a-version"), None);
        assert_eq!(MarkerEnv::from_python_version(""), None);
    }
}
