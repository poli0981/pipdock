# Phase 0 spikes

`docs/ROADMAP.md` Phase 0: **before any feature code**, because the architecture rests on
assumptions that have to be checked on a real machine. Each spike is a small script here plus
fixtures committed to `crates/pipdock-core/tests/fixtures/`, and its answer is recorded below.

| Spike | Question | Status |
|---|---|---|
| SP-1 | Exact shape of `uv pip install -U --dry-run`; is held-back attribution derivable? | not started — **blocked on installing uv** |
| SP-2 | `pip --dry-run --report` JSON + real stderr for every catalog code | not started |
| SP-3 | PEP 691 index size, SQLite ingest time, fuzzy-search latency over ~600 k names | not started |
| SP-4 | pip-audit invocation for a foreign environment | not started |
| SP-5 | Held-back attribution accuracy on a real numpy/scipy/pandas tangle | not started |
| SP-6 | Windows env discovery sweep (PEP 514, `py -0p`, Store Python, uv-managed) | one finding recorded below |

SP-1 is a **go/no-go**: if uv's plan output is too lossy to populate
`ResolutionReport::held_back[].blockers`, v1.0 ships pip-primary with uv behind a beta-engine flag.

## Findings

### SP-6 — `probe.py -I` hides user-site packages (measured 2026-07-27)

`docs/SECURITY.md` §2 specifies running the probe with `-I` (isolated mode) so a poisoned
environment cannot inject code into it. Isolated mode also disables **user site-packages**, and on
a system Python that is where most packages actually live.

Measured against `C:\Python314` on the dev machine:

| View | Distributions |
|---|---|
| `probe.py` with `-I` | 352 |
| `probe.py` without `-I` | 376 |
| `pip list` | 375 |

So `-I` hides **24 packages** that pip itself reports, and PipDock's Installed tab would disagree
with `pip list` by that much. The remaining 376 vs 375 gap is a second, smaller question — likely a
shadowed distribution that `importlib.metadata` reports twice and pip de-duplicates by normalized
name.

This does not affect the primary target: a venv has no user site, so all three numbers agree. It
affects **non-venv system Pythons that are not PEP 668-marked** — `C:\Python314` reports
`externally_managed: false`, so nothing blocks PipDock from managing it.

**Open decision for SP-6** (do not resolve unilaterally — it trades a documented security control
against list accuracy):

- keep `-I` and show a "user-site packages not shown" note on non-venv environments; or
- drop to `-E` (ignore `PYTHONPATH` only, keep user site), accepting that a package in user site
  can influence the probe's imports; or
- run the probe twice and reconcile, at the cost of a second interpreter start per scan.

### SP-6 — Python 3.14 metadata deprecation (fixed 2026-07-27)

Subscripting `dist.metadata["Requires-Python"]` for an absent header emits
`DeprecationWarning: Implicit None on return values is deprecated and will raise KeyErrors` on
Python 3.14, on **stderr**, for many distributions. `probe.py` now uses `.get()`. Worth keeping in
mind generally: the probe must stay clean on stderr, because the core treats probe noise as
`PD-ENV-003`.
