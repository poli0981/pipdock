# PipDock — Release & CI

*Version 0.2 · 2026-07-27 · CI reuses the `poli0981/.github` ops repo where a suitable reusable workflow exists, and is project-local where one does not. The ops repo's suite was audited on 2026-07-27 (see §1 note) — do not assume a reusable exists without checking.*

## 1. Workflows (`.github/workflows/`)

| Workflow | Kind | Triggers | Jobs |
|---|---|---|---|
| `ci-rust.yml` | **project-local** | PR, push main | `windows-latest`: `cargo fmt --check`, clippy `-D warnings`, `cargo test --locked`, `cargo audit` |
| `ci-node.yml` | caller → ops `reusable-web-react.yml` + one local job | PR, push main | Node 24, `npm ci`, lint / typecheck / Vitest (each runs only if the npm script exists), `vite build`; local job adds `npm audit --audit-level=high` |
| `ci-integration.yml` | project-local | PR touching `engine\|plan\|snapshot\|graph`, nightly | Windows runner, disposable-venv suite (TESTING L2), weekly latest-engine parser job |
| `codeql.yml` | caller → ops `reusable-codeql.yml` | PR, schedule | `languages: '["javascript-typescript"]'` only |
| `release.yml` | **project-local** | tag `v*` | `windows-latest` build, sign, bundle, publish (see §3) |
| `announce-release.yml` | caller → ops `announce-release.yml` | release published | Discord release announcement |
| `notify-ci-failure.yml` | caller → ops `notify-ci-failure.yml` | `workflow_run` completed | Discord CI-failure notification |

**Ops-repo audit (2026-07-27) — why the table looks like this:**

- The reusable suite covers assembly, c-cpp, chrome-extension, desktop-csharp, go, jvm, macos-swift, php, python, r, ruby and web-react. **There is no Rust reusable and no Tauri reusable**, and only `reusable-desktop-csharp.yml` uses a Windows runner — hence `ci-rust.yml` and `release.yml` are project-local.
- `reusable-web-react.yml` has **no `working-directory` input** (it runs `npm ci` at the repo root — hence `package.json` lives at the root with Vite's `root: './ui'`, see ARCHITECTURE §2) and **no `npm audit` step**, hence the extra local job.
- `reusable-codeql.yml` supports csharp, python, javascript-typescript, ruby, c-cpp, java-kotlin, go and swift. **CodeQL has no Rust analyzer**, so Rust security coverage comes from clippy `-D warnings` + `cargo audit` in `ci-rust.yml`, not from CodeQL.
- Portfolio notifications are **Discord-only**, driven by the ops repo's `.github/scripts/discord_notify.py`. There is no multi-platform `notify.py`.

**Permissions matrix (explicit in every caller — the Phase-5 lesson: callers without a `permissions:` block default everything to `none`):**

```yaml
# codeql.yml
permissions: { actions: read, contents: read, security-events: write }
# announce-release.yml / notify-ci-failure.yml
permissions: { contents: read, actions: read }
# release.yml
permissions: { contents: write }        # release assets
# ci-*.yml
permissions: { contents: read }
```

## 2. Branch & version policy

`main` protected (PR + green CI required). SemVer; tags `vX.Y.Z`. Conventional commits feed the changelog generator.

**Applied 2026-08-14, and it had been false until then** — `/branches/main/protection` returned 404 for four milestones, which is also why `gh pr merge --auto` merged immediately on this repo rather than waiting: with no required checks there is nothing for auto-merge to wait on. Exact settings:

| Setting | Value | Why |
|---|---|---|
| Required checks | `fmt · clippy · test`, `cargo audit`, `lint · typecheck · test · build / build`, `npm audit`, `analyze / Analyze (javascript-typescript)` | the five that run on **every** PR |
| Check app | pinned to `app_id: 15368` (`github-actions`) | a check from another app cannot satisfy a requirement |
| `enforce_admins` | `true` | the owner is the only contributor; protection that exempts them protects nothing |
| Required approvals | **0** | a solo owner cannot approve their own PR, so 1 would deadlock. A PR is still required |
| `strict` | `false` | up-to-date-before-merge is friction on a repo that rebase-merges |
| Linear history | `true` | matches rebase-merge |
| Force pushes / deletions | `false` | |

**`CI / Integration` is deliberately excluded, and this is the trap worth remembering:** its `pull_request` trigger is `paths:`-filtered, so on a docs-only or UI-only PR it never starts — and a required check that never starts stays *pending forever*. Requiring it would deadlock exactly the PRs it has no opinion about. `Re-capture against the latest engines` is schedule-only and excluded for the same reason. The bare `CodeQL` check belongs to the `github-advanced-security` app rather than Actions and is left out too.

**Two required contexts are named by the ops repo, not by this one.** `analyze / Analyze (javascript-typescript)` and `lint · typecheck · test · build / build` come from `poli0981/.github`'s reusables. If either job is renamed upstream, every PipDock PR deadlocks with no obvious cause and no failing check to point at — the fix is to update the protection payload, not the code.

**If protection wedges**, `PUT` the same endpoint with `enforce_admins: false`, or `DELETE /branches/main/protection`. Worth knowing before it is needed rather than discovering it during an incident.

Dependency updates use **Dependabot** (`.github/dependabot.yml`) — the portfolio standard, as evidenced by the ops repo's `dependabot-summary.yml`; there is no Renovate configuration anywhere in the ops repo. Four ecosystems: `cargo`, `npm`, `github-actions`, and `pip` scoped to `tools-requirements.txt` (the Code Health pins). Weekly schedule with minor/patch grouped per ecosystem; security advisories arrive as **immediate, ungrouped** PRs (Dependabot's default for security updates), and the `pip` ecosystem stays its own group so Code Health pin bumps are reviewable in isolation. Watching `pip`/`uv` releases to trigger the parser-fixture job is the scheduled job in `ci-integration.yml`, not the dependency bot.

