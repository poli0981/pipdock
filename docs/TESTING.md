# PipDock — Testing Strategy

*Version 0.1 · 2026-07-17*

## 1. What must never regress

1. Adapter parsing of pip/uv output (the product stands on it).
2. The data-flow invariants (DATA-FLOW §9): no mutation without accepted plan + snapshot.
3. Conflict presentation: held-back attribution sentences must match fixture ground truth.
4. Skip-and-continue semantics and summary counts.
5. Reverse-dependency guard correctness — including that it is *enforced*, not merely reported: a removal the user never accepted must be refused (`GuardAck`, PD-RES-004), and widening a removal must re-run the guard rather than proceed.

## 2. Layers

### L1 — Core unit tests (Rust, `cargo test`)

- **Parser fixtures:** captured real outputs under `tests/fixtures/{pip,uv}/{list,outdated,report,errors}/…` (seeded by spikes SP-1/SP-2, grown from every new engine release and every triaged bug). `insta` snapshot tests assert the normalized `ResolutionReport`. A CI job runs the capture script against the *latest* engine versions weekly and fails on unrecognized shapes → PD-ENG-003 before users hit it.
- **Error classifiers:** every catalog code has ≥ 1 stderr fixture; a test enforces "no code without fixture."
- **Graph:** reverse-dep construction and blocker attribution over synthetic metadata sets, including cycles and extras (`pkg[extra]`) markers.
- **Snapshot diff/rollback planner:** property-style tests — `apply(plan(diff(a,b)), b) == a` over generated freeze pairs.
- **Name/spec validation:** PEP 508/440 accept/reject tables.

### L2 — Engine integration (disposable venvs, Windows CI runner)

Each test creates a throwaway venv (`python -m venv` and `uv venv`), then exercises real engines:

| Scenario | Assertion |
|---|---|
| Fresh venv + install 3 small pure wheels | summary 3/0; `check` clean |
| Seed known conflict (install `A==old` requiring `B<x`, then plan `B` latest) | plan shows held-back with correct blocker; Compatible path succeeds; Force path warns and `check` reports breakage |
| Batch with 1 deliberately broken sdist | Phase B isolates it; counts n-1 ok / 1 failed with PD-BLD-*; env passes `check` |
| Uninstall a depended-on package | guard lists dependents; `--force` proceeds |
| Snapshot → mutate → rollback | env freeze equals snapshot |
| PEP 668 marker file planted | PD-ENV-002 block; override path passes flag |

Fixture packages are tiny pinned PyPI wheels (documented list in `tests/integration/pins.toml`) so CI is deterministic; network-touching tests are tagged and can run against a local `pypiserver` mirror when offline determinism is needed.

### L3 — Frontend (Vitest + Testing Library)

Component tests for `PdConflictRow` (3-way state), `PdPreviewDiff` grouping, `PdSummarySheet` counts, dimming/badging rules on `PdPackageTable`, and the design-token contrast test (all text/surface pairs ≥ WCAG AA computed at build). IPC mocked at the typed wrapper layer with recorded core payloads — the same JSON as L1 fixtures, guaranteeing UI and core agree on shapes (types generated via tauri-specta make drift a compile error).

### L4 — CLI (assert_cmd + insta)

Golden-output tests per command against a mocked core; TTY vs non-TTY prompt behavior; exit-code table verified exhaustively; `--json` payloads validated against `pipdock schema` output.

### L5 — E2E smoke (tauri-driver, P1 gate for release)

One scripted run on a Windows runner: launch → legal gate → select seeded venv → update-all happy path → summary visible → rollback. Kept minimal; breadth lives in L2.

## 3. Coverage & gates

Core target ≥ 85 % line coverage (`cargo llvm-cov`), classifiers and plan module ≥ 95 %. PRs blocked on: fmt, clippy `-D warnings`, L1, L3, L4; L2 runs on PRs touching `engine|plan|snapshot|graph` paths and nightly otherwise. The weekly latest-engine parser job files an issue automatically on failure (feeds PD-ENG-003 early warning).

## 4. Manual test charter (per release)

A short exploratory pass on a real machine: dirty 150+ package env, corporate-proxy simulation (PD-NET-002 copy), non-ASCII Windows username paths, VI locale sweep of all mutation dialogs, cancel mid-Phase-B, and SmartScreen/first-run installer experience.
