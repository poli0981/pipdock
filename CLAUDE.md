# PipDock — working notes for Claude

Windows GUI + CLI for bulk-managing Python packages. Tauri 2 + Rust core + React 19.
Repo `poli0981/pipdock` · GPL-3.0-only · **Status: 1.3.0. M1, M2, Phase 3, Phase 4, the P1 wave, the Security tab and the dependency view are all complete.** 1.0.0 shipped the release pipeline, a protected `main`, the manual charter and legal documents that match the code; 1.1.0 added pin auto-suggest, requirements export/import, the cache manager and the `Ctrl+K` palette; 1.2.0 added the Security tab (PRD P1-1); 1.3.0 added the dependency view (P1-6), which is a *mode of the package screen* rather than a tab and adds no runtime dependency — read its ROADMAP record before touching `graph/`, because the shape of the screen is a consequence of measurements rather than of taste. **What is left of Post-1.0 is one thing** — the scheduled check (P1-7) — and `docs/ROADMAP.md` now carries the survey of what blocks it, which for two versions it was cited for and did not contain: no toast component, no notification/autostart/tray plugin, nothing on a timer, and no surface that can report a background failure.

## Read the docs before changing anything

The design is fully specified. Read in this order; do not invent behaviour that contradicts them.

1. `docs/PRD.md` — goals G1–G7, personas, P0/P1/P2 feature matrix, non-goals
2. `docs/ARCHITECTURE.md` — crate layout, `Engine` trait, `probe.py`, storage, Tauri IPC, version manifest
3. `docs/DATA-FLOW.md` — update/install/uninstall/rollback state machines, engine command mapping, **invariants §9**
4. `docs/UI-SPEC.md` — design tokens, screens, click budgets, keyboard map
5. `docs/CLI-SPEC.md` — commands, flags, exit codes, JSON contracts
6. `docs/ERROR-CATALOG.md` — the launch codes and their stderr classifiers. **34 in Rust**
   (`Code::ALL`, pinned by two tests); the doc tabulates fewer rows because it folds
   `PD-HLT-001..004` into one, so it shows **31 rows**. Adding a code means the catalog,
   `Code::ALL`, both counts, the "no code without a fixture" gate in `classifier_corpus.rs`,
   **both `locales/*/errors.json`**, a regenerated `codes.json`
   (`cargo run -p xtask -- ipc-fixtures`), regenerated bindings, and re-blessed goldens —
   `golden__schema-Code.snap` **and every schema that embeds it**, which since 1.2.0 is
   **four**: `StepResult`, `ExecutionSummary`, `HealthReport` and `AuditReport`, the last two
   via `ToolProblem.code`. **Five snapshot files in all.** This line named three while saying
   four, and stayed that way through 1.2.0 — grep `PD-ENV-001` across
   `crates/pipdock-cli/tests/snapshots/` rather than trusting the list. Nine places; the
   gates will tell you which you forgot, including `i18n.test.ts`, which fails on a code with
   no copy in either language
7. `docs/CODE-HEALTH-SPEC.md`, `docs/SECURITY.md`, `docs/TESTING.md`, `docs/RELEASE-CI.md`, `docs/I18N.md`, `docs/ROADMAP.md`
8. `legal/` — EULA, Disclaimer, Privacy, Third-Party Notices (public; the in-app legal gate links to these GitHub URLs)

## Hard invariants — never violate these

1. **No mutating engine call without (a) a `ResolutionReport` accepted this session and (b) a successful snapshot write.** Snapshot failure aborts the plan, executing nothing (`PD-SNP-001`). See DATA-FLOW §9.
2. **Subprocess via argv arrays only — never a shell.** `tokio::process::Command` with explicit args. Package names validated against PEP 508 and versions against PEP 440 *before* they reach argv. See SECURITY §2.
3. **Never reimplement dependency resolution.** The selected engine resolves in dry-run mode; PipDock parses and explains. See ARCHITECTURE §1.2.
4. **Rust emits catalog codes and structured data only.** All human phrasing lives in the i18next catalogs under `ui/src/locales/`. Codes, package names, versions and stderr are never translated. See I18N §1.
5. **`--break-system-packages` is never passed implicitly.** PEP 668 environments are blocked by default; the flag is only sent when the user has explicitly enabled the Settings override. See SECURITY §3.

Two more that follow from them: every user-visible failure carries exactly one catalog code, and a
failed package never aborts a batch (skip-and-continue, then a `"13 successful, 2 failed"` summary).

