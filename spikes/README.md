# Phase 0 spikes — findings

`docs/ROADMAP.md` Phase 0: **before any feature code**, because the architecture rests on
assumptions that have to be checked on a real machine. Each spike is a script here plus fixtures
committed under `crates/pipdock-core/tests/fixtures/`.

Run on 2026-07-27 · Windows 11 Pro 26300 · i7-14700KF · Python 3.12.10 / 3.14.6 ·
pip 25.0.1 and 26.1.2 · uv 0.11.32 and 0.10.12.

| Spike | Verdict |
|---|---|
| SP-1 uv dry-run shape | **GO for both engines** — but for a different reason than the doc assumed |
| SP-2 pip report & stderr corpus | 8 fixtures per engine; **one blocker found and fixed** |
| SP-3 index economics | **PASS**, with one architectural requirement and one thin margin |
| SP-4 pip-audit foreign env | Answered: freeze-file mode only |
| SP-5 held-back attribution | Answered by SP-1 — **no engine reports attribution at all** |
| SP-6 Windows discovery | Answered; **two identity bugs avoided** |

Reproduce with:

```bash
py -3.12 spikes/capture.py --engine pip
```

---

## SP-1 — uv dry-run shape · **GO**

**The question was the wrong shape.** ROADMAP asked whether uv's text plan is lossy enough that
v1.0 should ship pip-primary with uv behind a beta flag. The answer is that uv is *not* the weak
side — on the one output that matters most it is clearly better than pip.

### What both engines actually do

`<engine> install -U <pkg>` **ignores the constraints of packages already installed**:

| Scenario | Plan produced | Exit |
|---|---|---|
| Seed `httpx==0.23.0` (needs `httpcore>=0.15,<0.16`), then `install -U httpcore` | `httpcore 0.15.0 → 1.0.9` — **silently breaks httpx** | 0 |
| Same, but restating the installed package: `install -U httpcore httpx==0.23.0` | *no changes* — httpcore correctly held at 0.15.0 | 0 |

pip and uv behave **identically** here. There is no "held back" concept in either engine's
`install -U` output, because neither holds anything back — they break the environment and exit 0.

> **Design requirement (both adapters).** Every plan must restate the full installed set as
> explicit requirements. Without it, PipDock's core promise — preview before touch — would preview
> a plan that quietly breaks the environment, and `pip check` would only discover it afterwards.
> This is not an optimisation; it is the difference between the product working and not.

### Where uv wins

Given an unsatisfiable set, uv names the exact constraint and the chain:

```text
  × No solution found when resolving dependencies:
  ╰─▶ Because httpx==0.23.0 depends on httpcore>=0.15.0,<0.16.0 and you
      require httpcore>=1.0, we can conclude that your requirements and
      httpx==0.23.0 are incompatible.
```

pip, for the same input, says only:

```text
ERROR: Cannot install httpcore>=1.0 and httpx==0.23.0 because these package versions have
conflicting dependencies.
```

uv gives the blocker **and** its constraint; pip gives neither. `docs/ERROR-CATALOG.md` PD-RES-001
tells the user to make per-package Skip/Force choices — with uv, PipDock can also tell them *why*,
straight from the engine.

### Output channel

**uv writes its entire plan to stderr; stdout is empty.** All nine uv fixtures have `stdout=0B`.
The adapter must read stderr for the plan, which also means engine stderr cannot be treated as
"error output only" — the error classifier has to run *after* a plan-parse attempt, not before.

uv 0.10.12 and 0.11.32 produce identical formats for every scenario, so the parser has at least
one minor version of stability. Minimum pinned at **0.10.0**.

---

## SP-2 — pip report & stderr corpus · **blocker found**

### `pip install --dry-run --report -` crashes on Windows

pip's `--report` writes its JSON through the vendored `rich`, which on Windows routes via
`legacy_windows_render` and the **console** codec. Under cp1252 it raises `UnicodeEncodeError` the
moment any report field holds a non-ASCII character — pip exits 2 and emits no report:

```text
File "...\encodings\cp1252.py", line 19, in encode
UnicodeEncodeError: 'charmap' codec can't encode characters in position 860-863
```

Reproduced with a plain `pip install -U --dry-run --quiet --report - idna`. Confirmed on **pip
25.0.1 and pip 26.1.2** — current, not legacy.

It is **data-dependent**, which makes it worse than a hard failure: the `held-back` fixture's
32 KB report is all-ASCII and succeeds, so this would pass local testing and fail on users'
machines depending on which packages they have.

