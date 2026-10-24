# PipDock — Release & CI

*Version 0.2 · 2026-07-27 · CI reuses the `poli0981/.github` ops repo where a suitable reusable workflow exists, and is project-local where one does not. The ops repo's suite was audited on 2026-07-27, and its CodeQL and notifier reusables re-read on 2026-10-17 (see §1 note) — do not assume a reusable exists without checking.*

## 1. Workflows (`.github/workflows/`)

| Workflow | Kind | Triggers | Jobs |
|---|---|---|---|
| `ci-rust.yml` | **project-local** | PR, push main | `fmt · clippy · test` (`windows-latest`): `cargo fmt --check`, clippy `-D warnings`, `cargo test --workspace --locked`. `cargo audit` (`ubuntu-latest`): a prebuilt `cargo-audit`, `--deny warnings`. `cargo deny` (`ubuntu-latest`): licences, sources, bans (`deny.toml`) |
| `ci-node.yml` | caller → ops `reusable-web-react.yml` + two local jobs | PR, push main | Node 24, `npm ci`, lint / typecheck / Vitest (each runs only if the npm script exists), `vite build`; local jobs `npm audit --audit-level=high` (after `npm ci --ignore-scripts`) and `tauri lockstep` (`scripts/check-tauri-lockstep.mjs`) |
| `ci-integration.yml` | project-local | PR touching what its end-to-end steps run through (the path list is in the file), nightly, Mondays, dispatch | Windows runner, Python 3.10 and 3.14: the disposable-venv suite (TESTING L2); Mondays and dispatch only, the fixture-drift re-capture against the newest pip and uv |
| `codeql.yml` | caller → ops `reusable-codeql.yml` | PR, push main, weekly | `languages: '["javascript-typescript", "rust", "actions"]'` |
| `security-scan.yml` | **project-local** | daily, dispatch | `cargo audit` and `npm audit` against `main` (SECURITY §7) |
| `sonar.yml` | **project-local** | PR, push main, dispatch (`main` only) | `token present` (`ubuntu-latest`) → `coverage · analysis` (`windows-latest`): vitest and `cargo llvm-cov` LCOV, clippy's JSON, then `sonarqube-scan-action` with `sonar.projectVersion` from `package.json` and `sonar.qualitygate.wait`. Skipped when the run has no `SONAR_TOKEN` (Dependabot and fork PRs), and for a dispatch from any branch but `main` — the plan analyses `main` and PRs only. `sonar-project.properties` holds the scope |
| `release.yml` | **project-local** | tag `v*`; dispatch (`tag`, `dry_run`) | `Build · checksum` (`windows-latest`, read-only) → `Attest · draft` (`ubuntu-latest`); see §3 |
| `announce-release.yml` | caller → ops `announce-release.yml` | release published; dispatch (re-runs) | Discord release announcement |
| `notify-ci-failure.yml` | caller → ops `notify-ci-failure.yml` | `workflow_run` completed for `CI / Rust`, `CI / Node`, `CI / Integration`, `Security scan`, `SonarCloud` — **by name** | Discord CI-failure notification |

**`notify-ci-failure.yml` names its workflows, and a new workflow is added to it by its exact `name:`.** `workflows: ['*']` — the ops repo's own caller example — is not a wildcard to `workflow_run`: it matched nothing, so `main` stayed red from 2026-09-23 with every failure unreported. The reusable reports only failures on the default branch, and skips by name any workflow that has a dedicated notifier (CodeQL, Dependabot, deploy, Pages, release), which is why `CodeQL` and `Release` are not listed: listing them would only create skipped runs. A failed `Release` run therefore reaches no Discord channel; whoever pushed the tag has to watch it.

**Ops-repo audit (2026-07-27; CodeQL and the notifier re-read 2026-10-17) — why the table looks like this:**

