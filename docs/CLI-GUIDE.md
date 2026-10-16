# PipDock CLI — a guide

`docs/CLI-SPEC.md` is the contract: every flag, every exit code, every JSON shape. This is the
guide — what you actually type, in the order you would need it.

Written by running `pipdock` on Windows, and kept to what the code does as the CLI changed: where a
number or a code appears, it came out of a real invocation or out of the code that produces it, not
out of the spec.

---

## Installing it

The installer puts `pipdock.exe` next to the GUI, in `%LOCALAPPDATA%\PipDock\`. **PipDock does not
edit your `PATH`.** Add that folder yourself if you want to type `pipdock` from anywhere:

```powershell
$dir = "$env:LOCALAPPDATA\PipDock"
[Environment]::SetEnvironmentVariable('Path', "$([Environment]::GetEnvironmentVariable('Path','User'));$dir", 'User')
```

Open a new terminal afterwards — the current one keeps the old `PATH`.

```powershell
pipdock --version
```

---

## The two flags that apply to everything

**`--env <PATH>`** picks the interpreter. It accepts either the interpreter itself
(`C:\proj\.venv\Scripts\python.exe`) or the environment directory (`C:\proj\.venv`). Omit it and
PipDock uses the default you set with `pipdock env use`, falling back to a venv in the working
directory — `.venv`, `venv`, `env` or `.env`, in that order. Choosing an environment in the GUI
does not change what the CLI acts on.

**`--json`** switches to machine-readable output: **exactly one JSON document on stdout**, whatever
the command, so `pipdock install requests --json | ConvertFrom-Json` works. Prompts, warnings,
progress and the engine's own output go to stderr. A failure prints a one-line envelope instead —
`{"code":"PD-ENV-001","message":"…","stderrTail":null}`. Ignore `--help`'s "NDJSON for streaming
commands": no command has ever emitted NDJSON, and until 1.4.0 `install` and `update` printed two
documents — the preview, then the summary — which is exactly what made that pipe fail. CLI-SPEC §6
lists which document each command prints, and the few paths that print none.

Two more worth knowing early:

- **`-y` / `--yes`** assumes the default answer at every prompt: a held-back package keeps its
  compatible version and an impossible one is skipped. `--yes` on its own never forces anything —
  see `--strategy latest` below for what does. Off a terminal, a command that would ask first
  refuses instead unless you pass it; `uninstall` and `pip-upgrade` never ask.
- **`--engine pip|uv`** overrides the configured engine for one invocation, without changing your
  setting. `pip-upgrade` ignores it: upgrading pip is pip's job, whatever is configured.

The logging flags — `--verbose`, `--log-file`, `--quiet` — are under
[When something goes wrong](#when-something-goes-wrong).

---

## Finding environments

```powershell
pipdock env list
```

Discovery covers venvs in the working directory, the PEP 514 registry, the `py` launcher and
`uv python list`. Real output on a machine with two registry Pythons:

```json
[
  {
    "envHash": "06a8abde…",
    "externallyManaged": false,
    "hiddenUserSite": "C:\\Users\\you\\AppData\\Roaming\\Python\\Python314\\site-packages",
    "interpreter": "C:\\Python314\\python.exe",
    "packages": 352,
    "pipVersion": "26.2.1",
    "python": "3.14.7",
    "source": "registry"
  }
]
```

**`hiddenUserSite` is not an error.** PipDock reads a non-venv Python in isolated mode, which skips
the per-user site-packages directory — so its package count can be lower than `pip list`. The field
is non-null exactly when that applies, and null inside a venv. See SECURITY §2.

**`venv_scan` matches exactly four directory names**: `.venv`, `venv`, `env`, `.env`. A virtual
environment called anything else is invisible to `env list`, and you have to name it with `--env`.

Make one the default so you can stop typing `--env`:

```powershell
pipdock env use C:\proj\.venv
```

Every later command acts on it — `--env` still wins for one invocation, and the working directory's
venv is only the fallback. If its interpreter goes away, commands say so on stderr and fall back to
the working directory's venv. Until 1.4.0 `env use` stored the default and nothing read it.

---

## Looking before you touch anything

```powershell
pipdock list
pipdock list --outdated
```

`list` is local and fast. `--outdated` asks PyPI for each package's latest release, so it is the
networked one — on a 352-package environment it is the slower of the two by a wide margin.

```powershell
pipdock doctor
```

`doctor` checks the engine, the interpreter and the environment's sanity, and **exits 1 when it
finds real problems** rather than 0 with a warning. That is what makes it usable as a gate. The
engine it judges is the one that would run — `--engine`, else your configured one — so a uv-only
venv (`uv venv` makes them without pip) passes under uv. Whether Code Health's tools environment
has been built is reported, and never fails it.

```powershell
pipdock engine
pipdock engine uv
```

`engine` shows both engines and their versions, and needs no environment — without one, pip, which
lives inside an interpreter, reads `no environment selected`. It does not say which engine is
configured. `pipdock engine uv` sets it, and refuses an engine that is not installed.

---

## Updating

```powershell
# Everything outdated. Pins still apply.
pipdock update --all

