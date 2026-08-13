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
- ~~**TESTING L2 in CI**~~ — **done 2026-07-29.** It had run nightly on 07-28 and 07-29 and failed both times; five causes were found and fixed (see *L2's first runs* below). Run `30460248664` on `main` is green on all three jobs, including the fixture-drift re-capture that had never passed.
- **TESTING L4** — `assert_cmd` is a dependency and the clap surface is covered, but the golden-output tests per command are not written.
- ~~**The SP-5 tangle env**~~ — **done 2026-07-29.** A 38-package numpy/scipy/pandas/matplotlib/scikit-learn/statsmodels environment on Python 3.12, with scipy pinned so numpy had to be held back. `update --all` reported *15 successful, 0 failed, 0 skipped*, exited 0, and left `pip check` clean; numpy stayed at 1.26.4, the scipy pin held (invariant 5), and the snapshot was written before any mutation (invariant 2). See *What the dogfood found* below — the answer to SP-5's question was "yes, but the explanation was wrong".
- **Settings beyond engine choice** — locale, thresholds and the PEP 668 override land with the GUI in M2.

### L2's first runs — what actually failed (2026-07-29)

The job ran and failed three times. **No cause was a product bug** — all five were in the test harness, and all are fixed. Worth reading before touching `capture.py` or the workflow, because most of them look like the product misbehaving.

**1. The rollback assertion targeted the wrong snapshot.** The step did `snapshot create` → `update --all` → `snapshot rollback latest` → `snapshot diff latest`, asserting the prose `matches snapshot`. But `latest` moves twice during that flow: `update --all` writes a `Trigger::Plan` snapshot before mutating, and `snapshot rollback` writes a `Trigger::Rollback` one before restoring (DATA-FLOW §8 — rollback is itself reversible). So the final `diff latest` compared the restored environment against the *pre-rollback* state and correctly reported every package as changed. The rollback itself had worked. Fixed by pinning the id from `snapshot list --json` immediately after `create`, and asserting structurally on `diff --json` rather than on prose.

**2. `fixture-drift` could never have gone green.** `spikes/capture.py` built each scenario in a `mkdtemp` directory whose name carries a random suffix, and `redact()` replaced only the temp *root* — so the suffix survived into every committed sidecar, and every re-capture differed. Four further sources of churn were found underneath it: pip's `[notice] To update, run: <venv>\Scripts\python.exe` (which also leaked the capturing user's home directory into a public repo), CPython object addresses in urllib3 retry warnings, pip's download progress bar with its transfer rate, and uv's per-phase timings (`Resolved 3 packages in 775ms`). Fixed by redacting all of them in `capture.py`, splitting the sidecar into `meta.json` (contract, gated) and `capture-provenance.json` (versions and argv, excluded from the gate), and capturing cache-free so a warm dev machine and a cold runner agree.

**3. The guard step failed on the outcome it was asserting.** Fixing the rollback let the job reach the uninstall-guard step, which requires `pipdock uninstall urllib3` to exit non-zero (CLI-SPEC §7). The guard did exactly that — but the runner ends a `pwsh` step with `exit $LASTEXITCODE`, and nothing had reset it. The other seven end-to-end steps were audited; each ends on a command that legitimately exits 0.

**4. The redaction regex ate prose.** With the drift diff finally readable, it showed `Some things HTTP Core does d<PATH>` — pip's `--report` embeds package descriptions, and `does do:\n\n* Sending…` reads as drive `o:` followed by `\n` path segments. A lookbehind requiring a non-alphanumeric before the drive letter is what distinguishes a path from prose.

**5. `report-encoding-crash` cannot be re-captured on CI.** The SP-2 `UnicodeEncodeError` needs a cp1252 console and a runner's is already UTF-8, so re-capturing there recorded an ordinary successful report and destroyed the evidence. Scenarios now carry a `reproducible` flag; host-dependent ones are skipped in bulk runs and stay capturable with `--only`. pip's report also echoes the PEP 508 environment markers, which carry the host OS — normalized, while the *interpreter* version markers are deliberately left alone, since those can change a resolution.

Verified two ways: locally, both engines' captures run twice back to back differ in **0 of 96 files**; on CI, run `30460248664` re-captured against the newest pip and uv and matched the committed fixtures byte for byte.

Two things worth keeping in mind, both recorded in `.gitattributes` and `CLAUDE.md`: `-text` is what preserves the CRLF bytes the fixtures need, and `-diff` — which was also set — only suppressed the textual diff, so the drift job's own diagnostics could report nothing but "Binary files differ". Faults 3–5 were all found *because* that was fixed. And the weekly drift job's `if:` matched the *nightly* cron, so the PyPI-heavy job the comment says would be "rude" to run daily was running daily; it now has its own `'17 4 * * 1'` entry.

### What the dogfood found (2026-07-29)

SP-5 asked whether graph-based blocker attribution matches reality on a real tangle. It does — and at scale it was also **saying things that were not true**. The held-back numpy came with eight constraints, four of which do not apply to Python 3.12:

```text
  numpy 1.26.4 (latest 2.5.1)
      pandas 2.1.4 requires numpy <2,>=1.22.4     <- python_version < "3.11"
      pandas 2.1.4 requires numpy <2,>=1.23.2     <- python_version == "3.11"
      pandas 2.1.4 requires numpy <2,>=1.26.0     <- the only pandas branch in force
      statsmodels 0.14.1 requires numpy <2,>=1.22.3   <- python_version == "3.10" and Windows
```

`Requirement` has always carried its marker, but only `extra ==` was honoured, so every marker-gated branch of a dependency was reported as though it were in force. That is not just noise — telling someone on 3.12 that "pandas requires numpy >=1.22.4" is false, and ERROR-CATALOG's premise is that what PipDock says about a failure can be trusted. The same bug sat under the **uninstall guard**, which would refuse a removal on the strength of a dependent that only needs the package on another Python.

Fixed in `graph/markers.rs`: `python_version` and `python_full_version` are evaluated, with `and`/`or`/parentheses. Platform markers are left alone — they are constants for a Windows-only v1 and reading them would mean plumbing more out of `probe.py` for no present gain. **An unrecognised marker keeps the requirement**, because over-reporting a constraint is noise while dropping one hides the reason a package is stuck. The same environment now reports five constraints, one per package, each genuinely in force.

One thing deliberately *not* changed: blockers are still derived from the installed set, so a package being upgraded by the very same plan can still be named. Here that meant matplotlib, pandas, scikit-learn and statsmodels appeared even though the plan moves all four to numpy-2-compatible versions, leaving pinned scipy as the only real blocker. Narrowing that needs post-plan metadata the preview does not have, and it makes the sentence over-broad rather than false. Worth revisiting when `PdConflictRow` is built.