## What the spikes proved (read `spikes/README.md` before writing an adapter)

6. **Every plan must restate the full installed set as explicit requirements.** `install -U <pkg>`
   ignores installed packages' constraints in **both** engines and will break them at exit 0.
7. **Every pip invocation needs `PYTHONIOENCODING=utf-8` + `PYTHONUTF8=1`.** Without it
   `--dry-run --report -` crashes on Windows/cp1252 (pip 25 *and* 26), data-dependently.
8. **uv writes its plan to stderr**, so classify errors only after a plan-parse attempt.
9. **`env_hash` lowercases the interpreter path on Windows**, or one env splits its pins and
   snapshots in two.
10. **PipDock enforces `Requires-Python` itself** (`src/compat.rs`) — the engines disagree, and the
    preview must not depend on which one is selected.

## Conventions

- **One session, one slice, one version.** A session takes a single item — a `docs/ROADMAP.md`
  entry, or something that surfaced while doing one — takes it *deep*, tests it properly, and ships
  it as one version, plus a patch release if it needs a hotfix. Not several shallow items. The
  record of what a slice cost goes in ROADMAP; what a *user* would notice goes in `changelog/`.
- **The changelog lives in `changelog/`, one file per major line** (`1.x.md`), with a pre-release
  line getting its own file (`2.0-beta.md`). Root `CHANGELOG.md` is a pointer only. Split a file by
  minor range at roughly 600 lines and never renumber an existing range file —
  `changelog/README.md` is the authority.

- Crates `pipdock-*`; React components prefixed `Pd` (`PdPackageRow`, `PdConflictDialog`)
- Tauri identifier `com.skullmute.pipdock`; CLI binary `pipdock`
- Frontend lives in `ui/`, but `package.json` sits at the **repo root** so the ops-repo
  `reusable-web-react.yml` (which has no `working-directory` input) works unmodified; Vite is
  configured with `root: './ui'`
- Rust ↔ TS types are generated by `cargo run -p xtask -- bindings` over `schemars` — never
  hand-mirror a type. (ARCHITECTURE §9 originally said `specta`/`tauri-specta`; `bindings.rs` records
  why it is not, and drift is a `cargo test` failure rather than a compile error)
- No component calls `invoke` directly; everything goes through the typed wrappers in `ui/src/ipc/`

## CI reality

`docs/RELEASE-CI.md` is authoritative *as corrected*: the ops repo `poli0981/.github` has **no
reusable Rust or Tauri workflow**, its notifiers are **Discord-only**, dependency bumps are
**Dependabot** (not Renovate), and its CodeQL reusable has **no Rust support**. So `ci-rust.yml` and
`release.yml` are project-local; `ci-node.yml`, `codeql.yml` and the notifiers are callers.
Every caller needs an explicit `permissions:` block — callers without one default to `none`.

## Current state

**M1 and M2 complete** (IPC bridge 2026-07-30; Installed+Updates, the mutation spine
and search+install all 2026-08-04). The app discovers, lists, previews, decides, executes, streams,
summarises and installs over real commands. **"Update everything" is 4 clicks and "install one" is
4 clicks**, both counted by hand in the running app; search is **22 ms median per keystroke**
against a 50 ms budget. **15 of UI-SPEC §6's 16 components exist** — `PdEnvSwitcher` alone does
not, and nothing needs it while the header shows the selected interpreter. (This line said 16 of 16
for two milestones; §6 itself was corrected in P4 and is the authority.) `docs/ROADMAP.md` Phase 2
has a table per stage and says where to pick up; read it before starting a slice.

**Phase 3 is complete** — six slices, exit criteria met and recorded. P4 put Code Health on screen: `PdHealthReport` is UI-SPEC §6's
fifteenth of sixteen, `EnvRow.healthProject` remembers the folder per environment, and
`health_save_report` writes Markdown + JSON. **`NOT_YET` is down to two**, both M3-general — P5 landed the
gated `ruff --fix`, PipDock's first write outside site-packages and `%LOCALAPPDATA%`. P4 *did* need new commands, contrary to the earlier claim — one shipped, and
cancellation was deferred on a measurement (a warm three-tool run is 1.3 s in release).

