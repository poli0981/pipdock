//! PEP 440 at the boundary where text becomes argv (SECURITY §2).
//!
//! SECURITY §2 has always said names *and versions* are validated before they reach an argv array.
//! Names were — [`crate::model::PkgName`] is the only way one gets there — but versions were plain
//! strings: a dock-bay entry of `2.0 ; python_version<'0'` became one argv token pip accepted as a
//! requirement whose marker is false, so the install silently did nothing, and an imported
//! `django>=5.0,<6` failed outright because the flow split on `==` alone. This module is the one
//! place that decides what a version and a specifier set may look like.
//!
//! It is a **grammar check**, not an ordering: nothing here compares versions (that is
//! [`crate::compat`]'s job, and the engines'). It accepts what `packaging`'s `VERSION_PATTERN`
//! accepts — epochs, pre/post/dev segments in all their spellings, local labels — so a legitimate
//! version is never refused, and it rejects everything else, which is what keeps whitespace,
//! markers (`;`), URLs (`@`) and option-looking tokens out of argv.

/// Is `raw` a PEP 440 version (the permissive `VERSION_PATTERN`, case-insensitive)?
#[must_use]
pub fn is_version(raw: &str) -> bool {
    Parser::new(raw).version()
}

/// Is `raw` safe to put after `===` (PEP 440 "arbitrary equality")?
///
/// Installed metadata can carry versions that are not PEP 440 — pip still lists them — and the
/// guard set must restate them. `===` compares strings, so the only requirement is that the token
/// cannot be read as anything *but* a version: ASCII alphanumerics and `. _ + ! -`, starting with
/// an alphanumeric. No whitespace, `;`, `@`, quotes, commas or operators.
#[must_use]
pub fn is_argv_safe(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'!' | b'-'))
}

/// Is `raw` a PEP 440 specifier set: `op version` clauses separated by commas?
///
/// Whitespace is allowed around commas and between an operator and its version (`~= 1.4`), never
/// inside a version. `==` and `!=` may end in `.*`; `===` takes any [`is_argv_safe`] token. An
/// empty string is **not** a specifier set — callers that allow "any version" check for that
/// themselves, so an empty clause like `>=1,` cannot slip through as one.
#[must_use]
pub fn is_specifier_set(raw: &str) -> bool {
    !raw.trim().is_empty() && raw.split(',').all(is_clause)
}

