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
use crate::model::{Dist, EngineId, PkgName, Version};
use crate::plan::{Change, ChangeKind, ImpossibleDetail, ResolutionReport};

/// A yanked release appearing in a plan.
///
/// SP-2: a yank is **not** a failure. Both engines exit 0 and install it, so this is a preview
/// warning row, not an error path (`docs/ERROR-CATALOG.md` PD-PKG-003).
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
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

impl ParsedPlan {
    /// The report with its yank warnings inside it, where the preview can see them.
    ///
    /// Both adapters used to take `.report` and drop `.yanked`, so the PD-PKG-003 warning both
    /// parsers worked to extract never reached a user.
    #[must_use]
    pub fn into_report(self) -> crate::plan::ResolutionReport {
        crate::plan::ResolutionReport {
            yanked: self.yanked,
            ..self.report
        }
    }
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

    let doc: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|e| shape_error(EngineId::Pip, &format!("pip report is not JSON: {e}")))?;

    let version = doc
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if version != "1" {
        // Fail loudly rather than mis-parsing a future format into a plausible-looking plan.
        return Err(shape_error(
            EngineId::Pip,
            &format!("unsupported pip report version {version:?}; expected \"1\""),
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
        let raw_version = meta
            .get("version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        // pip normalizes report versions to PEP 440; anything else is not something PipDock will
        // turn into an argv token, and the change is left out rather than mis-rendered.
        if !crate::pep440::is_argv_safe(raw_version) {
            continue;
        }
        let to = Version(raw_version.to_owned());

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
            incompatible: Vec::new(),
            forced: Vec::new(),
            yanked: Vec::new(),
            // pip's stderr — its warnings — not the `--report` document on stdout, which carries
            // every package's README and is already parsed above.
            raw: stderr.to_owned(),
        },
        yanked,
    })
}

/// pip's dry run, read **by its exit status first**.
///
/// A zero exit is a plan. A non-zero one is a failure classified from stderr like any other —
/// except an impossibility, which becomes `report.impossible`: **the same shape uv gives**
/// (ARCHITECTURE §3). It used to be a hard error, so under pip one unsatisfiable package in a
/// fifteen-package request failed the whole preview and the per-package *Skip* the preview offers
/// under uv never appeared.
///
/// # Errors
/// The classified engine failure; `PD-INT-001` for the SP-2 encoding crash whatever the exit.
pub fn pip_resolution(
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
    installed: &[Dist],
) -> Result<ParsedPlan> {
    // The SP-2 crash exits 2, and is PipDock's bug rather than pip's; `pip_report` says so.
    if exit_code == Some(0) || stderr.contains("UnicodeEncodeError") {
        return pip_report(stdout, stderr, installed);
    }
    if let Some(detail) = pip_impossible(stderr) {
        return Ok(impossible_plan(detail, stderr));
    }
    Err(PdError::from_engine_stderr(stderr))
}

/// A plan that is nothing but its impossibility.
fn impossible_plan(detail: ImpossibleDetail, text: &str) -> ParsedPlan {
    ParsedPlan {
        report: ResolutionReport {
            impossible: Some(detail),
            raw: text.to_owned(),
            ..ResolutionReport::default()
        },
        yanked: Vec::new(),
    }
}

