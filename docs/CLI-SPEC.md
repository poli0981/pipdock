# PipDock — CLI Specification

*Version 0.1 · 2026-07-17 · Binary: `pipdock` (clap, from `pipdock-cli` crate, same `pipdock-core` as the GUI)*

## 1. Principles

1. **Same core, same behavior.** Every CLI command maps 1:1 onto the core functions the GUI uses; the CLI adds no logic of its own.
2. **Safe by default, scriptable on request.** Interactive prompts appear on a TTY; in scripts (`--yes` / no TTY), a held-back package keeps its compatible version and an impossible one is **skipped** — never forced, unless `--strategy latest` asked for exactly that (§4). Off a TTY, a command that would ask first refuses instead unless `--yes` is given; `uninstall` and `pip-upgrade` ask nothing (§3).
3. **Machine-readable everywhere.** `--json` makes every command print one JSON document on stdout — the structs the GUI receives, wherever one exists (§6).
4. English-only in v1 (GUI carries EN/VI); messages reuse catalog codes so they stay greppable.

## 2. Global options

```text
--env <path>        interpreter or env dir (default: the one `env use` set, else a venv in the CWD)
--engine <pip|uv>   override configured engine for this invocation
--json              one JSON document on stdout (§6); prompts, progress and warnings go to stderr
--yes / -y          assume defaults on all prompts (held back → compatible, impossible → skip)
--quiet             less on the terminal: no streamed engine output, no progress lines
--verbose           the log on stderr as well, at DEBUG
--log-file <path>   a copy of the log, appended to <path>
--no-snapshot       DANGEROUS: skip pre-batch snapshot (CI images only; prints warning)
```

`--quiet` and `--verbose` cannot be combined (clap refuses it, exit 2). `--quiet` keeps the terminal to results, prompts, warnings and errors — `env use` and `tools sync` drop even their one-line confirmation — and changes nothing in the log. `--no-snapshot` applies to `install`, `update` and `uninstall`; `snapshot rollback` always snapshots the state it replaces (DATA-FLOW §8) and accepts the flag without acting on it.

**Which environment.** In order:

1. `--env` — an interpreter, or an environment directory whose `Scripts\python.exe` is used;
2. the default `pipdock env use` stored. If its interpreter is gone, stderr says `` warning: the default environment (<path>) no longer exists; `pipdock env use` sets another `` and the search goes on;
3. the first of `.venv`, `venv`, `env`, `.env` in the working directory that holds an interpreter.

Nothing found is PD-ENV-001 (exit 3), naming all three ways. Only `env use` writes the default — selecting an environment in the GUI does not change what the CLI acts on. `env list`, `info`, `index`, `tools` and `schema` never look for an environment; `search` and `self report-bug` use one when they find one and work without it; `engine` reports without one (§3). Until 1.4.0 the default was written and read by nothing, so `env use` reported success and every later command ignored it (X3).

**Logging.** Every invocation appends to `%LOCALAPPDATA%\PipDock\data\logs\pipdock-cli.<YYYY-MM-DD>.log` — one file per UTC day; the GUI writes `pipdock.<date>.log` beside it, so the two never interleave — and sweeps that folder of dated log files older than `LOG_RETENTION_DAYS` (14), leaving any other file alone. The file's level is `PIPDOCK_LOG`, a `tracing` filter directive such as `debug`, and INFO when it is unset or not a valid directive. At INFO the file holds the invocation's arguments, each subprocess's argv, exit code and duration, every stop and its reason, and each resolve, snapshot and run outcome; the lines an install, a removal or a tools-venv sync prints as it runs are DEBUG. `--verbose` adds stderr at DEBUG, whatever `PIPDOCK_LOG` says. `--log-file` appends the file's lines, at the file's level, to a path of your choosing; one it cannot open is a warning on stderr, not a failure. **Nothing is ever logged to stdout**, so `--json` stays one document under `--verbose`. An internal error (exit 10) prints `log: <today's file>` on stderr, in both modes. Until 1.4.0 there was no log at all: `tracing` was a dependency and never called, and `--verbose` and `--log-file` were parsed and ignored.

## 3. Commands

```text
pipdock env list                         # discovered envs, sources, PEP 668 flags
pipdock env use <path>                   # set the default env (§2)
pipdock list [--outdated]                # installed packages, from the probe; --outdated asks the index
pipdock search <query> [--limit n]       # local fuzzy index search (n defaults to 20)
pipdock info <pkg>                       # cached PyPI metadata
pipdock install <spec...> [--dry-run]    # spec = a name, optionally with a PEP 440 specifier set; dry-run prints the ResolutionReport
pipdock update [--all | <pkg...>]        # one or the other: neither is a usage error (exit 2)
        [--strategy compatible|latest]   # latest forces every held-back package, where nothing is asked (§4)
        [--except <pkg,...>]             # ad-hoc exclusions on top of pins; comma-separated or repeated
        [--dry-run]
