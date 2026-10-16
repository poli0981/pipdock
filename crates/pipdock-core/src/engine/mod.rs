//! The `Engine` trait and its two adapters.
//!
//! ARCHITECTURE §1.2: **explain, don't reimplement.** Resolution is always performed by the
//! selected engine in dry-run mode; this module parses and normalizes, and never computes version
//! resolution itself.
//!
//! ARCHITECTURE §1.5: engines are invoked with argv arrays via `tokio::process::Command`,
//! **never through a shell**.

pub mod parse;
pub mod pip;
pub mod uv;

use async_trait::async_trait;

use crate::errors::Result;
use crate::model::{
    CheckReport, Dist, EngineId, EngineInfo, ExecMode, OutdatedDist, PinnedSpec, PkgName, PyEnv,
    StepResult, StepStatus,
};
use crate::plan::{PlanRequest, ResolutionReport};
/// Re-exported: [`ProgressSink::new`] demands one, so anything driving a step needs the type.
/// Without this each head would take its own `tokio-util` dependency for one struct, and two
/// versions of it would not be the same token.
pub use tokio_util::sync::CancellationToken;

/// Where live subprocess output goes.
///
/// The GUI forwards these to the `plan-progress` Tauri event feeding the console drawer
/// (ARCHITECTURE §7); the CLI writes them as NDJSON when `--json` is set (CLI-SPEC §6).
pub type EventSink = tokio::sync::mpsc::UnboundedSender<ProgressEvent>;

/// Everything one step needs from whoever is driving it.
///
/// Replaces a bare [`EventSink`] on the mutating trait methods, because an adapter needs three
/// things from its caller and passing them separately is how the first two got lost:
///
/// * where to send output — the sink;
/// * **which step this is, and how many there are.** `step` was previously hardcoded to `0` at
///   all four call sites, since an adapter has no way to know its own index. That made
///   UI-SPEC §3's per-package section markers and §8's "13 of 15 complete" live region
///   unimplementable — the data they need was never emitted;
/// * whether to stop — the cancellation token, which the adapter hands to [`crate::exec::Command`].
#[derive(Debug, Clone)]
pub struct ProgressSink {
    /// Where lines go.
    pub tx: EventSink,
    /// Zero-based index of this step within the plan.
    pub step: usize,
    /// How many steps the plan has, so a caller can render progress without counting.
    pub total: usize,
    /// Tripped by `plan_cancel` (ARCHITECTURE §7).
    pub cancel: CancellationToken,
}

impl ProgressSink {
    /// Announce that step `step` is starting.
    ///
    /// Emitted by the executor rather than the adapter: an adapter runs one command and cannot
    /// know whether it is step 3 of 15, which is the same reason `step` itself lives here.
    pub fn started(&self, pkg: Option<PkgName>, phase: ExecMode) {
        let _ = self.tx.send(ProgressEvent::StepStarted {
            step: self.step,
            total: self.total,
            pkg,
            phase,
        });
    }

    /// Announce that step `step` has finished, and how.
    pub fn finished(&self, pkg: Option<PkgName>, phase: ExecMode, status: StepStatus) {
        let _ = self.tx.send(ProgressEvent::StepFinished {
            step: self.step,
            total: self.total,
            pkg,
            phase,
            status,
        });
    }

    /// A sink for a plan of `total` steps, starting at step zero.
    #[must_use]
    pub fn new(tx: EventSink, total: usize, cancel: CancellationToken) -> Self {
        Self {
            tx,
            step: 0,
            total,
            cancel,
        }
    }

    /// The same sink, reporting as step `step`.
    #[must_use]
    pub fn at(&self, step: usize) -> Self {
        Self {
            step,
            ..self.clone()
        }
    }

    /// True once the plan has been cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

/// Which stream a line came from.
///
/// The console drawer renders them differently, and for uv the distinction is load-bearing in a
/// way it is not for pip: uv writes its **plan** to stderr (SP-1), so "stderr" here does not mean
/// "something went wrong".
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    /// The engine's stdout.
    Stdout,
    /// The engine's stderr.
    Stderr,
}