/// pip's ResolutionImpossible, read into the shape uv's impossibility already has.
///
/// pip runs with `--quiet`, so its whole explanation is one line — `ERROR: Cannot install A and B
/// because these package versions have conflicting dependencies.` — followed by the
/// ResolutionImpossible pointer. The packages are the requirements that line names. `None` when
/// the stderr is not an impossibility at all, so the caller classifies it instead.
#[must_use]
pub fn pip_impossible(stderr: &str) -> Option<crate::plan::ImpossibleDetail> {
    if crate::errors::classify_stderr(stderr) != crate::errors::Code::ResImpossible {
        return None;
    }
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| l.contains("Cannot install "))
        .or_else(|| {
            stderr
                .lines()
                .map(str::trim)
                .find(|l| l.starts_with("ERROR:"))
        })?;
    let explanation = line.trim_start_matches("ERROR:").trim().to_owned();

    let mut packages = Vec::new();
    if let Some(rest) = explanation.strip_prefix("Cannot install ")
        && let Some((named, _)) = rest.split_once(" because")
    {
        for token in named.split([',']).flat_map(|t| t.split(" and ")) {
            let token = token.trim();
            let end = token
                .find(['=', '<', '>', '!', '~', '[', ' '])
                .unwrap_or(token.len());
            if let Ok(name) = PkgName::parse(&token[..end])
                && !packages.contains(&name)
            {
                packages.push(name);
            }
        }
    }
    Some(crate::plan::ImpossibleDetail {
        packages,
        explanation,
    })
}

/// pip prints the yank reason on stderr, not in the report JSON:
///
/// ```text
/// WARNING: The candidate selected for download or install is a yanked version: 'urllib3' candidate (…)
/// Reason for being yanked: Truncated response bodies when streaming a large compressed body.
/// ```
///
/// The reason is **that one line**, and it belongs to the package the warning above it names.
/// Flattening the whole stderr first made it run to the end of the output — over pip's
/// `[notice]` lines, which carry the user's home directory — and gave the first of two yanked
/// packages both reasons and the second none.
fn yank_reason_from_stderr(stderr: &str, pkg: &PkgName) -> Option<String> {
    let mut current: Option<PkgName> = None;
    for line in stderr.lines().map(str::trim) {
        if let Some((_, rest)) = line.split_once("is a yanked version: '") {
            current = rest
                .split_once('\'')
                .and_then(|(name, _)| PkgName::parse(name).ok());
            continue;
        }
        if let Some(reason) = line.strip_prefix("Reason for being yanked:")
            && current.as_ref() == Some(pkg)
        {
            let reason = reason
                .trim()
                .trim_end_matches(|c: char| c == '.' || c.is_whitespace());
            return (!reason.is_empty()).then(|| reason.to_owned());
        }
    }
    None
}

// -- uv ----------------------------------------------------------------------

/// uv's dry run, read **by its exit status first** (C3).
///
/// The adapter used to hand every outcome to [`uv_plan`], which looked for the words of a plan:
/// a build that failed still printed `Resolved 1 package`, so it parsed as an empty plan and the
/// preview said *no changes* for a package that could not be built; a refused connection matched
/// nothing and was reported as a uv newer than PipDock; an unknown name came through the resolver
/// as an impossibility. uv exits non-zero on every one of them, so a non-zero exit is classified
/// like any other engine failure, and only an impossibility becomes a report.
///
/// # Errors
/// The classified engine failure, or [`uv_plan`]'s for a zero exit it cannot read.
pub fn uv_resolution(
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
    installed: &[Dist],
) -> Result<ParsedPlan> {
    if exit_code == Some(0) {
        return uv_plan(stdout, stderr, installed);
    }
    let text = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    if crate::errors::classify_stderr(text) == Code::ResImpossible
        && let Some(detail) = uv_no_solution(text)
    {
        return Ok(impossible_plan(detail, text));
    }
    Err(PdError::from_engine_stderr(text))
}

/// Parse uv's dry-run plan — a run that **exited zero**; [`uv_resolution`] decides that.
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
                // `+ pkg==0.1.0 (from file:///…)` for a local or direct-URL source: the version is
                // the first token, and the rest is uv explaining where it comes from. Kept whole,
                // it reached argv as `pkg==0.1.0 (from file:///…)`.
                let Some(version) = version.split_whitespace().next() else {
                    continue;
                };
                if !crate::pep440::is_argv_safe(version) {
                    continue;
                }
                let version = Version(version.to_owned());
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
            incompatible: Vec::new(),
            forced: Vec::new(),
            yanked: Vec::new(),
            raw: text.to_owned(),
        },
        yanked: uv_yank_warnings(text),
    })
}

