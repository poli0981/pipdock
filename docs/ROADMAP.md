# PipDock — Roadmap

*Version 0.1 · 2026-07-17*

## Phase 0 — Spike week (before any feature code)

The entire architecture stands on assumptions that must be verified on a real machine first (the FrameLedger/QuoteAtlas discipline). Each spike is a small script + captured fixtures committed to `crates/pipdock-core/tests/fixtures/`.

| # | Spike | Question to answer | Exit criterion |
|---|---|---|---|
| SP-1 | **uv dry-run shape** | Exact stdout of `uv pip install -U --dry-run` (current + previous uv) across: clean upgrade, held-back, impossible. Is held-back attribution derivable, or does uv only show the resolved set? | Fixture set committed; adapter parsing strategy written; uv minimum version pinned. **Go/no-go for launching with both engines** — if uv's plan output is too lossy, v1.0 ships pip-primary with uv behind a "beta engine" flag. |
| SP-2 | **pip report & stderr corpus** | Capture `--dry-run --report` JSON + real stderr for every ERROR-CATALOG build/network/permission case (broken sdist, MSVC-missing, PEP 668, locked file, yanked, hash mismatch). | ≥ 1 fixture per launch catalog code. |
| SP-3 | **PEP 691 index economics** | Full name-index size, download time on VN broadband, SQLite ingest time, fuzzy-search latency on ~600 k names. | < 60 s cold refresh, < 50 ms search on the i7-14700KF and on a low-end reference VM; else design delta/compression fallback. |
| SP-4 | **pip-audit foreign-env mode** | Correct invocation for auditing an env the tool isn't installed in (freeze-file `--no-deps` vs newer options). | Command line + JSON fixture pinned (P1 feature, but flags decided now). |
| SP-5 | **Held-back attribution accuracy** | On a real numpy/scipy/pandas pin tangle: does graph-based blocker attribution match reality? | Preview sentences verified by hand against the tangle; ambiguity fallback path exercised. |
| SP-6 | **Windows env discovery sweep** | PEP 514 registry + `py -0p` + Microsoft Store Python + uv-managed on a real machine; Store-Python aliasing quirks. | Discovery module design note updated; Store Python either supported or explicitly detected-and-explained. |

## Phase 1 — M1 "Core + CLI" (~3–4 weeks)

Engine trait + both adapters (per SP-1 outcome), probe.py, discovery, index cache + search, plan/resolve/execute two-phase, snapshots, pins, guard graph, error catalog, CLI over all of it. **Exit:** TESTING L1+L2+L4 green in CI; `pipdock update --all` survives the SP-5 tangle env with correct summary; dogfood on Kokone's own bot venvs begins.

## Phase 2 — M2 "GUI shell + core flows" (~3–4 weeks)

Tauri app, design tokens, Environments/Installed/Updates/Search/Pins screens, preview + 3-way conflict UX, console drawer, summary sheet, snapshots UI, legal gate, EN/VI catalogs, settings. **Exit:** all UI-SPEC click budgets met by manual count; L3 green; VI sweep clean.

## Phase 3 — M3 "Health + polish" (~2 weeks)

Tools venv, deptry/vulture/ruff runners + report UI + gated fix, pip upkeep, bug-report deep link, offline states, keyboard map, icon/branding pass. **Exit:** CODE-HEALTH flows pass on two real projects (one pyproject, one requirements-only).

## Phase 4 — RC → v1.0 (~1–2 weeks)

Release pipeline live (signing, updater, checksums), manual charter executed, docs/README screenshots, legal files public, updater tested from RC→GA. **Exit:** RELEASE-CI §5 checklist fully checked; v1.0.0 published + notify fan-out.

## Post-1.0 (P1 wave)

Security tab (pip-audit), pin auto-suggest, requirements/constraints export-import, cache manager, command palette, dependency graph view, scheduled check. Then P2 candidates by demand: macOS/Linux, per-env engine, elevation broker, winget, JP locale.

## Standing risks

| Risk | Mitigation |
|---|---|
| uv output churn between releases | weekly latest-engine parser CI job (TESTING L1) + PD-ENG-003 graceful degrade + "switch to pip" escape hatch |
| pip UX flags deprecations (e.g. report format changes) | same job covers pip; adapter version-gates per engine version |
| Index size growth | SP-3 fallback design kept in the drawer |
| Scope creep from Health into a linter IDE | CODE-HEALTH §7 non-goals are contractual |