Four P4 defects worth not repeating, all found by running: an empty state that said *no issues
found* before anything had run; a project declaring no dependencies reported as a deptry failure
rather than as deptry not being applicable; `env_probe` hardcoding `EnvSource::Manual`, so
*Upgrade pip* relabelled a registry-discovered interpreter; and a test asserting a version
Dependabot owns, which made merging a ruff bump break `main` after CI had gone green on a base
predating the code that would have caught it.
Three tool facts are pinned by tests because the specs had them wrong: all three exit **non-zero on
findings** and **vulture uses 3**; deptry emits a flat list keyed by *module* to a **file path** and
**cannot be told which environment to compare against**; ruff's docs URL is keyed by rule **name**.

P2 built the tools venv — `health::sync_tools_venv` fills
`%LOCALAPPDATA%\PipDock\data\tools\.venv` from the shipped pins over `pipdock tools sync`, and there is
no `TOOLS_PYTHON_MAX` because deptry ships an abi3 wheel (ROADMAP's premise for that const was
wrong; the record says why). P1 closed **P0-10**: pip's version is on every Environments row and
*Upgrade pip* is 2 clicks. **Next is P3** (the three runners + `HealthReport`), then P4 (the Health
screen) and P5 (the gated `ruff --fix`); `~/.claude/plans/b-n-c-th-b-t-snappy-rabin.md` has all
four decomposed, including six corrections to the specs found by running the pinned tools.
P2 touched no `NOT_YET` entry: it is CLI-only on purpose, so P3's `health_run` owns the
implicit sync rather than the frontend getting two ways to reach one operation.

**`NOT_YET` is two**, and only two: `env_add_manual` and `logs_tail`, both M3-general.
`src-tauri/src/lib.rs:25-28` is the authority and three tests keep it honest in both directions —
this file used to say "four" in one paragraph and "two" in another, each true when written and
neither updated.

Three rules P1 left behind, all found by running rather than reading:

- **A venv is never externally managed**, and `sysconfig.get_path("stdlib")` inside one resolves to
  the *base*. `probe.py` was missing the early return pip's own `check_externally_managed` opens
  with, so every venv built from a uv-managed, Debian, Homebrew or Fedora Python refused every
  mutation. Anything reading a path out of `sysconfig` inside a venv is asking about the base.
- **`envs::venv_scan` matches exactly `.venv`, `venv`, `env`, `.env`.** A scratch venv named
  anything else is invisible to `env list` and to the GUI, and `env_add_manual` (*Browse…*) is
  still owed — so hand-verification of any environment feature needs one of those four names in
  the working directory.
- **Every pip below 22.2 is broken on Python 3.12+** (`distutils`, `pkgutil.ImpImporter`), so the
  one case *Upgrade pip* exists for is also a pip that cannot run. `upgrade_pip` falls back to
  `ensurepip`, and decides by asking `pip --version` rather than matching a traceback.

Three rules from S2/S3 that bind everything after them:

- **"N pinned excluded" in the table is presentation, not enforcement.** DATA-FLOW §9.5 is enforced
  by `pins::filter_upgrades` at the plan boundary; a preview must show the flow's
  `excluded_pins()`, not a number the UI computed, or the two drift.
- **Never render a state you have not loaded.** Row state is three-valued
  (`unknown`/`current`/`outdated`) because `pkg_list` is local and `pkg_outdated` is networked.
  Treating "not yet known" as "up to date" dims 200 rows and un-dims a handful a second later.
- **A Tauri command's future must be `Send`, and `Store` is not `Sync`.** Never hold the store
  guard across an await — it does not compile at the command boundary, and where it does compile
  (inside core) it serializes every other command behind a network call. Read what you need, drop
  the guard, then await. This is why `UpdateFlow::start` takes pins rather than a `&Store`, and why
  `index::metadata`/`refresh` take the app-data path. It has bitten twice; assume it will again.
- **Two slices that key on different things need two resets.** The package slice is keyed to the
  *selected* environment and the snapshot timeline to the *open* one, and they move independently —
  so folding the timeline into `NO_PACKAGES` meant a rescan wiped a freshly-loaded timeline whenever
  the package slice happened to be stale. After a rollback that is exactly the case. A shared reset
  constant is only correct for fields with a shared key.

- **A spec can promise a field that has never existed.** SECURITY §6 asked for a *severity* per
  finding and PRD P1-1 for "severity-sorted"; pip-audit publishes no severity under **either**
  vulnerability service, and never has. §6 also said pip-audit runs from the Code Health tools venv,
  which the code had refused since P2 with three tests enforcing the refusal. Both were found by
  *running the pinned tool and reading its JSON*, not by reading the specs against each other. When
  a spec sentence describes an external tool, the tool is the authority — and the correction belongs
  in the spec, not in a workaround that invents the missing field.

