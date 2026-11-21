# PipDock — Release & CI

*Version 0.3 · 2026-11-21 · Every workflow is project-local. Until the repository moved to the poli0981org organization (2026-11-21) four of them were callers into the owner's personal ops repo, `poli0981/.github`, at `@main`; §1 says what replaced each and why.*

## 1. Workflows (`.github/workflows/`)

| Workflow | Triggers | Jobs |
|---|---|---|
| `ci-rust.yml` | PR, push main | `fmt · clippy · test` (`windows-latest`): `cargo fmt --check`, clippy `-D warnings`, `cargo test --workspace --locked`. `cargo audit` (`ubuntu-latest`): a prebuilt `cargo-audit`, `--deny warnings`. `cargo deny` (`ubuntu-latest`): licences, sources, bans (`deny.toml`) |
| `ci-node.yml` | PR, push main | `lint · typecheck · test · build` (Node 26, `npm ci`, ESLint, `tsc`, Vitest, `vite build`); `npm audit --audit-level=high` (after `npm ci --ignore-scripts`); `npm licences` (`scripts/check-licenses.mjs` and its `node --test` suite — §2); `tauri lockstep` (`scripts/check-tauri-lockstep.mjs`) |
| `ci-integration.yml` | PR touching what its end-to-end steps run through (the path list is in the file), nightly, Mondays, dispatch | Windows runner, Python 3.10 and 3.14: the disposable-venv suite (TESTING L2); Mondays and dispatch only, the fixture-drift re-capture against the newest pip and uv |
| `codeql.yml` | PR, push main, weekly | `Analyze (<language>)` for `javascript-typescript`, `rust` and `actions`, all `build-mode: none` |
| `qodana.yml` | PR, push main, dispatch | `tokens present` → `qodana · rust` (Docker, `qodana-rust.yaml`) and `qodana · ts/js` (native, `qodana-js.yaml`), one Qodana Cloud project and token each; skipped without a token, like `sonar.yml` |
| `security-scan.yml` | daily, dispatch | `cargo audit` and `npm audit` against `main` (SECURITY §7) |
| `sonar.yml` | PR, push main, dispatch (`main` only) | `token present` (`ubuntu-latest`) → `coverage · analysis` (`windows-latest`): vitest and `cargo llvm-cov` LCOV, clippy's JSON, then `sonarqube-scan-action` with `sonar.projectVersion` from `package.json` and `sonar.qualitygate.wait`. Skipped when the run has no `SONAR_TOKEN` (Dependabot and fork PRs), and for a dispatch from any branch but `main` — the plan analyses `main` and PRs only. `sonar-project.properties` holds the scope and the project, `poli0981org_pipdock` since 2026-11-21 |
| `release.yml` | tag `v*`; dispatch (`tag`, `dry_run`) | `Build · checksum` (`windows-latest`, read-only) → `Attest · draft` (`ubuntu-latest`); see §3 |
| `notify.yml` | `workflow_run` of the workflows above, **by name**; release published; dispatch (`tag`, to announce again) | `ci-failure` → `DISCORD_CI_WEBHOOK`; `announce` → `DISCORD_RELEASES_WEBHOOK` and `DISCORD_REPO_WEBHOOK`, both through `.github/scripts/discord_notify.py` (standard library; `DRY_RUN=1` prints instead of posting) |

**`notify.yml` names its workflows, and a new workflow is added to it by its exact `name:`.** `workflows: ['*']` — the ops repo's old caller example — is not a wildcard to `workflow_run`: it matched nothing, so `main` stayed red from 2026-09-23 with every failure unreported. It reports a failure from any event but a pull request, filtering on the event alone — never on a branch name, which a fork controls. CodeQL, Qodana and Release are listed: the ops-repo notifier skipped anything named like CodeQL or Release because of "dedicated notifiers" PipDock never had, so a failed Release run reached no channel at all.

**Why nothing calls the ops repo any more (2026-11-21).** The repository moved to the poli0981org organization so that the projects the owner maintains seriously sit apart from the ones made for fun, and a project in the organization should not run code another account can change. The four callers were exactly that: `@main` of `poli0981/.github`, unpinned, and its notifier checked out that repository's `main` and ran *its* Python script with PipDock's Discord webhooks in the environment. Each was small enough to copy, as OmniDeck — the organization's other project — had already done:

