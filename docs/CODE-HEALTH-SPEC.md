# PipDock — Code Health Specification

*Version 0.1 · 2026-07-17 · In v1 by owner decision. The Python analogue of npm's knip, scoped tightly.*

## 1. Concept & boundaries

Code Health answers three questions about a **project folder** (not an environment):

| Question | Tool | Output |
|---|---|---|
| Which declared dependencies are unused, missing, or transitive-only? | **deptry** | dependency issues table |
| Which code is dead (unused functions/classes/vars)? | **vulture** | dead-code findings with confidence % |
| Does the code violate common standards, and what can be auto-fixed? | **ruff** (`check`, opt-in `--fix`; an opt-in `format --check` was specified and never built) | lint findings; safe-fix count |

Hard boundaries: **report-only** for deptry and vulture (no auto-removal of code or dependencies — too dangerous to automate); the only write path is `ruff check --fix`, gated behind an explicit confirm — `ruff format` is not wired at all (§5 says why); PipDock never edits `pyproject.toml`/`requirements.txt` for the user — deptry findings link to the *Uninstall* flow instead, where the reverse-dep guard applies.

## 2. Isolation: the tools venv

The three tools are **never installed into the user's environment**. PipDock maintains its own hidden env:

```text
%LOCALAPPDATA%\PipDock\data\tools\
├─ .venv/                  # created with the newest discovered Python (≥3.10), no upper bound
├─ tools-requirements.txt  # the resolved three-tool subset actually installed, LF, no comments
├─ tools-constraints.txt   # the tools' whole dependency tree, pin lines only, LF — passed as -c (1.4.0)
└─ manifest.json           # installed pin set + hash; mismatch with shipped pins ⇒ re-sync
```

Bootstrap: on first Health run (or pin-set change), create/sync the venv from `tools-requirements.txt`, constrained by `tools-constraints.txt`, with progress in the console drawer. Pin policy: exact `==` pins resolved at PipDock release time — for the tools and, since 1.4.0, for everything beneath them (Dependabot PRs bump the tools, in their own group per RELEASE-CI §2); no floating versions at runtime. Offline: if bootstrap can't reach PyPI, Health reports PD-NET-011 and stays disabled; other tabs unaffected.

**Amended by Phase 3 · P2, 2026-08-12** — four corrections, each made against a running bootstrap:

- **pip unconditionally, not "the configured engine".** The venv is created with `python -m venv` and populated with pip whatever the user selected. uv is a preference about the *user's* environments; Health going dark with `PD-ENG-001` because uv is not on PATH would be a failure with no relation to what was asked for, and pip is present wherever Python is.
- **No upper Python bound, and no `TOOLS_PYTHON_MAX`.** ROADMAP assumed "deptry ships compiled wheels; no cp314 wheel means an sdist build". deptry does ship a Rust extension but builds it against CPython's **stable ABI** (`deptry-0.25.1-cp310-abi3-win_amd64.whl`), verified installing on 3.12.10 and 3.14.6 from the same file; vulture and ruff are `py3-none-*`. The one version-tagged member of the closure, `tomli`, also publishes a `py3-none-any` fallback. The install passes `--only-binary=:all:`, so a future gap is a clean `PD-NET-011` rather than an sdist build ending at `PD-BLD-001` — which would tell someone who clicked *Health* to install Visual Studio Build Tools.
- **The repo's `tools/tools-requirements.txt` is the release-time ledger; the file written into `%LOCALAPPDATA%` is a rendering of it.** The ledger carries a fourth pin, `pip-audit`, for the Security tab (PRD P1-1). Health installs three, and P1-1 did **not** change that: pip-audit ships in a venv of its own (`health::AUDIT_TOOLS`, `health::audit_dir`) so that its `msgpack` dependency — which publishes no universal wheel — can fail on a new CPython without taking Code Health down with it. `pins_for` renders each set separately, which is what keeps the two pin hashes independent: bumping one tool never re-syncs the other's venv. Rendering rather than copying is also what keeps the build machine's line endings out of the pin hash, which is taken over the *parsed* pin set for that reason.
- **A sync rebuilds the venv rather than repairing it, and deletes `manifest.json` first.** `pip install -r` over satisfied pins is a no-op, so repairing a deleted `ruff.exe` installed nothing and then failed verification with `PD-HLT-001` — the state the code tells the user to re-sync out of. Deleting the manifest first means a torn sync reads as "never synced" rather than as a manifest claiming tools that are half-replaced.

`python -m venv` exiting non-zero is **`PD-HLT-004`**, added in the same slice: `PD-NET-011` is `Area::Net` and would exit 6, so a script retrying on network failure would loop forever against a broken interpreter.