> **Mitigation (mandatory, every pip invocation).** Set `PYTHONIOENCODING=utf-8` and
> `PYTHONUTF8=1` in the child environment. Verified: exit 0, valid JSON on stdout.
> `spikes/capture.py` applies this as `SAFE_ENV`, and the `report-encoding-crash` fixture keeps
> the failure as evidence so a regression is recognisable.

### Catalog notes from the corpus

- **PD-PKG-001 vs PD-PKG-002 are not separable from pip stderr.** A Requires-Python rejection and
  an unknown name both produce `No matching distribution found for X`. ERROR-CATALOG's note about
  cross-checking index metadata is not optional — it is the only way to tell them apart.
- **PD-PKG-003 (yanked) is not an error.** Both engines exit **0** and install the yanked release,
  emitting a warning. pip carries it structurally (`"is_yanked": true` in the report) *and* on
  stderr; uv only on stderr. So the yank belongs in the **preview**, as a warning row, not in the
  failure path the catalog currently implies.
- **The engines disagree about Requires-Python.** `scipy==1.7.3` declares `>=3.7,<3.11`. On Python
  3.12 pip refuses it; **uv planned to install it**. PipDock's preview will therefore differ by
  engine for the same request. Needs a decision before M1 (see open questions).
- pip's own upgrade notice (`[notice] A new release of pip is available`) appears on stderr in
  almost every capture. Classifiers must not treat a non-empty stderr as failure.

Fixtures: `crates/pipdock-core/tests/fixtures/{pip,uv}/<scenario>/{stdout,stderr}.txt` +
`meta.json` recording argv, engine version, exit code and the question each answers.

Still to capture (need conditions this machine cannot produce):
`PD-BLD-001` (MSVC missing — MSVC is installed here), `PD-NET-002` (TLS interception),
`PD-PRM-002` (locked file), `PD-SYS-001/002` (long path, disk full).

---

## SP-3 — PEP 691 index economics · **PASS**

| Measure | Result | Budget |
|---|---|---|
| Projects on PyPI | **858,476** | doc assumed ~600 k |
| Index on the wire (gzip) | 8.91 MiB | — |
| Decompressed | 40.17 MiB | — |
| Fetch | 1.25 s | — |
| Parse + normalize + SQLite ingest | 1.70 s | — |
| **Cold refresh, total** | **2.95 s** | < 60 s ✅ |
| SQLite file | 47.86 MiB | — |
| Load all names into memory | 82 ms, once at startup | — |
| **Worst keystroke (nucleo, in memory)** | **42.1 ms** (`"re"`) | < 50 ms ✅ |
| Full SQLite column scan | 218 ms | ❌ 4× over |

> **Architectural requirement.** Searching by scanning SQLite per keystroke costs 218 ms and
> misses the budget fourfold. The name list must be loaded into memory **once** and matched there;
> SQLite is the persistence layer, not the search path.

Two cautions:

- **The margin is thin, and this is fast hardware.** 42.1 ms on an i7-14700KF leaves ~16 %
  headroom. SP-3 explicitly asked about a low-end reference VM, which was not available here — on
  slower hardware this misses. Keep the delta/prefilter fallback in the drawer as ROADMAP says;
  the obvious first move is a cheap substring prefilter before the fuzzy pass on short queries,
  since `"re"` (the widest match) is the worst case.
- **Raw nucleo ranking is not shippable.** Searching `requests` ranks `requests-ntlm` above
  `requests`; `numpy` ranks `numpy-utils` first. The Search screen needs an exact- and
  prefix-match boost on top of the fuzzy score, or it will look broken on the most common queries.

Reproduce: `py -3.14 spikes/sp3_index.py` then
`cargo run --release --example sp3_search -- spikes/out/index.db`.

---

## SP-4 — pip-audit against a foreign environment · **freeze-file mode only**

pip-audit 2.10.1:

- `--path <site-packages> --no-deps` is **rejected**: *"The --no-deps flag can only be used with
  --requirement (-r)"*. `--path` is therefore not an alternative to the documented approach.
- `-r <freeze> --no-deps -f json` **works**: against a seeded env it found 10 advisories in
  `urllib3==2.0.0`.

`docs/SECURITY.md` §6 is confirmed as written. Two implementation notes: the JSON output
**repeats advisory ids** (`PYSEC-2023-192` twice), so findings need de-duplication before they
reach the UI; and pip-audit writes a `pip-compile`/hashes advisory to stderr on every run, which
must not be treated as a failure.

---

## SP-5 — held-back attribution accuracy · **answered by SP-1**