/// One event on the `plan-progress` channel (ARCHITECTURE §7).
///
/// Deferred from Stage 1 to the slice that could verify it. It was a bare line, which made two
/// documented features unimplementable: UI-SPEC §3's per-package section markers in the console
/// drawer had nothing to mark a section with, and §8's "13 of 15 complete" live region had no
/// event that meant "one finished". Neither can be recovered from the text — an engine's output
/// does not reliably say which package it is about, and counting lines is not counting steps.
///
/// A tagged lifecycle instead: every step emits exactly one [`Self::StepStarted`], any number of
/// [`Self::Line`]s, and exactly one [`Self::StepFinished`]. That makes the drawer's grouping and
/// the live region's counter both mechanical rather than inferred.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProgressEvent {
    /// A step is about to run. Opens a section in the console drawer.
    StepStarted {
        /// Zero-based index of the step within the plan.
        step: usize,
        /// How many steps the plan has, so the caller can render progress without counting.
        total: usize,
        /// The package this step is for, absent for a batch covering the whole set.
        #[serde(skip_serializing_if = "Option::is_none")]
        pkg: Option<PkgName>,
        /// Which execution phase is running.
        phase: ExecMode,
    },
    /// One line of the engine's output, verbatim and never localized (I18N §2).
    Line {
        /// Zero-based index of the step within the plan.
        step: usize,
        /// The package this line belongs to, absent for batch-wide output.
        #[serde(skip_serializing_if = "Option::is_none")]
        pkg: Option<PkgName>,
        /// Which execution phase is running.
        phase: ExecMode,
        /// Which stream produced it.
        stream: Stream,
        /// The line itself.
        line: String,
    },
    /// A step has finished. Closes its section, and advances the live region's counter.
    StepFinished {
        /// Zero-based index of the step within the plan.
        step: usize,
        /// How many steps the plan has.
        total: usize,
        /// The package this step was for.
        #[serde(skip_serializing_if = "Option::is_none")]
        pkg: Option<PkgName>,
        /// Which execution phase produced it.
        phase: ExecMode,
        /// How it ended.
        status: StepStatus,
    },
}

impl ProgressEvent {
    /// The text a plain consumer should show, if any.
    ///
    /// The CLI streams engine output to stderr and has no use for the markers; this keeps that a
    /// one-liner rather than a match at every call site.
    #[must_use]
    pub fn line(&self) -> Option<&str> {
        match self {
            Self::Line { line, .. } => Some(line),
            Self::StepStarted { .. } | Self::StepFinished { .. } => None,
        }
    }
}

/// The contract both adapters implement. See ARCHITECTURE §3.
#[async_trait]
pub trait Engine: Send + Sync {
    /// Which engine this is.
    fn id(&self) -> EngineId;

    /// Version and availability for `env`.
    async fn info(&self, env: &PyEnv) -> EngineInfo;

    /// Everything installed in `env`.
    async fn list_installed(&self, env: &PyEnv) -> Result<Vec<Dist>>;

    /// Installed packages with a newer release available.
    async fn list_outdated(&self, env: &PyEnv) -> Result<Vec<OutdatedDist>>;

    /// Dry-run resolve `req` against `env`, normalized into a [`ResolutionReport`].
    ///
    /// This is the only way a plan may be produced; DATA-FLOW §9.1 forbids any mutating call
    /// without a report accepted in the same session. `ctx` carries the cancellation token —
    /// a resolve used to be the one engine call nothing could stop, so *Stop* while pip
    /// backtracked left it running for up to ten minutes — and what the flow changes about the
    /// guard set (forced pins, holds, exclusions).
    async fn resolve(
        &self,
        env: &PyEnv,
        req: &PlanRequest,
        ctx: &ResolveCtx,
    ) -> Result<ResolutionReport>;

    /// Install an exact pinned set, holding everything in `set.constraints` where it is.
    ///
    /// `Batch` mode passes the whole set in one invocation (Phase A); `Isolated` mode is the
    /// per-package retry loop (Phase B), where a failure must **not** stop the caller's loop.
    async fn install(
        &self,
        env: &PyEnv,
        set: &InstallSet,
        mode: ExecMode,
        sink: ProgressSink,
    ) -> Result<StepResult>;

    /// Remove packages. Always sequential; the reverse-dependency guard runs once up front
    /// against the full removal set (ARCHITECTURE §8).
    async fn uninstall(
        &self,
        env: &PyEnv,
        names: &[PkgName],
        sink: ProgressSink,
    ) -> Result<StepResult>;

    /// `pip check` / `uv pip check`, normalized.
    async fn check(&self, env: &PyEnv) -> Result<CheckReport>;