- `reusable-web-react.yml` became `ci-node.yml`'s `lint · typecheck · test · build` job. It took two things with it: the `pages: write` and `id-token: write` grants its always-skipped Pages job forced on the caller (permissions are validated at startup regardless of `if:`), and the `dist` artifact nothing downloaded. Its missing `working-directory` input was why `package.json` sits at the repository root; the layout stays, because SonarCloud, Qodana, the scripts and Tauri's configuration all assume it now (ARCHITECTURE §2).
- `reusable-codeql.yml` became `codeql.yml`'s matrix job, with `codeql-action` pinned by commit where the reusable used `@v4`, and the same `category` values, so the alerts keep their history. Rust and Actions run as they did through the reusable, which handed `languages` straight to the action.
- `notify-ci-failure.yml` and `announce-release.yml` became `notify.yml`, with the script vendored from OmniDeck's self-contained notifier.

**Every action is pinned by commit**, with its version in a comment for Dependabot to update — GitHub's own `actions/*` included, which had used major tags. A write permission sits on the job that needs it, never the workflow (SonarCloud S8233):

```yaml
# every workflow
permissions: { contents: read }
# codeql.yml — analyze
permissions: { actions: read, contents: read, packages: read, security-events: write }
# qodana.yml — rust, js
permissions: { contents: read, checks: write, pull-requests: write, security-events: write }
# release.yml — publish (§3)
permissions: { contents: write, id-token: write, attestations: write }
```

## 2. Branch & version policy

`main` protected (PR + green CI required). SemVer; tags `vX.Y.Z`, and `release.yml` refuses a tag that does not match all five version fields (§3). Commit subjects follow Conventional Commits, but nothing generates a changelog from them — this line said a generator did. `changelog/` is written by hand, for a user, under the rules in `changelog/README.md`; a draft release's notes are GitHub's generated list of merged pull requests.

**Protection was applied 2026-08-14, and had been false until then** — `/branches/main/protection` returned 404 for four milestones, which is also why `gh pr merge --auto` merged immediately on this repo rather than waiting: with no required checks there is nothing for auto-merge to wait on. Exact settings:

| Setting | Value | Why |
|---|---|---|
| Required checks | `fmt · clippy · test`, `cargo audit`, `cargo deny`, `lint · typecheck · test · build`, `npm audit`, `npm licences`, `tauri lockstep`, `Analyze (javascript-typescript)` | eight, all running on **every** PR (below) |
| Check app | pinned to `app_id: 15368` (`github-actions`) | a check from another app cannot satisfy a requirement |
| `enforce_admins` | `true` | the owner is the only contributor; protection that exempts them protects nothing |
| Required approvals | **0** | a solo owner cannot approve their own PR, so 1 would deadlock. A PR is still required |
| `strict` | `false` | up-to-date-before-merge is friction on a repo that rebase-merges |
| Linear history | `true` | matches rebase-merge |
| Conversation resolution | `true` | a review thread left open blocks the merge — read from the API on 2026-11-10; this table did not list it |
| Force pushes / deletions | `false` | |

**`CI / Integration` is deliberately excluded, and this is the trap worth remembering:** its `pull_request` trigger is `paths:`-filtered, so on a docs-only or UI-only PR it never starts — and a required check that never starts stays *pending forever*. Requiring it would deadlock exactly the PRs it has no opinion about. `Re-capture against the latest engines` runs on no PR at all (Mondays and dispatch only) and is excluded for the same reason. The bare `CodeQL` check belongs to the `github-advanced-security` app rather than Actions and is left out too.

**The payload has had two shapes.** The first, set on 2026-08-14, held five checks — `fmt · clippy · test`, `cargo audit`, `lint · typecheck · test · build / build`, `npm audit` and `analyze / Analyze (javascript-typescript)` — and two of those names were the ops repo's: a reusable workflow's check is `<caller job> / <callee job>`, so a rename upstream would have deadlocked every PipDock PR with no failing check to point at. The second, set on 2026-11-21 when the callers became local jobs, renames those two to `lint · typecheck · test · build` and `Analyze (javascript-typescript)` and adds three that had run on every PR since 1.4.0 as advice rather than gates: `cargo deny`, `npm licences` (new that day) and `tauri lockstep`, whose whole purpose is to stop one half of a Tauri bump merging alone. **A required name is now this repository's alone, so renaming a job means changing this payload in the same change.**

