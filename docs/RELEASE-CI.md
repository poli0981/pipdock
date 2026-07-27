# PipDock — Release & CI

*Version 0.1 · 2026-07-17 · CI is built as thin callers into the `poli0981/.github` ops repo's reusable workflows (align exact workflow filenames with the ops repo's current Rust/Node/Tauri suite at repo creation).*

## 1. Caller workflows (`.github/workflows/`)

| Caller | Reusable target (ops repo) | Triggers | Jobs |
|---|---|---|---|
| `ci-rust.yml` | reusable Rust workflow | PR, push main | fmt, clippy `-D warnings`, `cargo test --locked`, `cargo audit` |
| `ci-node.yml` | reusable Node workflow | PR, push main | Node 24, `npm ci`, eslint, Vitest, `npm audit --audit-level=high`, `vite build` |
| `ci-integration.yml` | (project-local) | PR touching `engine|plan|snapshot|graph`, nightly | Windows runner, disposable-venv suite (TESTING L2), weekly latest-engine parser job |
| `codeql.yml` | reusable CodeQL workflow | PR, schedule | JS/TS + Rust where supported |
| `release.yml` | reusable Tauri/build + project steps | tag `v*` | build, sign, bundle, publish (see §3) |
| `notify.yml` | reusable notify workflow → `notify.py` | release published | cross-post Telegram/Discord/Bluesky/Mastodon/X/Facebook |

**Permissions matrix (explicit in every caller — the Phase-5 lesson: callers without a `permissions:` block default everything to `none`):**

```yaml
# codeql.yml
permissions: { actions: read, contents: read, security-events: write }
# notify.yml
permissions: { contents: read, actions: read }
# release.yml
permissions: { contents: write }        # release assets
# ci-*.yml
permissions: { contents: read }
```

## 2. Branch & version policy

`main` protected (PR + green CI required). SemVer; tags `vX.Y.Z`. Conventional commits feed the changelog generator. Renovate config extends the ops-repo preset: weekly grouped PRs for crates/npm, **separate immediate PRs** for security advisories and for `tools-requirements.txt` (Code Health pins), and a watch on `pip`/`uv` releases to trigger the parser-fixture job.

## 3. Release pipeline (tag `v*`)

1. Windows runner: `npm ci && vite build` → `cargo tauri build --locked` producing NSIS `.exe` + `.msi`.
2. Sign updater artifacts with `TAURI_SIGNING_PRIVATE_KEY` (repo secret; public key baked into `tauri.conf.json`); generate `latest.json` for `tauri-plugin-updater` pointing at the GitHub Release assets.
3. Compute and publish `SHA256SUMS.txt`.
4. Draft GitHub Release with generated changelog; manual publish (owner review) → `notify.yml` fires.
5. Post-release checklist issue auto-opened: verify updater from previous version, verify SmartScreen behavior, spot-check EN/VI in the shipped build.

Secrets bootstrap: extend the ops repo's `setup-secrets.sh` with `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` entries for this repo.

## 4. Artifact & support policy

Two active lines at most: latest release gets features; previous minor gets security-only patches for 90 days. Updater always targets latest. MSI provided for org deployment; winget manifest submission is P2 (tracked in ROADMAP).

## 5. Repo scaffolding checklist (day one)

- [ ] GPL-3.0 `LICENSE` at root; `legal/` committed **publicly** (the in-app legal gate links to these GitHub URLs — CommandForge lesson: don't leave them gitignored).
- [ ] `.github/ISSUE_TEMPLATE/bug_report.yml` (shipped in this suite) + `config.yml` disabling blank issues.
- [ ] Branch protection, Renovate, CodeQL, secrets via `setup-secrets.sh`.
- [ ] `rust-toolchain.toml`, `.nvmrc` (24), `Cargo.lock`/`package-lock.json` committed.
- [ ] README badges: CI, CodeQL, release, license.
