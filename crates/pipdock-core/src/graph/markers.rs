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
//! `python_version` and `python_full_version` compare as versions. Since probe schema 3 the
//! platform markers — `sys_platform`, `os_name`, `platform_system`, `platform_machine`, … — are
//! known too, and compare as strings with `==`, `!=`, `in` and `not in`. They were left out at
//! first as "constants for a Windows-only v1", and that was the bug: a constant still has to be
//! *read*, and unread, ipython's `pexpect; sys_platform != "win32"` was in force on Windows, so
//! the dependency view reported pexpect missing from every Windows environment with ipython in
//! it (H25).
//!
//! **An unrecognised marker keeps the requirement.** Over-reporting a constraint is noise;
//! dropping one hides the reason a package is stuck, and ARCHITECTURE §3 would rather show
//! constraints without a culprit than quietly omit them.

use std::collections::BTreeMap;

use crate::compat::PyVersion;

/// The interpreter facts a marker can be evaluated against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerEnv {
    /// `sys.version_info[:2]` joined with a dot — what PEP 508 calls `python_version`.
    python_version: PyVersion,
    /// The full interpreter version — PEP 508's `python_full_version`.
    python_full_version: PyVersion,
    /// Every variable known as a string: the probe's marker environment and the two Python
    /// versions. A variable missing from here is unknown, and a requirement gated on it stays in
    /// force.
    vars: BTreeMap<String, String>,
}

impl MarkerEnv {
    /// Build from `PyEnv::python_version`, e.g. `"3.12.10"`, knowing nothing about the platform.
    ///
    /// Returns `None` for anything unparseable, which leaves marker evaluation switched off
    /// rather than guessing — the conservative direction.
    #[must_use]
    pub fn from_python_version(raw: &str) -> Option<Self> {
        Self::from_probe(raw, &BTreeMap::new())
    }

