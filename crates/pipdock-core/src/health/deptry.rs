//! deptry: which declared dependencies are unused, missing, or transitive-only.

use std::collections::BTreeMap;

use crate::errors::{Code, PdError, Result};

use super::report::{DeptryIssue, SourceLocation};

/// One entry of deptry's `--json-output` array, as deptry 0.25.1 writes it.
///
/// Deliberately a private mirror of deptry's shape rather than deserializing straight into
/// [`DeptryIssue`]: the wire type is PipDock's and outlives any one deptry release, and keeping
/// them separate makes a format change a compile error here instead of a silently empty report.
#[derive(serde::Deserialize)]
struct RawViolation {
    error: RawError,
    /// The **module** name — `yaml`, not `PyYAML`.
    module: String,
    #[serde(default)]
    location: Option<RawLocation>,
}

#[derive(serde::Deserialize)]
struct RawError {
    code: String,
    message: String,
}

#[derive(serde::Deserialize)]
struct RawLocation {
    file: String,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    column: Option<u32>,
}

/// Parse deptry's JSON report into issues grouped by dependency.
///
/// deptry emits a **flat list**, one object per violation-location, so the same dependency appears
/// once per place it was found. Grouping here rather than in the UI keeps the GUI, the CLI's
/// `--json` and P4's saved report identical.
///
/// Grouped on `(code, module)` rather than module alone: one dependency can be both `DEP001` and
/// `DEP003`, and merging those would produce a row whose message describes only one of them.
///
/// # Errors
/// `PD-HLT-002` when the document is not the array of violations deptry documents. That is the
/// closest honest code — the tool ran and produced something unusable — and the fixture corpus is
/// what stops a Dependabot bump reaching a user through this path.
pub fn parse(stdout: &str) -> Result<Vec<DeptryIssue>> {
    let raw: Vec<RawViolation> = serde_json::from_str(stdout.trim()).map_err(|e| {
        PdError::new(
            Code::HltToolFailed,
            format!("deptry emitted a report PipDock could not read: {e}"),
        )
    })?;

    // BTreeMap so the order is the dependency's, not the order deptry happened to walk files in —
    // two runs over the same project must produce the same report.
    let mut grouped: BTreeMap<(String, String), DeptryIssue> = BTreeMap::new();

    for violation in raw {
        let key = (violation.error.code.clone(), violation.module.clone());
        let issue = grouped.entry(key).or_insert_with(|| DeptryIssue {
            code: violation.error.code,
            dep: violation.module,
            message: violation.error.message,
            locations: Vec::new(),
        });

        if let Some(loc) = violation.location {
            let location = SourceLocation {
                file: loc.file,
                line: loc.line,
                column: loc.column,
            };
            // A dependency imported twice on the same line is one place, not two.
            if !issue.locations.contains(&location) {
                issue.locations.push(location);
            }
        }
    }

    Ok(grouped.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from deptry 0.25.1 over a project declaring an unused `httpx`.
    const REAL: &str = include_str!("../../tests/fixtures/health/deptry/issues.json");

    #[test]
    fn the_real_shape_parses() {
        let issues = parse(REAL).expect("deptry's own output parses");

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, "DEP002");
        assert_eq!(issues[0].dep, "httpx");
        assert!(issues[0].message.contains("not used in the codebase"));
    }

    #[test]
    fn a_whole_file_finding_keeps_its_file_and_drops_the_null_position() {
        // deptry reports an unused dependency against pyproject.toml with line and column null.
        // Rendering "pyproject.toml:0" would be a lie about where it is.
        let issues = parse(REAL).expect("parses");
        let loc = &issues[0].locations[0];

        assert_eq!(loc.file, "pyproject.toml");
        assert_eq!(loc.line, None);
        assert_eq!(loc.column, None);
    }

    #[test]
    fn a_clean_project_is_an_empty_report_not_an_error() {
        let issues = parse(include_str!(
            "../../tests/fixtures/health/deptry/clean.json"
        ))
        .expect("an empty array parses");
        assert!(issues.is_empty());
    }

    #[test]
    fn the_flat_list_is_grouped_by_dependency() {
        // The shape §5 assumed deptry produced. It does not — one object per violation-location —
        // so grouping is PipDock's job and this is the case that proves it happens.
        let doc = r#"[
            {"error":{"code":"DEP001","message":"'requests' imported but missing"},
             "module":"requests","location":{"file":"a.py","line":1,"column":1}},
            {"error":{"code":"DEP001","message":"'requests' imported but missing"},
             "module":"requests","location":{"file":"b.py","line":7,"column":1}}
        ]"#;

        let issues = parse(doc).expect("parses");
        assert_eq!(issues.len(), 1, "one dependency, two places");
        assert_eq!(issues[0].locations.len(), 2);
    }

    #[test]
    fn one_dependency_with_two_codes_stays_two_issues() {
        // Grouping on the module alone would merge these into a row whose message describes only
        // one of the two problems.
        let doc = r#"[
            {"error":{"code":"DEP001","message":"missing"},"module":"x","location":null},
            {"error":{"code":"DEP003","message":"transitive"},"module":"x","location":null}
        ]"#;

        assert_eq!(parse(doc).expect("parses").len(), 2);
    }

    #[test]
    fn a_repeated_location_is_recorded_once() {
        let doc = r#"[
            {"error":{"code":"DEP002","message":"unused"},"module":"x",
             "location":{"file":"a.py","line":3,"column":1}},
            {"error":{"code":"DEP002","message":"unused"},"module":"x",
             "location":{"file":"a.py","line":3,"column":1}}
        ]"#;

        assert_eq!(parse(doc).expect("parses")[0].locations.len(), 1);
    }

    #[test]
    fn a_shape_deptry_does_not_document_is_a_catalog_code() {
        let err = parse("{\"not\": \"an array\"}").expect_err("refused");
        assert_eq!(err.code, Code::HltToolFailed);
    }
}
