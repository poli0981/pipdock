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
                "requires_python": ">=3.8", "size_bytes": 131072}]}

Any failure prints {"error": ...} on stdout and exits non-zero, which the core
classifies as PD-ENV-003.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import sysconfig

SCHEMA_VERSION = 2


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

    **A virtual environment is never externally managed**, and returning early is
    not an optimisation. A venv has no stdlib of its own: `sysconfig.get_path`
    resolves to the *base* installation, so every venv created from a Python that
    ships the marker -- uv-managed builds, Debian, Homebrew, Fedora -- reported
    itself as managed and PipDock refused to mutate it. Found by making a venv
    from a uv-managed 3.11 and watching `pip-upgrade` refuse with PD-ENV-002 on
    an environment whose whole purpose is being safe to install into.

    This mirrors pip's own `check_externally_managed`, which opens with the same
    `running_under_virtualenv()` early return before it looks for the file. The
    `platstdlib` candidate below is a deliberate superset of pip's single
    `stdlib` check; on Windows they resolve to the same directory.
    """
    if sys.prefix != sys.base_prefix:
        return False

    candidates = []
    for key in ("stdlib", "platstdlib"):
        try:
            path = sysconfig.get_path(key)
        except (KeyError, OSError):
            continue
        if path:
            candidates.append(os.path.join(path, "EXTERNALLY-MANAGED"))
    return any(os.path.isfile(p) for p in candidates)


def _hidden_user_site() -> str | None:
    """Return the user site-packages path when it exists but is NOT on this run's sys.path.

    Owner decision 2026-07-27 (spike SP-6): PipDock keeps running the probe with -I, because
    isolated mode is what stops a poisoned environment injecting code into it (SECURITY section 2).
    The cost is that -I also disables user site-packages, so on a non-venv system Python the probe
    reports fewer distributions than `pip list` does -- measured at 352 vs 375 on the dev machine.

    Rather than warn always, report the path only when packages really are being hidden, so the UI
    can show an accurate note instead of a permanent disclaimer. Returns None inside a venv (no
    user site applies) and None when the directory is absent or empty.
    """
    if sys.prefix != sys.base_prefix:
        return None  # venv: user site does not apply
    try:
        import site

        user_site = site.getusersitepackages()
    except Exception:  # noqa: BLE001 - absence of a user site is not an error
        return None
    if not user_site or not os.path.isdir(user_site):
        return None
    if user_site in sys.path:
        return None  # not hidden: it is already being read
    try:
        if not os.listdir(user_site):
            return None  # exists but empty: nothing is being hidden
    except OSError:
        return None
    return user_site


def _is_editable(dist) -> bool:
    """True for a PEP 660 editable install.

    Such a distribution has a perfectly valid RECORD -- it just lists the import shim and a
    .pth file rather than the package, so summing it reports a few hundred bytes for a project
    of any size. `direct_url.json`'s `dir_info.editable` is the marker pip and uv both write.
    """
    try:
        raw = dist.read_text("direct_url.json")
    except Exception:  # noqa: BLE001 - absence is the common case, not an error
        return False
    if not raw:
        return False
    try:
        info = json.loads(raw)
    except ValueError:
        return False
    return bool(isinstance(info, dict) and info.get("dir_info", {}).get("editable"))


def _size_bytes(dist) -> int | None:
    """Installed size in bytes, summed from the RECORD manifest.

    RECORD is read and parsed directly rather than through `Distribution.files`. That is not
    premature: `files` constructs a PackagePath object per installed file, and on this machine's
    352-package system Python it took the probe from 551 ms to 5,492 ms -- a 10x regression on a
    code path the Installed screen runs every time an environment is opened. Parsing the text
    ourselves costs 29 ms for the same answer.

    RECORD is CSV of `path,hash,size`. The path may be quoted and contain commas; the trailing
    two fields never do, so splitting from the right is both correct and cheap.

    Returns None rather than 0 whenever the number would be a lie, because a wrong size is
    worse than an absent one:

    * `.egg-info` distributions have no RECORD -- their SOURCES.txt carries no sizes;
    * editable installs, per _is_editable above;
    * anything whose metadata cannot be read at all.

    Even when present the number is a lower bound: it is uncompressed installed bytes as
    recorded at install time, and excludes __pycache__ written afterwards.
    """
    try:
        record = dist.read_text("RECORD")
    except Exception:  # noqa: BLE001 - unreadable metadata is reported as unknown, not fatal
        return None
    if not record:
        return None

    total = 0
    measured = False
    for line in record.splitlines():
        parts = line.rsplit(",", 2)
        if len(parts) != 3:
            continue
        size = parts[2].strip()
        if size.isdigit():
            total += int(size)
            measured = True
    if not measured:
        return None
    return None if _is_editable(dist) else total


def _dists() -> list[dict[str, object]]:
    """Read installed distribution metadata.

    A single unreadable distribution must not fail the whole probe -- broken
    metadata in one package is common and the user still needs the rest of the
    list. Such entries are reported with an `error` field instead.

    Distributions are deduplicated by normalized name, **first occurrence winning**.
    `importlib.metadata.distributions()` walks sys.path in order and yields one entry per
    metadata directory it finds, so a package discoverable twice is reported twice -- most
    commonly after `pip install -e`, where the project's own src/<name>.egg-info joins the
    venv's .dist-info via the .pth file. `pip list` shows such a package once, and so must
    PipDock: a duplicate row means an ambiguous key when the Installed table joins against
    the outdated set, and two rows the user cannot tell apart. First-wins matches sys.path
    precedence, which is the copy an `import` would actually resolve to.
    """
    from importlib.metadata import distributions

    out: list[dict[str, object]] = []
    seen: set[str] = set()
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
            name = _normalize(raw_name)
            if name in seen:
                continue
            seen.add(name)
            out.append(
                {
                    "name": name,
                    "version": dist.version or "",
                    "requires_dist": list(dist.requires or []),
                    "requires_python": meta.get("Requires-Python"),
                    "size_bytes": _size_bytes(dist),
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
            # Non-null means -I is hiding packages that pip would list; the UI shows a note.
            "hidden_user_site": _hidden_user_site(),
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