**Amended by 1.4.0 — the whole tree is pinned, not just the tools.** The ledger pins four tools and nothing beneath them, so every sync resolved their thirty-odd dependencies against whatever PyPI held that day: a report was reproducible for a build only as far as the top of the tree, and a broken or compromised release of `requests`, `rich` or `msgpack` reached every user on their next sync. `tools/tools-constraints.txt` is that tree, compiled once with

```text
uv pip compile tools/tools-requirements.txt --universal --python-version 3.10 --only-binary :all: --no-emit-package pip -o tools/tools-constraints.txt
```

— the command is in the file's header, which is the copy to run — embedded in the binary beside the ledger (`health::TOOLS_CONSTRAINTS`), written into the tools directory as its pin lines alone, and passed to the bootstrap's `pip install` as `-c`. One file serves both venvs, because a constraint installs nothing. `pip` is left out on purpose: it is the installer, pip-audit depends on it, and constraining it would make pip replace itself half-way through installing pip-audit.

- **Regenerate it whenever `tools/tools-requirements.txt` moves**, a Dependabot bump included. `every_tool_is_constrained_at_the_version_the_ledger_pins` fails `cargo test` on a ledger pin the constraints do not carry, and names the command: a tool installed at one version and constrained at another is something pip refuses, and every user's next Health click would be `PD-NET-011`.
- **The manifest hash still covers the tool pins alone**, so the change forced no existing venv to re-sync. By the same rule, a regeneration that moves only a dependency reaches a venv the next time it is rebuilt for another reason — a tool bump, a missing tool, a cleared cache — and not before.
- **Versions, not hashes.** `--require-hashes` is in ROADMAP's Audit backlog: it needs every wheel's hash across the Pythons and platforms the venvs are built on.

## 3. Inputs

- **Project folder** (persisted per environment in `index.db`): must contain Python sources; detection order for declared deps: `pyproject.toml` → `requirements*.txt` → none (deptry limited-mode notice).
- **Associated environment**: the currently selected env. Recorded on the report so a stale one can be told from a current one.

**Amended by Phase 3 · P3, 2026-08-12 — deptry is *not* told about the environment, because it cannot be.** This line used to say the env is "passed to deptry so 'installed vs declared vs imported' compares against reality". **deptry 0.25.1 has no such option.** It ignores `VIRTUAL_ENV` and reads whatever its own interpreter can import — verified by watching it report `click`, which exists only in PipDock's tools venv, as `DEP003` transitive. Running it from the user's interpreter with the tools venv on `PYTHONPATH` does not help either: deptry's own dependencies come along. §2's isolation and this line's comparison are in conflict at this version, and isolation wins.

The cost is bounded and named. deptry classifies an undeclared import as `DEP003` rather than `DEP001` when it can see the package, so the split is wrong for the nine packages the tools venv holds, and `DEP003` under-reports anything genuinely transitive in the *user's* environment. Both still mean "you imported something you did not declare", which §6 tells the user to fix the same way either way. `DEP003` is **not** suppressed: `--ignore DEP003` would turn a mislabelled finding into a missing one. The CLI and the Health screen say so where findings are shown.

Also amended: the report type is **`HealthReport`**, not the `CheckReport` ARCHITECTURE §7 and CLI-SPEC §6 named — that name is taken by `engine.check()` and is a published `pipdock schema` contract.

**What the tools actually emit**, since §5's sketch predates running them: deptry writes a *flat* list keyed by `module` (not a per-dependency object with a `locations` array) to a **file path**, not a stream — `--json-output` takes a location, so `-` would create a file called `-` in the project. vulture has no machine-readable output at all, and one of its eight message shapes names no identifier. ruff's documentation link is keyed by rule **name**, so §6's `.../rules/<code>` 404s; it is carried from ruff's own `url` field. All three exit **non-zero on findings**, and vulture uses **3**, not 1.

## 4. Invocations (from the tools venv's interpreter; argv arrays, CWD = project folder)

```text
deptry  . --json-output <tools>\runs\deptry-<pid>.json --no-ansi --ignore-notebooks --extend-exclude ".*<dir>/.*" …
vulture . --min-confidence 80 --exclude "*/<dir>/*,…"     # parse text output (stable format)
ruff check . --output-format json --exclude *.ipynb --exclude <dir> …
ruff check . --output-format json --exclude *.ipynb --exclude <dir> … --fix   # only from the Fix button / `pipdock health --fix` after confirm
```

