# PipDock — Roadmap

*Version 0.1 · 2026-07-17*

## Phase 0 — Spike week (before any feature code) — **RUN 2026-07-27**

The entire architecture stands on assumptions that must be verified on a real machine first (the FrameLedger/QuoteAtlas discipline). Each spike is a small script + captured fixtures committed to `crates/pipdock-core/tests/fixtures/`.

**Outcomes — full write-up in [`spikes/README.md`](../spikes/README.md).**

| Spike | Verdict | The one thing that changed |
|---|---|---|
| SP-1 | **GO — ship both engines** | Neither engine holds packages back on `install -U <pkg>`; both silently break installed dependents. **Every plan must restate the full installed set as explicit requirements.** uv's conflict attribution is *better* than pip's; uv writes its plan to **stderr**, not stdout. uv minimum pinned at 0.10.0. |
| SP-2 | Corpus captured; **blocker found and fixed** | `pip install --dry-run --report -` crashes with `UnicodeEncodeError` on Windows/cp1252 (pip 25 **and** 26). Every pip invocation must set `PYTHONIOENCODING=utf-8` + `PYTHONUTF8=1`. Also: yanked releases exit 0 (a preview warning, not a failure), and pip/uv disagree on Requires-Python enforcement. |
| SP-3 | **PASS** | 858 k projects; cold refresh 2.95 s (budget 60 s); worst keystroke 42.1 ms (budget 50 ms) — but only **in memory**: scanning SQLite per keystroke costs 218 ms. Margin is thin on fast hardware, and raw nucleo ranking puts `requests-ntlm` above `requests`. |
| SP-4 | Answered | `--path` cannot be combined with `--no-deps`; freeze-file mode (`-r <freeze> --no-deps -f json`) is the only option. SECURITY §6 confirmed. Findings arrive with duplicate advisory ids. |
| SP-5 | Answered by SP-1 | **No engine reports held-back items at all.** Resolved version, latest version and blocker must all be derived by PipDock — for pip as much as for uv, contrary to ARCHITECTURE §3's implication. |
| SP-6 | Answered | `uv python list` returns downloadable entries and duplicate shims; `env_hash` **must lowercase** the interpreter path on Windows or pins and snapshots split in two. `probe.py -I` hides 24 user-site packages on a system Python — open decision. Microsoft Store Python still unverified. |

**Both adapter-behaviour questions were decided 2026-07-27** and are implemented: PipDock enforces `Requires-Python` itself so the preview is engine-independent (`crates/pipdock-core/src/compat.rs`, ARCHITECTURE §3), and `probe.py` keeps `-I` while reporting `hidden_user_site` so the Installed screen can disclose the gap accurately (SECURITY §2). Two non-blocking questions remain at the end of `spikes/README.md`: search ranking scope, and how to obtain the five error fixtures this machine cannot produce.

| # | Spike | Question to answer | Exit criterion |
|---|---|---|---|
| SP-1 | **uv dry-run shape** | Exact stdout of `uv pip install -U --dry-run` (current + previous uv) across: clean upgrade, held-back, impossible. Is held-back attribution derivable, or does uv only show the resolved set? | Fixture set committed; adapter parsing strategy written; uv minimum version pinned. **Go/no-go for launching with both engines** — if uv's plan output is too lossy, v1.0 ships pip-primary with uv behind a "beta engine" flag. |
| SP-2 | **pip report & stderr corpus** | Capture `--dry-run --report` JSON + real stderr for every ERROR-CATALOG build/network/permission case (broken sdist, MSVC-missing, PEP 668, locked file, yanked, hash mismatch). | ≥ 1 fixture per launch catalog code. |
| SP-3 | **PEP 691 index economics** | Full name-index size, download time on VN broadband, SQLite ingest time, fuzzy-search latency on ~600 k names. | < 60 s cold refresh, < 50 ms search on the i7-14700KF and on a low-end reference VM; else design delta/compression fallback. |
| SP-4 | **pip-audit foreign-env mode** | Correct invocation for auditing an env the tool isn't installed in (freeze-file `--no-deps` vs newer options). | Command line + JSON fixture pinned (P1 feature, but flags decided now). |
| SP-5 | **Held-back attribution accuracy** | On a real numpy/scipy/pandas pin tangle: does graph-based blocker attribution match reality? | Preview sentences verified by hand against the tangle; ambiguity fallback path exercised. |
| SP-6 | **Windows env discovery sweep** | PEP 514 registry + `py -0p` + Microsoft Store Python + uv-managed on a real machine; Store-Python aliasing quirks. | Discovery module design note updated; Store Python either supported or explicitly detected-and-explained. |