pipdock uninstall <pkg...> [--force]     # guard names each dependent and its constraint; --force overrides; no prompt
pipdock pin add|remove <pkg> [--reason "…"] | pin list
pipdock snapshot list | create | diff <id> | rollback <id|latest>
pipdock doctor                           # the engine that would run + env sanity; exit 1 on real problems
pipdock audit                            # known advisories in the selected environment; exit 1 on findings
pipdock health [--path <dir>] [--tool deptry|vulture|ruff] [--fix]   # --fix = ruff only, prompts
                                                                    # --yes: proceeds on a clean tree,
                                                                    # REFUSES on a dirty one (exit 2)
pipdock pip-upgrade                      # upgrade pip inside the selected env: always pip, no snapshot
pipdock engine [pip|uv]                  # no argument: both engines' versions; with one: set it
pipdock index refresh                    # re-pull PEP 691 name index
pipdock tools sync [--force] [--python <path>]   # build/re-sync the Code Health tools venv
pipdock tools status                     # where it is, and whether it matches the shipped pins
pipdock schema <type>                    # JSON Schema for a core type, so scripts can pin against it (§6)
pipdock self report-bug                  # prints prefilled GitHub issue URL (ERROR-CATALOG §4)
```

**Specs.** `install` takes a name, optionally followed by a PEP 440 specifier set: `requests`, `django==5.0.1`, `"django>=5.0,<6"`, `"attrs ~= 23.1"`, `pkg===1.0`. An extra, a marker, a URL or anything else PEP 440 does not define is PD-PKG-002 (exit 1), refused before it reaches argv (SECURITY §2). `install --help` still says "`name` or `name==version`", which until 1.4.0 was the whole grammar: a request was split on `==`, so `django>=5.0,<6` became a package name.

**Before anything changes.** `install`, `update`, `uninstall` and `snapshot rollback` begin with a fresh probe of the environment, and three rules follow from it.

- *PEP 668* (SECURITY §3). An environment carrying the `EXTERNALLY-MANAGED` marker is refused with PD-ENV-002 (exit 3) before any engine command runs. The only way past is the GUI's Settings override, which the CLI reads from the store both heads share — **there is deliberately no CLI flag for it**: the override is meant to be hard to reach, and a flag would put it one shell-history search away. With the override on *and* the marker present, every mutating engine call is given `--break-system-packages`, and so is uv's dry-run resolve, which refuses a managed environment without it (pip's `--dry-run --report` never is). uv takes the flag in every version PipDock supports, pip from 23.0.1, which added it. Every such run says so on stderr: `warning: this Python is externally managed (PEP 668) and the Settings override is on, so --break-system-packages will be passed. Prefer a virtual environment.` pip 23.0 enforces PEP 668 with no way past it, so it is refused with PD-ENV-002 even then (`pip 23.0 enforces PEP 668 and has no override; --break-system-packages arrived in pip 23.0.1`); `--engine uv` gets through, and `pip-upgrade` does not help, because it refuses a managed environment whatever the override says. Below 23.0 pip enforces nothing and is passed nothing. A venv never carries the marker. Until 1.4.0 the override was saved and read by nothing, the gate trusted a cached flag rather than a probe, and a rollback had no gate at all.
- *Requires-Python* (hard invariant 10). PipDock enforces it itself, because the engines disagree: pip refuses an incompatible exact pin with the "No matching distribution" it gives a typo, and uv plans past an upper bound. An exact pin the user typed (`name==v`, `name===v`) is checked before any engine sees it; a release *Force latest* would install is checked before it is forced; and every release the engine planned is checked after. pip's report says what each planned release requires; for the rest — exact pins, forced releases, anything uv plans — PipDock asks PyPI (`https://pypi.org/pypi/<name>/<version>/json`), remembers each answer, and never blocks on one it cannot get. A planned release that excludes this interpreter is ruled out by name and the engine asked again, at most twice, so the engine rather than PipDock picks what comes next; one still planned after that stays in the plan, as a warning. Every such release is in the report's `incompatible` (§6), which the preview prints as *Left out — needs a different Python*; a request that is nothing but such pins fails with PD-PKG-001 instead (exit 1), naming the range and the interpreter's version. Until 1.4.0 nothing raised PD-PKG-001.
- *A preview is good for ten minutes, and for the environment it was made against* (DATA-FLOW §9.3). Between the decision and the snapshot — after the confirm, or after the guard for `uninstall` — the environment is probed again, or re-frozen for a rollback, and a preview older than ten minutes, or one the environment no longer matches, is refused with PD-RES-002 (exit 2). The refusal comes before the snapshot, so it costs nothing; re-run the command. Until 1.4.0 both checks existed and neither could fire.