# Named packages only.
pipdock update requests urllib3

# See the plan, change nothing.
pipdock update --all --dry-run
```

`update` and `install` run the engine in dry-run first, ask about any conflict, print what would
change and ask once more; only then do they check that the preview still describes the
environment, take a snapshot and execute. `--dry-run` stops at the preview. `pipdock update` with
neither names nor `--all` is a mistake rather than an empty plan, and exits 2.

A preview is good for **ten minutes**, and for the environment it was made against. Confirm later
than that, or after something else changed the environment, and the run is refused with
`PD-RES-002` (exit 2) before the snapshot is written — nothing has happened; re-run the command.

### When a version conflict appears

On a terminal, each held-back package is a question — *Compatible* (the default), *Skip* or *Force
latest* — asked on stderr before the preview, with the installed packages holding it back.
CLI-SPEC §4 has the exact prompt. An impossible set is never a question: its packages are skipped,
and with `--yes` or off a terminal the engine's explanation is printed on stderr first.

```powershell
pipdock update --all --strategy compatible   # default: take the resolver's compatible version
pipdock update --all --strategy latest       # force the latest, breaking whatever depends on it
```

`--strategy latest` forces only where nothing is asked — with `--yes`, or off a terminal. Then every
held-back package goes to its latest, and the preview's *Forced — will break* section names each
installed package that stops being satisfied, with the requirement of its that is overridden. On a
terminal it changes nothing: each package is still a question, and Enter still means *compatible*.
Off a terminal nothing runs without `--yes` anyway, so for a script the acknowledgement *is*
`--yes` — and under `--json --yes` no preview is printed at all, so read `forced` from a
`--dry-run --json` first. Until 1.4.0 forcing forced nothing: the decision was recorded and the
resolve ignored it.

Updating one package on its own can hold it back with no culprit — the preview says *(nothing
installed explains this)* — when its new release needs a newer version of one of its own
dependencies, because a selective update keeps every package it does not name where it is.
`update --all` is unaffected, and `--strategy latest` works around it (ROADMAP's audit backlog,
BL-1).

### Excluding things for one run

```powershell
pipdock update --all --except black,mypy
```

`--except` is additive on top of the pin list; it does not replace it. It takes a comma-separated
list, and can be repeated.

---

## Installing and removing

```powershell
pipdock install requests
pipdock install "django==5.0.1"
pipdock install "django>=5.0,<6"
pipdock install requests --dry-run
```

A spec is a name, optionally followed by a PEP 440 specifier set — a range included. Extras,
markers and URLs are refused (`PD-PKG-002`). `--help` still says `name` or `name==version`, which
was the whole grammar until 1.4.0. Names are validated against PEP 508 and specifiers against PEP
440 **before** they reach a command line — PipDock never builds a shell string, so there is nothing
for a crafted package name to escape into.

```powershell
pipdock uninstall requests
```

Removal runs the reverse-dependency guard first. If something still needs the package, the command
**stops and exits 2** rather than removing it, and prints the `pipdock uninstall` line that would
remove the dependents too. `--force` proceeds anyway, and prints what will break before it does.
**There is no confirm prompt:** when nothing depends on the package, it is removed at once, after
the snapshot.

### When a release needs a different Python

PipDock checks `Requires-Python` itself, because the engines disagree: pip refuses an incompatible
exact pin with the same "No matching distribution" it gives a typo, and uv plans past an upper
bound and fails later, building from source. On Python 3.14:

```powershell
pipdock install scipy==1.7.3              # PD-PKG-001, exit 1: nothing else was asked for
pipdock install scipy==1.7.3 six==1.17.0  # plans six; scipy is listed as left out
```

The preview's *Left out — needs a different Python* section — `incompatible` in the JSON — names
the release, the range it requires and the Python you have. An update does the same for what the
engine planned: a release that excludes your Python is ruled out and the engine asked again, so it,
not PipDock, picks what comes next. Where the engine does not say what a release requires, PipDock
asks PyPI and remembers the answer; one it cannot get never blocks.

---

## Externally managed Pythons (PEP 668)

A Python marked `EXTERNALLY-MANAGED` by whoever installed it — a uv-managed CPython, for one — is
refused by `install`, `update`, `uninstall` and `snapshot rollback` with `PD-ENV-002`, **exit 3**,
before any engine command runs, and by `pip-upgrade` too. The answer is a virtual environment: a
venv never carries the marker, even one built from such a Python.

There is an override, in the GUI's Settings, and **deliberately no CLI flag**: a flag would put it
one shell-history search away. The CLI reads the setting the GUI stores. With it on, and only for
an environment that carries the marker, PipDock passes `--break-system-packages` — and every run
says so on stderr:

```text
warning: this Python is externally managed (PEP 668) and the Settings override is on, so --break-system-packages will be passed. Prefer a virtual environment.
```

uv takes the flag; pip has since 23.0.1. **pip 23.0 itself is refused** with `PD-ENV-002` even with
the override, because it enforces PEP 668 and has no flag to get past it — and `pip-upgrade` cannot
rescue it, since it refuses a managed environment whatever the setting says. Use `--engine uv`, or
upgrade pip outside PipDock. Below 23.0 pip does not enforce the marker, and nothing is passed.
Until 1.4.0 the override was a checkbox nothing read.

---

## Pins

A pin keeps a package out of every update — one that names it as well as `--all` — until it is
unpinned. It is per-environment.

```powershell
pipdock pin add numpy --reason "pytorch build is pinned to this"
pipdock pin list
pipdock pin remove numpy
```

Pins are enforced at the plan boundary, not in the display layer: a pinned package is taken out of
the request before the engine is asked. The text output says what was left out
(`1 pinned package(s) excluded: numpy`); under `--json` that line is not printed, so a script reads
`pin list --json`.

---

## Snapshots and rollback

A snapshot is taken automatically before every batch. You can also take one by hand:

```powershell
pipdock snapshot create
pipdock snapshot list
pipdock snapshot diff <id>
pipdock snapshot rollback <id>
pipdock snapshot rollback latest
```

`rollback` shows what it will remove and restore, and asks first — `--yes` skips the question, and
off a terminal it refuses without one. Like any change it is refused for an externally managed
Python, and refused with `PD-RES-002` if the environment changed between the preview and your
answer, or the answer took more than ten minutes.

Three things worth knowing before you rely on this:

- **`diff` lists entries a rollback cannot restore** — editable installs and direct-URL installs.
  That is deliberate: they are excluded from the pin set and reported separately rather than
  silently dropped. Restoring an editable install means re-running `pip install -e` yourself.
- **A rollback snapshots the current state first**, so one rollback creates a second timeline
  entry. The trigger label on each entry is how you tell them apart — `latest` moves twice.
  `--no-snapshot` does not apply here.
- **`diff` compares the way a rollback with the current engine would.** uv's freeze never lists
  pip, setuptools or wheel, so across engines those three are left out of the comparison, and a
  rollback never removes pip. Until 1.4.0 a uv snapshot restored with pip uninstalled pip.

---

## Code Health

```powershell
pipdock health
pipdock health --path C:\proj
pipdock health --tool ruff --tool vulture
pipdock health --fix
```

deptry, vulture and ruff run from PipDock's own isolated tools environment, never from the
environment being inspected — so analysing a project never installs anything into it.

**`health` exits 1 when it finds something.** A linter that exits 0 on findings is useless in a
pre-commit hook, so this is the intended behaviour, not a failure.

`--fix` applies **ruff's safe fixes only**, and prompts first. It is the one operation in PipDock
that writes outside site-packages and `%LOCALAPPDATA%` — it rewrites your source. There is no
snapshot for it, because no snapshot in PipDock describes a source tree; your version control is
the safety net, and the confirm says so. It refuses entirely, before writing a byte, if any target
file cannot be written.

It fixes exactly what the report looked at, with the same exclusions; until 1.4.0 it ran without
them, and rewrote notebooks and, in a project that sets its own `exclude`, files under `.venv`. With
`--yes` it proceeds on a committed tree and refuses, exit 2, on one with uncommitted changes —
commit or stash first. Outside a git repository there is no tree to check, and `--yes` proceeds.
Under `--json` it prints the `FixReport` alone, and nothing at all when no fix ran.

The tools environment is built on first use. To manage it directly:

```powershell
pipdock tools status
pipdock tools sync
```

Two facts about the tools that are pinned by tests because the specs had them wrong: all three
exit non-zero on findings (**vulture uses 3**), and **deptry reports a module name, not the
distribution that provides it** — `PIL`, not `pillow`. deptry also cannot be told which environment
to compare against, which is disclosed wherever its findings are shown.

---

## Security advisories

```powershell
pipdock audit
pipdock audit --engine pip
```

`audit` checks the environment's installed releases against PyPI's advisory data with pip-audit,
run — like Code Health's tools — from an environment of PipDock's own, never from the one being
checked; the first run builds it. It has no flags of its own. **It exits 1 when it finds an
advisory**, like `health`. A pip-audit that could not run at all exits on its own error code
instead, because an empty list from a tool that never ran must not read as "nothing wrong".

What it checks is the engine's freeze, so `--engine` matters: pip's lists pip, setuptools and
wheel, and uv's does not. Until 1.4.0 `audit` used the configured engine whatever `--engine` said.

Findings are grouped under each package and version: an advisory's id, its fix — `fixed in …`, or
`no fix` — and its aliases. There is no severity, because pip-audit publishes none (SECURITY §6).
`audit` changes nothing; the remedy is an ordinary `pipdock update <pkg>`.

The cost is a network fetch, not the package count: measured at 68 s cold, and about 20 s warm for
one package or twelve — which is why it says how many packages it is checking before the wait
rather than after it.

---

## Search and metadata

```powershell
pipdock index refresh
pipdock search hello
pipdock info requests
```

`search` runs against a locally cached PyPI name index (PEP 691) — no network per keystroke. The
index is around 864,000 projects and 42 MB on the wire; `index refresh` re-pulls it.

---

## pip upkeep

```powershell
pipdock pip-upgrade
```

Upgrades pip **inside the selected environment**, which PipDock is otherwise careful never to
modify without a plan. It always uses pip, whatever `--engine` says — there is no uv way to upgrade
pip — and it asks nothing and takes no snapshot: a snapshot's only restore path would be pip
itself. It refuses an externally managed Python whatever the Settings override says. Worth
knowing: every pip below 22.2 is broken on Python 3.12+ (`distutils`, `pkgutil.ImpImporter`), so the
one case this command exists for is also a pip that cannot run — it falls back to `ensurepip`, and
decides by asking `pip --version` rather than by matching a traceback.

---

## Scripting it

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success — a dry run, and a plan with nothing to do, included |
| 1 | completed with per-package failures — see `counts.failed` in the JSON; **or `health`, `audit` or `doctor` found something**; or a failure coded `PD-PKG`, `PD-BLD`, `PD-PRM`, `PD-SYS` or `PD-HLT`, such as a malformed spec |
| 2 | nothing executed: resolution impossible, the uninstall guard tripped without `--force`, the confirm was declined (or there was no terminal to ask on and no `--yes`), the preview had gone stale, or a usage error |
| 3 | environment error, including a PEP 668 block |
| 4 | engine unavailable or too old, or an engine query stopped by its watchdog |
| 5 | snapshot failed — **nothing was executed**; or no such snapshot |
| 6 | network or index error |
| 10 | internal error; `log: <path>` follows on stderr |
| 130 | stopped by Ctrl-C during `install`, `update`, `uninstall` or `snapshot rollback`: the engine's process tree is killed, what ran is summarized, and stderr names the snapshot to roll back to. A second Ctrl-C quits at once |

Until 1.4.0 this table said it had been verified by running, and two of its rows were not true:
there was no log for code 10 to point at, and the CLI installed no Ctrl-C handler, so Ctrl-C killed
it outright — no summary, and no 130. Both rows describe the code now. Ctrl-C during any other
command still ends it the way it ends any console program, without a summary; the engine's process
tree goes with it either way.

Code **5** is the one to special-case in CI: it means the snapshot could not be written, so PipDock
refused to run the plan at all. Nothing changed.

### Pinning against the JSON shapes

```powershell
pipdock schema ResolutionReport
pipdock schema ExecutionSummary
pipdock schema HealthReport
```

`schema` prints the JSON Schema for a core type, generated from the same Rust definitions the
commands serialize — so a script can validate against it instead of guessing. An unknown type name
exits 1, and lists the names it knows. Which document each command prints is CLI-SPEC §6's table.

### A CI-shaped invocation

```powershell
pipdock update --all --env C:\proj\.venv --strategy compatible --yes --json
```

`--yes` with `--strategy compatible` never forces anything: a held-back package takes its
compatible version, a package that cannot be updated at all is skipped — and named on stderr — and
one failure does not abort the batch: you get `"13 successful, 2 failed"` with a reason per package.
Its stdout is one document, the `ExecutionSummary` when anything ran. Test `$LASTEXITCODE` before
reading it: exit 0 is also *nothing to do*, which prints a `FlowStep` —
`{"step":"nothing","reason":"nothing-to-do"}` — instead.

**`--no-snapshot` exists and you almost certainly do not want it.** It is the only documented
escape from the snapshot-before-mutation rule, it prints a warning, and it is there for throwaway
CI containers that have nothing to roll back to. It covers `install`, `update` and `uninstall`;
`snapshot rollback` snapshots the state it replaces regardless.

### Isolating the store

Everything PipDock keeps for itself — the store, snapshots, the tools and audit environments, the
log — lives under `%LOCALAPPDATA%\PipDock\data\`. Point `LOCALAPPDATA` somewhere else and you get a
throwaway store — useful for testing a workflow without touching your real index, settings or
snapshots:

```powershell
$env:LOCALAPPDATA = 'C:\tmp\pipdock-test'
pipdock env list
```

---

## When something goes wrong

Every user-visible failure carries exactly one catalog code — `PD-ENV-001`, `PD-NET-002`, and so
on. `docs/ERROR-CATALOG.md` lists all 35 (in 32 rows: `PD-HLT-001..004` share one), what causes
each and what to do about it. The code is stable; the message around it is localised.

Every run is logged, whether you ask or not: one file a day,
`%LOCALAPPDATA%\PipDock\data\logs\pipdock-cli.<date>.log`, beside the GUI's `pipdock.<date>.log`,
kept for 14 days. It records what PipDock ran — the arguments you gave, every command it spawned
with its exit code and duration, each resolve, snapshot and run outcome — and nothing in it is sent
anywhere. When you want more:

```powershell
pipdock --verbose --log-file pipdock.log update --all
$env:PIPDOCK_LOG = 'debug'; pipdock update --all
pipdock self report-bug
```

`--verbose` shows the log on stderr as it happens, at DEBUG, which includes every line an install
or a removal printed. `--log-file` appends a copy at the file's level — set `PIPDOCK_LOG` as well if you want
DEBUG in it — wherever a CI job can collect it. Neither ever writes to stdout, so `--json` output
stays one document; `--quiet` only quiets the terminal. An internal error (exit 10) ends with
`log: <path>` on stderr. Until 1.4.0 there was no log at all, and `--verbose` and `--log-file` were
accepted and ignored.

`self report-bug` prints a prefilled GitHub issue URL carrying PipDock's version, the OS, the
engine, and the selected environment's Python and engine versions. It attaches no log excerpt:
attach the day's log yourself, after reading it for paths you would rather not make public.
**Nothing is transmitted** — you open it, read what it filled in, and submit it yourself.

---

## What is GUI-only

These have **no CLI surface yet**, and that is deliberate rather than an oversight — each would add
a subcommand and churn the help goldens for something nothing scripts against today. The logic
lives in `pipdock-core` (`pins::suggest`, `requirements::parse`, `cache::usage`,
`ReverseDeps::view`), so adding one later is small:

| Feature | Where it is | Nearest CLI equivalent today |
|---|---|---|
| Pin suggestions | Pins screen | none — `pin list` shows what you have pinned |
| Requirements export | Environments → *Open* | `snapshot create`, then read the `.freeze.txt` |
| Requirements import | Environments → *Open* | `install` with the specs, ranges included, which is what import does anyway |
| Disk usage | Settings | `tools status` reports the tools venv; nothing reports sizes |
| Dependency view | a package row's `⇄` | `list --json` carries each package's `requiresDist`, which the view is drawn from; `doctor` reports requirements the engine's check finds broken |
| Command palette | `Ctrl+K` | not applicable |

If you want one of these on the command line, say so in an
[issue](https://github.com/poli0981/pipdock/issues) — the core function is already there.

**The PEP 668 override is GUI-only for a different reason, and stays so.** It is meant to be hard
to reach — a decision made in the GUI, behind a confirm — and a flag would put it one shell-history
search away. The CLI reads the setting the GUI stores, and says so on stderr every time it applies;
see [Externally managed Pythons](#externally-managed-pythons-pep-668).

## See also

- [CLI-SPEC.md](CLI-SPEC.md) — the contract: exhaustive flags, JSON shapes, exit codes
- [ERROR-CATALOG.md](ERROR-CATALOG.md) — all 35 codes and what to do about each
- [SECURITY.md](SECURITY.md) — threat model, PEP 668 policy, what PipDock connects to
- [CODE-HEALTH-SPEC.md](CODE-HEALTH-SPEC.md) — how deptry, vulture and ruff are invoked