- The reusable suite covers assembly, c-cpp, chrome-extension, desktop-csharp, go, jvm, macos-swift, php, python, r, ruby and web-react. **There is no Rust reusable and no Tauri reusable**, and only `reusable-desktop-csharp.yml` uses a Windows runner — hence `ci-rust.yml`, `release.yml` and `security-scan.yml` are project-local. There is no SonarQube reusable either (re-checked 2026-10-25), so `sonar.yml` is project-local too.
- `reusable-web-react.yml` has **no `working-directory` input** (it runs `npm ci` at the repo root — hence `package.json` lives at the root with Vite's `root: './ui'`, see ARCHITECTURE §2) and **no `npm audit` step**, hence the extra local job.
- `reusable-codeql.yml` *documents* csharp, python, javascript-typescript, ruby, c-cpp, java-kotlin, go and swift, but hands `languages` straight to `codeql-action`, with `build-mode: none` for anything outside its autobuild list — so `rust` and `actions` run through it unchanged. This bullet said **CodeQL has no Rust analyzer**, so Rust's coverage came from clippy and `cargo audit` alone; the premise stopped being true in 2025, and `codeql.yml` now asks for both. Only the JavaScript/TypeScript analysis is a required check (§2).
- Portfolio notifications are **Discord-only**, driven by the ops repo's `.github/scripts/discord_notify.py`. There is no multi-platform `notify.py`.

**Permissions matrix (explicit in every caller — the Phase-5 lesson: callers without a `permissions:` block default everything to `none`):**

```yaml
# codeql.yml
permissions: { actions: read, contents: read, packages: read, security-events: write }
# announce-release.yml / notify-ci-failure.yml
permissions: { contents: read, actions: read }
# ci-node.yml — the workflow, and the one job that calls the reusable
permissions: { contents: read }
#   web:     { contents: read, pages: write, id-token: write }
# release.yml — the workflow, then each job (§3)
permissions: { contents: read }
#   build:   { contents: read }
#   publish: { contents: write, id-token: write, attestations: write }
# ci-rust.yml / ci-integration.yml / security-scan.yml / sonar.yml
permissions: { contents: read }
```

**The lesson has a converse: a caller must grant at least what the callee's jobs declare, even for a job that will be skipped**, because permissions are validated at startup regardless of `if:`. `reusable-codeql.yml`'s analyze job asks for `packages: read`, and a caller without it got a `startup_failure` with no logs to explain it; `reusable-web-react.yml` declares a Pages-deploy job needing `pages: write` and `id-token: write`, skipped here by `deploy-pages: false` and validated all the same. A write is granted on the job that needs it, never the workflow (SonarCloud S8233), so no other job holds one. This block showed neither of those grants and gave `release.yml` `contents: write` across the board; since 1.4.0 the release's write lives on its publish job alone.

## 2. Branch & version policy

`main` protected (PR + green CI required). SemVer; tags `vX.Y.Z`, and `release.yml` refuses a tag that does not match all five version fields (§3). Commit subjects follow Conventional Commits, but nothing generates a changelog from them — this line said a generator did. `changelog/` is written by hand, for a user, under the rules in `changelog/README.md`; a draft release's notes are GitHub's generated list of merged pull requests.

**Protection was applied 2026-08-14, and had been false until then** — `/branches/main/protection` returned 404 for four milestones, which is also why `gh pr merge --auto` merged immediately on this repo rather than waiting: with no required checks there is nothing for auto-merge to wait on. Exact settings:

| Setting | Value | Why |
|---|---|---|
| Required checks | `fmt · clippy · test`, `cargo audit`, `lint · typecheck · test · build / build`, `npm audit`, `analyze / Analyze (javascript-typescript)` | the five that ran on **every** PR when the payload was set; four more do now (below) |
| Check app | pinned to `app_id: 15368` (`github-actions`) | a check from another app cannot satisfy a requirement |
| `enforce_admins` | `true` | the owner is the only contributor; protection that exempts them protects nothing |
| Required approvals | **0** | a solo owner cannot approve their own PR, so 1 would deadlock. A PR is still required |
| `strict` | `false` | up-to-date-before-merge is friction on a repo that rebase-merges |
| Linear history | `true` | matches rebase-merge |
| Force pushes / deletions | `false` | |

**`CI / Integration` is deliberately excluded, and this is the trap worth remembering:** its `pull_request` trigger is `paths:`-filtered, so on a docs-only or UI-only PR it never starts — and a required check that never starts stays *pending forever*. Requiring it would deadlock exactly the PRs it has no opinion about. `Re-capture against the latest engines` runs on no PR at all (Mondays and dispatch only) and is excluded for the same reason. The bare `CodeQL` check belongs to the `github-advanced-security` app rather than Actions and is left out too.

**Four more run on every PR and are not in the payload** — `cargo deny`, `tauri lockstep`, and CodeQL's `analyze / Analyze (rust)` and `analyze / Analyze (actions)` — all added in 1.4.0, after the five above were set (re-read 2026-10-17; the payload is unchanged). Until they are added, a red one is advice rather than a gate, which matters most for `tauri lockstep`: its whole purpose is to stop one half of a Tauri bump merging alone. `Security scan` runs on no PR and can never be required. SonarQube Cloud is not required either, in either form: `SonarCloud Code Analysis` is reported by the SonarCloud app, which the payload's `app_id` pin would not accept, and `sonar.yml`'s `coverage · analysis` is *skipped* on every PR without a token — Dependabot's — and a skipped job satisfies a requirement, so requiring it would gate exactly the PRs it never looks at.

**Two required contexts are named by the ops repo, not by this one.** `analyze / Analyze (javascript-typescript)` and `lint · typecheck · test · build / build` come from `poli0981/.github`'s reusables. If either job is renamed upstream, every PipDock PR deadlocks with no obvious cause and no failing check to point at — the fix is to update the protection payload, not the code.

**If protection wedges**, `PUT` the same endpoint with `enforce_admins: false`, or `DELETE /branches/main/protection`. Worth knowing before it is needed rather than discovering it during an incident.

Dependency updates use **Dependabot** (`.github/dependabot.yml`) — the portfolio standard, as evidenced by the ops repo's `dependabot-summary.yml`; there is no Renovate configuration anywhere in the ops repo. **Five ecosystems**, weekly on Mondays: `cargo`, `npm`, `github-actions`, `pip` at `/tools` (the Code Health pins) and, since 1.4.0, `rust-toolchain` — the `rust-toolchain.toml` channel, in a PR of its own because a toolchain brings new clippy lints. Minor and patch updates are grouped per ecosystem; security advisories arrive as **immediate, ungrouped** PRs (Dependabot's default for security updates), and the `pip` ecosystem stays its own group so Code Health pin bumps are reviewable in isolation — a bump there also needs `tools/tools-constraints.txt` regenerated (the command is in its header), and `cargo test` fails until it is (CODE-HEALTH-SPEC §2). Watching `pip`/`uv` releases to trigger the parser-fixture job is the scheduled job in `ci-integration.yml`, not the dependency bot. Since 1.4.0:

