//! Reading a `requirements.txt` — PRD P1-3.
//!
//! **Writing one needs no code.** A `pip freeze` document *is* a requirements file, and
//! [`crate::engine::Engine::freeze`] already produces it for snapshots. Export is that string and
//! a path. This module is the other direction.
//!
//! # Why not [`crate::snapshot::parse_freeze`]
//!
//! That parser is deliberately narrow: `name==version`, nothing else. Its narrowness is what makes
//! it the exact complement of [`crate::snapshot::unrestorable_lines`], an invariant a rollback
//! preview depends on — so widening it to read a hand-written requirements file would break the
//! thing it exists for. A file a person wrote has ranges, extras, markers, hashes, includes and
//! comments after the spec, and none of those belong in a freeze.
//!
//! # What is refused, and why refusing beats guessing
//!
//! `-r other.txt` and `-c constraints.txt` pull in files PipDock has not read and the user has not
//! seen in the preview. `-e .` and direct URLs install from somewhere other than the index. Each
//! is **reported by line** rather than skipped, because a silently shorter install list is the
//! failure mode DATA-FLOW §9 exists to prevent: the preview would be honest about what it is doing
//! and wrong about what the user asked for.

use crate::graph::Requirement;
use crate::model::PkgName;

/// A line the parser would not turn into an install spec, and what was wrong with it.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct SkippedLine {
    /// 1-based, so it matches what an editor shows.
    pub line: usize,
    /// The line, trimmed. Data — shown verbatim, never translated (I18N §2).
    pub text: String,
    /// Why, as a catalog-style discriminant the UI turns into a sentence.
    pub reason: SkipReason,
}

/// Why a line was not turned into a spec.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum SkipReason {
    /// `-r` or `-c`: another file, which PipDock has not read.
    Include,
    /// `-e`, or a `name @ url` / VCS requirement: installs from somewhere other than the index.
    NotFromIndex,
    /// An option line (`--index-url`, `--extra-index-url`, …) that changes how *everything*
    /// installs. Honouring one silently would change the meaning of the whole file.
    Option,
    /// The line is not a requirement PipDock can parse at all.
    Unparsed,
}

/// What [`parse`] made of a file.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct ParsedRequirements {
    /// Install specs, in file order, as `Intent::Install` wants them.
    pub specs: Vec<String>,
    /// Everything else, with its line number and reason.
    pub skipped: Vec<SkippedLine>,
}

impl ParsedRequirements {
    /// True when there is nothing to install.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }
}

/// Strip an end-of-line comment.
///
/// PEP 508 requires a space before the `#`, which is what makes this safe: a `#` inside a URL
/// fragment or a version's local segment has no space in front of it and survives.
fn strip_comment(line: &str) -> &str {
    match line.find(" #") {
        Some(at) => &line[..at],
        None => line,
    }
}

/// Drop `--hash=...` fragments and any trailing line-continuation marker.
///
/// Hashes are pip's integrity check against the index, not part of *what* to install, and PipDock
/// does not download the artefact itself. Keeping them would put an unparseable token into argv.
fn strip_hashes(line: &str) -> String {
    line.split_whitespace()
        .filter(|tok| !tok.starts_with("--hash") && *tok != "\\")
        .collect::<Vec<_>>()
        .join(" ")
}

