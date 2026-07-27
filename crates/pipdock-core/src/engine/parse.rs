//! Turning each engine's dry-run output into the one shared [`ResolutionReport`].
//!
//! ARCHITECTURE §3: **both adapters must emit the same shape** — that is the whole point of the
//! `Engine` trait. What the two engines actually give us could hardly be less alike, which is why
//! this normalization lives in one reviewable place rather than inside each adapter:
//!
//! | | pip | uv |
//! |---|---|---|
//! | channel | stdout | **stderr** (SP-1) |
//! | format | JSON report v1 | wrapped, decorated text |
//! | yank signal | `is_yanked` field | `warning: … is yanked` line |
//! | pre-change state | absent from the report | `- name==old` lines |
//!
//! Neither reports held-back items at all (SP-1), so [`ResolutionReport::held_back`] is populated
//! by the caller from the reverse-dependency graph, not here.

use crate::errors::{Code, PdError, Result};
use crate::model::{Dist, PkgName, Version};
use crate::plan::{Change, ChangeKind, ImpossibleDetail, ResolutionReport};

/// A yanked release appearing in a plan.
///
/// SP-2: a yank is **not** a failure. Both engines exit 0 and install it, so this is a preview
/// warning row, not an error path (`docs/ERROR-CATALOG.md` PD-PKG-003).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct YankWarning {
    /// The yanked distribution.
    pub pkg: PkgName,
    /// The version that was yanked.
    pub version: Version,
    /// Upstream's stated reason, when given.
    pub reason: Option<String>,
}

/// A parsed plan plus the warnings that belong beside it in the preview.
#[derive(Debug, Clone)]
pub struct ParsedPlan {
    /// The normalized report.
    pub report: ResolutionReport,
    /// Yanked releases the plan would install.
    pub yanked: Vec<YankWarning>,
}

// -- pip ---------------------------------------------------------------------

/// Parse pip's `install --dry-run --report -` JSON from stdout.
///
/// # Errors
/// `PD-ENG-002` when stdout is empty, which is what pip leaves behind when it is too old for
/// `--report` **or** when the SP-2 encoding crash struck; `PD-ENG-003` when the JSON is present
/// but not the shape we know.
pub fn pip_report(stdout: &str, stderr: &str, installed: &[Dist]) -> Result<ParsedPlan> {
    // Checked before anything is parsed, because the SP-2 crash does not leave stdout empty — it
    // leaves a *truncated* report, written up to the byte that could not be encoded. That is more
    // dangerous than no output: a partial document can still deserialize into a plausible-looking
    // plan that is quietly missing packages.
    if stderr.contains("UnicodeEncodeError") {
        return Err(PdError::new(
            Code::IntUnexpected,
            "pip crashed part-way through writing its report — the UTF-8 mitigation was not \
             applied (see spikes/README.md SP-2)",
        )
        .with_stderr(stderr));
    }

    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err(PdError::new(Code::EngPipTooOld, "pip produced no report").with_stderr(stderr));
    }

    let doc: serde_json::Value = serde_json::from_str(trimmed).map_err(|e| {
        PdError::new(
            Code::EngUvShapeUnknown,
            format!("pip report is not JSON: {e}"),
        )
    })?;

    let version = doc
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if version != "1" {
        // Fail loudly rather than mis-parsing a future format into a plausible-looking plan.
        return Err(PdError::new(
            Code::EngUvShapeUnknown,
            format!("unsupported pip report version {version:?}; expected \"1\""),
        ));
    }

    let current: std::collections::BTreeMap<&PkgName, &Version> =
        installed.iter().map(|d| (&d.name, &d.version)).collect();

    let mut changes = Vec::new();
    let mut yanked = Vec::new();

    for entry in doc
        .get("install")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let meta = entry.get("metadata").unwrap_or(&serde_json::Value::Null);
        let Some(raw_name) = meta.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Ok(name) = PkgName::parse(raw_name) else {
            continue;
        };
        let to = Version(
            meta.get("version")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        );

        if entry.get("is_yanked").and_then(serde_json::Value::as_bool) == Some(true) {
            yanked.push(YankWarning {
                pkg: name.clone(),
                version: to.clone(),
                reason: yank_reason_from_stderr(stderr, &name),
            });
        }

        // pip's report does not say what was there before, so the previous version comes from the
        // environment listing. `requested` distinguishes a package the user asked for from one
        // pulled in to satisfy it, which is what splits the preview's first two sections.
        let from = current.get(&name).map(|v| (*v).clone());
        let requested = entry
            .get("requested")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let kind = classify_change(from.as_ref(), &to, requested);

        changes.push(Change {
            name,
            from,
            to,
            kind,
        });
    }

    Ok(ParsedPlan {
        report: ResolutionReport {
            changes,
            held_back: Vec::new(),
            impossible: None,
            raw: stdout.to_owned(),
        },
        yanked,
    })
}