- **A watchdog is a unit, not a number.** `audit::run` inherited Code Health's 120 s `TOOL_TIMEOUT`
  and hit it on the first real run, reporting `PD-HLT-003` for a tool that was working. Code Health
  is a linter walking a source tree; an audit is a network fetch. Measured: **68 s cold** for one
  package, 20.0 s warm for *twelve* — twelve cost 1.4 s more than one, so the cost is a one-off
  database fetch and package count is not the risk. The same measurement reopened cancellation,
  which P4 had deferred on Code Health's 1.3 s. **A borrowed constant carries its original units
  with it.**

- **Adding a field to a struct that is summed is a bug the suite will not see.** `Target::Audit`
  gave `cache::Usage` a fourth entry and `total_bytes` still added three — every cache test passed
  and the number on screen was wrong with nothing to say so. The guard asserts on distinct powers
  of two so a missing term appears as its own bit rather than as a plausible total. Same shape as
  the `entryOf` ternary whose last branch was a hand-maintained negation of every case above it.

- **A dialog is not an enforcement point.** DATA-FLOW §5's three options are what the user *sees*;
  what stops a removal is `GuardAck` inside `UninstallFlow::execute`, beside `SnapshotProof`. The
  same shape three times now — `SnapshotProof`, `GuardAck`, and P5's `FixConsent`: the report and
  the proof are produced in one call and consumed in another, so the only way "somebody forgot to
  look" cannot happen is a value the executing call demands. Any future flow with a decision
  between two IPC messages needs its own.

- **`ruff --fix` takes no snapshot, and that is the reasoned answer rather than an oversight.**
  DATA-FLOW §9.1/§9.2 are scoped to a mutating *engine* call; a fix rewrites the user's source
  tree, which no snapshot in this application describes, so one taken here would have no consumer
  that could use it — invariant 2's own argument for the pip-upkeep exemption, a second time. The
  safety net is the user's own git, asked about **twice** (once to decide what the dialog renders,
  once inside `health_fix` to decide what it allows), and the exemption is stated in the confirm
  rather than left silent. DATA-FLOW invariant 6 is where this lives.

- **Measure in `--release`, and say which build a number came from.** A debug build measures bounds
  checks: the same index load is 572 ms in debug and 140 ms in release, and the same keystroke is
  176 ms against 16 ms. A design was justified with a figure four times too large before anyone
  noticed. `crates/pipdock-core/tests/search_latency.rs` refuses to run in debug for that reason.

Things worth knowing before you change any of it:

- **`cargo test` fails when `ui/src/ipc/generated.ts` is stale.** Fix with
  `cargo run -p xtask -- bindings`; the failure names that command and the first differing line.
- **The L4 goldens (`crates/pipdock-cli/tests/golden.rs`, 64 snapshots at 1.1.0) are the CLI's output
  contract.** A diff there is a real behaviour change — re-bless deliberately, never reflexively.
  They are what made the `core::flow` refactor provably behaviour-preserving.
- **Two tests hold the wire format**: `Code::ALL` must serialize as `as_str()`, and no
  `SCHEMA_TYPES` property may contain `_`. Both fail at `cargo test`.
- **A control that is not a tab stop is a control a keyboard user does not have.** Making the
  package row the only tab stop is right — 200 rows would otherwise be 600 — but every button
  inside one was `tabIndex={-1}`, so *Pin* and *Remove* were mouse-only, which is a WCAG 2.1.1
  failure. The ARIA grid answer is a roving tabindex (`←`/`→`, `Esc` back out), and P6 added it.
  `Enter` on a row is the **non-destructive** primary — pin, never uninstall.

- **The seam between a tested component and its untested parent is where bugs live.**
  `PdSummarySheet` had tests; `PdPlanPanel` had none; both rendered `plan.done`, so a successful
  run showed *Back to packages* twice and every suite stayed green. Found by a human using the
  installed build. When adding a component test, ask what renders it.

- **A hand-written fixture that matches the docs while the code does not is worse than none.**
  `fixtures.rs` says so in its own module doc, and `guard_report()` is computed for that reason —
  but the plan fixture's blockers were hand-written to the shape `plan::Blocker`'s doc describes
  while `graph::blockers_for` had drifted to building an English sentence. So `PdPreviewDiff`
  asserted the *right* sentence and proved nothing, the preview shipped reading `scipy requires
  scipy 1.11.4 requires numpy <1.28.0,>=1.21.6` from S3 to 1.1.0, and no suite could see it. If a
  fixture's value can be computed by the function that produces it in production, compute it.