/// uv's `No solution found` block, with its wrapping and its decoration undone.
///
/// uv 0.11 draws it with miette (`×`, `╰─▶`) and uv 0.13 with `error:` / `cause:` labels; the
/// preview shows the sentence, not either rendering, so both read
/// `No solution found when resolving dependencies: Because …`.
fn uv_no_solution(text: &str) -> Option<ImpossibleDetail> {
    let flat = flatten(&strip_uv_decoration(text));
    let idx = flat.find("No solution found when resolving")?;
    let explanation = flat[idx..]
        .trim()
        .replace("dependencies: :", "dependencies:")
        .replace(" : ", ": ");

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

// -- list / check ------------------------------------------------------------

/// Parse `list --format=json` from either engine.
///
/// DATA-FLOW §7 notes the two shapes "differ slightly"; in practice both emit
/// `[{"name": …, "version": …}, …]`, so one parser serves both and normalizes to [`Dist`].
/// Note this listing carries **no** `requires_dist` — only `probe.py` provides that, which is why
/// the reverse-dependency graph is built from the probe and not from here.
///
/// # Errors
/// [`shape_error`] when the output is not the expected JSON array.
pub fn list_json(stdout: &str, engine: EngineId) -> Result<Vec<Dist>> {
    let items: Vec<serde_json::Value> = serde_json::from_str(stdout.trim())
        .map_err(|e| shape_error(engine, &format!("list output: {e}")))?;
    Ok(items
        .into_iter()
        .filter_map(|v| {
            let name = PkgName::parse(v.get("name")?.as_str()?).ok()?;
            Some(Dist {
                name,
                version: Version(v.get("version")?.as_str()?.to_owned()),
                requires_dist: Vec::new(),
                requires_python: None,
                // Neither engine reports an installed size, so this listing can never carry one.
                // Another reason the Installed screen reads the probe rather than the engine.
                size_bytes: None,
            })
        })
        .collect())
}

/// Parse `list --outdated --format=json`.
///
/// # Errors
/// [`shape_error`] when the output is not the expected JSON array.
pub fn outdated_json(stdout: &str, engine: EngineId) -> Result<Vec<crate::model::OutdatedDist>> {
    let items: Vec<serde_json::Value> = serde_json::from_str(stdout.trim())
        .map_err(|e| shape_error(engine, &format!("outdated output: {e}")))?;
    Ok(items
        .into_iter()
        .filter_map(|v| {
            Some(crate::model::OutdatedDist {
                name: PkgName::parse(v.get("name")?.as_str()?).ok()?,
                current: Version(v.get("version")?.as_str()?.to_owned()),
                latest: Version(v.get("latest_version")?.as_str()?.to_owned()),
            })
        })
        .collect())
}

/// Output in a shape this adapter does not know.
///
/// `PD-ENG-003` means "this uv is newer than PipDock understands — switch to pip for now", which
/// is advice only uv can earn. pip's own unreadable output was reported with it too, telling a pip
/// user to switch to the engine they were already using; it is `PD-ENG-999` instead.
fn shape_error(engine: EngineId, detail: &str) -> PdError {
    let code = match engine {
        EngineId::Uv => Code::EngUvShapeUnknown,
        EngineId::Pip => Code::EngUnclassified,
    };
    PdError::new(code, detail.to_owned())
}

/// Normalize `pip check` / `uv pip check` output.
///
/// `ok` is the process's own verdict: both engines exit non-zero when they find problems, which
/// is a **successful check with findings**, not a command failure. Treating it as a failure would
/// turn every post-run verification on an already-broken environment into an error the user
/// cannot act on.
#[must_use]
pub fn check_text(stdout: &str, ok: bool) -> crate::model::CheckReport {
    let mut findings = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        // Three shapes, each taken from a real broken environment:
        //   pip  "anyio 3.7.1 requires sniffio, which is not installed."
        //   pip  "httpx 0.23.0 has requirement httpcore<0.16.0,>=0.15.0, but you have httpcore 1.0.9."
        //   uv   "The package `httpx` requires `httpcore>=0.15.0,<0.16.0`, but `1.0.9` is installed"
        // Only the first was read: pip's version conflicts — the case a post-run check exists
        // for — were dropped, and every uv finding was filed under a package called "the".
        let name = if let Some(rest) = line.strip_prefix("The package `") {
            rest.split_once('`').map(|(name, _)| name)
        } else if line.contains(" requires ") || line.contains(" has requirement ") {
            line.split_whitespace().next()
        } else {
            None
        };
        let Some(Ok(pkg)) = name.map(PkgName::parse) else {
            continue;
        };
        findings.push(crate::model::CheckFinding {
            pkg,
            requirement: line.to_owned(),
        });
    }
    crate::model::CheckReport {
        ok: ok && findings.is_empty(),
        findings,
        error: None,
    }
}

