# PipDock — CLI Specification

*Version 0.1 · 2026-07-17 · Binary: `pipdock` (clap, from `pipdock-cli` crate, same `pipdock-core` as the GUI)*

## 1. Principles

1. **Same core, same behavior.** Every CLI command maps 1:1 onto the core functions the GUI uses; the CLI adds no logic of its own.
2. **Safe by default, scriptable on request.** Interactive prompts appear on a TTY; in scripts (`--yes` / no TTY), conflicts default to **skip**, never force.
3. **Machine-readable everywhere.** `--json` on every read/report command emits the same structs the GUI receives.
4. English-only in v1 (GUI carries EN/VI); messages reuse catalog codes so they stay greppable.

## 2. Global options

```text
--env <path>        interpreter or env dir (default: last used / auto-detected .venv in CWD)
--engine <pip|uv>   override configured engine for this invocation
--json              machine-readable output (NDJSON for streaming commands)
--yes / -y          assume defaults on all prompts (conflicts → skip)
--quiet / --verbose log level; --log-file <path> to tee
--no-snapshot       DANGEROUS: skip pre-batch snapshot (CI images only; prints warning)
```

## 3. Commands

```text
pipdock env list                         # discovered envs, sources, PEP 668 flags
pipdock env use <path>                   # set default env
pipdock list [--outdated]                # installed table; dims/badges become columns in --json
pipdock search <query> [--limit n]       # local fuzzy index search
pipdock info <pkg>                       # cached PyPI metadata
pipdock install <spec...> [--dry-run]    # spec = name[==version]; dry-run prints the ResolutionReport
pipdock update [--all | <pkg...>]
        [--strategy compatible|latest]   # latest == force; requires --yes off-TTY acknowledgement
        [--except <pkg,...>]             # ad-hoc exclusions on top of pins
        [--dry-run]
pipdock uninstall <pkg...> [--force]     # guard names each dependent and its constraint; --force overrides
pipdock pin add|remove <pkg> [--reason "…"] | pin list
pipdock snapshot list | create | diff <id> | rollback <id|latest>
pipdock doctor                           # engine check + env sanity
pipdock audit                            # known advisories in the selected environment; exit 1 on findings
pipdock health [--path <dir>] [--tool deptry|vulture|ruff] [--fix]   # --fix = ruff only, prompts
                                                                    # --yes: proceeds on a clean tree,
                                                                    # REFUSES on a dirty one (exit 2)
pipdock pip-upgrade                      # upgrade pip inside --env (pip engine paths only)
pipdock engine [pip|uv]                  # show or set configured engine
pipdock index refresh                    # re-pull PEP 691 name index
pipdock tools sync [--force] [--python <path>]   # build/re-sync the Code Health tools venv
pipdock tools status                     # where it is, and whether it matches the shipped pins
pipdock self report-bug                  # prints prefilled GitHub issue URL (ERROR-CATALOG §4)
```

## 4. Interactive conflict handling (TTY)

`update`/`install` render the preview, then per needs-decision package:

```text
requests  held back at 2.30.0 (latest 2.32.3) — blocked by apiclient 1.4 (requires <2.31)
  [C]ompatible (default)   [S]kip   [F]orce latest   [A]bort plan
```

`--yes` answers `C` for held-back and `S` for impossible. `--strategy latest` pre-answers `F` and prints the breakage warning before a mandatory 3-second countdown (skippable with a second `-y`).

## 5. Exit codes

| Code | Meaning |
|---|---|
| 0 | success, all steps ok |
| 1 | completed with per-package failures (see JSON `counts.failed`); **or `health` found something** — a linter that exits 0 on findings is useless in a pre-commit hook, and `doctor` already uses this code for "found real problems" |
| 2 | plan aborted, nothing executed (resolution impossible & user/skip policy removed everything; uninstall guard tripped without `--force`, PD-RES-004) |
| 3 | environment error (PD-ENV-*, incl. PEP 668 block) |
| 4 | engine unavailable / version too old (PD-ENG-*) |
| 5 | snapshot failure — nothing was executed (PD-SNP-001) |
| 6 | network/index error (PD-NET-*) |
| 10 | internal error (PD-INT-*; log path printed) |
| 130 | user cancelled (Ctrl-C; child processes reaped, partial summary printed) |

## 6. JSON contracts

`--json` payloads are the serde-serialized core types (`Dist`, `OutdatedDist`, `ResolutionReport`, `ExecutionSummary`, `CheckReport`, `GuardReport`, `HealthReport`) — schema documented by `pipdock schema <type>` which prints the JSON Schema generated from the Rust types, so scripts can pin against it. Streaming commands (`update`, `install`, `health`) with `--json` emit NDJSON events matching the GUI's `plan-progress` payloads, terminated by a final `summary` object.

> **Not implemented, recorded 2026-08-12 (Phase 3 · P2).** No command in the binary emits NDJSON. `update`, `install`, `uninstall` and `tools sync` all stream engine output as plain lines to **stderr** and print a single final object to stdout; `health` does the same since P3, and `health --fix` prints one document — the `FixReport`, not the pre-fix report, which describes a state that no longer exists by the time the command returns. Either this paragraph or four commands are wrong, and closing it means changing the output shape of every streaming command at once — so it is its own slice, not a thing to half-do while adding a fifth. `tools sync` matches the existing behaviour deliberately rather than becoming the one command that is inconsistent with the other four.

`GuardReport.breaks` maps each package being removed to the installed packages that would break, and each of those carries the specifier that says so: `{"numpy": [{"pkg": "pandas", "version": "2.1.4", "constraint": "<2,>=1.26.0"}]}`. `constraint` is the **bare specifier tail** — the distribution name is the map key, so a caller joins the two halves itself; `version` is omitted when the graph does not know it, and `constraint` is empty for an unconstrained dependency. DATA-FLOW §5's dialog needs the specifier to say *"Removing X breaks Y (requires X>=1)"*, and a bare list of names tells the user what will break but not whether they can live with it.

`Dist.sizeBytes` is present only when it can be known: it is summed from the distribution's RECORD manifest, which `.egg-info` distributions do not have and which a PEP 660 editable install fills with its import shim rather than its sources. The field is **omitted** in those cases rather than reported as `0`, and it is always omitted on a `Dist` that came from `<engine> list` — neither engine reports a size. Where present it is a lower bound: uncompressed bytes as recorded at install time, excluding `__pycache__` written afterwards. `pipdock list`'s human table does not show it, as it already omits `requiresDist` and `requiresPython`.

## 7. Examples

```bash
# Nightly maintenance of a bot venv (task scheduler), safe strategy, log kept:
pipdock update --all --env C:\bots\scraper\.venv --yes --json --log-file C:\logs\pd.json

# Audit what an upgrade would do without touching anything:
pipdock update pandas numpy --dry-run --json | jq '.held_back'

# Refuse-to-break uninstall in CI (exit 2 — plan aborted — if the guard trips):
pipdock uninstall legacylib --json || echo "dependents exist, aborting"
```