- **A sentence assembled in Rust is a sentence the other head assembles again.** That was the
  mechanism above: the CLI prints `Blocker.constraint` alone, so building the phrase in core made
  the *CLI* look right and left the GUI to wrap it in `plan.blocker` a second time. Invariant 4 is
  not only about translation — a joined string cannot be un-joined by a catalog. Structured fields,
  one composition per head, as `BrokenDependent`/`PdUninstallDialog` have always done.

- **Never `sed -i` anything under `tests/fixtures/`.** Git Bash's sed rewrites in text mode and
  strips every CR, which is exactly what `.gitattributes`' `-text` exists to prevent; one command
  flattened ten engine fixtures and reported 160 changed lines where one was intended. Use a
  binary-mode read/write and assert the CRLF count per file.

- **A parser that is correct for trusted input is a hole for untrusted input.**
  `graph::Requirement::parse` splits the name at the first space, so `this is not a requirement`
  parses as package `this` with constraint `is not a requirement` — exactly right for
  `Requires-Dist` metadata, which never looks like prose, and a spec containing spaces heading for
  argv the moment the input is a file a user typed. Reuse the parser, then re-validate at the new
  boundary; SECURITY §2's obligation follows the *input*, not the function.

- **Before adding a size or a "clear" for something, check it exists.** `LOG_RETENTION_DAYS` has
  had no reader since it was written, so P1-C's cache manager covers three artefacts rather than
  the four the roadmap listed. A row reading "logs — 0 B" would have invented a subsystem.

- **A test can assert a substring you did not think of.** `PdPins.test.tsx` proves the screen
  offers no way to create a `Hold` pin with `queryByText(/hold/i)` — and **"threshold" matches
  that.** P1-A's Settings label is "Suggest pinning at" for that reason alone. Before adding copy
  to a screen, grep its test for regex assertions; a broad `/word/i` over a whole screen is a trap
  laid for a feature that does not exist yet.

- **A doc comment describing an intent nothing implements is worse than none.**
  `ReverseDeps::build` said auto-suggest "may still count" extra-gated edges while
  `dependent_count` had always filtered them. Whoever built the feature would have believed the
  comment or the code, and had no way to know which. When you find one, settle it in the comment
  rather than leaving both readings alive.

- **A green badge is not a green `main`.** The README badge is per *workflow file* and shows that
  file's last run, which may be a schedule on a different commit. `main` was red for a day at
  `3eeda05` — every cargo job failing before a crate compiled — because `Cargo.lock` carried a
  dependency edge to `thiserror 2.0.19` while its package entry said `2.0.20`, which `--locked`
  refuses to repair. Start with `gh run list --branch main`, not the badge. And when bumping a
  version: it lives in **five** places (`Cargo.toml`, `Cargo.lock`, `package.json`,
  `package-lock.json`, `tauri.conf.json`) and they move together or you have reproduced this bug.
  `cargo metadata --locked` is the one-command check.