Exclusions honored: `.venv`, `venv`, `node_modules`, `build`, `dist`, `.git` (`health::DEFAULT_EXCLUDES`), each in its tool's own dialect — deptry's are anchored regexes, vulture's one comma-separated glob list, ruff's repeated globs — and notebooks (§7) by `*.ipynb` for ruff and `--ignore-notebooks` for deptry. **There are no user globs and no confidence setting**: this paragraph promised "user globs in Settings" and the block above a confidence from `<settings, default 80>`, and neither setting was ever built — `RunOptions` carries both, no head sets them, and the confidence is always 80. Each tool has a watchdog timeout (120 s) → PD-HLT-003 partial report. The list also had `ruff format --check .` as an optional toggle, default off; no toggle exists (§5).

**Amended by 1.4.0 — the fix excludes what the report excluded, by construction.** The fix ran `ruff check . --fix` with none of the read-only run's exclusions (C4). Verified against ruff 0.17.1: it rewrote every `.ipynb` in the tree — which the report never shows — and, in a project whose `pyproject.toml` sets its own `exclude` (which replaces ruff's defaults), the files under `.venv/` as well. The fix's argv is now the report's argv plus `--fix` (`fix_argv` over `run::ruff_argv`, which the pre-write re-check shares too), and `the_fix_excludes_everything_the_preview_excluded` holds it there; re-run against both cases, neither file changes. A fix stopped by its watchdog says that some files may already have been rewritten, because ruff writes as it goes.

## 5. Results model & UI

```json
{ "project":"C:\\proj\\bot", "env":"…", "ranAt":"…", "toolVersions":{"deptry":"x","vulture":"y","ruff":"z"},
  "deptry":[{"code":"DEP002","dep":"httpx","kind":"unused","locations":[]}],
  "vulture":[{"path":"bot/util.py","line":88,"name":"old_parse","confidence":90}],
  "ruff":{"findings":[…],"fixable":17} }
```

UI (Health tab): run header (folder picker · env chip · Run) → three result tabs with counts; deptry unused-dep rows offer **"Review in Uninstall…"** (jumps to the guarded flow); ruff tab shows `Fix 17 safely fixable issues` → confirm dialog states file count and recommends a clean git tree (PipDock checks `git status --porcelain` if a repo is detected and warns on dirty). Export: `Save report` writes Markdown + JSON to a user-chosen path. CLI mirror: `pipdock health [--tool …] [--fix] [--json]`.

**The fix path, as built (Phase 3 · P5).** One dialog with two states, not two dialogs — two confirms break the click budget and the second is the one nobody reads. Cancel is rendered first and focused, so `Enter` without `Tab` cancels. The confirm carries one unconditional sentence — *PipDock cannot undo this; only your own version control can* — and reserves its danger block for a tree that really has uncommitted work, so a folder outside version control is not nagged about. `git -c core.fsmonitor=false --no-optional-locks status --porcelain --untracked-files=no` is asked **twice**: once when the dialog opens, to decide what to render, and again inside `health_fix`, to decide what to allow. The `-c` is 1.4.0's: a repository can name a program for `git status` to run — `core.fsmonitor` in its own `.git/config` — and a project folder downloaded rather than cloned brings that file with it, so asking "is it safe to write here" would have run it (SECURITY §2). A command-line `-c` wins over every config file. Before anything is written, the command re-reads the project (`PD-RES-002` if the counts moved) and checks every target is writable (`PD-PRM-003` if not) — the fix refuses **as a whole** rather than half-rewriting a tree nothing can restore. Only safe fixes are applied; `--unsafe-fixes` is never passed and **`ruff format` is not wired at all**, because formatting a whole project is a far larger blast radius than fixing seventeen lint findings and §7's non-goals are contractual.

## 6. Interpretation guidance (shown as inline help)

- deptry `DEP001` (missing) often means a dep used but undeclared — the fix is declaring it, which PipDock does **not** automate; copy suggests the exact line to add.
- vulture confidence < 100 can be a false positive (dynamic dispatch, exported API); copy says "review before deleting" and links vulture's whitelist mechanism.
- ruff findings link to the rule's docs anchor. **Corrected 2026-08-13 (P4):** the page is keyed by rule *name*, not code — `I001` lives at `.../rules/unsorted-imports`, and constructing the URL from the code 404s. `RuffFinding.url` carries ruff's own link and is used verbatim; it is null for a syntax error, which has no rule page. `capabilities/external-links.json` had to be widened to `https://docs.astral.sh/*` or every link failed silently.

## 7. Non-goals

No mypy/pytest orchestration, no complexity metrics, no CI annotations in v1 (the JSON export enables users to wire their own). No support for notebooks in v1 (`*.ipynb` excluded; noted in the report footer).