    /// Capture the environment as a freeze document, for snapshots.
    ///
    /// The two engines do not capture the same thing: pip is invoked with `--all` so pip and
    /// setuptools are included, while uv has no such flag and omits them (DATA-FLOW §7). A
    /// snapshot therefore records which engine produced it, and a rollback must be planned
    /// against that same understanding — which is why [`crate::snapshot::Meta`] stores the engine.
    async fn freeze(&self, env: &PyEnv) -> Result<String>;

    /// Upgrade pip inside `env`. The uv adapter returns an `Unsupported` error — DATA-FLOW §7
    /// says pip upkeep is surfaced only when pip is the active engine or present in the env.
    async fn upgrade_pip(&self, env: &PyEnv) -> Result<StepResult>;

    /// Probe `env`: the interpreter, its marker environment, and every distribution with its
    /// requirements.
    ///
    /// Provided, and no adapter overrides it: this is `probe.py`, not an engine command. It is on
    /// the trait so a flow can be driven end to end against a scripted engine — every flow probed
    /// through a free function, so none of `start`'s decisions (the PEP 668 gate, the graph, the
    /// drift basis) could be tested without a real interpreter, and the bugs in them were not.
    async fn probe(&self, env: &PyEnv) -> Result<crate::envs::ProbeResult> {
        crate::envs::probe(&env.interpreter, env.source).await
    }
}

/// What one resolve is given besides the request itself.
#[derive(Debug, Clone, Default)]
pub struct ResolveCtx {
    /// Stops the dry run; a stopped resolve returns an error with `Stop::Cancelled`.
    pub cancel: tokio_util::sync::CancellationToken,
    /// What the flow changes about the requirement lines the engine is given.
    pub adjust: GuardAdjust,
}

/// Changes to the requirement lines of a resolve, made by the flow — never by an adapter.
///
/// SP-1 makes the guard set (every installed package the user is not moving, pinned) the default,
/// and three decisions need to depart from it on purpose: **Force latest** pins a package to its
/// latest and *releases* the packages whose constraints were holding it (DISCLAIMER §2); a **Hold**
/// pin restates its own version; a release whose `Requires-Python` excludes this environment is
/// **excluded** so the engine picks another.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuardAdjust {
    /// Exact requirements added to the resolve; these names leave the upgrade list and the guard.
    pub pins: Vec<PinnedSpec>,
    /// Installed packages released from the guard set — what a forced package will break.
    pub release: Vec<PkgName>,
    /// `name!=version` lines, for releases that cannot install here.
    pub exclude: Vec<PinnedSpec>,
}

/// One install invocation: what to install, and what must not move while it happens.
///
/// SP-1's rule — restate the installed set or the resolver breaks it — was applied to the
/// *resolve* only. Phase B then installed each package as a bare `pip install name==v`, with no
/// guard: free to pull a dependency past an installed package's upper bound, which pip reports as
/// a warning and exits 0 on. `constraints` carries the target state of everything else, passed as
/// `-c`, which constrains without installing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallSet {
    /// The exact pins to install.
    pub specs: Vec<PinnedSpec>,
    /// Everything that must stay at these versions if the engine touches it at all.
    pub constraints: Vec<PinnedSpec>,
}

impl InstallSet {
    /// Install `specs`, constraining nothing else.
    #[must_use]
    pub const fn only(specs: Vec<PinnedSpec>) -> Self {
        Self {
            specs,
            constraints: Vec::new(),
        }
    }
}

/// The requirements (and constraints) files one engine invocation reads, deleted when dropped.
///
/// **Files, not argv.** Every plan restates the whole installed set (SP-1), one argv entry per
/// package — and at around 1,300 packages that passes Windows' 32,767-character command line, so
/// the spawn failed and the user was told to install an engine (PD-ENG-001). `-r`/`-c` keep the
/// command line a constant length. The files hold only rendered [`PinnedSpec`]s and validated
/// [`crate::model::Spec`]s — the same tokens argv held — written under fresh random names.
#[derive(Debug)]
pub struct RequirementFiles {
    requirements: crate::exec::TempFile,
    constraints: Option<crate::exec::TempFile>,
    /// What each file says, line by line, so [`Self::humanize`] can put it back.
    requirement_lines: Vec<String>,
    constraint_lines: Vec<String>,
}

