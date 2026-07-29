# PipDock — working notes for Claude

Windows GUI + CLI for bulk-managing Python packages. Tauri 2 + Rust core + React 19.
Repo `poli0981/pipdock` · GPL-3.0 · **Status: M1 complete (core + CLI); M2 (Tauri GUI) starting**.

## Read the docs before changing anything

The design is fully specified. Read in this order; do not invent behaviour that contradicts them.

1. `docs/PRD.md` — goals G1–G7, personas, P0/P1/P2 feature matrix, non-goals
2. `docs/ARCHITECTURE.md` — crate layout, `Engine` trait, `probe.py`, storage, Tauri IPC, version manifest
3. `docs/DATA-FLOW.md` — update/install/uninstall/rollback state machines, engine command mapping, **invariants §9**
4. `docs/UI-SPEC.md` — design tokens, screens, click budgets, keyboard map
5. `docs/CLI-SPEC.md` — commands, flags, exit codes, JSON contracts
6. `docs/ERROR-CATALOG.md` — the launch codes and their stderr classifiers. **30 in Rust**
   (`Code::ALL`, pinned by a test); the doc tabulates 28 because it folds `PD-HLT-001..003` into
   one row
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

- Crates `pipdock-*`; React components prefixed `Pd` (`PdPackageRow`, `PdConflictDialog`)
- Tauri identifier `com.skullmute.pipdock`; CLI binary `pipdock`
- Frontend lives in `ui/`, but `package.json` sits at the **repo root** so the ops-repo
  `reusable-web-react.yml` (which has no `working-directory` input) works unmodified; Vite is
  configured with `root: './ui'`
- Rust ↔ TS types are generated with `specta`/`tauri-specta` — never hand-mirror a type
- No component calls `invoke` directly; everything goes through the typed wrappers in `ui/src/ipc/`

## CI reality

`docs/RELEASE-CI.md` is authoritative *as corrected*: the ops repo `poli0981/.github` has **no
reusable Rust or Tauri workflow**, its notifiers are **Discord-only**, dependency bumps are
**Dependabot** (not Renovate), and its CodeQL reusable has **no Rust support**. So `ci-rust.yml` and
`release.yml` are project-local; `ci-node.yml`, `codeql.yml` and the notifiers are callers.
Every caller needs an explicit `permissions:` block — callers without one default to `none`.

## Current state

**M1 complete. M2 Stage 1 (the IPC bridge) complete 2026-07-30.** The app runs: legal gate →
Environments → Settings, over real commands. `docs/ROADMAP.md` Phase 2 has the full Stage 1 table
and says where to pick up — read it before starting a slice.

Next is **S2** (Installed + Updates, read-only), then **S3** (the mutation spine, the biggest
slice). Three things are deliberately deferred to the slice that can verify them, and ROADMAP
Phase 2 lists them: the `plan-progress` lifecycle enum, a Windows Job Object for whole-tree kill,
and the post-cancel summary copy.

Things worth knowing before you change any of it:

- **`cargo test` fails when `ui/src/ipc/generated.ts` is stale.** Fix with
  `cargo run -p xtask -- bindings`; the failure names that command and the first differing line.
- **The L4 goldens (`crates/pipdock-cli/tests/golden.rs`, 46 snapshots) are the CLI's output
  contract.** A diff there is a real behaviour change — re-bless deliberately, never reflexively.
  They are what made the `core::flow` refactor provably behaviour-preserving.
- **Two tests hold the wire format**: `Code::ALL` must serialize as `as_str()`, and no
  `SCHEMA_TYPES` property may contain `_`. Both fail at `cargo test`.
- **Run it, don't just test it.** Every one of the four unplanned bugs found during Stage 1 came
  from executing against a real environment or a real runner, not from reading code or passing
  tests: the watchdog that never killed anything, `--json` that never parsed, attribution that
  told 3.12 users something false, and a drift job that could not go green.

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