- **Cooldowns.** A release must be 3 days old before cargo or npm proposes it (14 for a major), and 7 for Actions and the tool pins. Most yanked or compromised releases are pulled inside that window, and nothing here needs a dependency the day it ships.
- **Tauri moves in lockstep.** The `tauri` crate and `@tauri-apps/api` must share a minor or the app refuses to start, and Dependabot bumps Cargo and npm in separate PRs, each green on its own. So each ecosystem groups its Tauri packages (`tauri`, `tauri-*`; `@tauri-apps/*`) ahead of its catch-all group, and `ci-node.yml`'s `tauri lockstep` job fails either PR until its counterpart lands beside it, comparing the core and each plugin with its npm package on `Cargo.lock` and `package-lock.json` — what is built, not what the manifests ask for. **A multi-ecosystem group was considered and not used:** it would put both halves in one PR, but what it does to the dependencies outside its patterns is undocumented, and a wrong guess would silently stop every other update.
- **TypeScript is held below 6.1** (`ignore: typescript >=6.1.0`): typescript-eslint 8.71 peers `typescript >=4.8.4 <6.1.0`, so a 6.1 bump fails `npm ci` with ERESOLVE before a single check runs. Remove it once typescript-eslint widens its range (8.72 still peers `<6.1.0`, re-read 2026-10-25, with TypeScript at 7.0).
- **Vitest moves as one** (group `vitest`: `vitest`, `@vitest/*`, every update type). `@vitest/coverage-v8`, which `sonar.yml` needs, peers `vitest` at its exact version, so a bump of either alone fails `npm ci` the way the TypeScript one would.
- **Labels.** PRs are labelled `dependencies` plus `rust`, `frontend`, `ci` or `code-health`, and the labels must exist in the repository or Dependabot opens its PRs without them (§5).

## 3. Release pipeline (tag `v*`)

Two jobs, so the token that can write a release never runs the build (SECURITY §5).