## Phase 1 — M1 "Core + CLI" — **substantially complete 2026-07-27**

Engine trait + both adapters (per SP-1 outcome), probe.py, discovery, index cache + search, plan/resolve/execute two-phase, snapshots, pins, guard graph, error catalog, CLI over all of it. **Exit:** TESTING L1+L2+L4 green in CI; `pipdock update --all` survives the SP-5 tangle env with correct summary; dogfood on Kokone's own bot venvs begins.

### What works

Every P0 CLI command except `health`: `env list|use`, `list [--outdated]`, `search`, `info`, `install`, `update`, `uninstall`, `pin add|remove|list`, `snapshot list|create|diff|rollback`, `doctor`, `pip-upgrade`, `engine [pip|uv]`, `index refresh`, `schema`, `self report-bug`. 173 tests; `cargo fmt`, `clippy -D warnings` and `npm audit` clean.

Verified against real environments, not only unit tests:

- the held-back conflict from SP-1 is reported and attributed — `httpcore 0.15.0 (latest 1.0.9)` / `httpx 0.23.0 requires httpcore <0.16.0,>=0.15.0` — which **no engine produces**;
- `update --all` resolves the same tangle to latest, applies it, and leaves `pip check` clean;
- `snapshot rollback` restores an environment exactly (PRD §6's "env diff vs snapshot = ∅");
- the uninstall guard refuses to break `requests` and exits non-zero, per CLI-SPEC §7;
- exit codes match CLI-SPEC §5.

### Findings that changed the design

| | |
|---|---|
| `plan_requirements` | Every resolve restates the installed set, or both engines break installed dependents at exit 0 (SP-1). |
| `-U` in the dry-run argv | Without it pip plans nothing and every package looks held back for no reason. The unit test had encoded the omission instead of the document. |
| Search ranking | Tiered, not score-ordered, and ranked by selection rather than a full sort: 90.5 ms → 16.5 ms against a 50 ms budget. |
| `SnapshotProof` | `--no-snapshot` is a named waiver, not an `Option`, so forgetting a snapshot cannot look like deliberately waiving one. |

### Not yet done

- **`health`** — belongs to M3 with the tools venv (CODE-HEALTH-SPEC).
- **TESTING L2 in CI** — ~~has never run~~. It ran nightly on 2026-07-28 and 2026-07-29 and failed both times, for two causes that were diagnosed and fixed on 2026-07-29 (see *L2's first runs* below). Re-verify with `gh workflow run ci-integration.yml`.
- **TESTING L4** — `assert_cmd` is a dependency and the clap surface is covered, but the golden-output tests per command are not written.
- **The SP-5 tangle env** — the exit criterion names a real numpy/scipy/pandas environment. The httpx/httpcore construction proved the mechanism; the large-environment run is still owed, and is the natural first dogfooding step.
- **Settings beyond engine choice** — locale, thresholds and the PEP 668 override land with the GUI in M2.

### L2's first runs — what actually failed (2026-07-29)

The job ran and failed three times. Neither cause was a product bug; both were in the test harness, and both are fixed.

**1. The rollback assertion targeted the wrong snapshot.** The step did `snapshot create` → `update --all` → `snapshot rollback latest` → `snapshot diff latest`, asserting the prose `matches snapshot`. But `latest` moves twice during that flow: `update --all` writes a `Trigger::Plan` snapshot before mutating, and `snapshot rollback` writes a `Trigger::Rollback` one before restoring (DATA-FLOW §8 — rollback is itself reversible). So the final `diff latest` compared the restored environment against the *pre-rollback* state and correctly reported every package as changed. The rollback itself had worked. Fixed by pinning the id from `snapshot list --json` immediately after `create`, and asserting structurally on `diff --json` rather than on prose.

**2. `fixture-drift` could never have gone green.** `spikes/capture.py` built each scenario in a `mkdtemp` directory whose name carries a random suffix, and `redact()` replaced only the temp *root* — so the suffix survived into every committed sidecar, and every re-capture differed. Four further sources of churn were found underneath it: pip's `[notice] To update, run: <venv>\Scripts\python.exe` (which also leaked the capturing user's home directory into a public repo), CPython object addresses in urllib3 retry warnings, pip's download progress bar with its transfer rate, and uv's per-phase timings (`Resolved 3 packages in 775ms`). Fixed by redacting all of them in `capture.py`, splitting the sidecar into `meta.json` (contract, gated) and `capture-provenance.json` (versions and argv, excluded from the gate), and capturing cache-free so a warm dev machine and a cold runner agree.

Verified by running both engines' captures twice back to back: **0 of 96 fixture files differ between two identical runs**, and 173 tests stay green.

Two things worth keeping in mind, both recorded in `.gitattributes` and `CLAUDE.md`: `-text` is what preserves the CRLF bytes the fixtures need, and `-diff` — which was also set — only suppressed the textual diff, so the drift job's own diagnostics could report nothing but "Binary files differ". And the weekly drift job's `if:` matched the *nightly* cron, so the PyPI-heavy job the comment says would be "rude" to run daily was running daily; it now has its own `'17 4 * * 1'` entry.

### Where to pick up

Pushed to `main` and green: **CI / Rust, CI / Node and CodeQL all pass** on the published commit. Three configuration bugs were fixed getting there, each recorded in its own commit message: caller workflows must grant *at least* what the callee declares (a `startup_failure` with no logs), `cargo audit` was pinned to a toolchain too old to build itself, and its informational advisories needed scoping rather than silencing.

Immediate, in rough order:

1. **Triage six open Dependabot PRs.** Close #3 (TypeScript 7) and #4 (rusqlite 0.40) — both are the holds listed in ARCHITECTURE §10, and §10's rule is that those are closed rather than merged. #1 and #2 (`actions/setup-python`, `actions/setup-node` 6→7) show a stale `cargo audit` failure predating `566ee27` and need a rebase before their CI means anything. #6 (schemars 1.2.2) should land before M2 starts, since the type-generation strategy rides on schemars output. #5 (@types/node) whenever.
2. **Re-verify `ci-integration.yml`** with `gh workflow run` on both matrix legs after the fixes above.
3. **Repo settings** — branch protection, secrets, and the updater keypair, per RELEASE-CI §5. None of these are committable and all are owner-only.
4. **Dogfood on a real environment** — the SP-5 numpy/scipy/pandas tangle, which is both the outstanding M1 exit criterion and the first honest test of the held-back sentences at scale.

Then M2. The core carries everything the GUI needs; `ui/src/ipc` fixes the command names, the design tokens and EN/VI catalogs are scaffolded, and the shell renders.

## Phase 2 — M2 "GUI shell + core flows" (~7–8 weeks)

Tauri app, design tokens, Environments/Installed/Updates/Search/Pins screens, preview + 3-way conflict UX, console drawer, summary sheet, snapshots UI, legal gate, EN/VI catalogs, settings. **Exit:** all UI-SPEC click budgets met by manual count; L3 green; VI sweep clean.

The estimate was 3–4 weeks until 2026-07-29, when surveying the code for M2 showed the bridge is scaffolding rather than substance: `ui/src/ipc/index.ts` fixes 26 command **names** with zero wrappers, `src-tauri` registers exactly one command (`app_info`, a smoke test), and 2 of 16 `Pd*` components, 1 of 5 Zustand stores and 2 of 14 locale catalogs exist. Scope is unchanged; the estimate moved to match it.

Three pieces of M2 have no home in the core yet and should be sequenced before the screens:

- **A shared flow layer.** `plan_and_run` (`crates/pipdock-cli/src/run.rs`, ~200 lines) is the whole DATA-FLOW §3 machine and lives only in the CLI — PEP 668 gate, pins filtering, the conflict re-resolve loop, snapshot-or-waive, `AcceptedPlan::accept`, two-phase execute. So do rollback execution and the uninstall guard. The GUI needs all of it, and duplicating it would mean two implementations of the hard invariants.
- **Cancellation.** Nothing exists: no token, no `child.kill()`. `plan_cancel` is a declared TS name with no Rust counterpart, and `exec.rs`'s 600 s watchdog uses `tokio::time::timeout`, which drops the future — tokio children are not kill-on-drop, so the child outlives it today.
- **Progress.** `ProgressEvent.step` is hardcoded `0` at all four producer sites, so the console drawer's per-package markers and the live region's "13 of 15" are unimplementable as written; and `scan-progress` has no producer at all.

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