## 3. Release pipeline (tag `v*`)

1. Windows runner: `npm ci` → **`npm run bundle`**, producing NSIS `.exe` + `.msi`. That one script owns the whole recipe so the workflow cannot drift from what a developer runs locally, and it encodes two things that are easy to get wrong:

   - **`--locked` goes after `--`.** It is cargo's flag; `tauri build` rejects it as its own with *"unexpected argument"* before a single crate compiles.
   - **The CLI sidecar is declared in an overlay** (`src-tauri/tauri.bundle.conf.json`, merged with `--config`), not in `tauri.conf.json`. `tauri-build` validates `externalBin` at build-script time, so declaring it in the base config makes plain `cargo clippy` and `cargo test` fail in a fresh clone with *"resource path … doesn't exist"* — a bundling concern leaking into every compile.

   The frontend is built by tauri's `beforeBuildCommand`, so the workflow does not run `vite build` itself.
2. Compute and publish `SHA256SUMS.txt`.
3. Draft GitHub Release with generated changelog; manual publish (owner review) → `announce-release.yml` fires the Discord announcement.
4. Post-release checklist issue auto-opened: verify SmartScreen behavior, spot-check EN/VI in the shipped build.

There is no signing step. **PipDock does not update itself** (SECURITY §5), so no `latest.json` is produced, no artifact is signed, and no keypair exists to manage.

Secrets bootstrap: set per-repo with `gh secret set` (the ops repo has no `setup-secrets.sh`) — the Discord webhooks the ops-repo notifiers expect (`DISCORD_RELEASES_WEBHOOK`, `DISCORD_REPO_WEBHOOK`, `DISCORD_CI_WEBHOOK`; the callers pass `secrets: inherit`). That is the whole list.

## 4. Artifact & support policy

Two active lines at most: latest release gets features; previous minor gets security-only patches for 90 days. Users move between versions by downloading from the Releases page — there is no update channel to target. MSI provided for org deployment; winget manifest submission is P2 (tracked in ROADMAP), and would be the natural place to add an upgrade path that is not PipDock's own code.

## 5. Repo scaffolding checklist (day one)

- [x] GPL-3.0 `LICENSE` at root; `legal/` committed **publicly** (the in-app legal gate links to these GitHub URLs — CommandForge lesson: don't leave them gitignored).
- [x] `.github/ISSUE_TEMPLATE/bug_report.yml` (shipped in this suite) + `config.yml` disabling blank issues.
- [x] `.github/dependabot.yml` covering cargo, npm, github-actions and the Code Health pins.
- [x] `rust-toolchain.toml`, `.nvmrc` (24), `Cargo.lock`/`package-lock.json` committed.
- [x] **Repo settings (owner, via the GitHub UI or `gh` — not committable):** branch protection on `main`; disable Advanced Security → CodeQL *Default setup* (the reusable is advanced-setup and is rejected at upload time otherwise); enable Discussions for the issue-template contact links.
- [x] **Secrets via `gh secret set`:** the Discord webhooks (`DISCORD_RELEASES_WEBHOOK`, `DISCORD_REPO_WEBHOOK`, `DISCORD_CI_WEBHOOK`). No signing key — see §3.
- [x] App icon settled (`src-tauri/icons/`). It was called a placeholder because nobody had chosen it; the owner chose it in Phase 4, which is what "a brand decision" needed.
- [x] README badges: CI, CodeQL, release, license.

**Closed 2026-08-14, and three of the last four were already true.** CodeQL *Default setup* read
`not-configured` and Discussions were already on — both had been done and never ticked, which is
its own small lesson about a checklist nobody reads back. The Discord secrets were set the same
day. Branch protection was the only genuinely outstanding item; §2's exact settings are below.
