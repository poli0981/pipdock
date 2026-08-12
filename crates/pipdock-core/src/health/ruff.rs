//! ruff: lint findings, and the only tool whose fixes PipDock ever applies.

use crate::errors::{Code, PdError, Result};

use super::report::{FixApplicability, RuffFinding, RuffFindings};

/// One entry of `ruff check --output-format json`, as ruff 0.16.0 writes it.
#[derive(serde::Deserialize)]
struct RawFinding {
    #[serde(default)]
    code: Option<String>,
    name: String,
    message: String,
    filename: String,
    location: RawPosition,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    fix: Option<RawFix>,
}

#[derive(serde::Deserialize)]
struct RawPosition {
    row: u32,
    column: u32,
}

#[derive(serde::Deserialize)]
struct RawFix {
    applicability: FixApplicability,
}

/// Parse ruff's JSON report.
///
/// **Give this stdout only.** ruff writes findings to stdout and its own warnings — incompatible
/// rule pairs, deprecated selectors — to stderr. Merging the streams puts `warning: ...` in front
/// of the array and every run fails to parse; found by capturing a fixture with `2>&1`.
///
/// # Errors
/// `PD-HLT-002` when the document is not the array ruff documents.
pub fn parse(stdout: &str) -> Result<RuffFindings> {
    let raw: Vec<RawFinding> = serde_json::from_str(stdout.trim()).map_err(|e| {
        PdError::new(
            Code::HltToolFailed,
            format!("ruff emitted a report PipDock could not read: {e}"),
        )
    })?;

    let findings = raw
        .into_iter()
        .map(|f| RuffFinding {
            code: f.code,
            name: f.name,
            message: f.message,
            filename: f.filename,
            row: f.location.row,
            column: f.location.column,
            // Carried, never constructed: the URL is keyed by rule *name*, so building it from the
            // code would 404 on every finding (CODE-HEALTH-SPEC §6, amended).
            url: f.url,
            fix: f.fix.map(|fix| fix.applicability),
        })
        .collect();

    Ok(RuffFindings {
        findings,
        fixable: 0,
        fixable_files: 0,
    }
    .recount())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from ruff 0.16.0: an unsorted import block and two unused imports, all safe-fixable.
    const REAL: &str = include_str!("../../tests/fixtures/health/ruff/findings.json");

    #[test]
    fn the_real_shape_parses_and_counts_its_fixables() {
        let out = parse(REAL).expect("ruff's own output parses");

        assert_eq!(out.findings.len(), 3);
        assert_eq!(out.fixable, 3);
        assert_eq!(out.fixable_files, 1, "all three are in one file");
    }

    /// The correction to CODE-HEALTH-SPEC §6, pinned so nobody reintroduces the construction.
    #[test]
    fn the_docs_url_is_keyed_by_rule_name_not_code() {
        let out = parse(REAL).expect("parses");
        let sorted = out
            .findings
            .iter()
            .find(|f| f.code.as_deref() == Some("I001"))
            .expect("the import-sort finding");

        assert_eq!(
            sorted.url.as_deref(),
            Some("https://docs.astral.sh/ruff/rules/unsorted-imports"),
            "constructing .../rules/I001 would 404"
        );
        assert_eq!(sorted.name, "unsorted-imports");
    }

    #[test]
    fn a_finding_with_no_fix_is_not_counted_as_fixable() {
        let out =
            parse(include_str!("../../tests/fixtures/health/ruff/nofix.json")).expect("parses");

        assert_eq!(out.findings.len(), 1);
        assert_eq!(out.findings[0].fix, None);
        assert_eq!(out.fixable, 0);
        assert_eq!(out.fixable_files, 0);
    }

    /// ruff 0.16.0 reports a syntax error as `invalid-syntax` with a null `url` — **not** a null
    /// `code`, which is what §6 assumed. Both are tolerated; this pins what actually happens.
    #[test]
    fn a_syntax_error_parses_and_offers_nothing_to_fix() {
        let out = parse(include_str!(
            "../../tests/fixtures/health/ruff/syntax-error.json"
        ))
        .expect("parses");

        assert!(!out.findings.is_empty());
        assert_eq!(out.findings[0].code.as_deref(), Some("invalid-syntax"));
        assert_eq!(out.findings[0].url, None, "a syntax error has no rule page");
        assert_eq!(out.fixable, 0);
    }

    #[test]
    fn only_safe_fixes_are_counted() {
        // P5 runs plain `ruff check --fix`, which applies safe fixes only. Counting the others
        // would name a number in the confirm dialog that the fix could not deliver.
        let doc = r#"[
            {"code":"A","name":"a","message":"m","filename":"x.py",
             "location":{"row":1,"column":1},"url":null,"fix":{"applicability":"safe"}},
            {"code":"B","name":"b","message":"m","filename":"y.py",
             "location":{"row":1,"column":1},"url":null,"fix":{"applicability":"unsafe"}},
            {"code":"C","name":"c","message":"m","filename":"z.py",
             "location":{"row":1,"column":1},"url":null,"fix":{"applicability":"display"}}
        ]"#;

        let out = parse(doc).expect("parses");
        assert_eq!(out.fixable, 1);
        assert_eq!(out.fixable_files, 1);
    }

    #[test]
    fn fixable_files_counts_files_not_findings() {
        // The number P5's dialog names is files, and `git diff --stat` is what the user checks it
        // against. Reporting findings there would read as a lie about blast radius.
        let doc = r#"[
            {"code":"A","name":"a","message":"m","filename":"same.py",
             "location":{"row":1,"column":1},"url":null,"fix":{"applicability":"safe"}},
            {"code":"A","name":"a","message":"m","filename":"same.py",
             "location":{"row":9,"column":1},"url":null,"fix":{"applicability":"safe"}}
        ]"#;

        let out = parse(doc).expect("parses");
        assert_eq!(out.fixable, 2);
        assert_eq!(out.fixable_files, 1);
    }

    #[test]
    fn a_clean_project_is_an_empty_report() {
        let out =
            parse(include_str!("../../tests/fixtures/health/ruff/clean.json")).expect("parses");
        assert!(out.findings.is_empty());
        assert_eq!(out.fixable, 0);
    }

    #[test]
    fn ruffs_warnings_on_stderr_would_break_a_merged_capture() {
        // Not hypothetical: the first fixture capture used `2>&1` and every parse failed. The
        // caller must pass stdout alone, and this is what says so if anyone changes that.
        let merged = "warning: `D203` and `D211` are incompatible.\n[]";
        assert_eq!(
            parse(merged).expect_err("refused").code,
            Code::HltToolFailed
        );
    }
}