impl RequirementFiles {
    /// Write `requirements` (one per line) and, when there are any, `constraints`.
    ///
    /// # Errors
    /// As [`crate::exec::TempFile::create`].
    pub fn write(requirements: &[String], constraints: &[PinnedSpec]) -> Result<Self> {
        let text = |lines: &[String]| {
            let mut text = lines.join("\n");
            text.push('\n');
            text
        };
        let requirement_lines = requirements.to_vec();
        let constraint_lines: Vec<String> =
            constraints.iter().map(PinnedSpec::to_requirement).collect();
        let requirements =
            crate::exec::TempFile::create("pipdock-req", "txt", &text(&requirement_lines))?;
        let constraints = if constraint_lines.is_empty() {
            None
        } else {
            Some(crate::exec::TempFile::create(
                "pipdock-con",
                "txt",
                &text(&constraint_lines),
            )?)
        };
        Ok(Self {
            requirements,
            constraints,
            requirement_lines,
            constraint_lines,
        })
    }

    /// Engine text with every reference to these files replaced by the line it points at.
    ///
    /// pip names a requirement that came from a file as `-r <path> (line N)`. Once the plan went
    /// through a file, its impossibility read `Cannot install -r C:\…\pipdock-req-….txt (line 1)
    /// and h11==0.12.0`: the preview lost the name of the package the user asked for, and every
    /// error row showed a path that had already been deleted. Found by running pip, not by
    /// reading it — uv names requirements by their text and needs none of this.
    #[must_use]
    pub fn humanize(&self, text: &str) -> String {
        let mut out = replace_file_refs(
            text,
            &format!("-r {}", self.requirements.path().display()),
            &self.requirement_lines,
        );
        if let Some(c) = &self.constraints {
            out = replace_file_refs(
                &out,
                &format!("-c {}", c.path().display()),
                &self.constraint_lines,
            );
        }
        out
    }

    /// `-r <requirements> [-c <constraints>]`, one argv entry each.
    #[must_use]
    pub fn args(&self) -> Vec<String> {
        let mut out = vec![
            "-r".to_owned(),
            self.requirements.path().display().to_string(),
        ];
        if let Some(c) = &self.constraints {
            out.push("-c".to_owned());
            out.push(c.path().display().to_string());
        }
        out
    }
}

/// `<reference> (line N)` becomes line N; `(from <reference> (line N))` goes entirely, since the
/// requirement it annotates is already named just before it.
fn replace_file_refs(text: &str, reference: &str, lines: &[String]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(reference) {
        let (head, after) = rest.split_at(at);
        let after = &after[reference.len()..];
        let parsed = after.strip_prefix(" (line ").and_then(|tail| {
            let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
            let n: usize = tail[..digits].parse().ok()?;
            let close = tail[digits..].strip_prefix(')')?;
            Some((n, close))
        });
        let Some((n, close)) = parsed else {
            // Not a line reference; keep it as it was.
            out.push_str(head);
            out.push_str(reference);
            rest = after;
            continue;
        };
        if let (Some(before), Some(close)) = (head.strip_suffix("(from "), close.strip_prefix(')'))
        {
            out.push_str(before.strip_suffix(' ').unwrap_or(before));
            rest = close;
            continue;
        }
        out.push_str(head);
        match n.checked_sub(1).and_then(|i| lines.get(i)) {
            Some(line) => out.push_str(line),
            None => out.push_str(&rest[at..rest.len() - close.len()]),
        }
        rest = close;
    }
    out.push_str(rest);
    out
}

/// The adapter for `id`.
///
/// Both heads reach this point by different routes — the CLI from `--engine` or the stored
/// setting, the GUI from the stored setting alone — and both then need a `Box<dyn Engine>`. The
/// mapping itself lives here so neither can grow its own idea of what "uv" selects (G5). The CLI
/// previously owned the only copy, along with a second declaration of the settings key it reads.
#[must_use]
pub fn for_id(id: EngineId) -> Box<dyn Engine> {
    match id {
        EngineId::Pip => Box::new(pip::PipEngine),
        EngineId::Uv => Box::new(uv::UvEngine),
    }
}

/// pip's floor for `install --dry-run --report -`. Below this, planning is impossible and the app
/// offers a one-click pip upgrade (`PD-ENG-002`, DATA-FLOW §7).
pub const PIP_MIN_VERSION_FOR_REPORT: (u32, u32) = (22, 2);

/// uv's floor, pinned by spike SP-1: 0.10.12 and 0.11.32 produce identical plan formats.
pub const UV_MIN_VERSION: (u32, u32) = (0, 10);

