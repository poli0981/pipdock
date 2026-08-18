# pip-audit capture

SP-4's exit criterion asked for "command line + JSON fixture pinned". The command line was pinned
in prose in `spikes/README.md` in Phase 0; **the fixture never was**, and P1-1 needed it.

Captured 2026-08-18 with the ledger's pinned **pip-audit 2.10.1**, against the spike's own target
environment (`spikes/out/target-env`, a Python 3.12 venv holding `urllib3==2.0.0`), by the exact
invocation SECURITY §6 and SP-4 settled on:

```
pip-audit -r spikes/out/freeze.txt --no-deps -f json
```

Exit code **1** — findings, not failure.

Not redacted, and it does not need to be: unlike the pip/uv captures there is no path, no
duration, no address and no environment marker in it. The only `://` in the file is inside the
advisory prose. `spikes/capture.py` does not produce this, for the same reason it does not produce
`health/sample` — reconstructing the seeded environment is the expensive part, not the run.

## What it pins, and why each matters

* **`PYSEC-*` ids.** pip-audit's *default* vulnerability service is PyPI, not OSV. That is what
  lets PipDock keep `legal/PRIVACY-POLICY.md` §3's "exactly one destination" true.
* **`aliases` carries the `CVE-*` and `GHSA-*`.** PRD P1-1 says "known CVEs"; the CVE is an alias,
  never the primary id.
* **Duplicate ids.** `PYSEC-2023-192` and `PYSEC-2023-212` each appear **twice**, so the ten rows
  here are **eight** advisories. The same run under `--vulnerability-service osv` returns sixteen
  rows and the *same eight* — the set difference is empty in both directions, so the service
  choice costs no coverage and dedup is load-bearing either way.
* **No severity field, under either service.** The keys are exactly `id`, `fix_versions`,
  `aliases` and `description`; pip-audit's own service model has no severity at all. PRD P1-1's
  "severity-sorted" and SECURITY §6's "(CVE/GHSA id, severity, fixed-in)" were both written
  against a field that does not exist, and P1-1 corrects them rather than inventing one.

`urllib3-2.0.0.stderr.txt` is the companion: pip-audit writes a `pip-compile`/hashes advisory to
stderr on **every** run, which must never be read as failure.
