# Changelog

One file per **major line**. `1.x.md` holds every 1.y.z release; `0.x.md` would have held the
pre-1.0 ones had any been published (none were — the first release was 1.0.0, and `0.1.0` existed
only as a working version in the manifests).

| File | Covers |
|---|---|
| [`1.x.md`](1.x.md) | 1.0.0 → current |

A **pre-release line gets its own file**, named for the line it leads to — `2.0-beta.md` for
`2.0.0-beta.1` onward. It is not merged into `2.x.md` when the line goes stable: the beta file is
what someone reads to find out *when* a behaviour changed during the run-up, and folding it in
would double the length of the file people actually read while losing that.

## When a major line gets long, split by minor range

The point of this directory is that a reader opens one file and finds their version without
scrolling past releases they do not care about. So the rule is about **reading length**, not about
release count:

**Split when a file passes roughly 600 lines**, into contiguous minor ranges — `1.0-1.4.md`,
`1.5-1.9.md` — and leave `1.x.md` as a stub pointing at both, so an old link never 404s. Add the
new files to the table above in the same commit.

Two things that make the split cheap when it comes:

- **Never renumber an existing file.** A range file is named for the range it holds and keeps that
  name forever, even after the next split. `1.0-1.4.md` does not become `1.0-1.6.md`.
- **Newest first within a file**, so the top of the file is the part most people want and the
  split point is always at the bottom.

## What goes in an entry

One `## x.y.z — YYYY-MM-DD` heading per release, newest first, and under it only what a *user* of
PipDock would notice: added, changed, fixed, and anything they have to do (a consent gate that
re-shows, a cache that needs clearing). Internals belong in `docs/ROADMAP.md`, which records what a
slice cost and what it taught; the two are deliberately different documents for different readers,
the way `docs/CLI-GUIDE.md` and `docs/CLI-SPEC.md` are.

Link the release's PRs. Do not quote commit counts or defect tallies — a number written about the
repository inside the repository goes stale the moment it is committed, which
`docs/AI-DISCLOSURE.md` learned three times.

## One version per session

Each working session takes **one** item — a `docs/ROADMAP.md` entry, or something that surfaced
while doing it — takes it deep, tests it properly, and ships it as **one version**, plus a patch
release if that version needs a hotfix. That is why entries here are small and frequent rather than
large and rare, and it is written down in `CLAUDE.md` under *Conventions*.