### Where to pick up

Pushed to `main` and green: **CI / Rust, CI / Node and CodeQL all pass** on the published commit. Three configuration bugs were fixed getting there, each recorded in its own commit message: caller workflows must grant *at least* what the callee declares (a `startup_failure` with no logs), `cargo audit` was pinned to a toolchain too old to build itself, and its informational advisories needed scoping rather than silencing.

Done 2026-07-29, closing out M1's CI debt:

- ~~Dependabot triage~~ — six PRs resolved. #3 (TypeScript 7) and #4 (rusqlite 0.40) closed per ARCHITECTURE §10; #1, #2, #5, #6 merged. **No PRs are open.**
- ~~L2 green~~ — all three jobs, both matrix legs, on `main`.
- ~~TESTING L4~~ — `tests/golden.rs`, 46 snapshots. See *What L4 covers and what it does not* below.

Immediate, in rough order:

1. **Repo settings** — branch protection, the Discord webhook secrets, and CodeQL/Discussions, per RELEASE-CI §5. None of these are committable and all are owner-only. The updater keypair that used to be on this list is gone: PipDock no longer updates itself (SECURITY §5).

**M1 is otherwise closed.** Everything else on this list is done, and Stage 1 of M2 — the IPC bridge — is the next piece of work.

### What L4 covers and what it does not

TESTING §2 asks for goldens "against a mocked core". There is no seam to mock at — `run.rs` builds its engine from the global options and drives a real interpreter, and `print_preview` / `print_summary` are private functions that `println!` rather than return. So `tests/golden.rs` covers what is deterministic without an environment: the clap surface, the exit-code table, the `--json` error envelope, and `schema <T>` for all fifteen `SCHEMA_TYPES`. Preview and summary rendering stays with L2.

Those schema snapshots are also the review surface for M2's wire-format change. They pin two contract bugs as they stand today:

- `ExecutionSummary` serializes `plan_id` / `stderr_tail`, where DATA-FLOW §6 documents `planId` / `stderrTail`
- `Code` serializes as its Rust variant name (`"EnvInterpreterMissing"`), where ERROR-CATALOG documents `PD-ENV-001`

The error envelope, which `main.rs` hand-builds rather than deriving, already has the documented shape — so the binary currently disagrees with itself across its two `--json` paths. Fixing that is Stage 1 work, and the goldens make it a reviewable diff.

When the flow refactor moves rendering out of the CLI ("the flow never prints"), the preview and summary renderers become pure functions of core types and should gain their own snapshots there.

Then M2. The core carries everything the GUI needs; `ui/src/ipc` fixes the command names, the design tokens and EN/VI catalogs are scaffolded, and the shell renders.

## Phase 2 — M2 "GUI shell + core flows" (~7–8 weeks)

Tauri app, design tokens, Environments/Installed/Updates/Search/Pins screens, preview + 3-way conflict UX, console drawer, summary sheet, snapshots UI, legal gate, EN/VI catalogs, settings. **Exit:** all UI-SPEC click budgets met by manual count; L3 green; VI sweep clean.

The estimate was 3–4 weeks until 2026-07-29, when surveying the code for M2 showed the bridge was scaffolding rather than substance: 26 command **names** with zero wrappers, one registered Tauri command, 2 of 16 `Pd*` components, 1 of 5 stores. Scope is unchanged; the estimate moved to match it.

### Stage 1 — the IPC bridge — **done 2026-07-30**

Ten commits, `0ccee04..a553bf7`. Everything M2's screens stand on now exists, and there is a running app.

| | What landed |
|---|---|
| **Wire format** | `#[serde(rename)]` on all 30 `Code` variants so they serialize as `PD-*`, and `rename_all = "camelCase"` on every IPC-crossing struct. `--json` had never matched DATA-FLOW §6 or ERROR-CATALOG §3 — and the two `--json` paths in the binary disagreed with each other. Held by a `Code::ALL` round-trip test and a test that walks every `SCHEMA_TYPES` schema rejecting any property containing `_`. |
| **Codegen** | `ui/src/ipc/generated.ts` from schemars via `cargo run -p xtask -- bindings`. Deviates from ARCHITECTURE §9's specta; reasoning and the revisit condition are in `crates/pipdock-core/src/bindings.rs`. Drift is a **test** failure, so it fires locally as well as in CI. |
| **`core::flow`** | `UpdateFlow`, `UninstallFlow`, `RollbackFlow` — resumable state machines, because the GUI's two decision points are IPC round trips with a render in between. `plan::execute_rollback` is new: core had `rollback_plan()` and nothing to run it, so the CLI hand-assembled an `ExecutionSummary`. **The flow never prints.** |
| **Cancellation** | `CancellationToken` through `exec.rs`, plus `kill_on_drop(true)` — the 600 s watchdog had never killed anything. A cancelled Phase A does **not** fall through to Phase B, and a step killed by us is `Skipped`, not `Failed`. |
| **Progress** | `ProgressSink` carries `step`/`total`; `scan-progress` finally has a producer and a payload (`ScanProgress`). |
| **Vertical slice** | Legal gate, Environments, Settings. `core::settings`, `docsHash` computed in `build.rs`, capabilities split and scoped. |

**Four bugs turned up that were not in the plan**, all found by running things rather than reading them: the watchdog leak; `--json` unparseable because `snapshot … written` went to stdout; blocker attribution naming constraints from other Pythons (the SP-5 dogfood); and `updater:default` granted to a plugin whose pubkey was a placeholder.

### Stage 2 — Installed + Updates (read-only) — **done 2026-08-04**

Three PRs, `d7da4d7..`. Both screens render from a real environment, and the slice's exit criteria
are met.

| | What landed |
|---|---|
| **Docs** | ARCHITECTURE §7's command list rewritten as a table of **32** — it was prose with `a\|b\|c` groupings, ambiguous enough that Stage 1 added `app_info` and `plan_decide` without amending it. Closes open owner decision #1. DATA-FLOW §7's "List installed" row corrected: it claimed an engine command **neither head has ever called**. |
| **Core** | `engine::for_id` — the CLI held the only copy of engine selection plus a duplicate `KEY_ENGINE` and a byte-for-byte copy of `store::default_app_data`. `Dist.sizeBytes`, summed from RECORD. `PkgName` validates on deserialize. |
| **Bridge** | `pkg_list`, `pkg_outdated`, `pin_list\|add\|remove`, registered and wrapped. `pkg_list` reads the probe, not the engine; both take a `PyEnv` so the outdated path never probes. |
| **UI** | `PdBadge`, `PdEmptyState`, `PdPackageRow`, `PdPackageTable` (virtualized) — 8 of 16 components now exist. One screen serving both tabs, `useEnvStore` grown, EN+VI catalogs, `Space`/`Ctrl+A`. |
| **L3** | The repo's first rendered-component tests, fed from fixtures generated out of the real Rust types by `cargo run -p xtask -- ipc-fixtures` and held current by a staleness test. |

