//! The bug-report deep link — ERROR-CATALOG §4.
//!
//! Owner requirement: the console error is attached to the issue template. The whole of PipDock's
//! "telemetry" story is this module — a URL, printed or opened, that the user reviews and submits
//! themselves. **Nothing is ever sent automatically**, and nothing here performs a request.
//!
//! It lives in core rather than in either head because both build the same link. `pipdock self
//! report-bug` printed one and the GUI needed another; two builders means two URLs that agree
//! until the day the template gains a field, and the CLI's version is the one nobody would notice
//! had drifted.
//!
//! § 4.3's budget is the interesting constraint: GitHub rejects very long URLs, so the excerpt is
//! truncated to [`crate::BUG_REPORT_EXCERPT_CHARS`] characters, **tail-biased** — the end of a log
//! is where the failure is, and the head is startup noise. The full log is offered to the
//! clipboard separately, which is what makes the truncation acceptable rather than lossy.

use crate::model::EngineId;

/// Where the template lives. Not configurable: an issue URL a caller can set is a phishing vector.
const ISSUE_URL: &str = "https://github.com/poli0981/pipdock/issues/new?template=bug_report.yml";

/// What a head knows about the failure it is reporting.
#[derive(Debug, Clone, Default)]
pub struct BugReport {
    /// The environment's Python version, when one is selected.
    pub python: Option<String>,
    /// The active engine.
    pub engine: Option<EngineId>,
    /// Its version, when it could be read.
    pub engine_version: Option<String>,
    /// The catalog code being reported, when the report is about one failure.
    pub code: Option<crate::errors::Code>,
    /// Engine output. Truncated by [`excerpt`] before it reaches the URL.
    pub log: String,
}

/// Build the prefilled issue URL (ERROR-CATALOG §4.2).
#[must_use]
pub fn bug_report_url(report: &BugReport, os: &str) -> String {
    let mut url = format!(
        "{ISSUE_URL}&pd-version={}&os={}",
        urlencode(env!("CARGO_PKG_VERSION")),
        urlencode(os),
    );
    if let Some(engine) = report.engine {
        url.push_str(&format!("&engine={}", urlencode(engine.as_str())));
    }
    if let Some(python) = report.python.as_deref().filter(|p| !p.is_empty()) {
        let engine_note = match (report.engine, report.engine_version.as_deref()) {
            (Some(id), Some(v)) if !v.is_empty() => format!(" · {} {v}", id.as_str()),
            _ => String::new(),
        };
        url.push_str(&format!(
            "&python={}",
            urlencode(&format!("Python {python}{engine_note}"))
        ));
    }
    if let Some(code) = report.code {
        url.push_str(&format!("&error-code={}", urlencode(code.as_str())));
    }

    let excerpt = excerpt(&report.log);
    if !excerpt.is_empty() {
        url.push_str(&format!("&log-excerpt={}", urlencode(&excerpt)));
    }
    url
}

/// The tail of `log`, at most [`crate::BUG_REPORT_EXCERPT_CHARS`] characters.
///
/// Tail-biased on purpose: a failing install's last twenty lines are the traceback, and its first
/// hundred are the resolver saying what it downloaded. Cut at a **line** boundary where one is
/// available, so the excerpt does not open mid-token and read as corruption — and at a `char`
/// boundary regardless, because slicing a `String` by bytes panics on the first non-ASCII path.
#[must_use]
pub fn excerpt(log: &str) -> String {
    let limit = crate::BUG_REPORT_EXCERPT_CHARS;
    if log.chars().count() <= limit {
        return log.to_owned();
    }

    let skip = log.chars().count() - limit;
    let tail: String = log.chars().skip(skip).collect();
    // Prefer starting after the first newline in the window; if the tail is one enormous line,
    // take it as it is rather than returning nothing.
    match tail.find('\n') {
        Some(i) if i + 1 < tail.len() => tail[i + 1..].to_owned(),
        _ => tail,
    }
}

/// Percent-encode for a query parameter.
fn urlencode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_url_carries_what_the_template_asks_for() {
        let url = bug_report_url(
            &BugReport {
                python: Some("3.12.4".to_owned()),
                engine: Some(EngineId::Pip),
                engine_version: Some("26.1.2".to_owned()),
                code: Some(crate::errors::Code::BldBackendFailed),
                log: "boom".to_owned(),
            },
            "Windows 11",
        );

        assert!(url.starts_with(ISSUE_URL));
        assert!(url.contains("&error-code=PD-BLD-002"), "{url}");
        assert!(url.contains("&engine=pip"), "{url}");
        assert!(url.contains("&log-excerpt=boom"), "{url}");
        // Spaces are `+`, not literal — a raw space makes the whole link unclickable in a terminal.
        assert!(url.contains("os=Windows+11"), "{url}");
    }

    #[test]
    fn an_empty_report_still_produces_a_usable_link() {
        // The Report bug button exists on rows that have no environment and no code — a scan that
        // failed, say. It must still open the template rather than a broken URL.
        let url = bug_report_url(&BugReport::default(), "Windows 11");
        assert!(url.starts_with(ISSUE_URL));
        assert!(!url.contains("log-excerpt"), "{url}");
        assert!(!url.contains("error-code"), "{url}");
    }

    #[test]
    fn the_excerpt_keeps_the_end_and_cuts_at_a_line() {
        let long = (0..4000)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let cut = excerpt(&long);

        assert!(cut.chars().count() <= crate::BUG_REPORT_EXCERPT_CHARS);
        assert!(cut.ends_with("line 3999"), "the tail is what matters");
        assert!(!cut.starts_with("line 0"), "the head should be dropped");
        // Cut at a line boundary, so it does not open mid-token and read as corruption.
        assert!(cut.starts_with("line "), "{:?}", &cut[..20.min(cut.len())]);
    }

    #[test]
    fn a_non_ascii_log_does_not_panic() {
        // Slicing by bytes would panic on the first Vietnamese path or package description; the
        // excerpt is taken by `char`.
        let log = "môi trường ".repeat(2000);
        let cut = excerpt(&log);
        assert!(cut.chars().count() <= crate::BUG_REPORT_EXCERPT_CHARS);
    }
}
