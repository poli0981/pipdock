# AI disclosure

*Last updated 2026-10-17, for PipDock 1.4.0.*

**PipDock was written with substantial help from an AI coding assistant, under human review.** This
page says which model, what it did, what it did not do, and how you can check any of it yourself.

It exists because the fact is not obvious from the outside and you should not have to infer it from
commit trailers. Nothing here changes the licence or the warranty position — see
[`legal/EULA.md`](../legal/EULA.md) for those.

## The model

| | |
|---|---|
| Model | **Claude Opus 5** — first commit through 1.3.0 · **Claude Opus 5.5** — from 1.4.0 |
| Vendor | **Anthropic** ([anthropic.com](https://www.anthropic.com)) |
| Interface | [Claude Code](https://claude.com/claude-code), Anthropic's CLI/desktop coding agent |
| Period | 2026-07-27 → 2026-08-21 (first commit to 1.3.0) · 2026-10-16 → 2026-10-17 (1.4.0) |

The trailer on each commit records which model produced it, and `git log` is the authoritative
history — the table above is a summary of it, not a substitute.

## What it was used for

- **Source code** — Rust core, CLI, Tauri bridge, React frontend. Most of the implementation.
- **Tests** — unit, integration and the CLI golden snapshots.
- **Debugging** — reading failing CI logs, reproducing defects, and diagnosing them.
- **Translation** — the Vietnamese catalogue (`ui/src/locales/vi/`), reviewed by a native speaker
  who is also the project owner.
- **Documentation** — the specifications under `docs/`, this page included.

## What review meant in practice

Every change reached `main` through a pull request. Through 1.3.0 the owner read and merged each
one. **1.4.0 was different, and is recorded as such:** it was an audit of the whole codebase, and
the owner reviewed and approved a written plan — scope, every decision the plan asked them to make,
and the instruction to carry it through to a release — and the assistant then committed, opened the
pull request, merged it once CI was green, tagged, and published, under that instruction. The pull
request, its checks and every commit are public. Beyond reading diffs, four things were
load-bearing, and all four caught real defects the review-by-reading did not:

1. **A green test suite was never sufficient.** Thirty-five defects reached `main` or nearly did,
   and *not one* was caught by the suite that was green at the time. They were found by running
   the thing — against a real interpreter, a real CI runner, a real browser, a real installer.
   `docs/ROADMAP.md` records each one against the stage it was found in.
2. **Specifications were treated as claims, not facts.** Several were wrong about the tools they
   described: all three Code Health tools exit non-zero on findings and vulture uses 3; deptry
   reports a *module* name rather than the distribution providing it; ruff's rule URL is keyed by
   rule name, not code. Each is now pinned by a test because a document was not enough.
3. **CI is a second reviewer that cannot be talked round.** Five checks block every pull
   request — formatting, clippy and the test suite; `cargo audit`; `npm audit`; lint, typecheck,
   tests and build of the frontend; and CodeQL for JavaScript/TypeScript — and `main` is protected
   so that the owner cannot bypass them. Integration tests against real pip and uv in disposable
   venvs run on every pull request that touches what they cover, and `cargo deny`, CodeQL for Rust
   and Actions, and a Tauri version check run on all of them without being required.
4. **The built application was driven by hand before release.** Before 1.0.0 the owner did it —
   on a real machine, in Windows Sandbox and in a Windows 10 VM — following `docs/TESTING.md` §4's
   charter, and several defects were found that way and only that way. For 1.4.0 the assistant
   drove the installed build itself, through screen control on the owner's machine; the 1.4.0
   record in `docs/ROADMAP.md` lists what was checked and what was not.

## What was not delegated

- **Product decisions.** Scope, the feature matrix, the non-goals, and what 1.0 would and would not
  ship are the owner's. So is the decision to remove the self-updater, which is why PipDock has no
  auto-update path (see [`SECURITY.md`](SECURITY.md) §5).
- **The decision to release.** Through 1.3.0 every merge, tag and publish was the owner's own
  action. For 1.4.0 the owner delegated the mechanics, in writing, with the conditions attached —
  CI green, and the installers matching `SHA256SUMS.txt` before the draft was published — and the
  decision that 1.4.0 should ship remained theirs.
- **Anything requiring a legal or security judgment** was raised for a decision rather than taken —
  for example the PEP 668 override's default, and whether a `mailto:` scheme belonged in the
  browser-launch allowlist.

## Copyright and licence

Copyright in PipDock is held by **Kokone (poli0981)**, and the work is licensed **GPL-3.0-only**.
Using an AI assistant does not change either. Commits carry a
`Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>` trailer through 1.3.0 and
`Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` from 1.4.0, as an accurate record of how
the work was produced — it is attribution, not an assignment of rights, and Anthropic claims no
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