/// Build the extra requirements a dry-run resolve must be given alongside the user's own.
///
/// **This is the single most important consequence of spike SP-1.** `<engine> install -U <pkg>`
/// ignores the constraints of packages already installed: seeded with `httpx 0.23.0`, which
/// requires `httpcore<0.16`, both pip and uv plan `httpcore 1.0.9` and exit 0, silently breaking
/// it. Restating the installed set as explicit `name==version` requirements is what makes the
/// resolver hold packages back instead.
///
/// Returned here is the guard group: **everything installed that the user is not moving**, pinned
/// to its current version. Packages the user chose to upgrade or install are deliberately absent
/// so they remain free to move.
///
/// Without this, PipDock would preview a plan that breaks the environment — the exact failure the
/// product exists to prevent.
#[must_use]
pub fn plan_requirements(req: &PlanRequest, installed: &[crate::model::Dist]) -> Vec<PinnedSpec> {
    use std::collections::BTreeSet;

    let moving: BTreeSet<&PkgName> = req
        .upgrades
        .iter()
        .chain(req.installs.iter().map(|s| &s.name))
        .collect();

    installed
        .iter()
        .filter(|d| !moving.contains(&d.name))
        // A version that could be read as more than a version — whitespace, a marker, a URL —
        // never reaches argv. pip itself cannot install such a distribution, so restating it
        // would only fail the resolve; leaving it out leaves it untouched, which is what a guard
        // is for.
        .filter(|d| crate::pep440::is_argv_safe(&d.version.0))
        .map(|d| PinnedSpec {
            name: d.name.clone(),
            version: d.version.clone(),
        })
        .collect()
}

/// The complete requirement list a dry-run resolve is given.
///
/// Three groups, and **all three are required**:
///
/// 1. the packages the user wants moved, as **bare names**, so `-U` is free to take them anywhere;
/// 2. the packages the user wants installed, with whatever specifier they gave;
/// 3. the guard set from [`plan_requirements`], pinned to current versions.
///
/// Omitting group 1 is not a small mistake: the engine is then asked to upgrade nothing, reports
/// no changes, and every package the user selected looks "held back" for no discoverable reason.
/// Omitting group 3 is the SP-1 failure — the resolver breaks installed dependents at exit 0.
#[must_use]
pub fn plan_argv_specs(req: &PlanRequest, installed: &[crate::model::Dist]) -> Vec<String> {
    plan_lines(req, installed, &GuardAdjust::default())
}

/// [`plan_argv_specs`] with the flow's [`GuardAdjust`] applied.
#[must_use]
pub fn plan_lines(
    req: &PlanRequest,
    installed: &[crate::model::Dist],
    adjust: &GuardAdjust,
) -> Vec<String> {
    use std::collections::BTreeSet;

    let pinned: BTreeSet<&PkgName> = adjust.pins.iter().map(|p| &p.name).collect();
    let released: BTreeSet<&PkgName> = adjust.release.iter().collect();

    let mut out: Vec<String> = req
        .upgrades
        .iter()
        .filter(|n| !pinned.contains(n))
        .map(ToString::to_string)
        .collect();

    out.extend(req.installs.iter().map(|s| match &s.version_req {
        Some(v) if v.starts_with(['=', '<', '>', '!', '~']) => format!("{}{v}", s.name),
        Some(v) => format!("{}=={v}", s.name),
        None => s.name.to_string(),
    }));

    out.extend(adjust.pins.iter().map(PinnedSpec::to_requirement));
    out.extend(
        plan_requirements(req, installed)
            .iter()
            .filter(|g| !pinned.contains(&g.name) && !released.contains(&g.name))
            .map(PinnedSpec::to_requirement),
    );
    out.extend(
        adjust
            .exclude
            .iter()
            .map(|e| format!("{}!={}", e.name, e.version)),
    );
    out
}

/// The package a step is about, when a step covers exactly one.
///
/// Phase A installs the whole set at once, so its progress lines belong to no single package;
/// Phase B is per-package and its lines do. The console drawer uses this to draw section markers
/// (UI-SPEC §3).
#[must_use]
pub fn single_pkg(specs: &[PinnedSpec]) -> Option<PkgName> {
    match specs {
        [only] => Some(only.name.clone()),
        _ => None,
    }
}