/// pip prints the yank reason on stderr, not in the report JSON.
fn yank_reason_from_stderr(stderr: &str, pkg: &PkgName) -> Option<String> {
    let flat = flatten(stderr);
    let idx = flat.find("Reason for being yanked:")?;
    // Only attribute the reason when this package is the one being discussed.
    if !flat[..idx].to_ascii_lowercase().contains(pkg.as_str()) {
        return None;
    }
    let tail = flat[idx + "Reason for being yanked:".len()..].trim();
    Some(
        tail.trim_end_matches(|c: char| c == '.' || c.is_whitespace())
            .to_owned(),
    )
}

// -- uv ----------------------------------------------------------------------

/// Parse uv's dry-run plan.
///
/// **The plan arrives on stderr** (SP-1); stdout is empty for every uv fixture captured. The
/// format is a decorated, hard-wrapped text block:
///
/// ```text
/// Resolved 3 packages in 831ms
/// Would download 2 packages
/// Would install 2 packages
///  - httpcore==0.15.0
///  + httpcore==1.0.9
/// warning: `urllib3==2.0.0` is yanked (reason: "...")
/// ```
///
/// # Errors
/// `PD-ENG-003` when the text matches none of the shapes this parser knows — the documented
/// "uv newer than the adapter" path, which the weekly parser job in CI exists to catch first.
pub fn uv_plan(stdout: &str, stderr: &str, installed: &[Dist]) -> Result<ParsedPlan> {
    // uv puts everything on stderr, but tolerate stdout in case a future version moves it.
    let text = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };

    if let Some(detail) = uv_no_solution(text) {
        return Ok(ParsedPlan {
            report: ResolutionReport {
                changes: Vec::new(),
                held_back: Vec::new(),
                impossible: Some(detail),
                raw: text.to_owned(),
            },
            yanked: Vec::new(),
        });
    }

    let recognized = [
        "Would make no changes",
        "Would install",
        "Would download",
        "Resolved ",
    ]
    .iter()
    .any(|marker| text.contains(marker));
    if !recognized {
        return Err(PdError::new(
            Code::EngUvShapeUnknown,
            "uv output matched no known plan shape; the adapter may be older than uv",
        )
        .with_stderr(text));
    }

    let current: std::collections::BTreeMap<&PkgName, &Version> =
        installed.iter().map(|d| (&d.name, &d.version)).collect();

    // `- name==old` lines carry what uv would remove, which is how a change gets its `from`
    // version even for a package that was not in the installed listing.
    let mut removals: std::collections::BTreeMap<PkgName, Version> =
        std::collections::BTreeMap::new();
    let mut additions: Vec<(PkgName, Version)> = Vec::new();

    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        let trimmed = line.trim_start();
        let Some((sign, rest)) = trimmed.split_at_checked(1) else {
            continue;
        };
        let rest = rest.trim();
        match sign {
            "-" | "+" if rest.contains("==") => {
                let Some((name, version)) = rest.split_once("==") else {
                    continue;
                };
                let Ok(name) = PkgName::parse(name.trim()) else {
                    continue;
                };
                let version = Version(version.trim().to_owned());
                if sign == "-" {
                    removals.insert(name, version);
                } else {
                    additions.push((name, version));
                }
            }
            _ => {}
        }
    }

    let changes = additions
        .into_iter()
        .map(|(name, to)| {
            let from = removals
                .get(&name)
                .cloned()
                .or_else(|| current.get(&name).map(|v| (*v).clone()));
            // uv does not mark which requirements the user asked for, so a package that was
            // already installed is an upgrade and anything else is a new install. Calling a
            // transitive pull a NewInstall would only mis-title a preview section, never
            // mis-state a version.
            let kind = classify_change(from.as_ref(), &to, from.is_none());
            Change {
                name,
                from,
                to,
                kind,
            }
        })
        .collect();

    Ok(ParsedPlan {
        report: ResolutionReport {
            changes,
            held_back: Vec::new(),
            impossible: None,
            raw: text.to_owned(),
        },
        yanked: uv_yank_warnings(text),
    })
}

