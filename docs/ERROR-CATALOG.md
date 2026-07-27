# PipDock — Error Catalog

*Version 0.1 · 2026-07-17. Single source of truth: `crates/pipdock-core/src/errors/catalog.rs`. Every user-visible failure carries exactly one code; classifiers run over engine stderr in priority order (first match wins), falling back to `PD-ENG-999`.*

## 1. Code space

`PD-<AREA>-<NNN>` — areas: `ENV` environment, `ENG` engine, `RES` resolution, `BLD` build, `PKG` package/index, `NET` network, `PRM` permissions, `SNP` snapshot, `SYS` system, `HLT` health, `INT` internal.

## 2. Catalog (v1 launch set)

| Code | Detection (stderr/context pattern) | Cause | User action (EN; VI mirrored in locale files) |
|---|---|---|---|
| PD-ENV-001 | interpreter path missing/not executable | env deleted or moved | Re-scan environments; remove stale entry |
| PD-ENV-002 | `externally-managed-environment` / `EXTERNALLY-MANAGED` marker | PEP 668 protected Python | Use a virtual env; system override only via Settings (discouraged) |
| PD-ENV-003 | probe.py non-zero / unparseable | broken env metadata | Open logs; recreate the venv |
| PD-ENG-001 | engine binary not found | pip/uv missing | Install engine or switch engine in Settings |
| PD-ENG-002 | pip < 22.2 with plan requested | no `--dry-run --report` | One-click "Upgrade pip" then retry |
| PD-ENG-003 | uv output shape unrecognized | uv newer than adapter | Update PipDock; temporarily switch engine to pip |
| PD-RES-001 | `ResolutionImpossible` / uv equivalent | constraints cannot be satisfied | Per-package Skip/Force choices in the preview |
| PD-RES-002 | plan stale (>10 min) or env drift hash mismatch | env changed since preview | Re-run preview |
| PD-BLD-001 | `Microsoft Visual C++ 14.0 or greater is required` | missing MSVC build tools | Install VS Build Tools, or prefer a wheel-providing version |
| PD-BLD-002 | `error in pyproject.toml` / `Backend … failed` / `metadata-generation-failed` | broken sdist build backend | Try previous version; report upstream (owner's example case) |
| PD-BLD-003 | `error: Microsoft Visual C++`-absent + `Failed building wheel` generic | sdist-only build failed | Same as above; details in log |
| PD-PKG-001 | `No matching distribution found` + requires-python mismatch in metadata | package needs different Python | Shows required range vs env version |
| PD-PKG-002 | `No matching distribution found` (plain) | name/version typo or yanked | Check spelling/version; `pipdock info <pkg>` |
| PD-PKG-003 | `File was already yanked` / yanked marker | yanked release requested | Pick a non-yanked version |
| PD-PKG-004 | hash mismatch / `THESE PACKAGES DO NOT MATCH THE HASHES` | corrupted download | Purge engine cache (P1 UI; CLI: engine cache purge) and retry |
| PD-NET-001 | timeout / `Connection aborted` / DNS failure | offline or PyPI unreachable | Offline banner; retry; search stays local |
| PD-NET-002 | TLS/SSL verification failure | proxy/AV interception | Corporate proxy note; never suggests disabling verification |
| PD-NET-010 | index refresh failed | PEP 691 fetch error | Retry later; stale index still searchable (staleness shown) |
| PD-NET-011 | tools venv bootstrap fetch failed | offline during Health setup | Health disabled until online |
| PD-PRM-001 | `PermissionError` writing site-packages | admin-owned Python (Program Files) | v1 blocks: use a venv (elevation is P2-3) |
| PD-PRM-002 | file lock (`WinError 32`) | package in use by a running process | Close Python processes using the env; retry |
| PD-SNP-001 | snapshot write failed pre-execution | disk/permission issue on app data | **Plan aborted, nothing executed**; free space / check AV |
| PD-SNP-002 | rollback target release unavailable on PyPI | deleted/yanked upstream | Partial rollback offered; affected pins listed |
| PD-SYS-001 | `path too long` patterns | MAX_PATH without long-path opt-in | Enable Windows long paths (help link) |
| PD-SYS-002 | disk full (`No space left`/`WinError 112`) | storage | Free space |
| PD-HLT-001..003 | tool missing / non-zero / watchdog timeout | tools venv issue | Re-sync tools env; partial report shown |
| PD-INT-001 | panic/unexpected | PipDock bug | Report-bug deep link prefilled |
| PD-ENG-999 | unclassified engine failure | unknown | stderr tail shown; report-bug encouraged (feeds catalog growth) |

Classifier hygiene: patterns live beside unit tests with captured real stderr fixtures (collected in spike SP-2 and grown from bug reports); a fixture must exist for every code before it ships.

## 3. Presentation rules

GUI: `code · localized one-liner · [Details ⌄ stderr tail ≤ 40 lines] · [Copy full log] · [Report bug]`. CLI: `error[PD-XXX-NNN]: <one-liner>` on stderr; `--json` embeds `{code, reason, stderrTail}`. Localization: only the one-liner and action text are localized; codes and stderr never are.

## 4. Bug-report pipeline (owner requirement: console error attached to template)

1. App keeps a per-plan ring buffer (last 64 KB) of engine stdout/stderr plus the app log tail.
2. **Report bug** builds a prefilled GitHub new-issue URL against `.github/ISSUE_TEMPLATE/bug_report.yml`:
   `https://github.com/poli0981/pipdock/issues/new?template=bug_report.yml&pd-version=…&os=…&engine=…&python=…&error-code=…&log-excerpt=…`
3. URL budget: GitHub rejects very long URLs, so `log-excerpt` is truncated to ≈ 6 000 characters (tail-biased); the full log is simultaneously copied to the clipboard and the dialog says so ("paste into the issue if asked").
4. Nothing is ever sent automatically — the user reviews the issue form in their browser. This is the entire "telemetry" story.