/// Build the [`StepResult`] for a finished install.
///
/// Failure is recorded, **not raised**. ARCHITECTURE §8 and the owner requirement behind it: a
/// failed package must not stop the batch, so this returns an `Ok` carrying a `Failed` status and
/// the classified code. The only thing that aborts a run is a snapshot failure.
#[must_use]
pub fn step_result(specs: &[PinnedSpec], out: &crate::exec::Output) -> StepResult {
    use crate::model::StepStatus;

    let pkg = single_pkg(specs).unwrap_or_else(|| {
        // A batch step is reported against the first package purely so the row has a name; the
        // summary shows Phase A as one line regardless.
        specs.first().map_or_else(
            || PkgName::parse("batch").unwrap_or_else(|_| unreachable!("literal is valid")),
            |s| s.name.clone(),
        )
    });

    if out.ok() {
        StepResult {
            pkg,
            from: None,
            to: specs.first().map(|s| s.version.clone()),
            status: StepStatus::Ok,
            code: None,
            stderr_tail: None,
        }
    } else {
        let err = crate::errors::PdError::from_engine_stderr(&out.stderr);
        StepResult {
            pkg,
            from: None,
            to: specs.first().map(|s| s.version.clone()),
            status: StepStatus::Failed,
            code: Some(err.code),
            stderr_tail: err.stderr_tail,
        }
    }
}