/// uv's `No solution found` block, with its wrapping undone.
fn uv_no_solution(text: &str) -> Option<ImpossibleDetail> {
    let flat = flatten(text);
    let idx = flat.find("No solution found when resolving")?;
    let explanation = flat[idx..].trim().to_owned();

    // uv names the packages inside prose ("Because httpx==0.23.0 depends on httpcore>=0.15.0"),
    // so pull out the `name==version` and `name>=…` tokens it mentions.
    let mut packages = Vec::new();
    for token in explanation.split_whitespace() {
        let candidate = token
            .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_' && c != '.');
        let head = candidate
            .split(['=', '<', '>', '!', '~'])
            .next()
            .unwrap_or_default();
        if head.len() < 2 || head == candidate {
            continue;
        }
        if let Ok(name) = PkgName::parse(head)
            && !packages.contains(&name)
        {
            packages.push(name);
        }
    }

    Some(ImpossibleDetail {
        packages,
        explanation,
    })
}

/// `warning: \`urllib3==2.0.0\` is yanked (reason: "…")`
fn uv_yank_warnings(text: &str) -> Vec<YankWarning> {
    let mut out = Vec::new();
    for line in flatten(text).split("warning:").skip(1) {
        if !line.contains("is yanked") {
            continue;
        }
        let Some(spec) = line.split('`').nth(1) else {
            continue;
        };
        let Some((name, version)) = spec.split_once("==") else {
            continue;
        };
        let Ok(pkg) = PkgName::parse(name.trim()) else {
            continue;
        };
        let reason = line
            .split_once("reason:")
            .map(|(_, r)| r.trim().trim_start_matches('"').to_owned())
            .map(|r| {
                r.trim_end_matches(')')
                    .trim_end()
                    .trim_end_matches('"')
                    .to_owned()
            });
        out.push(YankWarning {
            pkg,
            version: Version(version.trim().to_owned()),
            reason,
        });
    }
    out
}

// -- shared ------------------------------------------------------------------

/// Collapse wrapping and CRLF so multi-word phrases can be found.
///
/// uv hard-wraps its diagnostics mid-sentence, so a literal search for a phrase that spans the
/// wrap point finds nothing. The same normalization is applied by the error classifiers, and for
/// the same reason.
fn flatten(text: &str) -> String {
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
            out.push(ch);
        }
    }
    out
}

fn classify_change(from: Option<&Version>, to: &Version, requested: bool) -> ChangeKind {
    match from {
        None if requested => ChangeKind::NewInstall,
        None => ChangeKind::NewDependency,
        Some(old) => {
            if is_downgrade(&old.0, &to.0) {
                ChangeKind::Downgrade
            } else {
                ChangeKind::Upgrade
            }
        }
    }
}