/// Read a requirements file into install specs.
///
/// Joins continuation lines first, so a requirement split over several physical lines is one
/// logical requirement — pip allows it and a file that uses it would otherwise produce one
/// unparseable fragment per line.
#[must_use]
pub fn parse(text: &str) -> ParsedRequirements {
    let mut out = ParsedRequirements::default();

    // (line number of the *first* physical line, joined text).
    let mut logical: Vec<(usize, String)> = Vec::new();
    let mut pending: Option<(usize, String)> = None;
    for (i, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        let continues = trimmed.ends_with('\\');
        let body = trimmed.trim_end_matches('\\').trim();
        match pending.as_mut() {
            Some((_, acc)) => {
                acc.push(' ');
                acc.push_str(body);
            }
            None => pending = Some((i + 1, body.to_owned())),
        }
        if !continues && let Some(entry) = pending.take() {
            logical.push(entry);
        }
    }
    if let Some(entry) = pending.take() {
        logical.push(entry);
    }

    for (line, raw) in logical {
        let body = strip_comment(&raw).trim();
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        let body = strip_hashes(body);
        let body = body.trim();

        let skip = |reason: SkipReason| SkippedLine {
            line,
            text: body.to_owned(),
            reason,
        };

        if body.starts_with("-r") || body.starts_with("--requirement") {
            out.skipped.push(skip(SkipReason::Include));
            continue;
        }
        if body.starts_with("-c") || body.starts_with("--constraint") {
            out.skipped.push(skip(SkipReason::Include));
            continue;
        }
        if body.starts_with("-e") || body.starts_with("--editable") {
            out.skipped.push(skip(SkipReason::NotFromIndex));
            continue;
        }
        if body.starts_with('-') {
            out.skipped.push(skip(SkipReason::Option));
            continue;
        }
        // `name @ url`, and bare URLs. Both install from outside the index, and both would reach
        // argv as something PipDock never validated.
        if body.contains('@') || body.contains("://") {
            out.skipped.push(skip(SkipReason::NotFromIndex));
            continue;
        }

        // `Requirement::parse` already understands `name[extras] (spec); marker` and is exercised
        // against real `Requires-Dist` metadata — the closest thing to a PEP 508 line parser in
        // the tree, and reusing it is what stops this file inventing a second grammar.
        //
        // **But its input is normally trusted and this input is not.** It splits the name at the
        // first space, so `this is not a requirement` parses happily as name `this` with
        // constraint `is not a requirement` — which `render` would emit as a spec containing
        // spaces, straight into argv. `Requires-Dist` never looks like that; a file a person typed
        // can. So the constraint is checked here, which is the same obligation SECURITY §2 puts on
        // every other user-supplied version.
        match Requirement::parse(body) {
            Some(req) if is_specifier(&req.constraint) => out.specs.push(render(&req)),
            _ => out.skipped.push(skip(SkipReason::Unparsed)),
        }
    }

    out
}

/// Is this a PEP 440 version specifier set, rather than prose that happened to follow a word?
///
/// Deliberately a shape check, not a full parse — the same trade [`crate::pins`] makes for a
/// `Hold` version, and for the same reason: refusing a legitimate specifier would make a package
/// uninstallable, while the job here is only to reject whitespace and anything that is not part
/// of a specifier's alphabet. An empty constraint is valid and means "any version".
fn is_specifier(raw: &str) -> bool {
    if raw.is_empty() {
        return true;
    }
    raw.starts_with(['<', '>', '=', '!', '~'])
        && raw.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    '<' | '>' | '=' | '!' | '~' | '.' | ',' | '*' | '-' | '_' | '+'
                )
        })
}

/// Render a parsed requirement back as a spec `Intent::Install` accepts.
///
/// **The marker is dropped, and the extras with it.** A marker is a condition on the *installing*
/// environment that the resolver evaluates itself, and PipDock's job here is to say which packages
/// to ask for. Keeping either would put a token into argv that
/// `plan::build_request`'s `name==version` split does not understand.
fn render(req: &Requirement) -> String {
    if req.constraint.is_empty() {
        req.name.as_str().to_owned()
    } else {
        format!("{}{}", req.name.as_str(), req.constraint)
    }
}

