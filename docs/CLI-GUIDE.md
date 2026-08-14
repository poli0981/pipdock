# PipDock CLI — a guide

`docs/CLI-SPEC.md` is the contract: every flag, every exit code, every JSON shape. This is the
guide — what you actually type, in the order you would need it.

Everything here was run against `pipdock 1.1.0` on Windows. Where a number or a code appears, it
came out of a real invocation, not out of the spec.

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
PipDock uses the last environment you selected, falling back to a `.venv` in the working directory.

**`--json`** switches to machine-readable output. Streaming commands emit NDJSON — one JSON
document per line — so a long update can be consumed as it runs rather than at the end.

Two more worth knowing early:

- **`-y` / `--yes`** assumes the default answer at every prompt. Conflicts resolve to **skip**,
  never to force. There is deliberately no flag that makes `--yes` mean "force latest".
- **`--engine pip|uv`** overrides the configured engine for one invocation, without changing your
  setting.

---

## Finding environments

```powershell
pipdock env list
```

Discovery covers venvs in the working directory, the PEP 514 registry and the `py` launcher. Real
output on a machine with two registry Pythons:

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
is non-null exactly when that applies, and null inside a venv. See CLI-SPEC §7.

**`venv_scan` matches exactly four directory names**: `.venv`, `venv`, `env`, `.env`. A virtual
environment called anything else is invisible to `env list`, and you have to name it with `--env`.

Make one the default so you can stop typing `--env`:

```powershell
pipdock env use C:\proj\.venv
```

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
finds real problems** rather than 0 with a warning. That is what makes it usable as a gate.

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

Every mutating command runs the engine in dry-run first, prints what would change, takes a
snapshot, and only then executes. `--dry-run` stops after the first step.

### When a version conflict appears

```powershell
pipdock update --all --strategy compatible   # default: take the resolver's compatible version
pipdock update --all --strategy latest       # force the latest, breaking whatever depends on it
```

`--strategy latest` off a TTY requires an explicit acknowledgement — it will not silently break an
environment because a script passed `--yes`.

### Excluding things for one run

```powershell
pipdock update --all --except black,mypy
```

`--except` is additive on top of the pin list; it does not replace it.

---

## Installing and removing

```powershell
pipdock install requests
pipdock install "django==5.0.1"
pipdock install requests --dry-run
```

Specs are `name` or `name==version`. Names are validated against PEP 508 and versions against PEP
440 **before** they reach a command line — PipDock never builds a shell string, so there is nothing
for a crafted package name to escape into.

```powershell
pipdock uninstall requests
```

Removal runs the reverse-dependency guard first. If something still needs the package, the command
**stops and exits 2** rather than removing it. `--force` proceeds anyway, and prints what will
break before it does.

---

## Pins

A pin excludes a package from bulk updates. It is per-environment.

```powershell
pipdock pin add numpy --reason "pytorch build is pinned to this"
pipdock pin list
pipdock pin remove numpy
```

Pins are enforced at the plan boundary, not in the display layer — an update that would move a
pinned package never reaches the engine.

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

Two things worth knowing before you rely on this:

- **`diff` lists entries a rollback cannot restore** — editable installs and direct-URL installs.
  That is deliberate: they are excluded from the pin set and reported separately rather than
  silently dropped. Restoring an editable install means re-running `pip install -e` yourself.
- **A rollback snapshots the current state first**, so one rollback creates a second timeline
  entry. The trigger label on each entry is how you tell them apart — `latest` moves twice.

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
modify without a plan. Worth knowing: every pip below 22.2 is broken on Python 3.12+
(`distutils`, `pkgutil.ImpImporter`), so the one case this command exists for is also a pip that
cannot run — it falls back to `ensurepip`, and decides by asking `pip --version` rather than by
matching a traceback.

---

## Scripting it

### Exit codes

Verified by running, not copied from the spec:

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | completed with per-package failures — see `counts.failed` in the JSON; **or `health`/`doctor` found something** |
| 2 | plan aborted, nothing executed (resolution impossible, or the uninstall guard tripped without `--force`) |
| 3 | environment error, including a PEP 668 block |
| 4 | engine unavailable or too old |
| 5 | snapshot failed — **nothing was executed** |
| 6 | network or index error |
| 10 | internal error; the log path is printed |
| 130 | cancelled (Ctrl-C); child processes are reaped and a partial summary is printed |

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
exits 1.

### A CI-shaped invocation

```powershell
pipdock update --all --env C:\proj\.venv --strategy compatible --yes --json
```

`--yes` with `--strategy compatible` never forces anything; a package that cannot be updated
compatibly is skipped and reported, and one failure does not abort the batch — you get
`"13 successful, 2 failed"` with a reason per package.

**`--no-snapshot` exists and you almost certainly do not want it.** It is the only documented
escape from the snapshot-before-mutation rule, it prints a warning, and it is there for throwaway
CI containers that have nothing to roll back to.

### Isolating the store

Every path PipDock writes lives under `%LOCALAPPDATA%\PipDock\data\`. Point `LOCALAPPDATA`
somewhere else and you get a throwaway store — useful for testing a workflow without touching your
real index, settings or snapshots:

```powershell
$env:LOCALAPPDATA = 'C:\tmp\pipdock-test'
pipdock env list
```

---

## When something goes wrong

Every user-visible failure carries exactly one catalog code — `PD-ENV-001`, `PD-NET-002`, and so
on. `docs/ERROR-CATALOG.md` lists all 34, what causes each and what to do about it. The code is
stable; the message around it is localised.

```powershell
pipdock --verbose --log-file pipdock.log update --all
pipdock self report-bug
```

`self report-bug` prints a prefilled GitHub issue URL carrying the version, OS and engine.
**Nothing is transmitted** — you open it, read what it filled in, and submit it yourself.

---

## What is GUI-only

1.1.0's four features have **no CLI surface yet**, and that is deliberate rather than an oversight
— each would add a subcommand and churn the help goldens for something nothing scripts against
today. The logic lives in `pipdock-core` (`pins::suggest`, `requirements::parse`, `cache::usage`),
so adding one later is small:

| Feature | Where it is | Nearest CLI equivalent today |
|---|---|---|
| Pin suggestions | Pins screen | none — `pin list` shows what you have pinned |
| Requirements export | Environments → *Open* | `snapshot create`, then read the `.freeze.txt` |
| Requirements import | Environments → *Open* | `install` with the specs, which is what import does anyway |
| Disk usage | Settings | `tools status` reports the tools venv; nothing reports sizes |
| Command palette | `Ctrl+K` | not applicable |

If you want one of these on the command line, say so in an
[issue](https://github.com/poli0981/pipdock/issues) — the core function is already there.

## See also

- [CLI-SPEC.md](CLI-SPEC.md) — the contract: exhaustive flags, JSON shapes, exit codes
- [ERROR-CATALOG.md](ERROR-CATALOG.md) — all 34 codes and what to do about each
- [SECURITY.md](SECURITY.md) — threat model, PEP 668 policy, what PipDock connects to
- [CODE-HEALTH-SPEC.md](CODE-HEALTH-SPEC.md) — how deptry, vulture and ruff are invoked