/// Compare two version strings by their numeric release segments.
///
/// Only used to label a preview row Upgrade or Downgrade, so an exotic version that does not
/// compare cleanly falls back to Upgrade rather than failing the parse.
fn is_downgrade(from: &str, to: &str) -> bool {
    use crate::compat::PyVersion;
    match (PyVersion::parse(from), PyVersion::parse(to)) {
        (Ok(a), Ok(b)) => b < a,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(pairs: &[(&str, &str)]) -> Vec<Dist> {
        pairs
            .iter()
            .map(|(n, v)| Dist {
                name: PkgName::parse(n).unwrap(),
                version: Version((*v).to_owned()),
                requires_dist: Vec::new(),
                requires_python: None,
            })
            .collect()
    }

    #[test]
    fn an_empty_pip_report_is_reported_as_a_pip_that_cannot_plan() {
        let err = pip_report("", "", &[]).expect_err("empty report must fail");
        assert_eq!(err.code, Code::EngPipTooOld);
    }

    #[test]
    fn the_sp2_crash_is_distinguished_from_an_old_pip() {
        // One means "upgrade pip", the other means "PipDock dropped its own mitigation";
        // sending the user to upgrade pip would waste their time.
        let err = pip_report("", "UnicodeEncodeError: 'charmap' codec can't encode", &[])
            .expect_err("crash must fail");
        assert_eq!(err.code, Code::IntUnexpected);
    }

    #[test]
    fn a_truncated_report_is_rejected_rather_than_half_parsed() {
        // The crash writes output up to the offending byte, so stdout holds a partial document.
        // Deserializing it could yield a plan that is silently missing packages — the worst
        // possible outcome for a tool whose promise is "preview before touch".
        let partial = r#"{"version":"1","install":[{"requested":true,"metadata":{"name":"idn"#;
        let err = pip_report(partial, "UnicodeEncodeError: 'charmap' codec", &[])
            .expect_err("partial report must not parse");
        assert_eq!(err.code, Code::IntUnexpected);
    }

    #[test]
    fn an_unknown_pip_report_version_fails_loudly() {
        let err = pip_report(r#"{"version":"2","install":[]}"#, "", &[])
            .expect_err("future format must not be guessed at");
        assert_eq!(err.code, Code::EngUvShapeUnknown);
    }

    #[test]
    fn uv_output_that_matches_nothing_yields_the_documented_code() {
        let err = uv_plan("", "some future uv phrasing nobody has seen", &[])
            .expect_err("unknown shape must fail");
        assert_eq!(err.code, Code::EngUvShapeUnknown);
    }

    #[test]
    fn uv_no_changes_is_a_valid_empty_plan() {
        let text = "Using Python 3.12.10 environment at: C:\\tmp\\venv\r\n\
                    Resolved 8 packages in 406ms\r\nChecked 8 packages in 1ms\r\n\
                    Would make no changes\r\n";
        let plan = uv_plan("", text, &[]).expect("no-changes is a plan");
        assert!(plan.report.changes.is_empty());
        assert!(plan.report.is_clean());
    }

    #[test]
    fn uv_upgrade_lines_pair_removals_with_additions() {
        let text = "Resolved 3 packages in 831ms\r\nWould download 2 packages\r\n\
                    Would uninstall 2 packages\r\nWould install 2 packages\r\n \
                    - httpcore==0.15.0\r\n + httpcore==1.0.9\r\n - h11==0.12.0\r\n + h11==0.16.0\r\n";
        let plan = uv_plan("", text, &[]).expect("parse");
        let httpcore = plan
            .report
            .changes
            .iter()
            .find(|c| c.name.as_str() == "httpcore")
            .expect("httpcore change");
        assert_eq!(httpcore.from.as_ref().map(|v| v.0.as_str()), Some("0.15.0"));
        assert_eq!(httpcore.to.0, "1.0.9");
        assert_eq!(httpcore.kind, ChangeKind::Upgrade);
    }

    #[test]
    fn uv_reports_impossibility_with_its_explanation_intact() {
        let text = "  × No solution found when resolving dependencies:\r\n  \
                    ╰─▶ Because httpx==0.23.0 depends on httpcore>=0.15.0,<0.16.0 and you\r\n      \
                    require httpcore>=1.0, we can conclude that your requirements and\r\n      \
                    httpx==0.23.0 are incompatible.\r\n";
        let plan = uv_plan("", text, &[]).expect("parse");
        let detail = plan.report.impossible.expect("impossible detail");

        // The wrapping must be undone, or the explanation reads as broken fragments in the UI.
        assert!(
            detail
                .explanation
                .contains("depends on httpcore>=0.15.0,<0.16.0 and you require")
        );
        assert!(detail.packages.iter().any(|p| p.as_str() == "httpx"));
        assert!(detail.packages.iter().any(|p| p.as_str() == "httpcore"));
    }

    #[test]
    fn uv_yank_warnings_carry_the_reason() {
        let text = "Resolved 1 package in 740ms\r\nWould install 1 package\r\n + urllib3==2.0.0\r\n\
                    warning: `urllib3==2.0.0` is yanked (reason: \"Truncated response bodies\")\r\n";
        let plan = uv_plan("", text, &[]).expect("parse");
        assert_eq!(plan.yanked.len(), 1);
        assert_eq!(plan.yanked[0].pkg.as_str(), "urllib3");
        assert_eq!(plan.yanked[0].version.0, "2.0.0");
        assert_eq!(
            plan.yanked[0].reason.as_deref(),
            Some("Truncated response bodies")
        );
        // A yank is a warning, not a failure: the plan itself stays clean.
        assert!(plan.report.is_clean());
    }

    #[test]
    fn a_downgrade_is_labelled_as_one() {
        let text = "Resolved 1 package\r\nWould install 1 package\r\n \
                    - httpcore==1.0.9\r\n + httpcore==0.15.0\r\n";
        let plan = uv_plan("", text, &installed(&[("httpcore", "1.0.9")])).expect("parse");
        assert_eq!(plan.report.changes[0].kind, ChangeKind::Downgrade);
    }

    #[test]
    fn pip_splits_requested_installs_from_transitive_ones() {
        let doc = r#"{"version":"1","install":[
            {"requested":true,"is_yanked":false,"metadata":{"name":"httpcore","version":"1.0.9"}},
            {"requested":false,"is_yanked":false,"metadata":{"name":"h11","version":"0.16.0"}}
        ]}"#;
        let plan = pip_report(doc, "", &[]).expect("parse");
        let by = |n: &str| {
            plan.report
                .changes
                .iter()
                .find(|c| c.name.as_str() == n)
                .expect("change")
                .kind
        };
        assert_eq!(by("httpcore"), ChangeKind::NewInstall);
        assert_eq!(by("h11"), ChangeKind::NewDependency);
    }

    #[test]
    fn pip_uses_the_installed_listing_for_the_previous_version() {
        // pip's report never says what was there before; without the listing every upgrade would
        // render as a fresh install.
        let doc = r#"{"version":"1","install":[
            {"requested":true,"is_yanked":false,"metadata":{"name":"idna","version":"3.18"}}
        ]}"#;
        let plan = pip_report(doc, "", &installed(&[("idna", "3.4")])).expect("parse");
        assert_eq!(
            plan.report.changes[0].from.as_ref().map(|v| v.0.as_str()),
            Some("3.4")
        );
        assert_eq!(plan.report.changes[0].kind, ChangeKind::Upgrade);
    }

    #[test]
    fn pip_yank_flag_becomes_a_warning_not_a_failure() {
        let doc = r#"{"version":"1","install":[
            {"requested":true,"is_yanked":true,"metadata":{"name":"urllib3","version":"2.0.0"}}
        ]}"#;
        let stderr = "WARNING: The candidate selected for download or install is a yanked \
                      version: 'urllib3' candidate (version 2.0.0 ...)\r\n\
                      Reason for being yanked: Truncated response bodies when streaming.\r\n";
        let plan = pip_report(doc, stderr, &[]).expect("parse");
        assert_eq!(plan.yanked.len(), 1);
        assert_eq!(
            plan.yanked[0].reason.as_deref(),
            Some("Truncated response bodies when streaming")
        );
        assert!(
            plan.report.is_clean(),
            "a yank must not make the plan dirty"
        );
    }
}
