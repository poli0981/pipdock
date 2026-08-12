# PipDock — Code Health Specification

*Version 0.1 · 2026-07-17 · In v1 by owner decision. The Python analogue of npm's knip, scoped tightly.*

## 1. Concept & boundaries

Code Health answers three questions about a **project folder** (not an environment):

| Question | Tool | Output |
|---|---|---|
| Which declared dependencies are unused, missing, or transitive-only? | **deptry** | dependency issues table |
| Which code is dead (unused functions/classes/vars)? | **vulture** | dead-code findings with confidence % |
| Does the code violate common standards, and what can be auto-fixed? | **ruff** (`check`, opt-in `--fix`, opt-in `format --check`) | lint findings; safe-fix count |

Hard boundaries: **report-only** for deptry and vulture (no auto-removal of code or dependencies — too dangerous to automate); the only write path is `ruff --fix`/`ruff format`, gated behind an explicit confirm; PipDock never edits `pyproject.toml`/`requirements.txt` for the user — deptry findings link to the *Uninstall* flow instead, where the reverse-dep guard applies.

## 2. Isolation: the tools venv

The three tools are **never installed into the user's environment**. PipDock maintains its own hidden env:

```text
%LOCALAPPDATA%\PipDock\tools\
├─ .venv/                  # created with the newest discovered Python (≥3.10), no upper bound
├─ tools-requirements.txt  # the resolved three-tool subset actually installed, LF, no comments
└─ manifest.json           # installed pin set + hash; mismatch with shipped pins ⇒ re-sync
```

Bootstrap: on first Health run (or pin-set change), create/sync the venv from `tools-requirements.txt`, with progress in the console drawer. Pin policy: exact `==` pins resolved at PipDock release time (Dependabot PRs bump them, in their own group per RELEASE-CI §2); no floating versions at runtime. Offline: if bootstrap can't reach PyPI, Health reports PD-NET-011 and stays disabled; other tabs unaffected.

**Amended by Phase 3 · P2, 2026-08-12** — four corrections, each made against a running bootstrap:

- **pip unconditionally, not "the configured engine".** The venv is created with `python -m venv` and populated with pip whatever the user selected. uv is a preference about the *user's* environments; Health going dark with `PD-ENG-001` because uv is not on PATH would be a failure with no relation to what was asked for, and pip is present wherever Python is.
- **No upper Python bound, and no `TOOLS_PYTHON_MAX`.** ROADMAP assumed "deptry ships compiled wheels; no cp314 wheel means an sdist build". deptry does ship a Rust extension but builds it against CPython's **stable ABI** (`deptry-0.25.1-cp310-abi3-win_amd64.whl`), verified installing on 3.12.10 and 3.14.6 from the same file; vulture and ruff are `py3-none-*`. The one version-tagged member of the closure, `tomli`, also publishes a `py3-none-any` fallback. The install passes `--only-binary=:all:`, so a future gap is a clean `PD-NET-011` rather than an sdist build ending at `PD-BLD-001` — which would tell someone who clicked *Health* to install Visual Studio Build Tools.
- **The repo's `tools/tools-requirements.txt` is the release-time ledger; the file written into `%LOCALAPPDATA%` is a rendering of it.** The ledger carries a fourth pin, `pip-audit`, for the post-1.0 Security tab (PRD P1-1). Health installs three. Rendering rather than copying is also what keeps the build machine's line endings out of the pin hash, which is taken over the *parsed* pin set for that reason.
- **A sync rebuilds the venv rather than repairing it, and deletes `manifest.json` first.** `pip install -r` over satisfied pins is a no-op, so repairing a deleted `ruff.exe` installed nothing and then failed verification with `PD-HLT-001` — the state the code tells the user to re-sync out of. Deleting the manifest first means a torn sync reads as "never synced" rather than as a manifest claiming tools that are half-replaced.

`python -m venv` exiting non-zero is **`PD-HLT-004`**, added in the same slice: `PD-NET-011` is `Area::Net` and would exit 6, so a script retrying on network failure would loop forever against a broken interpreter.

## 3. Inputs

- **Project folder** (persisted per environment in `index.db`): must contain Python sources; detection order for declared deps: `pyproject.toml` → `requirements*.txt` → none (deptry limited-mode notice).
- **Associated environment**: the currently selected env; passed to deptry so "installed vs declared vs imported" compares against reality.

## 4. Invocations (from the tools venv's interpreter; argv arrays, CWD = project folder)

```text
deptry .  --json-output <tmp>\deptry.json
vulture . --min-confidence <settings, default 80>        # parse text output (stable format)
ruff check . --output-format json
ruff check . --fix        # only from the Fix button / `pipdock health --fix` after confirm
ruff format --check .     # optional toggle, default off
```

Exclusions honored: `.venv`, `venv`, `node_modules`, `build`, `dist`, `.git`, plus user globs in Settings. Each tool has a watchdog timeout (default 120 s) → PD-HLT-003 partial report.

## 5. Results model & UI

```json
{ "project":"C:\\proj\\bot", "env":"…", "ranAt":"…", "toolVersions":{"deptry":"x","vulture":"y","ruff":"z"},
  "deptry":[{"code":"DEP002","dep":"httpx","kind":"unused","locations":[]}],
  "vulture":[{"path":"bot/util.py","line":88,"name":"old_parse","confidence":90}],
  "ruff":{"findings":[…],"fixable":17} }
```

UI (Health tab): run header (folder picker · env chip · Run) → three result tabs with counts; deptry unused-dep rows offer **"Review in Uninstall…"** (jumps to the guarded flow); ruff tab shows `Fix 17 safely fixable issues` → confirm dialog states file count and recommends a clean git tree (PipDock checks `git status --porcelain` if a repo is detected and warns on dirty). Export: `Save report` writes Markdown + JSON to a user-chosen path. CLI mirror: `pipdock health [--tool …] [--fix] [--json]`.

## 6. Interpretation guidance (shown as inline help)

- deptry `DEP001` (missing) often means a dep used but undeclared — the fix is declaring it, which PipDock does **not** automate; copy suggests the exact line to add.
- vulture confidence < 100 can be a false positive (dynamic dispatch, exported API); copy says "review before deleting" and links vulture's whitelist mechanism.
- ruff findings link to the rule's docs anchor (`https://docs.astral.sh/ruff/rules/<code>`).

## 7. Non-goals

No mypy/pytest orchestration, no complexity metrics, no CI annotations in v1 (the JSON export enables users to wire their own). No support for notebooks in v1 (`*.ipynb` excluded; noted in the report footer).