- **A `docs/` fix and a `legal/` fix are two different changes.** Removing the self-updater updated
  `THIRD-PARTY-NOTICES.md` and not `PRIVACY-POLICY.md`, so the shipped privacy policy described a
  feature that no longer existed — and, worse, still told users to delete `%LOCALAPPDATA%\PipDock\`
  for "a complete reset", which is the folder the program installs into. `docs/SECURITY.md` had
  recorded the data-root move correctly the whole time. Anything under `legal/` is what a *user*
  reads; grep it separately whenever behaviour changes. Every `legal/*.md` and `LICENSE` is hashed
  into `PIPDOCK_LEGAL_DOCS_HASH`, so any edit re-shows the gate once — that is the mechanism
  working, not a regression.

- **A spec sentence is not a test.** UI-SPEC §8 asserted for two milestones that `Ctrl+1..N` refused
  while a plan owned the screen. It did not: `App.tsx` guarded on `guardOpen`, and `PANEL_PHASES`
  excludes `guard` on purpose, so the two sets are disjoint and guarding on one guarded neither.
  Invisible until the plan ended and dropped the user on a tab they never chose.

- **Appending to `NAV_KEYS` is free; inserting is not.** `Ctrl+9` for About cost one array entry.
  Four comments claimed "a ninth entry would renumber `Ctrl+1..8`" and had the reason wrong —
  Snapshots stays a mode of Environments because it would sit *beside* Environments, an insert.

- **Building and installing are their own test tier, and nothing else substitutes.** The first
  bundle ever produced exposed three defects in one afternoon: `release.yml` could not have run
  (`--locked` is cargo's flag and goes after `--`), the installers shipped no `pipdock.exe` at all,
  and the program installed *into* the data directory — making SECURITY §8's "delete the folder to
  reset" advice uninstall the app. None was visible in a diff. See `docs/ROADMAP.md`'s Phase 4
  section for the layout rules that came out of it.

- **Run it, don't just test it.** Every unplanned bug in Stage 1 *and* Stage 2 came from executing
  against a real environment, a real runner or a real browser — never from reading code or from a
  passing suite. Stage 1: the watchdog that never killed anything, `--json` that never parsed,
  attribution that told 3.12 users something false, a drift job that could not go green. Stage 2:
  a probe 10× slower than before (found by timing it), a size that was *wrong* rather than missing
  for editable installs and a package listed twice (both found by one real `pip install -e`), and
  every screen fetching twice on mount (found by logging what actually crossed the bridge).
  Stage 3: a cancellation test that passed while the orphan lived on. Stage 4: an index load
  reported four times too large because it was measured in debug, a keystroke budget that was
  being missed by 30 ms, and a plan started from Search that resolved into a screen which does not
  render it — the command ran, the flow parked, and the user saw nothing change.
- **`probe.py` is on the hot path.** The Installed screen re-probes on every environment open, so
  anything added to `_dists()` is paid 200+ times per open. `Distribution.files` looks harmless and
  costs 10×; read the metadata files directly and measure before committing.

## Things that look like bugs but are not

- `probe.py` reporting fewer packages than `pip list` on a **non-venv** Python is the documented
  `-I` trade-off; `hidden_user_site` is non-null exactly then.
- `snapshot diff` showing entries a rollback cannot restore (editable installs, direct URLs) is
  deliberate: they are excluded from the pin set and reported separately rather than dropped.
- `pipdock engine uv` failing when uv is not on PATH is intentional — storing a preference that
  cannot be honoured just moves the failure to the next command.
- **The app has no self-update and no `tauri-plugin-updater`.** Removed on purpose (SECURITY §5):
  a self-updater is a standing remote-code-execution path into a tool that already runs
  subprocesses against the user's interpreters, and it has to be defended forever — a key that
  must never leak, an endpoint that must never be spoofable. Tauri makes adding it back a
  three-line change, so this is written down. Users get new builds from GitHub Releases and
  verify `SHA256SUMS.txt`. Do not reintroduce it without an explicit owner decision.
- Fixtures under `tests/fixtures/{pip,uv}/` are **not** literal engine bytes: `spikes/capture.py`
  redacts absolute paths, object addresses, progress bars and uv's timings. Without that they
  differ on every run and every machine, so the weekly drift job could never go green — and pip's
  "[notice] To update, run: …" shipped the capturing user's home directory to a public repo. It is
  safe because nothing reads a path or a duration out of them: every `CLASSIFIERS` entry keys off
  message text, and the uv adapter only tests `contains("Resolved ")`. Line endings are still
  byte-exact, which is what `.gitattributes`' `-text` protects.
- `meta.json` next to `capture-provenance.json` is a deliberate split. `meta.json` is the contract
  the drift gate watches; provenance records engine/interpreter versions and argv, churns on every
  release, and is excluded from the gate.
- **`snapshot::Meta` carries `#[serde(alias)]` on its renamed fields.** It is the on-disk
  `.meta.json` *and* an IPC type; without the aliases every snapshot written before the camelCase
  change becomes unparseable, and the rollback the user was relying on silently is not offered.
  Removable only once no pre-1.0 snapshot can still be on disk.
- **`take_snapshot()` and `execute()` are two calls on purpose**, so the CLI can print the snapshot
  id before execution and the GUI can draw DATA-FLOW §3's distinct states. The cost is that
  skipping the snapshot becomes expressible, which is why `flow::proof_from` refuses `NotTaken`
  with `PD-SNP-001`. Do not "simplify" them back into one.
- **Fixtures under `tests/fixtures/{pip,uv}/` are not literal engine bytes** — see the redaction
  note above. Related: `ScanProgress.label` and every path, version and engine id in the UI are
  data and are never translated (I18N §2).