    /// Build from the interpreter version and the probe's marker environment (probe schema 3).
    #[must_use]
    pub fn from_probe(raw: &str, markers: &BTreeMap<String, String>) -> Option<Self> {
        let full = PyVersion::parse(raw).ok()?;
        // `python_version` is major.minor only. Keeping the patch would make the common
        // `python_version == "3.12"` false on 3.12.10, which is backwards.
        let mut parts = raw.split('.');
        let major = parts.next()?;
        let minor = parts.next().unwrap_or("0");
        let short_raw = format!("{major}.{minor}");
        let short = PyVersion::parse(&short_raw).ok()?;
        let mut vars = markers.clone();
        vars.entry("python_version".to_owned()).or_insert(short_raw);
        vars.entry("python_full_version".to_owned())
            .or_insert_with(|| raw.to_owned());
        Some(Self {
            python_version: short,
            python_full_version: full,
            vars,
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

    // PEP 508's containment, for any variable: `lhs in rhs` asks whether lhs occurs in rhs.
    if op == "in" || op == "not in" {
        let value = env.vars.get(var)?;
        let found = if flipped {
            value.contains(literal)
        } else {
            literal.contains(value.as_str())
        };
        return Some(found == (op == "in"));
    }

    let version = match var {
        "python_version" => Some(&env.python_version),
        "python_full_version" => Some(&env.python_full_version),
        _ => None,
    };
    if let Some(version) = version {
        // `"3.11" > python_version` means the same as `python_version < "3.11"`.
        let op = if flipped { mirror(op) } else { op };
        return crate::compat::satisfies_clause(&format!("{op}{literal}"), version);
    }

    // Everything else compares as a string. Ordering a platform name is not something a
    // requirement can mean, so `<` and friends stay unevaluated.
    let value = env.vars.get(var)?;
    match op {
        "==" | "===" => Some(value == literal),
        "!=" => Some(value != literal),
        _ => None,
    }
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

    /// Whitespace is skipped by **character**, not by byte. `at += 1` stepped into the middle of
    /// a no-break space (two bytes) or an ideographic space (three), and the next slice panicked —
    /// on a marker read out of some installed package's metadata (H19).
    fn skip_ws(&mut self) {
        let rest = &self.src[self.at..];
        self.at += rest.len() - rest.trim_start().len();
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
        // Prefix first, then look past it: slicing `word.len()` bytes in before knowing the text
        // starts with `word` cut a multi-byte character in half and panicked.
        let Some(after) = self.src[self.at..].strip_prefix(word) else {
            return false;
        };
        // Must be a whole word: `android_api` must not be read as the operator `and`.
        if after
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_')
        {
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
        // The word operators, PEP 508's string containment.
        if self.eat_word("in") {
            return Some("in");
        }
        let before = self.at;
        if self.eat_word("not") && self.eat_word("in") {
            return Some("not in");
        }
        self.at = before;
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

    fn windows(v: &str) -> MarkerEnv {
        // What probe schema 3 reported for CPython 3.14.7 on Windows 11, x64.
        let markers = [
            ("implementation_name", "cpython"),
            ("os_name", "nt"),
            ("platform_machine", "AMD64"),
            ("platform_python_implementation", "CPython"),
            ("platform_system", "Windows"),
            ("sys_platform", "win32"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        MarkerEnv::from_probe(v, &markers).expect("parses")
    }

    #[test]
    fn a_requirement_gated_to_another_platform_is_not_in_force() {
        // ipython's pexpect, read on Windows: reported missing from every Windows environment
        // that had ipython, until the platform markers were read.
        let e = windows("3.14.7");
        assert!(!applies(
            Some(r#"sys_platform != "win32" and sys_platform != "emscripten""#),
            Some(&e)
        ));
        assert!(applies(Some(r#"sys_platform == "win32""#), Some(&e)));
        assert!(applies(Some(r#"platform_system == "Windows""#), Some(&e)));
        assert!(!applies(Some(r#"os_name == "posix""#), Some(&e)));
        assert!(applies(
            Some(r#"platform_machine == "x86_64" or platform_machine == "AMD64""#),
            Some(&e)
        ));
        assert!(!applies(
            Some(r#"platform_python_implementation == "PyPy""#),
            Some(&e)
        ));
    }

    #[test]
    fn containment_reads_in_both_directions() {
        let e = windows("3.14.7");
        assert!(!applies(Some(r#""linux" in sys_platform"#), Some(&e)));
        assert!(applies(Some(r#""win" in sys_platform"#), Some(&e)));
        assert!(applies(Some(r#"sys_platform in "win32 cygwin""#), Some(&e)));
        assert!(applies(
            Some(r#"sys_platform not in "linux darwin""#),
            Some(&e)
        ));
        assert!(!applies(
            Some(r#"python_version not in "3.13 3.14""#),
            Some(&e)
        ));
    }

    #[test]
    fn what_the_probe_did_not_report_still_keeps_the_requirement() {
        // Windows probes leave out platform_release (a WMI query); ordering a platform name means
        // nothing. Both stay "applies".
        let e = windows("3.14.7");
        assert!(applies(Some(r#"platform_release >= "10""#), Some(&e)));
        assert!(applies(Some(r#"sys_platform > "a""#), Some(&e)));
    }

    #[test]
    fn unicode_whitespace_never_panics_the_lexer() {
        // Markers come from any installed package's metadata. Each of these sliced a multi-byte
        // character in half.
        let e = windows("3.14.7");
        for marker in [
            "python_version\u{a0}>= \"3.8\"",
            "\u{3000}python_version >= \"3.8\"",
            "python_version >= \"3.8\"\u{2003}and\u{2003}os_name == \"nt\"",
            "é",
            "andé",
            "python_version >= \"3.8\" ändé",
            "\u{a0}",
        ] {
            let _ = applies(Some(marker), Some(&e));
            let _ = applies(Some(marker), None);
        }
        assert!(applies(
            Some("python_version\u{a0}>=\u{a0}\"3.8\""),
            Some(&e)
        ));
    }

    #[test]
    fn an_unparseable_interpreter_version_disables_evaluation() {
        assert_eq!(MarkerEnv::from_python_version("not-a-version"), None);
        assert_eq!(MarkerEnv::from_python_version(""), None);
    }
}