**Four bugs turned up that were not in the plan, three of them found by running:**

1. **`Distribution.files` is a 10× trap.** The obvious way to sum RECORD took the probe from 551 ms
   to **5,492 ms** on a 352-package environment — on the path the Installed screen runs on every
   environment open. Parsing RECORD text directly costs 79 ms.
2. **The naive size is wrong for editables, not merely missing.** A PEP 660 install has a *valid*
   RECORD listing only its import shim: **240 bytes for a package whose sources are 8.1 MiB**.
   Detected via `direct_url.json` and reported as absent, because a wrong number is worse than none.
3. **A distribution could be listed twice.** An editable puts its own `.egg-info` on `sys.path`, so
   `importlib.metadata` finds it there *and* at the venv's `.dist-info`. `pip list` shows it once.
   For the table it is an ambiguous join key and two rows a user cannot tell apart.
4. **Every screen fetched twice on mount.** React runs effects twice in development and the
   `loadedFor` guard is set only *after* the await, so both runs proceed. `env_scan` had done this
   since Stage 1, invisibly, because both scans return the same rows.

**Two decisions worth knowing before S3:**

- **Row state is three-valued, not two.** `pkg_list` is local, `pkg_outdated` is networked, so there
  is a window where outdatedness is unknown. Treating it as "up to date" dims all 200 rows and
  un-dims a handful a second later — visibly wrong, and untrue while it lasts.
- **"N pinned excluded" is presentation, not enforcement.** DATA-FLOW §9.5 is enforced by
  `pins::filter_upgrades` at the plan boundary. **S3's preview must report the flow's
  `excluded_pins()`, not the number the table computed.**

Also resolved: UI-SPEC §8's "select-all-visible", which has no meaning under virtualization. It is
the current filtered set, and §8 now says so.

### Stage 3 — the mutation spine — **done 2026-08-04**

Two PRs. The app now previews, decides, executes, streams and summarises — PRD G1's promise that
every mutating operation is previewed before it runs is real for the first time in the GUI.

| | What landed |
|---|---|
| **Cancellation** | A Windows Job Object, so a cancel reaches the build backends `python -m pip` spawns and not only pip. `win32job` is a safe wrapper, so `unsafe_code = "forbid"` stays absolute. |
| **`plan-progress`** | A tagged lifecycle (`stepStarted` / `line` / `stepFinished`) instead of a bare line, plus `stream`. ARCHITECTURE §7 amended. |
| **Wire types** | `FlowStep`, `Intent`, `Decision` and `ProgressEvent` cross IPC; `roundsRemaining` makes `MAX_CONFLICT_ROUNDS` visible for the first time. |
| **Plan session** | `PlanSlot` in `AppState` holds the resumable flow between the four calls that drive it; `PD-RES-003` refuses a second plan. |
| **UI** | `usePlanStore`, `PdPreviewDiff`, `PdConflictRow`, `PdConsoleDrawer`, `PdSummarySheet` — 13 of UI-SPEC §6's 16 components now exist. |
| **L3** | The last three of TESTING §2's five obligations, so the list is complete and §3's PR gate means something. |

**Exit criteria, counted and measured in a running app:** "update everything" is **exactly 4
clicks** — Updates → Select all → Update → Confirm — and it is still 4 with a conflict left at its
default, which is §5's second row. Cancelling mid-execution yields a coherent summary with
`Skipped` rows and the cancelled banner. The live region reads "2 of 4 complete" and the drawer
groups output by package, both of which were unimplementable before the lifecycle enum existed.

**Three specification gaps closed, each recorded in UI-SPEC §4:**

1. **`Will downgrade` has a section.** `ChangeKind` has four variants and §4 named three groups. A
   *compatible* resolve routinely moves a package down, and `2.0 → 1.9` under a heading reading
   "Will upgrade" is misleading about the change most likely to surprise.
2. **`Keep compatible` is disabled on impossible rows.** §4 said every needs-decision row hosts the
   full 3-way control, but `default_decision(is_impossible = true, …)` returns `Skip` — there is no
   compatible version to keep. The UI mirrors the core rather than offering a choice it would
   refuse.
3. **The round counter is visible.** `MAX_CONFLICT_ROUNDS` had been in core since M1 with nothing
   surfacing it, so a user could hit the cap unwarned.

**And the third Stage 1 deferral, `ExecutionSummary.cancelled` copy**, which had no specified
wording: the summary now says the run stopped part-way, that a package may have been left
half-written, and points at the snapshot.

**One design change forced by the compiler, worth knowing.** `UpdateFlow::start` took a `&Store`;
`Store` wraps a `rusqlite::Connection` and is `Send` but **not `Sync`**, so a future holding one
across an await is not `Send` and a Tauri command cannot return it. It takes the pins instead —
one synchronous read at each call site, and the flow no longer touches a database at all.

### Stage 4 — search, the dock bay and install — **done 2026-08-04**

Five commits. Both exit criteria met and measured in the running app. *(The "16 of 16 components"
claim recorded here was wrong: the count had folded in components that are not on UI-SPEC §6's list.
It was 13 of 16 then and after S5 — see §6, which now says so.)*

| | What landed |
|---|---|
| **Index** | `IndexSlot` in `AppState` holds the 864k-name index; warmed on first Search open, never blocking a search. `index_search`, `index_refresh`, `pkg_metadata`. |
| **UI** | `useIndexStore`, `PdSearch`, `PdDockBay`, `PdOfflineBanner`, the metadata panel, `/` to focus. |
| **Install** | The dock bay resolves through `Intent::Install`, into the same preview and summary S3 built. |

**The keystroke budget, measured rather than assumed — and it was being missed.**

| | Median | p95 | Over 50 ms |
|---|---|---|---|
| First measurement | 57.4 ms | 79.8 ms | 18 of 19 |
| After two fixes | **22 ms** | **24.5 ms** | **0 of 19** |

The two fixes were a **leading-edge debounce** — it was trailing-only, spending 16 ms of a 50 ms
budget doing nothing on every keystroke, while the comment above it had claimed leading-edge
behaviour since the day it was written — and **`SEARCH_LIMIT` 50 → 20**, because React commits
every row on every keystroke and the row count is the largest lever on the render half.