/// The normalized form of a specifier set: whitespace removed around operators and commas.
///
/// Only call on something [`is_specifier_set`] accepted. `~= 1.4, <2` becomes `~=1.4,<2`, a single
/// argv-safe token.
#[must_use]
pub fn normalize_specifier_set(raw: &str) -> String {
    raw.split(',')
        .map(|clause| {
            let clause = clause.trim();
            let (op, rest) = split_operator(clause).unwrap_or(("", clause));
            format!("{op}{}", rest.trim())
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn is_clause(clause: &str) -> bool {
    let clause = clause.trim();
    let Some((op, version)) = split_operator(clause) else {
        return false;
    };
    let version = version.trim();
    if version.is_empty() || version.contains(char::is_whitespace) {
        return false;
    }
    match op {
        "===" => is_argv_safe(version),
        "==" | "!=" => {
            // `==1.4.*` is a prefix match; the wildcard may only replace the final release part.
            version
                .strip_suffix(".*")
                .map_or_else(|| is_version(version), is_release_prefix)
        }
        // `~=` needs at least two release segments (PEP 440); the grammar check is enough here
        // and the engine will say so if it disagrees.
        _ => is_version(version),
    }
}

fn split_operator(clause: &str) -> Option<(&'static str, &str)> {
    // Longest first, so `===` is not read as `==` followed by `=1.0`.
    ["===", "==", "!=", "<=", ">=", "~=", "<", ">"]
        .into_iter()
        .find_map(|op| clause.strip_prefix(op).map(|rest| (op, rest)))
}

/// `1`, `1.4`, `1!2.0` — what may precede `.*` in a prefix match.
fn is_release_prefix(raw: &str) -> bool {
    let mut p = Parser::new(raw);
    p.epoch();
    p.release() && p.done()
}

/// A recursive-descent reader over `packaging`'s `VERSION_PATTERN`, lowercased.
struct Parser {
    s: Vec<u8>,
    at: usize,
}

impl Parser {
    /// Not trimmed: a version is one argv token, and `" 1.0"` is not one. Callers that read a
    /// clause with spaces around it (`~= 1.4`) trim the clause, not the version inside it.
    fn new(raw: &str) -> Self {
        Self {
            s: raw.to_ascii_lowercase().into_bytes(),
            at: 0,
        }
    }

    fn done(&self) -> bool {
        self.at == self.s.len()
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn eat_word(&mut self, words: &[&str]) -> bool {
        // Longest first is the caller's job; `words` is ordered that way below.
        for w in words {
            if self.s[self.at..].starts_with(w.as_bytes()) {
                self.at += w.len();
                return true;
            }
        }
        false
    }

    fn digits(&mut self) -> bool {
        let start = self.at;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.at += 1;
        }
        self.at > start
    }

    fn sep(&mut self) {
        if matches!(self.peek(), Some(b'-' | b'_' | b'.')) {
            self.at += 1;
        }
    }

    /// `(N!)?` — only consumed when the digits really are followed by `!`.
    fn epoch(&mut self) {
        let save = self.at;
        if !(self.digits() && self.eat(b'!')) {
            self.at = save;
        }
    }

    /// `N(.N)*`
    fn release(&mut self) -> bool {
        if !self.digits() {
            return false;
        }
        loop {
            let save = self.at;
            if self.eat(b'.') && self.digits() {
                continue;
            }
            self.at = save;
            return true;
        }
    }

    /// An optional segment `[-_.]? label [-_.]? N?`; restores the position when absent.
    fn labelled(&mut self, labels: &[&str]) -> bool {
        let save = self.at;
        self.sep();
        if self.eat_word(labels) {
            let after = self.at;
            self.sep();
            if !self.digits() {
                // The separator after a label is only part of the segment when digits follow.
                self.at = after;
            }
            true
        } else {
            self.at = save;
            false
        }
    }

    fn post(&mut self) {
        let save = self.at;
        // `-N` is an implicit post-release.
        if self.eat(b'-') && self.digits() {
            return;
        }
        self.at = save;
        self.labelled(&["post", "rev", "r"]);
    }

    fn local(&mut self) -> bool {
        if !self.eat(b'+') {
            return true;
        }
        let part = |p: &mut Self| {
            let start = p.at;
            while p.peek().is_some_and(|b| b.is_ascii_alphanumeric()) {
                p.at += 1;
            }
            p.at > start
        };
        if !part(self) {
            return false;
        }
        while matches!(self.peek(), Some(b'-' | b'_' | b'.')) {
            self.at += 1;
            if !part(self) {
                return false;
            }
        }
        true
    }

    fn version(mut self) -> bool {
        if self.s.is_empty() {
            return false;
        }
        self.eat(b'v');
        self.epoch();
        if !self.release() {
            return false;
        }
        self.labelled(&["alpha", "beta", "preview", "pre", "rc", "a", "b", "c"]);
        self.post();
        self.labelled(&["dev"]);
        self.local() && self.done()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_versions_are_accepted_in_every_spelling() {
        for v in [
            "1",
            "1.0",
            "2.32.3",
            "1!2.0",
            "1.0a1",
            "1.0.alpha.1",
            "1.0-beta2",
            "1.0rc1",
            "1.0c3",
            "1.0.post1",
            "1.0-1",
            "1.0.rev2",
            "1.0.dev0",
            "1.0a1.dev2",
            "1.0+ubuntu.1",
            "1.0+abc-def_ghi",
            "v1.2",
            "2026.10.16",
            "1.0.post",
            "1.0.dev",
        ] {
            assert!(is_version(v), "{v} is PEP 440");
        }
    }

    #[test]
    fn anything_that_is_not_a_version_is_refused() {
        for v in [
            "",
            " ",
            "2.0 ; python_version<'0'",
            "1.0 --break-system-packages",
            "1.0@https://evil",
            "latest",
            "1..0",
            "1.0+",
            "1.0+a..b",
            ">=1.0",
            "1.0,2.0",
            "1.0 2.0",
            "'1.0'",
            "1.0;",
            "SNAPSHOT",
            " 1.0",
            "1.0 ",
        ] {
            assert!(!is_version(v), "{v:?} must not pass as a version");
        }
    }

    #[test]
    fn specifier_sets_accept_what_requirements_files_contain() {
        for s in [
            "==2.32.3",
            ">=5.0,<6",
            ">=5.0, <6",
            "~= 1.4",
            "!=1.0.*",
            "==1.4.*",
            "===1.0-legacy",
            ">1,!=1.5,<2",
            "<=2026.1",
        ] {
            assert!(is_specifier_set(s), "{s:?} is a specifier set");
        }
    }

    #[test]
    fn specifier_sets_refuse_prose_markers_urls_and_options() {
        for s in [
            "",
            ">=1,",
            "is not a requirement",
            "==1.0 ; python_version<'3'",
            "==1.0 --break-system-packages",
            "@ https://example.com/x.whl",
            "==",
            "1.0",
            ">=1.0 <2",
            "==1.*.0",
            "~=1.0.*",
            "===has space",
        ] {
            assert!(!is_specifier_set(s), "{s:?} must be refused");
        }
    }

    #[test]
    fn normalizing_removes_only_the_whitespace_pep_440_allows() {
        assert_eq!(normalize_specifier_set("~= 1.4, < 2"), "~=1.4,<2");
        assert_eq!(normalize_specifier_set(">=5.0,<6"), ">=5.0,<6");
    }

    #[test]
    fn arbitrary_equality_takes_a_legacy_version_but_nothing_that_could_be_more() {
        assert!(is_argv_safe("1.0-SNAPSHOT"));
        assert!(is_argv_safe("2.0.0.final"));
        assert!(!is_argv_safe("1.0 ; x"));
        assert!(!is_argv_safe("-1.0"));
        assert!(!is_argv_safe("1.0@x"));
        assert!(!is_argv_safe(""));
    }
}