**`uninstall` asks nothing.** The guard runs once over the whole removal set before anything is touched (DATA-FLOW §5). Clear, the packages are removed at once, after the snapshot. Tripped, the command refuses with PD-RES-004 and exit 2 and prints the `pipdock uninstall …` line that would remove the dependents too — unless `--force` was given, which prints `warning: --force given; proceeding despite the breakage above` and removes. `--yes` changes nothing here.

**`pip-upgrade`** always runs pip, whatever `--engine` or the configured engine says: there is no uv way to upgrade pip (DATA-FLOW §7). It asks nothing and takes no snapshot (DATA-FLOW §9's pip-upkeep exemption), refuses a PEP 668 environment whatever the override says, and falls back to `ensurepip` for a pip too broken to run — every pip below 22.2 on Python 3.12+.

**`doctor`** judges the engine that would run — `--engine`, else the configured one — and exits 1 when that engine is unavailable, or its check finds broken requirements or cannot run. Until 1.4.0 it required pip whatever was configured, which failed every uv-only venv (`uv venv` makes them without pip). Whether the Code Health tools venv is built is reported and never affects the exit.

**`audit`** checks the engine's freeze, so `--engine` decides what is audited: pip's `freeze --all` lists pip, setuptools and wheel, and uv's freeze does not. pip-audit runs from a venv of its own (`data\audit\`, built on first use; SECURITY §6) against PyPI's advisory service. Exit 1 on findings; a pip-audit that could not run exits on its problem's code instead (§5), so an empty report from a tool that never ran cannot read as clean. Until 1.4.0 it used the configured engine whatever `--engine` said.

**`engine`** with no argument reports both engines' availability and version, and needs no environment: uv is a standalone binary, and pip, a module of some interpreter, is reported unavailable without one. It does not say which engine is configured. With an argument it sets the configured engine — the setting the GUI shows — and refuses (PD-ENG-001) one that is not available here, because storing a preference that cannot be honoured moves the failure to the next command; setting pip needs an environment to look for it in. Until 1.4.0 the report failed in any directory without a `.venv`.

**`health --fix`** under `--yes` proceeds on a clean tree and refuses a dirty one (`error[PD-RES-002]: … uncommitted change(s) here; --yes will not rewrite files you have not committed. Commit or stash first.`, exit 2): the default over uncommitted work is *don't*, and the remedy is `git commit` or `git stash` rather than a flag. Outside a git repository, or without git, there is no tree to be dirty, and `--yes` proceeds. Off a TTY without `--yes` it refuses (exit 2), and `--fix` with a `--tool` list that leaves out ruff is refused before anything runs (`error[PD-PKG-002]: --fix needs ruff; it is the only tool that writes`, exit 2).

## 4. Interactive conflict handling (TTY)

`update`/`install` resolve, then ask about each held-back package before the preview is printed — on stderr, in both modes, like every prompt:

```text
requests  held back at 2.30.0 (latest 2.32.3)
  — apiclient 1.4 requires requests<2.31
  [C]ompatible (default)   [S]kip   [F]orce latest:
```

`s` drops the package from this plan, and `f` forces its latest and names what that breaks (`  this will break: apiclient`); anything else — Enter, `c`, even `skip` — keeps the compatible version. Any answer but *Compatible* re-resolves, and the questions repeat for whatever the new report holds back, for at most three rounds (`MAX_CONFLICT_ROUNDS`); after that the plan is confirmable as it stands. An impossibility is not asked about: its packages are skipped — and on a TTY, as the code stands, nothing says so, because the engine's explanation is printed only where nothing is asked (below). Then the preview, and the confirm, which is where a plan is abandoned:

```text
Apply 4 change(s)? [y/N]:
```

`y` or `yes` proceeds; anything else prints `aborted` and exits 2 — under `--json`, with the `ResolutionReport` as the document. `--dry-run` stops at the preview, but after the questions, because the decisions are part of what the report describes.

**With `--yes`, or without a TTY on stdin,** nothing is asked: a held-back package keeps its compatible version and an impossibility's packages are skipped. The impossibility is said first, on stderr — the engine's explanation, then `skipping: <names>` — so `every package was skipped` does not arrive unexplained. Off a TTY *without* `--yes` the confirm refuses rather than assuming: `error[PD-RES-002]: not a terminal; pass --yes to apply without confirming`, exit 2. (PD-RES-002 describes a stale preview; a code for "confirmation required" is the audit backlog's X17, ROADMAP.)

**`--strategy latest`** answers *Force latest* for every held-back package — only where nothing is asked, so with `--yes` or off a TTY. The report then carries `forced` (§6), and the preview a *Forced — will break* section naming each installed package whose requirement is overridden. On a TTY it changes nothing: each package is still a question, and Enter is still *Compatible*. Off a TTY nothing runs without `--yes` in any case, which makes `--yes` the "explicit acknowledgement" its help text describes; `--yes` without `--strategy latest` never forces. Under `--json --yes` no preview is printed at all, so a script that forces should read `forced` from a `--dry-run` first.

> **Corrected in 1.4.0.** This section said the preview came before the questions, offered an `[A]bort plan` answer, and had `--strategy latest` print its breakage warning before "a mandatory 3-second countdown (skippable with a second `-y`)". The binary has asked first since its first version, and has never had an abort answer — abandoning a plan is the final confirm's `N` — or a countdown; `--yes` is a bool, so there is no second `-y`, which `health --fix` already recorded. Nor did forcing force anything until 1.4.0: the decision was recorded in the request and no resolve read it.

## 5. Exit codes

| Code | Meaning |
|---|---|
| 0 | success, all steps ok — a dry run, a plan with nothing to change and `nothing-to-do` included |
| 1 | completed with per-package failures (see JSON `counts.failed`); **or `health` or `audit` found something** — a linter that exits 0 on findings is useless in a pre-commit hook, and `doctor` already uses this code for "found real problems"; or a failure coded PD-PKG-*, PD-BLD-*, PD-PRM-*, PD-SYS-* or PD-HLT-*, such as a malformed spec (PD-PKG-002) or a request this Python cannot install (PD-PKG-001) |
| 2 | plan aborted, nothing executed: resolution impossible and the skip policy removed everything (`everything-skipped`); uninstall guard tripped without `--force` (PD-RES-004); the confirm declined, or not askable — no TTY and no `--yes`; a stale or drifted preview (PD-RES-002); `health --fix` refused or declined; or a usage error — clap's own, `update` with neither names nor `--all`, `--fix` with a `--tool` list that leaves out ruff |
| 3 | environment error (PD-ENV-*, incl. PEP 668 block) |
| 4 | engine unavailable / version too old / stopped by its watchdog (PD-ENG-*) — outside a batch; inside one, a watchdog is a per-package failure (1) |
| 5 | snapshot failure — nothing was executed (PD-SNP-001); or no such snapshot (PD-SNP-002) |
| 6 | network/index error (PD-NET-*) |
| 10 | internal error (PD-INT-*); `log: <path>` on stderr, in both modes (§2) |
| 130 | stopped by Ctrl-C or Ctrl-Break during `install`, `update`, `uninstall` or `snapshot rollback`; see below |

An error's exit is derived from its code's area (`run::exit_for`) rather than written out per call site, so a new code cannot land on an undocumented number, and a test holds every code in `Code::ALL` to this table.

**Ctrl-C** — or Ctrl-Break, which a supervising process can send to one process group — during `install`, `update`, `uninstall` or `snapshot rollback` stops the run rather than the process. stderr says `stopping — press Ctrl-C again to quit now`, the flow's cancellation token trips, and the engine's whole process tree is killed (exec's job object). What completed is summarized — under `--json` the `ExecutionSummary`, with `"cancelled": true` — and stderr names the way back: `` stopped. What ran is above; `pipdock snapshot rollback <id>` puts back the state from before this run. `` Exit 130. Stopped during a resolve, nothing was planned or run: the document is `{"step":"nothing","reason":"cancelled"}`, stderr says `stopped before anything was planned; nothing ran`, exit 130. A **second** Ctrl-C exits 130 at once, with no summary, for a step that is not listening — a probe, a snapshot write. Every other command installs no handler, and Ctrl-C ends it the way it ends any console program: no summary, and not 130. Either way the engine's process tree goes with it. Until 1.4.0 that was true of every command, so Ctrl-C mid-batch printed nothing and said nothing about the snapshot just written (X6).

## 6. JSON contracts

**`--json` prints exactly one JSON document on stdout per command, and nothing else goes there.** Prompts, the preview shown before a prompt, warnings, progress prose and the engine's streamed output go to stderr — in both modes — and so does the log under `--verbose` (§2). Each document is the serde-serialized core type where one exists, the structs the GUI receives, pretty-printed; `pipdock schema <type>` prints the JSON Schema generated from the Rust type, so scripts can pin against it, and an unknown name lists the types it knows (PD-PKG-002, exit 1).

| Command | The document |
|---|---|
| `install`, `update` | The `ResolutionReport` when nothing runs — `--dry-run` or a plan with no changes (exit 0), a confirm declined or not askable (exit 2). The `ExecutionSummary` when something runs (0, 1 or 130). A `FlowStep` when there is nothing to plan: `{"step":"nothing","reason":…}` with `nothing-to-do` (0), `everything-skipped` (2) or `cancelled` (130) |
| `uninstall` | The `GuardReport` when the guard trips without `--force` (exit 2); otherwise the `ExecutionSummary`. With `--force`, the summary alone |
| `snapshot rollback` | The `RollbackPreview` when nothing runs — the environment already matches (0), or the confirm was declined or not askable (2); otherwise the `ExecutionSummary` |
| `snapshot create` | The new snapshot's `SnapshotMeta` |
| `snapshot list` | `[SnapshotMeta]` |
| `snapshot diff` | `Diff`, as a rollback with this engine would see it; the lines no index can restore are printed in text mode only |
| `env list` | An array, one object per discovered interpreter: `interpreter`, `source`, `python`, `externallyManaged`, `hiddenUserSite`, `packages` (a count), `envHash`, `pipVersion` — or `interpreter`, `source` and `error: {code, message}` for one that could not be probed, without failing the rest |
| `env use` | The `PyEnv` that is now the default |
| `list` | `[Dist]`, from the probe; with `--outdated`, `[OutdatedDist]` |
| `search` | `[Hit]` |
| `info` | `{meta: PackageMeta, freshness: Freshness}` |
| `pin list` | `[Pin]` |
| `pin add` | The `Pin` written — always `"mode": "exclude"` |
| `pin remove` | `{pkg, removed}`; `removed` is false, and the exit still 0, when it was not pinned |
| `doctor` | `{interpreter, python, envHash, externallyManaged, hiddenUserSite, engines: {pip, uv}, check, toolsVenv: {path, need, manifest}}`; `check` is null when the check could not run |
| `audit` | `AuditReport` |
| `health` | `HealthReport`; with `--fix`, the `FixReport` alone when a fix ran — see below |
| `pip-upgrade` | `{before, after}`, each pip's `EngineInfo` |
| `engine` | `[EngineInfo, EngineInfo]`, pip then uv |
| `engine <pip\|uv>` | The `EngineInfo` of the engine just set |
| `index refresh` | `RefreshReport` |
| `tools sync` | `{"state": "fresh"}` when the venv already matched (without `--force`); otherwise the venv's manifest after the sync |
| `tools status` | `{path, need, manifest}` |
| `schema <type>` | The JSON Schema, printed with or without `--json` |
| `self report-bug` | `{url}` |

**A failure prints the envelope and only the envelope** — `{"code":"PD-ENV-001","message":"…","stderrTail":null}`, on one line, with the exit §5 gives its code; `stderrTail` is the engine's stderr tail when there was one. Without `--json` the same failure is `error[PD-ENV-001]: …` on stderr.

**What prints no document.** Usage errors — clap's own, `update` with neither names nor `--all`, `--fix` with a `--tool` list that leaves out ruff — say so on stderr and exit 2 with stdout empty. `health --fix` prints the `FixReport` alone when a fix ran, because the pre-fix report describes a state that no longer exists by the time the command returns; when no fix runs — nothing was safely fixable (exit 0 or 1, by the findings), or the fix was refused or declined (exit 2) — it prints nothing, the report having been withheld for a `FixReport` that never came. A second Ctrl-C (§5) exits wherever it is.

**What 1.4.0 added and removed.** `ResolutionReport` always carries `changes` and `heldBack`; `impossible` is omitted when there is none, and the three fields 1.4.0 added — `incompatible`, `forced`, `yanked` — when they are empty, so a report that needs none of them reads as it did, less `raw`. `incompatible` (PD-PKG-001) is what was left out because its `Requires-Python` excludes this interpreter — `{pkg, version, requiresPython, python}`, structured so a head can set the range against the version rather than print a sentence (hard invariant 4). `forced` is each package *Force latest* moves, in `heldBack`'s shape: `latest` is the version it is forced to, and `blockers` are the installed packages whose requirement it breaks — DISCLAIMER §2's *Will break* rows, before anything runs. `yanked` (PD-PKG-003) is each withdrawn release the plan would install, with upstream's `reason` when it gave one: a warning rather than a failure, since both engines install them — and only the JSON carries it, because the text preview does not print it. `raw` left the wire: it was pip's whole `--report` document, every package's README included, and nothing read it. In `ExecutionSummary`, a row's `from` is the version it moved from — until 1.4.0 always null, so every upgrade read as a fresh install — `check.ok` is true only when the post-run check **ran** and found nothing, with `check.error` carrying the code when it could not run (a case that used to read `ok: true`), and `cancelled` is §5's Ctrl-C.

> **Settled in 1.4.0.** This section promised NDJSON for the streaming commands — events matching the GUI's `plan-progress` payloads, "terminated by a final `summary` object" — and from 2026-08-12 recorded that no command emitted any. None ever did. What the binary printed instead was not one document either: `install` and `update` printed the `ResolutionReport` and then the `ExecutionSummary`, so `pipdock install x --json | ConvertFrom-Json` failed on the second; `uninstall --force` printed the `GuardReport` before its summary; `snapshot rollback` printed its preview as prose; and `engine`, `env use`, `pin add`, `pin remove`, `snapshot create`, `pip-upgrade` and `self report-bug` ignored `--json` altogether (X5). The rule is now one document per command, and the table above is the contract. `--help` still describes `--json` as "NDJSON for streaming commands"; that line is wrong.

`GuardReport.breaks` maps each package being removed to the installed packages that would break, and each of those carries the specifier that says so: `{"numpy": [{"pkg": "pandas", "version": "2.1.4", "constraint": "<2,>=1.26.0"}]}`. `constraint` is the **bare specifier tail** — the distribution name is the map key, so a caller joins the two halves itself; `version` is omitted when the graph does not know it, and `constraint` is empty for an unconstrained dependency. DATA-FLOW §5's dialog needs the specifier to say *"Removing X breaks Y (requires X>=1)"*, and a bare list of names tells the user what will break but not whether they can live with it.

`Dist.sizeBytes` is present only when it can be known: it is summed from the distribution's RECORD manifest, which `.egg-info` distributions do not have and which a PEP 660 editable install fills with its import shim rather than its sources. The field is **omitted** in those cases rather than reported as `0`, and it is always omitted on a `Dist` that came from `<engine> list` — neither engine reports a size. Where present it is a lower bound: uncompressed bytes as recorded at install time, excluding `__pycache__` written afterwards. `pipdock list`'s human table does not show it, as it already omits `requiresDist` and `requiresPython`.

## 7. Examples

```bash
# Nightly maintenance of a bot venv (task scheduler), safe strategy, log kept:
pipdock update --all --env C:\bots\scraper\.venv --yes --json --log-file C:\logs\pd.log > C:\logs\pd-summary.json

# Audit what an upgrade would do without touching anything:
pipdock update pandas numpy --dry-run --json | jq '.heldBack'

# Refuse-to-break uninstall in CI (exit 2 — plan aborted — if the guard trips):
pipdock uninstall legacylib --json || echo "dependents exist, aborting"
```

The log file holds log lines, not JSON; the summary is the document on stdout. `heldBack` is camelCase, like every key on the wire; this example used to say `.held_back`, which `jq` answers with `null`.