**Still not required, and why.** CodeQL's `Analyze (rust)` and `Analyze (actions)` run on every PR and stay advice. `Security scan` runs on no PR and can never be required. SonarQube Cloud is not required in either form: `SonarCloud Code Analysis` is reported by the SonarCloud app, which the payload's `app_id` pin would not accept, and `sonar.yml`'s `coverage · analysis` is *skipped* on every PR without a token — Dependabot's — and a skipped job satisfies a requirement, so requiring it would gate exactly the PRs it never looks at. Qodana is not required for the same reason, and because one of its two linters is a JetBrains early-access build behind an external service.

**If protection wedges**, `PUT` the same endpoint with `enforce_admins: false`, or `DELETE /branches/main/protection`. Worth knowing before it is needed rather than discovering it during an incident.

Dependency updates use **Dependabot** (`.github/dependabot.yml`) — the standard of the owner's ops repo when the CI was set up, and of the poli0981org organization now; nothing here uses Renovate. **Five ecosystems**, weekly on Mondays: `cargo`, `npm`, `github-actions`, `pip` at `/tools` (the Code Health pins) and, since 1.4.0, `rust-toolchain` — the `rust-toolchain.toml` channel, in a PR of its own because a toolchain brings new clippy lints. Minor and patch updates are grouped per ecosystem; security advisories arrive as **immediate, ungrouped** PRs (Dependabot's default for security updates), and the `pip` ecosystem stays its own group so Code Health pin bumps are reviewable in isolation — a bump there also needs `tools/tools-constraints.txt` regenerated (the command is in its header), and `cargo test` fails until it is (CODE-HEALTH-SPEC §2). Watching `pip`/`uv` releases to trigger the parser-fixture job is the scheduled job in `ci-integration.yml`, not the dependency bot. Since 1.4.0:

