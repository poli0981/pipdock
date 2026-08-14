# AI disclosure

*Last updated 2026-08-14, for PipDock 1.0.0.*

**PipDock was written with substantial help from an AI coding assistant, under human review.** This
page says which model, what it did, what it did not do, and how you can check any of it yourself.

It exists because the fact is not obvious from the outside and you should not have to infer it from
commit trailers. Nothing here changes the licence or the warranty position — see
[`legal/EULA.md`](../legal/EULA.md) for those.

## The model

| | |
|---|---|
| Model | **Claude Opus 5** |
| Vendor | **Anthropic** ([anthropic.com](https://www.anthropic.com)) |
| Interface | [Claude Code](https://claude.com/claude-code), Anthropic's CLI/desktop coding agent |
| Period | 2026-07-27 → 2026-08-14 (first commit to 1.0.0) |

Earlier work in the same repository used earlier Claude models; the trailer on each commit records
which one, and `git log` is the authoritative history.

## What it was used for

- **Source code** — Rust core, CLI, Tauri bridge, React frontend. Most of the implementation.
- **Tests** — unit, integration and the CLI golden snapshots.
- **Debugging** — reading failing CI logs, reproducing defects, and diagnosing them.
- **Translation** — the Vietnamese catalogue (`ui/src/locales/vi/`), reviewed by a native speaker
  who is also the project owner.
- **Documentation** — the specifications under `docs/`, this page included.

## What review meant in practice

Every change reached `main` through a pull request that the owner read and merged. Beyond reading
diffs, four things were load-bearing, and all four caught real defects the review-by-reading did
not:

1. **A green test suite was never sufficient.** Thirty-five defects reached `main` or nearly did,
   and *not one* was caught by the suite that was green at the time. They were found by running
   the thing — against a real interpreter, a real CI runner, a real browser, a real installer.
   `docs/ROADMAP.md` records each one against the stage it was found in.
2. **Specifications were treated as claims, not facts.** Several were wrong about the tools they
   described: all three Code Health tools exit non-zero on findings and vulture uses 3; deptry
   reports a *module* name rather than the distribution providing it; ruff's rule URL is keyed by
   rule name, not code. Each is now pinned by a test because a document was not enough.
3. **CI is a second reviewer that cannot be talked round.** `cargo clippy -D warnings`,
   `cargo audit`, `npm audit`, CodeQL, and integration tests against real pip and uv in disposable
   venvs, all blocking on every pull request. `main` is protected and the owner cannot bypass it.
4. **The owner drove the built application by hand** before 1.0.0 — on a real machine, in Windows
   Sandbox and in a Windows 10 VM — following `docs/TESTING.md` §4's charter. Several defects were
   found that way and only that way.

## What was not delegated

- **Product decisions.** Scope, the feature matrix, the non-goals, and what 1.0 would and would not
  ship are the owner's. So is the decision to remove the self-updater, which is why PipDock has no
  auto-update path (see [`SECURITY.md`](SECURITY.md) §5).
- **Merging, tagging and publishing.** Every release is a human action.
- **Anything requiring a legal or security judgment** was raised for a decision rather than taken —
  for example the PEP 668 override's default, and whether a `mailto:` scheme belonged in the
  browser-launch allowlist.

## Copyright and licence

Copyright in PipDock is held by **Kokone (poli0981)**, and the work is licensed **GPL-3.0-only**.
Using an AI assistant does not change either. Commits carry a
`Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>` trailer as an accurate record of how the
work was produced — it is attribution, not an assignment of rights, and Anthropic claims no
ownership of output produced with its models.

## Third-party code

PipDock's dependencies are other people's work under their own licences, listed in
[`legal/THIRD-PARTY-NOTICES.md`](../legal/THIRD-PARTY-NOTICES.md). Nothing in this repository was
copied from another project. Where an approach came from upstream documentation or a specification
— PEP 508, PEP 440, PEP 668, PEP 691, PEP 514 — the relevant document is cited in the code or in
`docs/`.

## Checking any of this

```bash
git log --format='%h %an %s'                      # every commit and its author
git log --format=%b | grep -c 'Co-Authored-By'    # how many carry the trailer
gh pr list --state merged                         # every change, and the review it went through
```

At the `v1.0.0` tag: **177 commits, 168 of them carrying the trailer.** The nine without are
Dependabot bumps and a handful of hand-written mechanical commits.

Those two figures are stated as of a tag rather than as of `main` on purpose. A count written into
a file changes the moment the file is committed — which caught this page out twice while it was
being written — so it is pinned to something that does not move, and the commands above are the
thing to trust.

## Questions

`code@poli0981.dev`, or open a discussion at
[github.com/poli0981/pipdock](https://github.com/poli0981/pipdock/discussions). If you think
something in this page is inaccurate, that is worth an issue.
