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
6. `docs/ERROR-CATALOG.md` — the 25 launch codes and their stderr classifiers
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

**M1 complete.** The CLI works against real environments: every P0 command except `health` (which
belongs to M3). See `docs/ROADMAP.md` Phase 1 for what is verified and what changed during
implementation. Still owed: the L4 golden-output tests (`assert_cmd` and `insta` are dev-deps and
currently unused) and the SP-5 numpy/scipy/pandas dogfood.

Next is M2, the Tauri GUI. Read `docs/ROADMAP.md` Phase 2 before starting — the bridge is thinner
than it looks: `ui/src/ipc/index.ts` fixes 26 command *names* with zero wrappers, `src-tauri`
registers exactly one command (`app_info`, a smoke test), and 2 of 16 `Pd*` components, 1 of 5
Zustand stores and 2 of 14 locale catalogs exist.

## Things that look like bugs but are not

- `probe.py` reporting fewer packages than `pip list` on a **non-venv** Python is the documented
  `-I` trade-off; `hidden_user_site` is non-null exactly then.
- `snapshot diff` showing entries a rollback cannot restore (editable installs, direct URLs) is
  deliberate: they are excluded from the pin set and reported separately rather than dropped.
- `pipdock engine uv` failing when uv is not on PATH is intentional — storing a preference that
  cannot be honoured just moves the failure to the next command.
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
