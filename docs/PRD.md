# PipDock — Product Requirements Document

*Version 0.1 · 2026-07-17 · Owner: Kokone (poli0981)*

## 1. Problem statement

Managing an existing Python environment with pip is a blind, one-package-at-a-time chore:

1. `pip list --outdated` tells you *what* is outdated but not *what will happen* if you upgrade it.
2. pip's resolver silently holds packages back or fails with a wall of red text (`ResolutionImpossible`) that most users cannot act on.
3. `pip uninstall` will happily remove a package that ten other packages depend on, without warning.
4. A failed package in a manual bulk update leaves the environment in an unknown intermediate state with no record of what it looked like before.
5. GUI alternatives are either abandoned, IDE-locked (PyCharm's package tool), or thin wrappers that hide the resolver instead of explaining it.

PipDock's thesis: **don't reimplement the resolver — explain it.** Wrap pip/uv's own dry-run resolution in a UI that previews changes, names the package responsible for every conflict, and makes every destructive operation reversible.

## 2. Goals

- G1 — Every mutating operation (install / update / uninstall) is **previewed** via engine dry-run before execution.
- G2 — Conflicts are **explained in one sentence** ("`requests 2.32` is held back because `apiclient 1.4` requires `requests<2.31`") and resolved with a 3-way choice.
- G3 — Every batch operation is **reversible** via automatic snapshots.
- G4 — Core happy paths complete in **≤ 5 clicks** (see UI-SPEC click budgets).
- G5 — GUI and CLI share **one Rust core**; behavior never diverges.
- G6 — PipDock **never modifies its own runtime** (standalone binary; the managed environments are external).
- G7 — Zero telemetry; network traffic limited to PyPI + GitHub Releases.

## 3. Personas

| Persona | Situation | What PipDock gives them |
|---|---|---|
| **The hobby dev** | One global venv per project, updates rarely, terrified of breaking things | Preview + rollback = confidence to update at all |
| **The data tinkerer** | 200-package Anaconda-refugee env, numpy/pandas/scipy pin tangle | Held-back explanations; "compatible" strategy resolves the tangle without force |
| **The automation writer** (Kokone's own use case) | Many small venvs for bots/scrapers, wants scripted upkeep | `pipdock update --all --json` in scheduled tasks; per-env pins |
| **The cleanup-minded maintainer** | Inherited project with mystery dependencies | Code Health: deptry finds unused deps, vulture finds dead code |

## 4. Feature matrix

### P0 — v1.0 (ship blockers)

| # | Feature | Notes |
|---|---|---|
| P0-1 | Environment discovery & switcher | PEP 514 registry + `py -0p` + `uv python list` + known venv dirs + manual browse. Recents persisted. |
| P0-2 | PEP 668 guard | Detect `EXTERNALLY-MANAGED`; block by default with explanation; explicit opt-in override only. |
| P0-3 | Installed list | Per-env; up-to-date rows dimmed; outdated rows badged and mirrored into Updates tab. |
| P0-4 | Updates flow | Group dry-run resolve → preview diff → 3-way conflict choices → two-phase execution → `pip check` post-verify → summary report. |
| P0-5 | Search & install | Local PEP 691 name-index cache + fuzzy search; per-package metadata on demand (PyPI JSON API); install queue ("dock bay"). |
| P0-6 | Bulk uninstall + reverse-dep guard | Warn "removing X breaks Y, Z" before execution; `--force` override. |
| P0-7 | Pins | Per-env pin list; pinned packages excluded from *Select all*; lock icon in lists. |
| P0-8 | Snapshots & rollback | Auto snapshot (freeze + metadata) before every batch; diff view; minimal-ops rollback. |
| P0-9 | Engine setting (pip / uv) | Auto-detect uv on first run; switch any time in Settings; engine badge in status bar. |
| P0-10 | pip upkeep | Show pip version per env; one-click `python -m pip install --upgrade pip`. |
| P0-11 | Code Health tab | deptry + vulture + ruff from PipDock's isolated tools env; report + safe `ruff --fix`. (Promoted to v1 by owner decision 2026-07.) |
| P0-12 | CLI core parity | env / list / search / install / update / uninstall / pin / snapshot / doctor / health. JSON output. |
| P0-13 | Legal gate | First-run modal: EULA, License, Disclaimer, Privacy, Third-Party → GitHub links; consent stored with docs-version hash. |
| P0-14 | i18n EN + VI | GUI fully localized; CLI English-only in v1. |
| P0-15 | Summary reports | "13 successful, 2 failed" + per-package error-catalog reason; copy log; Report-bug deep link. |
| P0-16 | Structured logging | Rolling local log files (tracing); in-app live console during execution. |

### P1 — v1.x

| # | Feature | Notes |
|---|---|---|
| P1-1 | Security tab | pip-audit (freeze-file mode) listing known CVEs per installed package, severity-sorted. |
| P1-2 | Pin auto-suggest | Reverse-dependency count ≥ threshold (default 5, configurable) ⇒ suggest pin with reason. |
| P1-3 | Export / import | `requirements.txt` and `constraints.txt` export; import-as-queue. |
| P1-4 | Cache manager | Show / purge engine cache (`pip cache`, `uv cache`) and PipDock's index cache. |
| P1-5 | Command palette | Ctrl+K fuzzy action launcher (terminal-tech signature feature). |
| P1-6 | Dependency graph view | Visual "who holds this back" graph for a selected package. |
| P1-7 | Scheduled check | Optional background outdated-check with toast (no auto-apply, ever). |

### P2 — later

| # | Feature | Notes |
|---|---|---|
| P2-1 | macOS / Linux builds | Tauri makes this near-free; test matrix is the real cost. |
| P2-2 | Per-env engine override | e.g. uv globally, pip for one legacy env. |
| P2-3 | Elevation broker | Manage admin-owned Pythons (Program Files) via the CommandForge two-process pattern. v1 detects read-only site-packages and blocks with guidance instead. |
| P2-4 | Conda detection | Read-only awareness + "open in conda" hand-off. Never mutate conda envs with pip. |
| P2-5 | Portable mode | Config beside the exe for USB use. |
| P2-6 | JP locale | Extend i18n to EN/VI/JP baseline. |

## 5. Non-goals (v1)

- **Not a project manager.** PipDock manages *environments*, not `pyproject.toml` workflows. It does not create lockfiles, does not replace `uv sync` / poetry, and never edits the user's project files (Code Health reports; only `ruff --fix` writes, and only on explicit click).
- **No conda support.** Mixing pip into conda envs corrupts them; v1 refuses with an explanation.
- **No auto-updates of user packages.** PipDock never applies changes without an explicit confirm. Scheduled *checks* only (P1-7).
- **No package publishing / building.** Out of scope entirely.
- **No elevation in v1** (see P2-3).

## 6. Success metrics

| Metric | Target |
|---|---|
| Clicks for "update everything" happy path | ≤ 4 |
| Cold outdated-scan, 200-pkg env, uv engine | < 3 s |
| Fuzzy search keystroke → results | < 50 ms (local index) |
| Batch of 15 with 2 failures | 13 applied, 2 reported with catalog codes, env passes `pip check` |
| Rollback of any snapshot | Env diff vs snapshot = ∅ (excluding unavailable-on-PyPI edge case, which must be reported) |
| Crash-free sessions | > 99.5 % |

## 7. Open questions (tracked for spike week)

1. Exact shape of `uv pip install --dry-run` output for adapter parsing (no stable JSON report — see ROADMAP SP-1).
2. pip-audit invocation mode for auditing a *foreign* env (freeze-file vs `--python`) — SP-4.
3. Whether PEP 691 full-name-index download size/refresh cadence needs delta handling — SP-3.