/// The names a parse produced, for a caller that wants to check them against what is installed.
#[must_use]
pub fn names(parsed: &ParsedRequirements) -> Vec<PkgName> {
    parsed
        .specs
        .iter()
        .filter_map(|s| {
            let end = s
                .find(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '.' | '-' | '_'))
                .unwrap_or(s.len());
            PkgName::parse(&s[..end]).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_freeze_document_round_trips() {
        // The commonest input by far: what PipDock's own export writes.
        let got = parse("requests==2.32.3\nurllib3==2.2.1\n");
        assert_eq!(got.specs, ["requests==2.32.3", "urllib3==2.2.1"]);
        assert!(got.skipped.is_empty());
    }

    #[test]
    fn ranges_and_unconstrained_names_survive() {
        let got = parse("django>=5.0,<6\nrich\n");
        assert_eq!(got.specs, ["django>=5.0,<6", "rich"]);
    }

    #[test]
    fn comments_and_blank_lines_say_nothing() {
        let got = parse("# top\n\nrequests==2.32.3  # pinned by ops\n\n# end\n");
        assert_eq!(got.specs, ["requests==2.32.3"]);
        assert!(got.skipped.is_empty(), "a comment is not a skipped line");
    }

    #[test]
    fn a_hash_is_dropped_rather_than_reaching_argv() {
        // pip's integrity check against the index. PipDock does not fetch the artefact, and the
        // token is not a thing the engine's spec grammar accepts.
        let got = parse("requests==2.32.3 --hash=sha256:abc123\n");
        assert_eq!(got.specs, ["requests==2.32.3"]);
        assert!(got.skipped.is_empty());
    }

    #[test]
    fn a_continuation_is_one_requirement_not_two_fragments() {
        let got = parse("requests==2.32.3 \\\n    --hash=sha256:abc \\\n    --hash=sha256:def\n");
        assert_eq!(got.specs, ["requests==2.32.3"]);
        assert!(got.skipped.is_empty());
    }

    #[test]
    fn extras_and_markers_are_dropped_from_the_spec() {
        // Both are conditions the resolver evaluates for itself; neither is part of "which
        // package". A marker reaching argv is a token `build_request` cannot split.
        let got = parse("requests[socks]>=2.0; python_version >= \"3.10\"\n");
        assert_eq!(got.specs, ["requests>=2.0"]);
    }

    #[test]
    fn an_include_is_reported_by_line_rather_than_skipped_quietly() {
        // The file it names has not been read and will not appear in the preview. A silently
        // shorter install list is the failure DATA-FLOW §9 exists to prevent.
        let got = parse("requests==2.32.3\n-r dev-requirements.txt\n");
        assert_eq!(got.specs, ["requests==2.32.3"]);
        assert_eq!(got.skipped.len(), 1);
        assert_eq!(got.skipped[0].line, 2);
        assert_eq!(got.skipped[0].reason, SkipReason::Include);
        assert_eq!(got.skipped[0].text, "-r dev-requirements.txt");
    }

    #[test]
    fn editable_and_url_requirements_are_refused_as_not_from_the_index() {
        let got = parse("-e .\nfoo @ https://example.invalid/foo.whl\nhttps://x.invalid/b.whl\n");
        assert!(got.specs.is_empty());
        assert_eq!(got.skipped.len(), 3);
        assert!(
            got.skipped
                .iter()
                .all(|s| s.reason == SkipReason::NotFromIndex)
        );
    }

    #[test]
    fn an_index_option_changes_the_whole_file_and_is_refused() {
        // `--index-url` changes where *everything* comes from. Honouring it silently would make
        // the preview describe an install from somewhere the user was never shown.
        let got = parse("--index-url https://internal.invalid/simple\nrequests==2.32.3\n");
        assert_eq!(got.specs, ["requests==2.32.3"]);
        assert_eq!(got.skipped[0].reason, SkipReason::Option);
    }

    #[test]
    fn line_numbers_are_the_ones_an_editor_shows() {
        let got = parse("# a\n\n-r b.txt\n");
        assert_eq!(got.skipped[0].line, 3);
    }

    #[test]
    fn prose_is_reported_rather_than_becoming_a_package_named_after_its_first_word() {
        // `Requirement::parse` splits the name at the first space, so this yields name `this`
        // with constraint `is not a requirement` — perfectly reasonable for `Requires-Dist`,
        // which never looks like prose, and a spec containing spaces heading for argv here.
        let got = parse("this is not a requirement\n");
        assert!(got.specs.is_empty(), "got {:?}", got.specs);
        assert_eq!(got.skipped[0].reason, SkipReason::Unparsed);
    }

    #[test]
    fn every_specifier_shape_pip_accepts_survives_the_check() {
        // The check is a shape test, not a PEP 440 parse: rejecting a legitimate specifier would
        // make a package uninstallable, which is the worse failure.
        for spec in [
            "requests==2.32.3",
            "requests>=2.0,<3",
            "requests~=2.32.0",
            "requests!=2.31.0",
            "requests===2.32.3",
            "requests==2.32.*",
            "requests>=1.0.0-rc.1",
            "requests==1!2.0+local.1",
        ] {
            let got = parse(&format!("{spec}\n"));
            assert_eq!(got.specs, [spec], "rejected a valid specifier: {spec}");
        }
    }

    #[test]
    fn names_reads_back_what_was_asked_for() {
        let parsed = parse("requests==2.32.3\ndjango>=5.0,<6\nrich\n");
        let read = names(&parsed);
        let got: Vec<&str> = read.iter().map(PkgName::as_str).collect();
        assert_eq!(got, ["requests", "django", "rich"]);
    }
}