/// Build the [`StepResult`] for a finished removal. Same skip-and-continue rule as installs.
#[must_use]
pub fn removal_result(names: &[PkgName], out: &crate::exec::Output) -> StepResult {
    use crate::model::StepStatus;

    let pkg = names.first().cloned().unwrap_or_else(|| {
        PkgName::parse("batch").unwrap_or_else(|_| unreachable!("literal is valid"))
    });

    if out.ok() {
        StepResult {
            pkg,
            from: None,
            to: None,
            status: StepStatus::Ok,
            code: None,
            stderr_tail: None,
        }
    } else {
        let err = crate::errors::PdError::from_engine_stderr(&out.stderr);
        StepResult {
            pkg,
            from: None,
            to: None,
            status: StepStatus::Failed,
            code: Some(err.code),
            stderr_tail: err.stderr_tail,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Dist, Version};
    use crate::plan::Strategy;

    fn dist(name: &str, version: &str) -> Dist {
        Dist {
            name: PkgName::parse(name).unwrap(),
            version: Version(version.into()),
            requires_dist: Vec::new(),
            requires_python: None,
            size_bytes: None,
        }
    }

    #[test]
    fn documented_pip_floor() {
        // DATA-FLOW §7 and ERROR-CATALOG PD-ENG-002 both name 22.2; keep them in step.
        assert_eq!(PIP_MIN_VERSION_FOR_REPORT, (22, 2));
        // SP-1 verified two uv minors produce identical plan text.
        assert_eq!(UV_MIN_VERSION, (0, 10));
    }

    #[test]
    fn the_installed_set_is_restated_so_the_resolver_cannot_break_it() {
        // The SP-1 scenario: upgrading httpcore while httpx 0.23.0 is installed. httpx must be
        // pinned in the request, or both engines plan httpcore 1.0.9 and break it at exit 0.
        let installed = [
            dist("httpx", "0.23.0"),
            dist("httpcore", "0.15.0"),
            dist("h11", "0.12.0"),
        ];
        let req = PlanRequest {
            upgrades: vec![PkgName::parse("httpcore").unwrap()],
            installs: Vec::new(),
            strategy: Strategy::Compatible,
        };

        let guards = plan_requirements(&req, &installed);
        let rendered: Vec<String> = guards.iter().map(PinnedSpec::to_requirement).collect();

        assert!(
            rendered.contains(&"httpx==0.23.0".to_owned()),
            "httpx must be pinned"
        );
        assert!(rendered.contains(&"h11==0.12.0".to_owned()));
        assert!(
            !rendered.iter().any(|r| r.starts_with("httpcore==")),
            "the package being upgraded must stay free to move"
        );
    }

    #[test]
    fn a_package_being_installed_is_not_pinned_to_its_old_version() {
        let installed = [dist("idna", "3.4")];
        let req = PlanRequest {
            upgrades: Vec::new(),
            installs: vec![crate::model::Spec {
                name: PkgName::parse("idna").unwrap(),
                version_req: None,
            }],
            strategy: Strategy::Compatible,
        };
        assert!(plan_requirements(&req, &installed).is_empty());
    }

    #[test]
    fn the_resolve_command_asks_for_the_upgrades_as_well_as_the_guards() {
        // Regression: an earlier version passed only the guard set, so the engine was asked to
        // upgrade nothing. It reported no changes, and every selected package then looked "held
        // back" with no discoverable cause — the feature appeared to work and told users nothing.
        let installed = [
            dist("httpx", "0.23.0"),
            dist("httpcore", "0.15.0"),
            dist("h11", "0.12.0"),
        ];
        let req = PlanRequest {
            upgrades: vec![PkgName::parse("httpcore").unwrap()],
            installs: Vec::new(),
            strategy: Strategy::Compatible,
        };

        let argv = plan_argv_specs(&req, &installed);

        assert!(
            argv.contains(&"httpcore".to_owned()),
            "the package being upgraded must be asked for, unpinned: {argv:?}"
        );
        assert!(
            argv.contains(&"httpx==0.23.0".to_owned()),
            "guards still present"
        );
        assert!(argv.contains(&"h11==0.12.0".to_owned()));
        assert!(
            !argv.iter().any(|a| a.starts_with("httpcore==")),
            "the upgrade target must not also be pinned: {argv:?}"
        );
    }

    #[test]
    fn a_forced_package_is_pinned_and_what_held_it_is_released() {
        // Force latest (DISCLAIMER §2): httpcore goes to its latest *exactly*, and httpx — whose
        // constraint was holding it — leaves the guard set, or the engine would hold it right back.
        let installed = [
            dist("httpx", "0.23.0"),
            dist("httpcore", "0.15.0"),
            dist("h11", "0.12.0"),
        ];
        let req = PlanRequest {
            upgrades: vec![PkgName::parse("httpcore").unwrap()],
            installs: Vec::new(),
            strategy: Strategy::Compatible,
        };
        let adjust = GuardAdjust {
            pins: vec![PinnedSpec {
                name: PkgName::parse("httpcore").unwrap(),
                version: Version("1.0.9".into()),
            }],
            release: vec![PkgName::parse("httpx").unwrap()],
            exclude: Vec::new(),
        };
        assert_eq!(
            plan_lines(&req, &installed, &adjust),
            ["httpcore==1.0.9", "h11==0.12.0"]
        );
    }

    #[test]
    fn an_excluded_release_rides_alongside_the_request() {
        // A release whose Requires-Python excludes this environment is ruled out by name, and the
        // engine is left to pick the next one — PipDock still does not resolve (invariant 3).
        let installed = [dist("httpx", "0.23.0"), dist("httpcore", "0.15.0")];
        let req = PlanRequest {
            upgrades: vec![PkgName::parse("httpcore").unwrap()],
            installs: Vec::new(),
            strategy: Strategy::Compatible,
        };
        let adjust = GuardAdjust {
            exclude: vec![PinnedSpec {
                name: PkgName::parse("httpcore").unwrap(),
                version: Version("1.0.9".into()),
            }],
            ..GuardAdjust::default()
        };
        assert_eq!(
            plan_lines(&req, &installed, &adjust),
            ["httpcore", "httpx==0.23.0", "httpcore!=1.0.9"]
        );
        assert_eq!(
            plan_lines(&req, &installed, &GuardAdjust::default()),
            plan_argv_specs(&req, &installed),
            "no adjustment is exactly the SP-1 default"
        );
    }

    #[test]
    fn updating_everything_still_asks_for_something() {
        // With --all every package is moving, so the guard set is empty. If only guards were
        // passed the command would carry no requirements at all.
        let installed = [dist("a", "1.0"), dist("b", "2.0")];
        let req = PlanRequest {
            upgrades: vec![PkgName::parse("a").unwrap(), PkgName::parse("b").unwrap()],
            installs: Vec::new(),
            strategy: Strategy::Compatible,
        };

        assert!(
            plan_requirements(&req, &installed).is_empty(),
            "no guards, as expected"
        );
        assert_eq!(plan_argv_specs(&req, &installed), ["a", "b"]);
    }

    #[test]
    fn install_specifiers_are_passed_through_intact() {
        let req = PlanRequest {
            upgrades: Vec::new(),
            installs: vec![
                crate::model::Spec {
                    name: PkgName::parse("httpx").unwrap(),
                    version_req: Some("0.28.1".into()),
                },
                crate::model::Spec {
                    name: PkgName::parse("idna").unwrap(),
                    version_req: None,
                },
                crate::model::Spec {
                    name: PkgName::parse("certifi").unwrap(),
                    version_req: Some(">=2025".into()),
                },
            ],
            strategy: Strategy::Compatible,
        };
        let argv = plan_argv_specs(&req, &[]);
        assert!(
            argv.contains(&"httpx==0.28.1".to_owned()),
            "bare version becomes =="
        );
        assert!(argv.contains(&"idna".to_owned()));
        assert!(
            argv.contains(&"certifi>=2025".to_owned()),
            "an operator is kept as given"
        );
    }

    #[test]
    fn an_empty_environment_needs_no_guards() {
        let req = PlanRequest {
            upgrades: Vec::new(),
            installs: Vec::new(),
            strategy: Strategy::Compatible,
        };
        assert!(plan_requirements(&req, &[]).is_empty());
    }

    #[test]
    fn the_guard_set_leaves_out_a_version_that_is_not_argv_safe() {
        let installed = [
            crate::model::Dist {
                name: PkgName::parse("good").expect("name"),
                version: crate::model::Version("1.0".into()),
                requires_dist: Vec::new(),
                requires_python: None,
                size_bytes: None,
            },
            crate::model::Dist {
                name: PkgName::parse("odd").expect("name"),
                version: crate::model::Version("1.0 ; python_version < '0'".into()),
                requires_dist: Vec::new(),
                requires_python: None,
                size_bytes: None,
            },
        ];
        let req = PlanRequest {
            upgrades: Vec::new(),
            installs: Vec::new(),
            strategy: crate::plan::Strategy::Compatible,
        };
        let guards = plan_requirements(&req, &installed);
        assert_eq!(guards.len(), 1);
        assert_eq!(guards[0].to_requirement(), "good==1.0");
    }

    #[test]
    fn a_huge_environment_still_fits_on_a_command_line() {
        // Every plan restates the installed set (SP-1). As argv, ~1,300 packages passed Windows'
        // 32,767-character limit, the spawn failed with os error 206, and the user was told to
        // install an engine. As a file, the command line no longer grows with the environment.
        let installed: Vec<crate::model::Dist> = (0..2000)
            .map(|i| crate::model::Dist {
                name: PkgName::parse(&format!("package-with-a-long-name-{i:04}")).expect("name"),
                version: crate::model::Version("12.34.5678".into()),
                requires_dist: Vec::new(),
                requires_python: None,
                size_bytes: None,
            })
            .collect();
        let req = PlanRequest {
            upgrades: Vec::new(),
            installs: Vec::new(),
            strategy: crate::plan::Strategy::Compatible,
        };
        let lines = plan_argv_specs(&req, &installed);
        assert!(
            lines.iter().map(|l| l.len() + 1).sum::<usize>() > 32_767,
            "the fixture must be big enough to have broken argv"
        );
        let files = RequirementFiles::write(&lines, &[]).expect("written");
        let argv = pip::PipEngine::argv_dry_run(&files.args());
        let command_line: usize = argv.iter().map(|a| a.len() + 1).sum();
        assert!(
            command_line < 1_000,
            "command line is {command_line} characters"
        );
    }

    #[test]
    fn pip_names_a_file_requirement_by_its_line_and_the_line_comes_back() {
        // The sentence pip 26.2.1 printed when the plan went through a file, verbatim but for
        // the path, which is this run's own.
        let files = RequirementFiles::write(
            &["httpcore>=1.0".to_owned(), "httpx==0.23.0".to_owned()],
            &[PinnedSpec {
                name: PkgName::parse("h11").unwrap(),
                version: Version("0.12.0".into()),
            }],
        )
        .expect("written");
        let args = files.args();
        let (req, con) = (&args[1], &args[3]);
        let stderr = format!(
            "ERROR: Cannot install -r {req} (line 1) and -r {req} (line 2) because these package \
             versions have conflicting dependencies.\n\
             ERROR: Could not find a version that satisfies the requirement httpcore>=1.0 \
             (from -r {req} (line 1)) (from versions: none)\n\
             The user requested (constraint) -c {con} (line 1)\n\
             -r {req} (line 9) is not a line this run wrote\n"
        );
        let out = files.humanize(&stderr);
        assert!(
            out.contains("Cannot install httpcore>=1.0 and httpx==0.23.0 because"),
            "{out}"
        );
        assert!(
            out.contains("requirement httpcore>=1.0 (from versions: none)"),
            "{out}"
        );
        assert!(out.contains("(constraint) h11==0.12.0"), "{out}");
        assert!(
            out.contains(&format!("-r {req} (line 9)")),
            "an unknown line is left alone rather than invented: {out}"
        );
        // And the impossibility now names the package the user asked for.
        let detail = parse::pip_impossible(&out).expect("impossible");
        let names: Vec<&str> = detail.packages.iter().map(PkgName::as_str).collect();
        assert_eq!(names, ["httpcore", "httpx"]);
    }
}
