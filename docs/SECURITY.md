# PipDock — Security

*Version 0.1 · 2026-07-17*

## 1. Threat model (scoped)

PipDock's job is to run package managers against local environments. The realistic risks are: (a) PipDock being tricked into running something other than the intended engine command, (b) users being nudged into damaging their system Python, (c) compromised update delivery of PipDock itself, and (d) PipDock's own dependency supply chain. Malicious *packages* the user chooses to install are out of scope — pip/uv execute setup code by design; PipDock's mitigations are the preview step, snapshots, and the (P1) audit tab, not sandboxing.

## 2. Command execution hygiene

- All engine/tool invocations use argv arrays via `tokio::process::Command`; **no shell is ever involved**, so quoting/injection classes are structurally absent.
- Package names are validated against the PEP 508 name grammar (`^[A-Za-z0-9]([A-Za-z0-9._-]*[A-Za-z0-9])?$` after normalization) and version specs against PEP 440 before entering argv; anything else is rejected client-side with PD-PKG-002.
- Interpreter paths come only from discovery (registry/launcher/uv) or an explicit user file-pick; they are canonicalized and existence-checked (PD-ENV-001) before use.
- `probe.py` is stdlib-only, written to a temp file with a random name, executed with `-I` (isolated mode: ignores `PYTHONPATH`, user site) so a poisoned env cannot inject code into the probe.
  - **Owner decision 2026-07-27 (spike SP-6): `-I` stays, and the cost is disclosed rather than hidden.** Isolated mode also disables user site-packages, so on a *non-venv* system Python the probe sees fewer distributions than `pip list` does — measured at 352 vs 375 on the reference machine. Virtual environments, the primary target, have no user site and are unaffected. The probe therefore reports `hidden_user_site`: the user-site path when it exists, is off `sys.path`, and is non-empty; `null` otherwise. Only when it is non-null does the Installed screen show a note naming the path (`env.partialListing`), so the disclosure is accurate rather than a permanent disclaimer. Dropping to `-E` was rejected: a package in user site is part of the untrusted surface the isolation exists to exclude.
- Child processes run with inherited user privileges only; PipDock never elevates in v1 (PD-PRM-001 blocks instead — elevation broker is P2 with its own review).

## 3. PEP 668 policy (protecting the system Python)

Environments carrying an `EXTERNALLY-MANAGED` marker are **blocked by default** with an explanation and a "create a venv instead" pointer. The override lives in Settings, off by default, with explicit warning copy; when enabled, every mutating screen for that env shows a persistent warning chip, and the flag corresponding to `--break-system-packages` is passed only then. PipDock never adds that flag silently — this is a hard rule.

## 4. Network surface

| Destination | Purpose | Notes |
|---|---|---|
| `pypi.org` | PEP 691 name index, per-package JSON metadata | HTTPS with certificate verification; redirects restricted to `pypi.org`/`files.pythonhosted.org` |
| — everything else | none | engines make their own PyPI connections per their configs (user `pip.ini`/`uv.toml` respected, incl. private indexes — PipDock does not rewrite index URLs) |

`pypi.org` is the **only** host PipDock itself connects to. It does not update itself (§5), so it has no reason to reach `github.com` — the legal gate and the bug-report deep link hand a URL to the user's browser rather than fetching anything. The webview cannot reach either: `connect-src` in `tauri.conf.json` allows only `'self'` and the IPC origin, so all network access goes through Rust.

**Two hosts may be handed to the browser, and the allowlist is the record of why.** `capabilities/external-links.json` scopes `opener:allow-open-url` to `https://github.com/*` and, since Phase 3 · P4, `https://docs.astral.sh/*` — every ruff finding carries its own rule page and CODE-HEALTH-SPEC §6 as amended requires using ruff's `url` rather than constructing one. Neither is a connection PipDock makes; both are URLs it asks the OS to open. The widening is recorded here rather than only in `capabilities/` because an allowlist entry is a change to this document's subject matter, not a config detail — and because the failure mode of *not* widening it is silent: the promise rejects, nothing opens, and nothing anywhere says so.

No telemetry, no crash reporting endpoints, no analytics. TLS failures surface as PD-NET-002 and the app **never** offers to disable verification.

## 5. Update integrity (PipDock itself)

**PipDock does not update itself.** New versions come from the GitHub Releases page and are installed like any other download.

That is a deliberate removal, not an omission. A self-updater is a permanent remote-code-execution path into a tool that already runs subprocesses against the user's interpreters, and it has to be defended for the life of the product: a signing key that must never leak, an endpoint that must never be spoofable, and verification logic that must never be bypassable. For a tool people open occasionally to tidy an environment, that is a large standing liability bought for a small convenience.

What replaces it: SHA-256 checksums published per release (`SHA256SUMS.txt`) for manual verification, which the README tells users to check because SmartScreen warns on first run — the binaries are not EV-code-signed. Installer artifacts are NSIS + MSI from the Tauri bundler.

Consequences worth knowing: there is no signing keypair to generate or rotate, no `TAURI_SIGNING_PRIVATE_KEY` secret, no `latest.json`, and `tauri-plugin-updater` is not a dependency.

## 6. Auditing user environments (P1 Security tab)

pip-audit runs **from the tools venv** in freeze-file mode: snapshot-freeze the target env → `pip-audit -r <freeze> --no-deps --format json` → findings joined to installed rows (CVE/GHSA id, severity, fixed-in). Exact flags validated in spike SP-4 (pip-audit's foreign-env options evolve). Findings link to the OSV entry; "update to fixed version" hands off to the normal Update flow — audit never auto-applies anything.

## 7. Supply chain of PipDock itself

- CI: `cargo audit` (RustSec) + `npm audit --audit-level=high` gate every PR; Dependabot keeps Rust/npm/actions/tools-requirements pins current (see RELEASE-CI.md §2).
  - The `cargo audit` gate is on **vulnerabilities** and is never relaxed. `.cargo/audit.toml` ignores a fixed list of *informational* advisories — unmaintained/unsound — on transitive crates PipDock cannot influence, chiefly Tauri's archived GTK3 bindings, which a Windows-only build never compiles. They are listed individually with justification so a **new** advisory still fails CI. Adding macOS/Linux builds (P2-1) is the point at which that block must be re-examined rather than extended.
- `Cargo.lock` and `package-lock.json` committed; builds are `--locked`.
- CodeQL enabled via the ops-repo caller with the established permissions block, scoped to **`javascript-typescript` only** — CodeQL has no Rust analyzer. Rust's static-analysis coverage is clippy `-D warnings` plus `cargo audit`, both blocking in `ci-rust.yml`.
- Third-party licenses tracked in `legal/THIRD-PARTY-NOTICES.md`; `cargo about`/`license-checker` regenerate the inventory at release.

## 8. Data & privacy engineering

All state is local under `%LOCALAPPDATA%\PipDock\`. Logs may contain package names and paths — the bug-report flow shows the user exactly what will be prefilled and requires manual submission (ERROR-CATALOG §4). No identifiers are generated or stored beyond the random consent record. Deleting the app data folder is a complete reset (documented in Settings → Legal & About).

## 9. Vulnerability reporting

`SECURITY.md` at repo root (this file doubles as it) will carry a private reporting channel via GitHub Security Advisories on `poli0981/pipdock`; target acknowledgment ≤ 7 days. Coordinated disclosure preferred; in-range CVEs in shipped dependencies trigger a patch release via the standard pipeline.
