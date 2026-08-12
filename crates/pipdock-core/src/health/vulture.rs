//! vulture: dead code, parsed out of text because there is no other option.
//!
//! vulture has **no machine-readable output** — no JSON, no SARIF. CODE-HEALTH-SPEC §4 calls the
//! text format stable and it is, but "stable" is a property of the current release, so the fixture
//! corpus rather than this comment is what keeps a Dependabot bump from reaching a user.

use super::report::VultureFinding;

/// Parse vulture's text report.
///
/// One finding per line: `<path>:<line>: <message> (<NN>% confidence)`. Paths on Windows contain
/// `\` and drive letters contain `:`, so the line number is found by splitting from the **right**
/// rather than the left — `C:\proj\a.py:12:` has three colons and only the last-but-one is the one
/// that matters.
///
/// A line that does not match is skipped rather than failing the run. vulture prints its own
/// notices (an unreadable file, a bad `--exclude` glob) into the same stream, and losing eight real
/// findings because a ninth line was a warning would be the worse trade.
///
/// # Errors
/// Never. An unparseable report yields no findings; the caller decides whether the exit code made
/// that a failure.
pub fn parse(stdout: &str) -> Vec<VultureFinding> {
    stdout.lines().filter_map(parse_line).collect()
}

/// One `<path>:<line>: <message> (<NN>% confidence)` line, or `None`.
fn parse_line(line: &str) -> Option<VultureFinding> {
    let line = line.trim_end();

    // Confidence first: it anchors the right-hand end and its absence is the cheapest way to
    // recognize a line that is not a finding at all.
    let open = line.rfind('(')?;
    let confidence: u8 = line
        .get(open + 1..)?
        .trim_end_matches(')')
        .trim_end_matches("% confidence")
        .trim()
        .parse()
        .ok()?;

    let head = line.get(..open)?.trim_end();
    // `path:line: message` — split at the *second* colon from the left of the message, which is
    // the first `: ` after the line number. Find it by walking back from the message instead.
    let (locus, message) = head.split_once(": ")?;
    let (path, line_no) = locus.rsplit_once(':')?;
    let line_no: u32 = line_no.trim().parse().ok()?;

    Some(VultureFinding {
        path: path.to_owned(),
        line: line_no,
        message: message.trim().to_owned(),
        name: identifier(message),
        confidence,
    })
}

/// The identifier a message names, when it names one.
///
/// Seven of vulture's eight message kinds are `unused <typ> '<name>'`. The eighth is
/// `unreachable code after '<token>'`, where the quoted text is the *token the code follows*, not
/// a dead identifier — so quoting alone is not enough to decide, and this returns `None` there
/// rather than reporting a `return` as an unused name.
fn identifier(message: &str) -> Option<String> {
    if !message.starts_with("unused ") {
        return None;
    }
    let open = message.find('\'')?;
    let rest = message.get(open + 1..)?;
    let close = rest.find('\'')?;
    Some(rest.get(..close)?.to_owned())
}

/// Exit codes vulture uses (`vulture/utils.py`).
///
/// **3 means it found dead code**, which is a successful run with something to say. 1 and 2 are
/// real failures. This is the tool whose codes are least like the other two.
pub const EXIT_NO_DEAD_CODE: i32 = 0;
/// Dead code found — a successful run.
pub const EXIT_DEAD_CODE: i32 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from vulture 2.16 over a project with all eight kinds represented.
    const REAL: &str = include_str!("../../tests/fixtures/health/vulture/dead-code.txt");

    #[test]
    fn the_real_report_parses_every_line() {
        let findings = parse(REAL);
        assert_eq!(
            findings.len(),
            REAL.lines().filter(|l| !l.trim().is_empty()).count()
        );
    }

    #[test]
    fn a_named_finding_carries_its_identifier() {
        let findings = parse("pkg\\app.py:1: unused import 'os' (90% confidence)");

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].path, "pkg\\app.py");
        assert_eq!(findings[0].line, 1);
        assert_eq!(findings[0].name.as_deref(), Some("os"));
        assert_eq!(findings[0].confidence, 90);
    }

    /// The eighth message shape, and the reason `name` is optional.
    #[test]
    fn unreachable_code_reports_no_identifier() {
        let findings = parse("pkg\\app.py:12: unreachable code after 'return' (100% confidence)");

        assert_eq!(findings[0].confidence, 100);
        assert_eq!(
            findings[0].name, None,
            "'return' is the token the code follows, not a dead identifier"
        );
        assert!(findings[0].message.starts_with("unreachable code"));
    }

    /// The bug a left-to-right split would ship on every Windows machine.
    #[test]
    fn a_windows_path_with_a_drive_letter_still_finds_the_line_number() {
        let findings =
            parse(r"C:\proj\pkg\app.py:88: unused function 'old_parse' (60% confidence)");

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].path, r"C:\proj\pkg\app.py");
        assert_eq!(findings[0].line, 88);
        assert_eq!(findings[0].name.as_deref(), Some("old_parse"));
    }

    #[test]
    fn a_line_that_is_not_a_finding_is_skipped_rather_than_fatal() {
        // vulture prints its own notices into the same stream. Losing every real finding because
        // one line was a warning is the worse trade.
        let findings = parse(
            "some warning vulture felt like printing\n\
             pkg\\app.py:1: unused import 'os' (90% confidence)\n\
             \n",
        );

        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn no_dead_code_is_an_empty_report() {
        assert!(parse(include_str!("../../tests/fixtures/health/vulture/none.txt")).is_empty());
    }

    #[test]
    fn confidence_survives_being_a_hundred() {
        // A `u8` holds it, but a two-digit assumption anywhere would truncate.
        let findings = parse("a.py:1: unreachable code after 'return' (100% confidence)");
        assert_eq!(findings[0].confidence, 100);
    }
}
