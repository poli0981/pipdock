//! What a Code Health run produces (CODE-HEALTH-SPEC §5).
//!
//! The shapes here are **not** §5's sketch. deptry, vulture and ruff were run at their pinned
//! versions and their real output is what these describe; where §5 differs it is amended, and the
//! difference is recorded on the type it belongs to.

use std::collections::BTreeMap;

use crate::errors::Code;

use super::project::DeclaredSource;

/// A Code Health run, whole or partial.
///
/// The only type in this module that reaches the wire, so the only one in `SCHEMA_TYPES` — the
/// bindings generator hoists everything it references out of `$defs` into its own TS declaration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    /// Absolute project folder the tools ran in. Data, never localized (I18N §2).
    pub project: String,
    /// `canonical_interpreter` of the environment deptry compared against.
    pub env: String,
    /// When the run finished, RFC 3339.
    pub ran_at: String,
    /// The version each tool reported, out of the tools manifest.
    pub tool_versions: BTreeMap<String, String>,
    /// What the project declares its dependencies in (§3's detection order).
    pub declared: DeclaredSource,
    /// Which tools were asked to run.
    ///
    /// **What makes an empty list readable.** Without it, "no deptry findings" and "deptry never
    /// ran" are the same empty array, and the UI would have to render one as the other — which is
    /// the "never render a state you have not loaded" rule applied to a tab.
    pub ran: Vec<String>,
    /// deptry's findings, grouped by dependency.
    pub deptry: Vec<DeptryIssue>,
    /// vulture's findings, in the order it reported them.
    pub vulture: Vec<VultureFinding>,
    /// ruff's findings, plus the counts P5's confirm dialog needs.
    pub ruff: RuffFindings,
    /// Why a requested tool contributed nothing.
    ///
    /// **Empty means every tool in `ran` completed.** This is what makes `PD-HLT-003`'s "partial
    /// report shown" a shape rather than a promise: one tool failing does not fail the run, it
    /// lands here and the other two still report.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<ToolProblem>,
}

/// One tool that was asked to run and did not finish.
///
/// Deliberately `PdError`-shaped so the UI can hand it straight to `PdErrorRow` rather than
/// inventing a second error presentation for the same catalog codes.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolProblem {
    /// Which tool: `deptry`, `vulture` or `ruff`.
    pub tool: String,
    /// The catalog code, so the row is localized from the same table as every other error.
    pub code: Code,
    /// Developer-facing detail. Never shown as-is (I18N §1).
    pub message: String,
    /// Tail of the tool's stderr, capped as ERROR-CATALOG §3 caps every other one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_tail: Option<String>,
}

/// One deptry violation, grouped by the dependency it is about.
///
/// **deptry emits a flat list**, one object per `(code, module, location)`:
/// `{"error":{"code","message"},"module","location":{"file","line","column"}}`. §5 sketched a
/// per-dependency object with a `locations` array; that shape does not exist. Grouping happens here
/// rather than in the UI so the GUI, `pipdock health --json` and P4's saved report cannot disagree.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeptryIssue {
    /// deptry's own code — `DEP001`..`DEP004`. Never a PipDock catalog code.
    pub code: String,
    /// The name deptry reported.
    ///
    /// **A module name, not necessarily a distribution.** `yaml` is `PyYAML`, `cv2` is
    /// `opencv-python`. Anything handing this to a package operation has to reconcile it first.
    pub dep: String,
    /// deptry's message, verbatim.
    ///
    /// Not a `kind` derived from the code: that mapping would be PipDock's, maintained in two
    /// languages, against a tool that adds codes on its own schedule.
    pub message: String,
    /// Where it was found. Empty when deptry named no location.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<SourceLocation>,
}

/// A file and, when the tool knew it, a position in it.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct SourceLocation {
    /// Relative to the project folder where the tool reported it that way.
    pub file: String,
    /// 1-based. Absent for a whole-file finding such as an unused dependency in `pyproject.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// 1-based.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

/// One vulture finding.
///
/// **vulture has no machine-readable output.** Its text form is
/// `<path>:<line>: <message> (<NN>% confidence)`, and `message` is `unused <typ> '<name>'` for
/// seven kinds and `unreachable code after '<token>'` for the eighth — which is why `name` is
/// optional rather than parsed out of every line. §5 has it required; amended.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VultureFinding {
    /// As vulture printed it, relative to the project folder.
    pub path: String,
    /// 1-based.
    pub line: u32,
    /// The whole message, verbatim.
    pub message: String,
    /// The identifier, when the message names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// vulture's confidence percentage. Below 100 is a candidate, not a fact (§6).
    pub confidence: u8,
}

/// ruff's findings and the two counts the fix path is built on.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuffFindings {
    /// Every finding, in ruff's order.
    pub findings: Vec<RuffFinding>,
    /// How many carry a **safe** fix — what `ruff check --fix` would actually apply.
    ///
    /// Counted here rather than in the UI because P5's confirm dialog and the CLI's prompt have to
    /// name the same number, and two implementations of "count the safe ones" will not stay equal.
    pub fixable: usize,
    /// How many distinct files those touch. The number P5's dialog names.
    pub fixable_files: usize,
}

/// One ruff finding.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuffFinding {
    /// The rule code, e.g. `F401`.
    ///
    /// Optional because the field is not ours. ruff 0.16.0 always sets it — a syntax error comes
    /// through as `invalid-syntax` rather than null, which is **not** what CODE-HEALTH-SPEC §6
    /// assumed — but tolerating null costs one `??` in the UI and stops a format change from
    /// failing every run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The rule slug, e.g. `unused-import`.
    pub name: String,
    /// ruff's message, verbatim.
    pub message: String,
    /// Absolute, as ruff reports it.
    pub filename: String,
    /// 1-based.
    pub row: u32,
    /// 1-based.
    pub column: u32,
    /// ruff's own documentation link.
    ///
    /// **Carried, never constructed.** The URL is keyed by rule *name*, so §6's
    /// `https://docs.astral.sh/ruff/rules/<code>` 404s — `I001` lives at `.../unsorted-imports`.
    /// Null for a syntax error, which has no rule page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Whether ruff offers a fix, and how confident it is in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<FixApplicability>,
}

/// How safe ruff considers its own fix.
///
/// Only `Safe` is ever applied: P5 runs plain `ruff check --fix`, which is what `--unsafe-fixes`
/// exists to opt out of, and CODE-HEALTH-SPEC §1 makes the write path the narrow one.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum FixApplicability {
    /// Applied by `ruff check --fix`.
    Safe,
    /// Needs `--unsafe-fixes`, which PipDock never passes.
    Unsafe,
    /// Shown but never applied.
    Display,
}

impl RuffFindings {
    /// Recount `fixable` and `fixable_files` from `findings`.
    ///
    /// The two counts are derived rather than accumulated so they cannot drift from the list they
    /// describe — P5 refuses to run when the count it was confirmed against no longer matches.
    #[must_use]
    pub fn recount(mut self) -> Self {
        let safe: Vec<&RuffFinding> = self
            .findings
            .iter()
            .filter(|f| f.fix == Some(FixApplicability::Safe))
            .collect();

        self.fixable = safe.len();
        self.fixable_files = safe
            .iter()
            .map(|f| f.filename.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        self
    }
}