1. **`Build · checksum`** — Windows runner, a read-only token, and no credentials kept in the checkout (`persist-credentials: false`).
   - **The tag and all five version fields must agree** — `Cargo.toml`, `Cargo.lock` (`pipdock-core`'s entry), `package.json`, `package-lock.json` and `tauri.conf.json` — or the job stops before building. A tag pushed ahead of its bump commit, or a bump that missed a file, would otherwise build an installer whose About states another version.
   - `npm ci` → **`npm run bundle`**, producing NSIS `.exe` + `.msi`. That one script owns the whole recipe so the workflow cannot drift from what a developer runs locally, and it encodes two things that are easy to get wrong:
     - **`--locked` goes after `--`.** It is cargo's flag; `tauri build` rejects it as its own with *"unexpected argument"* before a single crate compiles.
     - **The CLI sidecar is declared in an overlay** (`src-tauri/tauri.bundle.conf.json`, merged with `--config`), not in `tauri.conf.json`. `tauri-build` validates `externalBin` at build-script time, so declaring it in the base config makes plain `cargo clippy` and `cargo test` fail in a fresh clone with *"resource path … doesn't exist"* — a bundling concern leaking into every compile.

     The frontend is built by tauri's `beforeBuildCommand`, so the workflow does not run `vite build` itself.
   - **`SHA256SUMS.txt`**, LF line endings, written byte for byte: `Set-Content` wrote CRLF, and `sha256sum -c` on any other platform reads the CR as part of the file name and reports the file missing.
   - The installers and the checksums are uploaded as the `release-artifacts` workflow artifact.
2. **`Attest · draft`** — Ubuntu runner, the only job granted `contents`, `id-token` and `attestations` write. It downloads the build, attests build provenance for the `.exe` and the `.msi` (`actions/attest-build-provenance`, verifiable with `gh attestation verify <file> --repo poli0981/pipdock`), and drafts the GitHub release with GitHub's generated notes and all three files (`softprops/action-gh-release`). Both actions are pinned by commit.
3. **Publishing the draft is a human decision**, taken per release; publishing fires `announce-release.yml`'s Discord announcement. For 1.4.0 the owner approved the plan and instructed the assistant to check the installers against `SHA256SUMS.txt` and then publish — `docs/AI-DISCLOSURE.md` records who did what.

**A dry run** — `workflow_dispatch` with `dry_run` — builds and checksums any ref, a branch before it merges included, and attests and drafts nothing, so the release recipe is tested before a tag depends on it. Without `dry_run`, dispatch builds a version tag only (the `tag` input re-runs one); a branch is refused.

This list had a fourth step — a post-release checklist issue, opened automatically, for SmartScreen behaviour and an EN/VI spot-check of the shipped build. No workflow has ever opened one. Those checks belong to the per-release manual charter (TESTING §4).

There is no signing step. **PipDock does not update itself** (SECURITY §5), so no `latest.json` is produced, no artifact is code-signed, and no keypair exists to manage. The attestation is signed through Sigstore with a short-lived certificate bound to the workflow run — which is why the publish job needs `id-token: write` — so it adds no key either.

Secrets bootstrap: set per-repo with `gh secret set` (the ops repo has no `setup-secrets.sh`) — the Discord webhooks the ops-repo notifiers expect (`DISCORD_RELEASES_WEBHOOK`, `DISCORD_REPO_WEBHOOK`, `DISCORD_CI_WEBHOOK`; the callers pass `secrets: inherit`). That is the whole list.

## 4. Artifact & support policy

Two active lines at most: latest release gets features; previous minor gets security-only patches for 90 days. Users move between versions by downloading from the Releases page — there is no update channel to target. MSI provided for org deployment; winget manifest submission is P2 (tracked in ROADMAP), and would be the natural place to add an upgrade path that is not PipDock's own code.

## 5. Repo scaffolding checklist (day one)

- [x] GPL-3.0 `LICENSE` at root; `legal/` committed **publicly** (the in-app legal gate links to these GitHub URLs — CommandForge lesson: don't leave them gitignored).
- [x] `.github/ISSUE_TEMPLATE/bug_report.yml` (shipped in this suite) + `config.yml` disabling blank issues.
- [x] `.github/dependabot.yml` covering cargo, npm, github-actions and the Code Health pins — and the Rust toolchain since 1.4.0.
- [x] `rust-toolchain.toml`, `.nvmrc` (24), `Cargo.lock`/`package-lock.json` committed.
- [x] **Repo settings (owner, via the GitHub UI or `gh` — not committable):** branch protection on `main`; disable Advanced Security → CodeQL *Default setup* (the reusable is advanced-setup and is rejected at upload time otherwise); enable Discussions for the issue-template contact links.
- [x] **Secrets via `gh secret set`:** the Discord webhooks (`DISCORD_RELEASES_WEBHOOK`, `DISCORD_REPO_WEBHOOK`, `DISCORD_CI_WEBHOOK`). No signing key — see §3.
- [x] App icon settled (`src-tauri/icons/`). It was called a placeholder because nobody had chosen it; the owner chose it in Phase 4, which is what "a brand decision" needed.
- [x] README badges: CI, CodeQL, release, license.
- [ ] **Labels (owner, repo settings — opened by 1.4.0):** `dependencies`, `rust`, `frontend`, `ci` and `code-health` for Dependabot (§2), and `triage` for `bug_report.yml`. None existed on 2026-10-17, so Dependabot's PRs arrive unlabelled and a bug report gets `bug` alone.

**Closed 2026-08-14, and three of the last four were already true.** CodeQL *Default setup* read
`not-configured` and Discussions were already on — both had been done and never ticked, which is
its own small lesson about a checklist nobody reads back. The Discord secrets were set the same
day. Branch protection was the only genuinely outstanding item; §2's exact settings are above.
