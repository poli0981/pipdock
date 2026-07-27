"""PipDock environment probe.

Prints a single JSON document describing the interpreter it runs under and every
distribution installed in it. Rust cannot cheaply read a foreign environment's
metadata, so this helper is executed as:

    <env-python> -I <tempfile> --json

Constraints (docs/ARCHITECTURE.md section 4, docs/SECURITY.md section 2):

* Standard library only. No third-party imports, ever -- the environments this
  runs in are the user's, and may contain anything.
* Python 3.10 floor: `importlib.metadata` only, no `packaging`.
* Run with -I so PYTHONPATH and user site are ignored and a poisoned environment
  cannot inject code into the probe.
* Written to a temp file with a random name per invocation; never installed.

Output shape:

    {"python": "3.12.4", "prefix": "...", "externally_managed": false,
     "dists": [{"name": "requests", "version": "2.32.3",
                "requires_dist": ["urllib3<3,>=1.21.1"],
                "requires_python": ">=3.8"}]}

Any failure prints {"error": ...} on stdout and exits non-zero, which the core
classifies as PD-ENV-003.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import sysconfig

SCHEMA_VERSION = 1


def _normalize(name: str) -> str:
    """PEP 503 normalization, mirrored in model.rs so both sides agree on identity."""
    out: list[str] = []
    prev_sep = False
    for ch in name:
        if ch in "-_.":
            if not prev_sep:
                out.append("-")
            prev_sep = True
        else:
            out.append(ch.lower())
            prev_sep = False
    return "".join(out)


def _externally_managed() -> bool:
    """Detect the PEP 668 marker.

    The marker sits next to the stdlib in the `stdlib` scheme path. Its mere
    presence is what pip keys off, so PipDock does the same -- contents are not
    parsed. Drives PD-ENV-002.
    """
    candidates = []
    for key in ("stdlib", "platstdlib"):
        try:
            path = sysconfig.get_path(key)
        except (KeyError, OSError):
            continue
        if path:
            candidates.append(os.path.join(path, "EXTERNALLY-MANAGED"))
    return any(os.path.isfile(p) for p in candidates)


def _dists() -> list[dict[str, object]]:
    """Read installed distribution metadata.

    A single unreadable distribution must not fail the whole probe -- broken
    metadata in one package is common and the user still needs the rest of the
    list. Such entries are reported with an `error` field instead.
    """
    from importlib.metadata import distributions

    out: list[dict[str, object]] = []
    for dist in distributions():
        try:
            meta = dist.metadata
            if meta is None:
                continue
            # Use .get(), not subscripting: on Python 3.14 a missing header returns None with a
            # DeprecationWarning ("Implicit None on return values is deprecated") and is slated to
            # raise KeyError. The warning goes to stderr and would otherwise pollute every probe
            # run against a 3.14 environment.
            raw_name = meta.get("Name")
            if not raw_name:
                continue
            out.append(
                {
                    "name": _normalize(raw_name),
                    "version": dist.version or "",
                    "requires_dist": list(dist.requires or []),
                    "requires_python": meta.get("Requires-Python"),
                }
            )
        except Exception as exc:  # noqa: BLE001 - one bad dist must not sink the probe
            out.append({"name": None, "error": f"{type(exc).__name__}: {exc}"})

    out.sort(key=lambda d: (d.get("name") or ""))
    return out


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="PipDock environment probe")
    parser.add_argument(
        "--json",
        action="store_true",
        help="emit JSON (the only supported mode; accepted for explicitness)",
    )
    parser.parse_args(argv)

    try:
        payload = {
            "schema": SCHEMA_VERSION,
            "python": ".".join(str(p) for p in sys.version_info[:3]),
            "implementation": sys.implementation.name,
            "prefix": sys.prefix,
            "base_prefix": sys.base_prefix,
            "executable": sys.executable,
            "is_venv": sys.prefix != sys.base_prefix,
            "externally_managed": _externally_managed(),
            "dists": _dists(),
        }
    except Exception as exc:  # noqa: BLE001 - the core turns this into PD-ENV-003
        json.dump({"error": f"{type(exc).__name__}: {exc}"}, sys.stdout)
        sys.stdout.write("\n")
        return 1

    json.dump(payload, sys.stdout, ensure_ascii=False)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