// -- shared ------------------------------------------------------------------

/// uv's diagnostic decoration, removed line by line: miette's box-drawing glyphs (uv ≤ 0.12) and
/// the `cause:` labels that replaced them (0.13). A `cause:` becomes a colon after the line it
/// explains.
fn strip_uv_decoration(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line
            .trim()
            .trim_start_matches(['×', '╰', '├', '─', '▶', '│', ' ']);
        if let Some(rest) = trimmed.strip_prefix("cause:") {
            let ends_with_colon = out.trim_end().ends_with(':');
            let kept = out.trim_end().len();
            out.truncate(kept);
            out.push_str(if ends_with_colon { " " } else { ": " });
            out.push_str(rest.trim_start());
        } else {
            out.push_str(trimmed);
        }
        out.push('\n');
    }
    out
}

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
                size_bytes: None,
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
        // Not PD-ENG-003, whose advice is to switch to pip.
        assert_eq!(err.code, Code::EngUnclassified);
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
        let plan = uv_resolution(Some(1), "", text, &[]).expect("parse");
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

    #[test]
    fn both_uv_renderings_explain_an_impossibility_in_the_same_words() {
        let v011 = "  × No solution found when resolving dependencies:\n  \
                    ╰─▶ Because a==1.0 depends on b<2 and you require b>=2, we can conclude\n      \
                    that your requirements are unsatisfiable.\n";
        let v013 = "error: No solution found when resolving dependencies\n  \
                    cause: Because a==1.0 depends on b<2 and you require b>=2, we can conclude\n         \
                    that your requirements are unsatisfiable.\n";
        let explain = |text: &str| {
            uv_resolution(Some(1), "", text, &[])
                .expect("an impossibility is a report")
                .report
                .impossible
                .expect("impossible")
                .explanation
        };
        let expected = "No solution found when resolving dependencies: Because a==1.0 depends \
                        on b<2 and you require b>=2, we can conclude that your requirements are \
                        unsatisfiable.";
        assert_eq!(explain(v011), expected);
        assert_eq!(explain(v013), expected);
    }

    #[test]
    fn a_failed_uv_run_that_still_says_resolved_is_not_an_empty_plan() {
        // C3: uv prints "Resolved 1 package" before a build fails, which is one of the words
        // `uv_plan` looks for — so a package that could not be built previewed as "no changes".
        let text = "Resolved 1 package in 1.2s\n   Building numpy==1.19.5\n\
                    error: Failed to build `numpy==1.19.5`\n  \
                    cause: The build backend returned an error\n";
        let err = uv_resolution(Some(1), "", text, &[]).expect_err("a failure");
        assert_eq!(err.code, Code::BldBackendFailed);
    }

    #[test]
    fn two_yanked_packages_each_get_their_own_reason_and_nothing_else() {
        let doc = r#"{"version":"1","install":[
            {"requested":true,"is_yanked":true,"metadata":{"name":"urllib3","version":"2.0.0"}},
            {"requested":true,"is_yanked":true,"metadata":{"name":"idna","version":"3.5"}}
        ]}"#;
        let stderr = "WARNING: The candidate selected for download or install is a yanked \
                      version: 'urllib3' candidate (version 2.0.0 at https://x/urllib3.whl)\n\
                      Reason for being yanked: Truncated response bodies.\n\
                      WARNING: The candidate selected for download or install is a yanked \
                      version: 'idna' candidate (version 3.5 at https://x/idna.whl)\n\
                      Reason for being yanked: Bad wheel\n\
                      [notice] To update, run: C:\\Users\\someone\\venv\\python.exe -m pip \
                      install --upgrade pip\n";
        let plan = pip_report(doc, stderr, &[]).expect("parse");
        let reason = |n: &str| {
            plan.yanked
                .iter()
                .find(|y| y.pkg.as_str() == n)
                .and_then(|y| y.reason.clone())
        };
        assert_eq!(
            reason("urllib3").as_deref(),
            Some("Truncated response bodies")
        );
        assert_eq!(reason("idna").as_deref(), Some("Bad wheel"));
    }

    #[test]
    fn pip_and_uv_check_findings_name_the_package_that_is_broken() {
        // Captured from one broken environment, pip 26.2.1 and uv 0.13.1.
        let pip = "anyio 3.7.1 requires sniffio, which is not installed.\n\
                   httpx 0.23.0 requires sniffio, which is not installed.\n\
                   httpcore 1.0.9 has requirement h11>=0.16, but you have h11 0.12.0.\n\
                   httpx 0.23.0 has requirement httpcore<0.16.0,>=0.15.0, but you have httpcore 1.0.9.\n";
        let uv = "Checked 8 packages in 174ms\nFound 4 incompatibilities\n\
                  The package `httpx` requires `sniffio`, but it's not installed\n\
                  The package `httpx` requires `httpcore>=0.15.0,<0.16.0`, but `1.0.9` is installed\n\
                  The package `httpcore` requires `h11>=0.16`, but `0.12.0` is installed\n\
                  The package `anyio` requires `sniffio>=1.1`, but it's not installed\n";
        let names = |text: &str| -> Vec<String> {
            let report = check_text(text, false);
            assert!(!report.ok);
            report.findings.iter().map(|f| f.pkg.to_string()).collect()
        };
        assert_eq!(names(pip), ["anyio", "httpx", "httpcore", "httpx"]);
        assert_eq!(names(uv), ["httpx", "httpx", "httpcore", "anyio"]);
    }

    #[test]
    fn unreadable_pip_output_does_not_blame_uv() {
        // PD-ENG-003 tells the user to switch to pip; under pip that is advice to do nothing.
        let err = pip_report("not json", "", &[]).expect_err("unreadable");
        assert_eq!(err.code, Code::EngUnclassified);
        assert_eq!(
            list_json("not json", EngineId::Pip).expect_err("pip").code,
            Code::EngUnclassified
        );
        assert_eq!(
            list_json("not json", EngineId::Uv).expect_err("uv").code,
            Code::EngUvShapeUnknown
        );
    }

    #[test]
    fn a_uv_line_with_a_source_suffix_keeps_only_the_version() {
        // `+ pkg==0.1.0 (from file:///…)` reached argv whole as the version.
        let text = "Resolved 1 package in 5ms\nWould install 1 package\n + localpkg==0.1.0 (from file:///C:/src/localpkg)\n";
        let plan = uv_plan("", text, &[]).expect("parses");
        assert_eq!(plan.report.changes.len(), 1);
        assert_eq!(plan.report.changes[0].to.0, "0.1.0");
    }
}