The spike assumed the engine reports *what* was held back and PipDock supplies *who*. In fact
**neither engine reports a held-back item at all** — with the installed set restated, pip's report
is simply empty and uv says `Would make no changes`. There is nothing to attribute against.

So all three parts of the preview's "needs decision" row must be derived by PipDock:

| Part | Source |
|---|---|
| resolved version | the plan, or the absence of a change for that package |
| latest version | the index (already fetched for `list --outdated`) |
| the blocker and its constraint | the reverse-dependency graph from `probe.py`'s `requires_dist` |

ARCHITECTURE §3 already specifies graph-based attribution for uv. The correction is that it
applies to **pip equally** — the `--dry-run --report` JSON does not carry it, contrary to the
document's implication. ARCHITECTURE §3's ambiguity rule (show the constraint without a culprit
rather than guessing) becomes the main safeguard, so it needs test coverage from day one.

The numpy/scipy/pandas tangle from the original spike text was not exercised; the httpx/httpcore
construction answered the question more cleanly and deterministically. Worth revisiting during M1
against a real 200-package environment.

---

## SP-6 — Windows environment discovery · **two identity bugs avoided**

Sources swept: PEP 514 registry (HKCU and HKLM), `py -0p`, `uv python list`.

### `uv python list` needs filtering and de-duplication

It returns downloadable interpreters alongside installed ones (`<download available>`), which must
be dropped. It also surfaces **shims that duplicate a real interpreter** — here a Chocolatey shim
and the real install both report the same interpreter:

```text
cpython-3.14.6-windows-x86_64-none    C:\Python314\python.exe
cpython-3.14.6-windows-x86_64-none    C:\ProgramData\chocolatey\bin\python3.14.exe
```

Both resolve to `sys.executable = c:\python314\python.exe`. Path-string de-duplication is not
enough — dedupe on what the probe reports, not on the discovery path.

### `env_hash` must be case-normalized

Launched via the shim, Python reports `c:\python314`; launched directly, `C:\Python314`.
ARCHITECTURE §6 defines `env_hash` as the SHA-256 of the canonicalized interpreter path — on
Windows that **must lowercase** as part of canonicalization. Otherwise the same environment gets
two hashes, and its pins and snapshot history silently split in two depending on how it was
discovered. PEP 514 registry entries exist under both HKCU and HKLM here, giving another route to
the same duplication.

### `probe.py -I` hides user-site packages

`docs/SECURITY.md` §2 specifies `-I` (isolated mode) so a poisoned environment cannot inject code
into the probe. Isolated mode also disables user site-packages, where most packages live on a
system Python. Measured against `C:\Python314`:

| View | Distributions |
|---|---|
| `probe.py` with `-I` | 352 |
| `probe.py` without `-I` | 376 |
| `pip list` | 375 |

`-I` hides **24 packages pip itself reports**. Venvs are unaffected (no user site), so this only
bites non-venv system Pythons — and `C:\Python314` reports `externally_managed: false`, so nothing
blocks PipDock from managing it. The remaining 376 vs 375 gap is likely a shadowed distribution
that `importlib.metadata` reports twice and pip de-duplicates by normalized name.

**Open decision** — it trades a documented security control against list accuracy:

- keep `-I` and show a "user-site packages not shown" note on non-venv environments; or
- drop to `-E` (ignore `PYTHONPATH` only, keep user site); or
- probe twice and reconcile, at the cost of a second interpreter start per scan.

### Python 3.14 metadata deprecation (fixed)

Subscripting `dist.metadata["Requires-Python"]` for an absent header emits
`DeprecationWarning: Implicit None on return values is deprecated and will raise KeyErrors` on
Python 3.14, on stderr, for most distributions. `probe.py` now uses `.get()`. The probe must stay
stderr-clean, because noise there reads as `PD-ENV-003`.

Not swept: **Microsoft Store Python** is not installed on this machine, so the aliasing quirks
SP-6 asks about remain unverified. Store Python must still be either supported or explicitly
detected-and-explained before v1.0.

---

## Open questions for the owner

1. **Requires-Python divergence (SP-2).** pip refuses `scipy==1.7.3` on Python 3.12; uv plans it.
   Should PipDock enforce Requires-Python itself so the preview is engine-independent, or show
   each engine's own verdict?
2. **`probe.py -I` (SP-6).** Which of the three options above?
3. **Search ranking (SP-3).** Confirm that an exact/prefix boost over nucleo's score is in scope
   for M1 rather than deferred — the Search screen is unusable without it.
4. **`PD-BLD-001` fixture (SP-2).** Capture on a clean VM, or ship a labelled synthetic fixture?
   `docs/TESTING.md` §2 requires a fixture per code before ship.