- **Cooldowns.** A release must be 3 days old before cargo or npm proposes it (14 for a major), and 7 for Actions and the tool pins. Most yanked or compromised releases are pulled inside that window, and nothing here needs a dependency the day it ships.
- **Tauri moves in lockstep.** The `tauri` crate and `@tauri-apps/api` must share a minor or the app refuses to start, and Dependabot bumps Cargo and npm in separate PRs, each green on its own. So each ecosystem groups its Tauri packages (`tauri`, `tauri-*`; `@tauri-apps/*`) apart from its catch-all group — by `exclude-patterns` on the catch-all, since 2026-11-21: Dependabot gives a dependency to the *most specific* matching group, it counts a group with `update-types` as more specific than one with patterns alone, and so the order this bullet used to rely on did nothing (that week's Tauri bumps landed in #116 and #118, the catch-alls) — and `ci-node.yml`'s `tauri lockstep` job fails either PR until its counterpart lands beside it, comparing the core and each plugin with its npm package on `Cargo.lock` and `package-lock.json` — what is built, not what the manifests ask for. **A multi-ecosystem group was considered and not used:** it would put both halves in one PR, but what it does to the dependencies outside its patterns is undocumented, and a wrong guess would silently stop every other update.
- **TypeScript is held below 6.1** (`ignore: typescript >=6.1.0`): typescript-eslint 8.71 peers `typescript >=4.8.4 <6.1.0`, so a 6.1 bump fails `npm ci` with ERESOLVE before a single check runs. Remove it once typescript-eslint widens its range (8.73 still peers `<6.1.0`, re-read 2026-11-21, with TypeScript at 7.0).
- **rfd is held below 0.18** (`ignore: rfd >=0.18.0`, cargo): src-tauri takes rfd only because tauri-plugin-dialog compiles it, and 2.8.4, the newest, still requires `^0.17` — so #112's rfd 0.18 alone would have put a second rfd in the binary. Remove it when the plugin moves, and bump both together.
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
2. **`Attest · draft`** — Ubuntu runner, the only job granted `contents`, `id-token` and `attestations` write. It downloads the build, attests build provenance for the `.exe` and the `.msi` (`actions/attest-build-provenance`, verifiable with `gh attestation verify <file> --repo poli0981org/pipdock`; an attestation stays with the account that made it, so 1.4.0 and 1.5.0, built before the move to the organization, verify with `--owner poli0981 --signer-repo poli0981/pipdock` and no longer with any `--repo`), and drafts the GitHub release with GitHub's generated notes and all three files (`softprops/action-gh-release`). Both actions are pinned by commit.
3. **Publishing the draft is a human decision**, taken per release; publishing fires `notify.yml`'s Discord announcement. For 1.4.0 the owner approved the plan and instructed the assistant to check the installers against `SHA256SUMS.txt` and then publish — `docs/AI-DISCLOSURE.md` records who did what.

**A dry run** — `workflow_dispatch` with `dry_run` — builds and checksums any ref, a branch before it merges included, and attests and drafts nothing, so the release recipe is tested before a tag depends on it. Without `dry_run`, dispatch builds a version tag only (the `tag` input re-runs one); a branch is refused.

This list had a fourth step — a post-release checklist issue, opened automatically, for SmartScreen behaviour and an EN/VI spot-check of the shipped build. No workflow has ever opened one. Those checks belong to the per-release manual charter (TESTING §4).

There is no signing step. **PipDock does not update itself** (SECURITY §5), so no `latest.json` is produced, no artifact is code-signed, and no keypair exists to manage. The attestation is signed through Sigstore with a short-lived certificate bound to the workflow run — which is why the publish job needs `id-token: write` — so it adds no key either.

Secrets bootstrap: set per repository with `gh secret set`, none at the organization — the Discord webhooks `notify.yml` posts to (`DISCORD_RELEASES_WEBHOOK`, `DISCORD_REPO_WEBHOOK`, `DISCORD_CI_WEBHOOK`), `SONAR_TOKEN` for `sonar.yml` (re-issued for the `poli0981org` SonarQube Cloud organization on 2026-11-21), and `QODANA_TOKEN_RUST` and `QODANA_TOKEN_TS_JS` for the two Qodana Cloud projects `qodana.yml` reports to. That is the whole list; a transfer between owners keeps a repository's secrets, and it kept these.

## 4. Artifact & support policy

Two active lines at most: latest release gets features; previous minor gets security-only patches for 90 days. Users move between versions by downloading from the Releases page — there is no update channel to target. MSI provided for org deployment; winget manifest submission is P2 (tracked in ROADMAP), and would be the natural place to add an upgrade path that is not PipDock's own code.

## 5. Repo scaffolding checklist (day one)

- [x] GPL-3.0 `LICENSE` at root; `legal/` committed **publicly** (the in-app legal gate links to these GitHub URLs — CommandForge lesson: don't leave them gitignored).
- [x] `.github/ISSUE_TEMPLATE/bug_report.yml` (shipped in this suite) + `config.yml` disabling blank issues.
- [x] `.github/dependabot.yml` covering cargo, npm, github-actions and the Code Health pins — and the Rust toolchain since 1.4.0.
- [x] `rust-toolchain.toml`, `.nvmrc` (26 — it said 24 until 1.5.0, a month after every workflow had moved to 26), `Cargo.lock`/`package-lock.json` committed.
- [x] **Repo settings (owner, via the GitHub UI or `gh` — not committable):** branch protection on `main`; disable Advanced Security → CodeQL *Default setup* (`codeql.yml` is an advanced setup, and is rejected at upload time otherwise); enable Discussions for the issue-template contact links.
- [x] **Secrets via `gh secret set`:** the Discord webhooks (`DISCORD_RELEASES_WEBHOOK`, `DISCORD_REPO_WEBHOOK`, `DISCORD_CI_WEBHOOK`), `SONAR_TOKEN`, `QODANA_TOKEN_RUST`, `QODANA_TOKEN_TS_JS` (§3). No signing key — see §3.
- [x] App icon settled (`src-tauri/icons/`). It was called a placeholder because nobody had chosen it; the owner chose it in Phase 4, which is what "a brand decision" needed.
- [x] README badges: CI, CodeQL, release, license.
- [x] **Labels (owner, repo settings — opened by 1.4.0):** `dependencies`, `rust`, `frontend`, `ci` and `code-health` for Dependabot (§2), and `triage` for `bug_report.yml`. None existed on 2026-10-17; all six did by 2026-11-10, when `accessibility` joined them for `accessibility.yml`.
- [x] **Community health files (1.5.0):** `ACCESSIBILITY.md`, `CONTRIBUTING.md` and `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1) at the root; `.github/PULL_REQUEST_TEMPLATE.md` and `.github/ISSUE_TEMPLATE/accessibility.yml`; and a repository description. The ops repo's description says it carries community health files and it carries none, so nothing was inherited. `bug_report.yml` keeps its name: `crates/pipdock-core/src/report.rs` builds the in-app *Report bug* link from it.

**Closed 2026-08-14, and three of the last four were already true.** CodeQL *Default setup* read
`not-configured` and Discussions were already on — both had been done and never ticked, which is
its own small lesson about a checklist nobody reads back. The Discord secrets were set the same
day. Branch protection was the only genuinely outstanding item; §2's exact settings are above.