**A number I reported was wrong, and it mattered.** The 613 ms index load was a *debug* build; in
release it is 140 ms, and the worst keystroke is 16 ms rather than 176 ms. The design — hold the
index, warm on demand — is unchanged and still right at 140 ms, but it had been justified with a
figure four times too large. `crates/pipdock-core/tests/search_latency.rs` now **refuses to run in
debug** rather than printing a number that means nothing.

**Two bugs found by counting clicks**, neither reachable by any test written for them:

1. **A plan started from Search resolved into nowhere.** `PdPlanPanel` was owned by `PdPackages`,
   so the command ran, the flow parked, and the user saw unchanged search results. A plan belongs
   to the app — there is one at a time, and which tab is showing is not part of that.
2. **Every result offered [Add], including installed packages**, because only `PdPackages` ever
   loaded the installed set. That is exactly the mistake DATA-FLOW §4's chips exist to prevent.

### Stage 5 — uninstall, the guard dialog and the Pins screen — **done 2026-08-09**

Twelve commits. Every mutating command in ARCHITECTURE §7 now exists, and removal is the first flow
whose *whole* value is refusing to do what was asked.

| | What landed |
|---|---|
| **Core** | `GuardReport.breaks` carries `BrokenDependent { pkg, version, constraint }` — the specifier, not just the name. `GuardAck` makes the guard an enforcement point rather than a dialog. PEP 668 refused on removal. New code **PD-RES-004**. |
| **Execution** | `execute_uninstall` emits the `stepStarted`/`stepFinished` lifecycle it never had, checks the cancellation token in its loop, takes a `base` step index shared with `execute_rollback`, and honours `--no-snapshot`. |
| **Bridge** | One `Session` enum for all four flows, with typed claims; `uninstall_guard`, `uninstall_execute`; `plan_cancel` async, and now discarding a parked session. |
| **UI** | `PdDialog` (the repo's first modal), `PdUninstallDialog`, `PdPinChip`, the Pins screen, the row's ✕, and the `failed` phase. |
| **L3** | The first mocked `@/ipc`, the first store test, the first screen test. 6 files → 9; 59 tests → 79. |

**Exit criteria, counted and verified in a running app** (driven in the Browser pane against a
stubbed bridge, per the recipe that found Stage 2's double fetch): uninstall is **exactly 3 clicks**
— Installed → row ✕ → *Remove* — and 4 when the guard trips, by design. The dialog reads
`pandas 2.1.4 requires numpy<2,>=1.26.0`, naming the dependent **and** its constraint. *Remove
dependents too* issues a **second** `uninstall_guard` over the widened set with **no**
`uninstall_execute` in between. At the CLI, against a real venv, the guard refuses and exits 2 with
every package still installed.

**Five live bugs came out of it, none of them S5 features:**

1. **A claim that found nothing wedged the slot.** `plan_decide`/`plan_execute` wrote `Busy` before
   discovering there was no plan, and never released — so one out-of-order call from the UI made
   every later command answer `PD-RES-003` for a plan that had never existed.
2. **`execute_uninstall` emitted no lifecycle markers**, and every line carried `step: 0`. The CLI
   never noticed because it prints `event.line()` and nothing else; the console drawer groups on
   `stepStarted` and the live region counts `stepFinished`, so the GUI would have shown an empty
   drawer against a green suite.
3. **A cancelled removal removed everything, then reported `cancelled: true`.** The loop had no
   token check at all.
4. **`--no-snapshot` was parsed and ignored on the uninstall path** — the same waiver meant "waive"
   for an update and nothing for a removal, the one operation with no way back.
5. **The pin button was a tab stop**, so Space on it toggled the row's *selection* instead of
   pinning. The rule was enforced on the checkbox and forgotten one element later.

**And one specification gap closed:** DATA-FLOW §9.1 required "a `ResolutionReport` accepted in this
session", which the uninstall path cannot produce — there is nothing to resolve. It now names the
proof per plan shape, so the one flow with no preview is no longer the one flow the invariant did not
describe.

### Stage 6 — snapshots, the env detail and rollback — **done 2026-08-09**

Eight commits. Every snapshot the app has been writing since S1 is now reachable from it, and P0-8
is complete in both heads.

| | What landed |
|---|---|
| **Core** | `RollbackPreview` and `RollbackPlan` cross IPC; `snapshot::Meta` gains a schema rename so embedding it does not mint a second interface. `parse_freeze` and `unrestorable_lines` rebuilt on one classifier. |
| **Bridge** | `snapshot_list\|create\|diff\|rollback_preview\|rollback`. ARCHITECTURE §7's single rollback row becomes two, 32 → 33. |
| **UI** | `PdSnapshotTimeline`, `PdRollbackPreview`, the env detail as a *mode* of the Environments tab, the diff viewer, and the rollback the summary's copy has promised since S3. |
| **L3** | Two more generated fixtures; 9 → 11 test files, 79 → 91 tests. Goldens 52 → 54. |

**Exit criteria, verified by running.** At the CLI against a real venv: create → mutate → `snapshot
rollback <id>` → `snapshot diff <id>` prints *environment matches snapshot* — PRD §6's metric, and
the timeline afterwards shows the `rollback of …` entry above the one it restored, which is
`latest` moving twice, live. In the app: **rollback = 4 clicks** (Open → entry → *Rollback…* →
*Roll back*), 2 from a run's summary; the preview lists `PD-SNP-002` with the verbatim freeze line
**before** the confirm.

**Four bugs, three of which no test would have found:**

1. **A cancelled rollback killed the package in flight and installed the rest.** The restore loop
   had no token check — S5 gave one to the removal half and not to the install half. Found by
   writing the cancellation test the plan asked for.
2. **`parse_freeze` and `unrestorable_lines` did not partition their input.** Two independent
   filters that disagreed: a line like `Foo Bar==1.0` was dropped by both and vanished. In a
   rollback preview, whose whole job is saying what it cannot put back, that silence is the worst
   possible failure.
3. **`snapshot_list` fired twice on open** — Stage 2's double fetch, reintroduced exactly, because
   the guard is written after the await. Invisible in the UI, since both calls return the same rows.
4. **A rollback left the timeline empty.** The refetch and the rescan race, and the rescan wiped the
   result: the snapshot fields were inside `NO_PACKAGES`, whose reset is keyed to the *package*
   slice's environment. The two slices key on different things and now reset separately.

**Deliberately not done:** DATA-FLOW §8's dry-run resolve of the restore set, which is implemented
in **neither** head. It is a network round trip on an interaction budgeted as instant, and it would
resolve against the pre-removal environment — reporting conflicts the two-phase execution never
hits. §8 now says so, and the preview's promise is precise instead: it lists what *cannot* be
restored, not everything that might fail. Snapshot retention (`snapshot::delete` + keep-N) also
stays out; a destructive command against the user's only way back needs its own confirm.

### Stage 7 — the shell — **done 2026-08-09**

Eight commits. What the shell was missing was rarely a feature — it was a promise the docs had made
and nothing had kept.

| | What landed |
|---|---|
| **Fonts** | Inter and JetBrains Mono vendored. `--font-sans`/`--font-mono` had named them since the tokens were written and both fell through to a system fallback, because no `@font-face` was ever registered. |
| **i18n** | **26 of 32 catalog codes had no copy in either locale.** All 32 now do, in `locales/*/errors.json`, asserted against a generated `codes.json` — both directions, plus I18N §4's ≤ 90-char cap. |
| **Bug report** | `pipdock_core::report` shared by both heads; `LogRing` in `AppState` fed by three writers; `report_bug_url`; the error row's *Copy full log* and *Report bug*. |
| **a11y** | Every `--color-*` rebound under `forced-colors`, with borders on the destructive controls whose tint the UA erases. The pseudo-locale, dev-only and verified absent from a production build. |
| **Status line** | The log toggle does something, and `⚠ n` means something. |

| **Bridge** | `engine_info`, and three tests that make `COMMANDS` mean something. It listed **32 names while 19 were registered** — a wrapper for any of the other 13 typechecked and would have failed at runtime, on a command that looked implemented. |
| **Keyboard** | Focus follows the tab: `Ctrl+3` changed the screen and left focus behind, so the next Tab resumed inside the screen the user had just left. |

**Deliberately left for later, and named:** the `Enter` primary-action map and folding the six
`aria-live` regions into one — both are cross-screen behaviour that wants the VI sweep and a real
screen reader to verify, not a passing test. S7b (Settings' remaining controls) was always a
follow-up PR. `PdEmptyState` now covers every surface that renders a list.

**One thing worth keeping:** i18next's `nsSeparator` is `:`, so I18N §1's seven namespaces would
have meant `t('errors:PD-NET-001')` at every call site — churn across every screen, bought with lazy
loading a desktop app that bundles both catalogs cannot use. The app ships **one** namespace and two
files per locale; §1 is amended to say so rather than left describing something that was never built.

**M2 is complete.** Every P0 feature except Code Health (P0-11) is in both heads, all seven UI-SPEC
click budgets are met by hand count, and `NOT_YET` names the five commands that remain — each
against the slice that owes it.

### Where to pick up

**Phase 3 is complete.** P1 (pip upkeep), P2 (the tools venv), P3 (the runners and `HealthReport`),
P4 (the Health screen), P5 (the gated `ruff --fix`) and P6 (the tail) are all merged, each with a
stage table above, and the exit criteria are recorded with the numbers they were measured at.
**Next is Phase 4** — the release pipeline, the manual charter, and a clean install of the RC on a
machine that has never run PipDock.

`NOT_YET` is down to **two**, both M3-general: `env_add_manual` (*Browse…* has no surface) and
`logs_tail` (needs the logging subsystem). The M3 debt list is otherwise unchanged: dead
`pins::hold_requirements`, unread `allow_externally_managed`, DATA-FLOW §8's dry-run resolve,
snapshot retention, `PdEnvSwitcher`, CLI-SPEC §6's NDJSON gap, and `RunOptions`/the confidence floor
reaching Settings.

`NOT_YET` is down to three and **only `health_fix` is Phase 3's**. P4 did add two commands after
all — `health_save_report` and, had it been taken, `health_cancel` — so the earlier claim that it
needed none was wrong; both land implemented, so neither is owed.

The four things P4 was warned it would otherwise rediscover all held: `@tauri-apps/plugin-dialog`
really was absent from `package.json`, the opener really was scoped to GitHub only, UI-SPEC §6
really said 14, and deptry really does name a module rather than a distribution. What the warnings
did not cover were the four defects in P4's stage table above, every one of which came from running
the thing.

**The one open decision is now closed.** deptry cannot be told which environment to compare against;
the owner chose on 2026-08-13 to keep all four DEP codes and disclose the limitation where findings
are shown, rather than `--ignore DEP003` (which turns a mislabelled finding into a missing one) or
dropping deptry from v1. The screen carries the note under the same condition the CLI does, and the
saved Markdown carries it too.

**All three Stage 1 deferrals are closed**, each in S3 as planned — the `plan-progress` lifecycle enum, the Windows Job Object, and the `ExecutionSummary.cancelled` copy. Deferring them to the slice that could verify them worked: each was finished against a running UI or a real process tree rather than against a guess.

## Phase 3 — M3 "Health + polish" (~2 weeks)

Tools venv, deptry/vulture/ruff runners + report UI + gated fix, pip upkeep, bug-report deep link, offline states, keyboard map, icon/branding pass. **Exit:** CODE-HEALTH flows pass on two real projects (one pyproject, one requirements-only).

Decomposed **P1** pip upkeep · **P2** the tools venv · **P3** the runners + `HealthReport` · **P4** the Health screen · **P5** the gated `ruff --fix`. Three of the sentence's items are already spent: the bug-report deep link landed in S7, `PdOfflineBanner` in S4, and the keyboard map's `Ctrl+1..8` in S7 — what remains of "keyboard map" is `Enter` as primary action and folding the six `aria-live` regions. The icon/branding pass belongs with Phase 4's release slice; it churns binaries that must not sit inside a feature diff.

### Phase 3 · P6 — the tail — **done 2026-08-13**

Three commits, and **Phase 3 is closed**. The keyboard item turned out to be hiding a WCAG failure; the rest was owed bookkeeping.

| | What landed |
|---|---|
| **UI** | roving tabindex in `PdPackageRow` (`←`/`→`/`Esc`), `Enter` as the non-destructive primary, two live regions folded |
| **Core** | `uv::parse_dry_run` deleted — the last `todo!()` in the workspace — and `clippy::todo` flipped `allow` → `warn` |
| **Docs** | UI-SPEC §8's keyboard map and the two rules behind it; CLI-SPEC §6's stale "`health` is still the M3 stub" |

**Pin and Remove were unreachable without a mouse.** Every control in a package row is `tabIndex={-1}`, which is a sound trade for the checkbox — `Space` does its job from the row — but applied to the two action buttons it removed the actions entirely. The row was the only tab stop and there was nowhere further to go. Found by tabbing to a row in the running app and pressing every key that should have done something; invisible to a suite that clicks.

The audit's other finding is worth keeping: **`Enter` already worked everywhere else**, because every other primary action in the app is a real `<button>` and buttons handle it natively. "Zero `Enter` handlers" was true and misleading. The gap was one element that only looks like a control.

**The live-region pair that mattered was not the one being counted.** ROADMAP had said "six regions" for two slices; there were ten. But the duplication that actually reached a user was `PdConsoleDrawer` and the panel that opens it announcing the same `done/total`, so a screen reader heard every step twice. Eight now, with both genuine conflicts removed rather than the number merely reduced.

### Phase 3 exit criteria — **met 2026-08-13**

*"CODE-HEALTH flows pass on two real projects (one pyproject, one requirements-only)."* Run in `--release` against a warm tools venv:

| Project | Result | Time |
|---|---|---|
| requirements-only | 1 dependency, 3 dead-code, 3 lint (3 fixable in 1 file) | 1.4 s |
| declares nothing | `ran: [vulture, ruff]`, `problems: []` — deptry correctly not run | 1.3 s |
| seeded git repo | 6 fixable across 2 files; `--fix` changed **exactly** those two | — |

The fix path's refusals were exercised on the same repository: non-TTY without `--yes`, `--yes` over a dirty tree, and `attrib +R` on one target — the last confirming the fix refuses **as a whole**, leaving the other, writable file untouched.

**Still owed, and it needs the real Tauri runtime rather than the stubbed bridge:** the OS folder picker, a ruff rule link (to confirm the `docs.astral.sh` allowlist entry works), and the fix dialog end to end. All three are reachable only from `npm run tauri dev`.

### Phase 3 · P5 — the gated `ruff --fix` — **done 2026-08-13**

Five commits. **`NOT_YET` is down to two, both M3-general**, and PipDock has its first write outside site-packages and `%LOCALAPPDATA%`.

| | What landed |
|---|---|
| **Core** | `health::fix` — `dirty`, `FixConsent`, `consent_ok`, `ensure_writable`, `recheck`, `apply`, `FixReport`; `project::validate_project`; `run::ruff_argv` extracted so the re-check cannot drift |
| **Errors** | `PD-PRM-003`, internally raised. `Code::ALL` 33 → 34, and the checklist is **nine** places, not eight |
| **Bridge** | `health_fix` (the last Phase-3 `NOT_YET` row) and `health_dirty` |
| **CLI** | `pipdock health --fix`, with the `--yes` rule, the usage error, and a one-document `--json` |
| **UI** | the Fix button, one dialog with two states, `data-action="fix"` in the forced-colors list |
| **Docs** | DATA-FLOW gains **invariant 6** and §9.1's state machine; SECURITY §2 and §8; CODE-HEALTH-SPEC §5; CLI-SPEC §3; UI-SPEC §5 and §7 |

**The invariant question, answered rather than dodged.** DATA-FLOW §9.1 and §9.2 are scoped to a mutating *engine* call, and a ruff fix rewrites a source tree, which no snapshot describes. A snapshot taken here would have no consumer that could use it — invariant 2's own argument for the pip-upkeep exemption, a second time. So: **no snapshot**, a fifth proof shape (`HealthReport` + a server-checked `FixConsent`), and the exemption made *visible* the way P1's is, in the confirm's own copy.

**Three things found by running.**

1. **`--json --fix` printed two documents** — the pre-fix report and the `FixReport`. The first describes a state that no longer exists by the time the command returns, and CLI-SPEC §6 states the contract as one document. Found by reading stdout rather than asserting on it.
2. **A planted `git.exe` does not win.** This is the one place PipDock runs a PATH-resolved program with a user-controlled working directory. Windows' legacy search order would make a binary in a cloned repository run; Rust's `Command` does its own resolution and skips the current directory. Verified by planting one, not by reasoning about `CreateProcess`.
3. **The SECURITY paragraph was ahead of the code.** Writing "the project folder is validated and a folder inside the tools directory is refused" made it true rather than aspirational: `validate_project` now refuses it, `..` included, and running Health against `%LOCALAPPDATA%\PipDock	ools` answers `PD-ENV-003`.

**Verification, release build, against a seeded repository.** 6 fixable across 2 files with an unfixable `E722` in a third: after `--fix`, `git diff --stat` lists **exactly those two files** — the third untouched, which is what rules out a stray `ruff format` — and the run exits 1 with one finding remaining. Non-TTY without `--yes` refuses (exit 2). A dirty tree with `--yes` refuses naming the entry count (exit 2) and writes nothing. `attrib +R` on one target produces `PD-PRM-003` naming that file, and the **other**, perfectly writable, is not written either: the fix refuses as a whole rather than half-rewriting a tree nothing can restore.

**Deliberately not done:** `ruff format`. §4's `--check` toggle stays read-only and default-off; formatting a whole project is a far larger blast radius than fixing seventeen lint findings, and §7's non-goals are contractual.

### Phase 3 · P4 — the Health screen — **done 2026-08-13**

Ten commits, plus a Slice 0 that got the tool pins to their final version first. **Code Health is on screen**, `PdHealthReport` is UI-SPEC §6's fifteenth of sixteen, and both halves of Phase 3's exit criterion pass.

| | What landed |
|---|---|
| **Core** | `health::markdown`; `reporting()`, which keeps deptry out of `ran` for a project that declares nothing; `fixtures::{health_report, health_partial}` |
| **Bridge** | `EnvRow.healthProject` filled by `env_scan` **and** `env_probe`; `health_run` persists the folder; `health_save_report`; `env_probe` gains a `source` parameter |
| **UI** | `useHealthStore` (a third reset key, and the first that is a pair), `PdHealthReport`, `screens/PdHealth`, `pickProjectFolder`/`pickSavePath`, `apply` generalized so two stores share one console reducer |
| **Capabilities** | `docs.astral.sh` added to the opener allowlist, `dialog:allow-save` granted; both recorded in SECURITY §4 and the grant's own description |
| **Copy** | a `health` block in both locales, ~30 keys |

**Four defects found by running, none of them in the plan.**

1. **The screen said "no issues found" before anything had run.** With a folder remembered and no report yet — the identical lie a tab keyed on `ran` alone tells about a failed tool, one level up, written by someone who had just spent a commit preventing it. Empty states are the easiest place in this app to claim a result nobody produced.
2. **A project that declares nothing was reported as a deptry failure.** deptry raises `DependencySpecificationNotFoundError` and exits non-zero, which read as `PD-HLT-002`. PipDock had already detected the state and already told the user results would be limited — then ran a tool that cannot run in it and blamed the tool. Now skipped, and outside `ran`.
3. **`env_probe` hardcoded `EnvSource::Manual`.** So *Upgrade pip* on a registry-discovered interpreter relabelled it *Added manually* in the chip, and handed that relabelled `PyEnv` to every later `pkg_list`. A P1-era defect in the function P4 had to touch anyway.
4. **A test asserted a version Dependabot owns**, so merging the ruff bump broke `main`. Its CI had gone green on a base predating the health module.

**Deliberately not done: health-run cancellation.** Measured rather than assumed, which is what the plan asked for: a warm three-tool run is **1.3 s** and a cold one is that plus the ~15 s tools-venv bootstrap, both far under the 30 s threshold set before the number was known. It is also more than the bridge one-liner it looks like — `run_one` never passes the sink's token to `Command`, so wiring `set_cancel` alone would cancel only the sync, and `watchdog()` maps a cancel to `PD-INT-001`, so a cancelled run would render three "PipDock hit an internal error" rows. Worst case remains watchdog-bound at 3 × 120 s; revisit if a real project hits it.

**Exit criteria, release build, warm tools venv.** A requirements-only project reports 1 dependency, 3 dead-code and 3 lint findings (3 fixable in 1 file) in **1.4 s**; a project declaring nothing reports `ran: [vulture, ruff]` with an empty `problems` in **1.3 s**. In the running app against a stubbed bridge: `Ctrl+6` reaches Health, the `<h1>` takes focus, the remembered folder arrives on the row so the button reads *Change folder…*, `plugin:event|listen` crosses before `health_run`, the three tab counts are 4/3/6 against a fixture carrying 4/3/6, and on the partial report the ruff tab reads *did not finish* while vulture reads *not run* and the status line reads `⚠ 1`.

### Phase 3 · P3 — the runners and `HealthReport` — **done 2026-08-12**

Six commits. `pipdock health` runs deptry, vulture and ruff out of P2's tools venv and reports; `health_run` streams the same thing over `health-progress`. **`NOT_YET` is down to three, and only `health_fix` is still Phase 3's.**

| | What landed |
|---|---|
| **Core** | `health::{project,deptry,vulture,ruff,report,run}`; `HealthReport` and its six sub-types; `is_findings_exit`; the project folder in `health_projects`, keyed by `env_hash` |
| **Bridge** | `health_run`, its own `Sessions<HealthSession>` slot, and `forward_progress` taking the channel as a `&'static str` |
| **CLI** | `pipdock health [--path] [--tool …] [--json]`, exit 1 on findings; `--fix` refused until P5 |
| **Wire format** | `HealthReport` in `SCHEMA_TYPES` — one registration, seven TS declarations, because the generator hoists `$defs` |
| **Fixtures** | eight captured documents under `tests/fixtures/health/`, redacted and `-text` |

**Five things the specs had wrong, all found by running the pinned tools.**

1. **All three exit non-zero on findings, and disagree about how** — deptry 1, ruff 1, **vulture 3**, with vulture's 1 and 2 being real failures. A plain `!out.ok()` reports every successful run over a real project as `PD-HLT-002`, and a suite that only runs them over a clean directory agrees with it.
2. **deptry cannot be told about an environment.** §3 said the env is "passed to deptry"; there is no such flag, `VIRTUAL_ENV` is ignored, and it reads whatever its own interpreter imports. §2's isolation and §3's comparison conflict, isolation wins, and the cost is disclosed rather than papered over.
3. **deptry's `--json-output` is a file path, not a stream.** `-` would have created a file called `-` inside the user's project — the exact §1 boundary the flag was chosen to respect.
4. **ruff's docs URL is keyed by rule name**, so §6's `.../rules/<code>` 404s; and it writes findings to stdout with warnings on stderr, which corrupted the first fixture capture.
5. **A tool present but unrunnable reported `PD-ENG-001`** — "install the engine, or switch engine in Settings", for a corrupted `ruff.exe`. Now `PD-HLT-001`, which says re-sync the tools environment, which is the fix.

**Exit criteria, run against real projects.** `pipdock health` over a pyproject project reports 1 dependency, 3 dead-code and 3 lint findings and exits **1**; over a clean one it exits **0**; `--tool ruff` runs one tool and leaves `ran` naming only it, so an empty deptry tab is "not run" rather than "clean". Replacing `ruff.exe` with a text file of the same name — present, so the sync check says Fresh — produced the **partial report**: deptry and vulture reported normally, ruff landed in `problems` as `PD-HLT-001`, and the run still returned a report. Deleting `ruff.exe` outright instead triggers the implicit sync and repairs it.

**Deliberately not done:** `RunOptions` stays off the wire. §4 says the confidence floor comes "from settings", but `Settings` has three fields and none is this; adding them is a golden, a bindings regeneration and a Settings screen, which is its own slice.

### Phase 3 · P1 — pip upkeep — **done 2026-08-12**

Eight commits. **P0-10 is complete in both heads**, and three defects older than the slice went with it.

| | What landed |
|---|---|
| **Core** | `PipEngine::upgrade_pip` gains DATA-FLOW §2's PEP 668 guard and an `ensurepip` fallback; both heads call it directly rather than dispatching on the configured engine. `EnvRow.pip_version`, read out of the probe's own distribution list. |
| **Bridge** | `pip_upgrade` registered — `NOT_YET` is down to four, two of them M3-general. |
| **UI** | pip's version on every Environments row; *Upgrade pip* as a second inline button beside *Use*, below the planner floor only; a confirm that states the snapshot exemption. |
| **Docs** | ARCHITECTURE §7's return type corrected; DATA-FLOW §7 (pip unconditionally) and §9.1/§9.2 (a fourth plan shape, and a visible exemption); UI-SPEC §5 gains a row. |

**Three bugs that were not in the plan, none of them P1's own.**

1. **A venv was reported externally managed whenever its base was.** A venv has no stdlib, so `sysconfig.get_path("stdlib")` resolves to the base — and every venv built from a Python shipping the marker (uv-managed, Debian, Homebrew, Fedora) refused every mutation with `PD-ENV-002`. pip's own `check_externally_managed` opens with the venv early-return that `probe.py` was missing. Found by making a venv from a uv-managed 3.11 to test P1 and watching `pip-upgrade` refuse it.
2. **Every pip below the 22.2 floor is broken on Python 3.12+** — `distutils` and `pkgutil.ImpImporter` are both gone — so the button offered exactly when pip is too old ran a pip that could not start, and the traceback's `ModuleNotFoundError` classified as `PD-ENV-003`, "could not read this environment". Now falls back to `ensurepip`, which is stdlib and offline, chosen by asking `pip --version` rather than matching a traceback that differs by version.
3. **Five locale strings were being discarded by the two-file merge.** `common.json` and `errors.json` both carry a top-level `errors` object, and a shallow spread replaced one with the other. `errors.unknown` was among them — `PdErrorRow`'s fallback for an unrecognized code, the thing S7 built so English developer text could never reach a user, rendering as the literal `errors.unknown`. Invisible to both existing i18n tests: one checks `Code::ALL`, the other checks en/vi parity, and both locales lost the same keys.

**Exit criteria, counted by hand in the running app.** *Upgrade pip* → *Upgrade* is **2 clicks** from the landing screen. The row's version went 22.1.2 → 26.2.1 with exactly `["pip_upgrade", "env_probe"]` crossing the bridge — **no `env_scan`**, which is what "without a rescan" means and the only way to tell the two apart. The button then disappeared, the dialog's default focus was Cancel, and a refused upgrade rendered `PD-ENV-002` with localized copy and incremented `⚠ n`. Against real interpreters: 3.11 with a working pip 22.1.2 upgrades normally; 3.12 with the same pin, which cannot start, is repaired and ends at 26.2.1 running.

**Deliberately not done:** the *Upgrade pip* button does **not** appear merely because a newer pip exists. That needs `pkg_outdated` — networked, per environment, N calls on the landing screen — to surface something Installed and Updates already offer. What those screens cannot do is upgrade a pip so old the planner behind them refuses to run, which is the only case the button takes.

### Phase 3 · P2 — the tools venv — **done 2026-08-12**

Six commits. Code Health has an environment to run in, and `PD-HLT-001`'s shipped copy — *"Re-sync the tools environment"* — now refers to something the user can actually do.

| | What landed |
|---|---|
| **Core** | `health::pins`/`pins_hash`/`requirements_body` over `tools/tools-requirements.txt`, which had existed since the first scaffold commit and which nothing read. `sync_tools_venv(tools_dir, base_python, sink)` replaces the module's `todo!()`: create, install, verify. `ToolsManifest` + `SyncNeed` + `needs_sync`, `choose_tools_python`. |
| **Errors** | `PD-HLT-004` (`HltVenvCreateFailed`), the eight-place checklist in one commit. `Code::ALL` 32 → 33. |
| **CLI** | `pipdock tools sync [--force] [--python]` and `tools status`; `doctor` reports whether Health can run at all, without folding it into its exit rule. |
| **Wire format** | None. No Tauri command, no `NOT_YET` edit, no `COMMANDS` entry, no new `SCHEMA_TYPES`. P2 ships the `health-progress` **producer**; P3 ships the channel, when `health_run` exists to emit on it. |
| **Docs** | CODE-HEALTH-SPEC §2 amended on four points; CLI-SPEC §3 gains the commands and §6 gains a recorded, unclosed NDJSON gap. |

**Five findings that were not in the plan.**

1. **`TOOLS_PYTHON_MAX` should not exist, and the premise for it was wrong.** ROADMAP called it "the top unknown in all of M3" on the grounds that deptry ships compiled wheels with no cp314 build. deptry ships a Rust extension built against CPython's **stable ABI** — one `cp310-abi3` wheel, installed here on both 3.12.10 and 3.14.6. No ceiling was added; `--only-binary=:all:` makes a future gap a clean `PD-NET-011` instead of an sdist build.
2. **`tools/tools-requirements.txt` already existed and was already Dependabot-wired.** P2 consumes it rather than creating it, which deleted about half a day of the estimate.
3. **A re-sync could not repair the one state it exists to repair.** `pip install -r` over satisfied pins is a no-op, so replacing a deleted `ruff.exe` installed nothing and then failed verification with `PD-HLT-001` — and wedged, because the manifest had already been deleted. The venv is rebuilt now, not repaired. Found by deleting `ruff.exe`; invisible to a suite that only ever synced into an empty directory.
4. **`SyncNeed::ToolMissing(String)` could not be serialized.** `#[serde(tag = "state")]` is internal tagging and serde cannot represent a tagged newtype. It passed every unit test that constructed it and panicked the first time `tools status --json` met a missing tool. Now a struct variant, with a test that serializes all five states.
5. **The pin closure is not ABI-free, and the reason to exclude `pip-audit` is narrower than assumed.** deptry pulls `tomli`, which ships per-CPython wheels — but also a `py3-none-any` fallback, so a missing `cpXXX` build degrades to pure Python. `pip-audit`'s closure contains `msgpack`, which publishes **no** universal fallback and would hard-fail under `--only-binary=:all:`. That, not "exposure is zero", is why the ledger's fourth pin stays out of the venv.

**Exit criteria, measured by running it** (debug build, i7-14700KF, Python 3.14.6): cold bootstrap **15.2 s**, warm re-run **31 ms**, repair-after-quarantine **13.3 s**. `PIP_NO_INDEX=1` gives `error[PD-NET-011]`, exit **6**, an empty venv and — the thing that matters — **no `manifest.json` left behind**, so the next online run does not believe it is fresh. A forced `--python` onto 3.12.10 installs the same abi3 wheel. A sync killed mid-download leaves **zero orphan `pip.exe`** and no manifest.

**Deliberately not done:** deleting the dead `engine::uv::parse_dry_run` and flipping `clippy::todo` from `allow` to `warn`. After this slice that function holds the last `todo!()` in the workspace, so the flip is a clean two-line follow-up — but not inside a feature diff. `sync_tools_venv` also takes no `--break-system-packages` path and needs none: the tools venv is PipDock's own and is never PEP 668 managed.

## Phase 4 — RC → v1.0 (~1–2 weeks)

Release pipeline live (bundling, checksums), manual charter executed, docs/README screenshots, legal files public, a clean install of the RC verified on a machine that has never run PipDock. **Exit:** RELEASE-CI §5 checklist fully checked; v1.0.0 published + notify fan-out.

## Post-1.0 (P1 wave)

Security tab (pip-audit), pin auto-suggest, requirements/constraints export-import, cache manager, command palette, dependency graph view, scheduled check. Then P2 candidates by demand: macOS/Linux, per-env engine, elevation broker, winget, JP locale.

## Standing risks

| Risk | Mitigation |
|---|---|
| uv output churn between releases | weekly latest-engine parser CI job (TESTING L1) + PD-ENG-003 graceful degrade + "switch to pip" escape hatch |
| pip UX flags deprecations (e.g. report format changes) | same job covers pip; adapter version-gates per engine version |
| Index size growth | SP-3 fallback design kept in the drawer |
| Scope creep from Health into a linter IDE | CODE-HEALTH §7 non-goals are contractual |
