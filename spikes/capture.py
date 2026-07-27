"""Fixture capture harness for spikes SP-1 and SP-2.

Builds a disposable venv per scenario, runs the engine commands PipDock will run, and writes
stdout / stderr / exit code to `crates/pipdock-core/tests/fixtures/<engine>/<scenario>/`.

Why a harness rather than ad-hoc runs: `docs/TESTING.md` §1 makes adapter parsing the thing that
must never regress, and requires at least one captured fixture per error-catalog code. Those
fixtures have to be reproducible when a new pip or uv ships, so the capture has to be a script
that CI can re-run (RELEASE-CI §1, the weekly latest-engine parser job).

Usage:
    py -3.12 spikes/capture.py --engine pip
    py -3.12 spikes/capture.py --engine uv --uv-exe spikes/.venv-uv-latest/Scripts/uv.exe
    py -3.12 spikes/capture.py --list
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import venv
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
FIXTURES = REPO / "crates" / "pipdock-core" / "tests" / "fixtures"

# Deliberately small, pure-Python, long-published packages so the captures stay reproducible.
# httpx 0.23.0 pins httpcore>=0.15.0,<0.16.0, which is the cleanest held-back construction:
# httpcore's latest is 1.x, so the resolver must hold it two majors back.


@dataclass
class Scenario:
    """One captured situation: seed an environment, then run a command against it."""

    name: str
    #: Installed before the captured command runs. Not captured itself.
    seed: list[str] = field(default_factory=list)
    #: Requirements passed to the captured `install` command.
    target: list[str] = field(default_factory=list)
    #: True for the flows PipDock previews rather than executes.
    dry_run: bool = True
    #: Human-readable statement of what this fixture is evidence for.
    question: str = ""
    #: Error-catalog code this fixture is expected to feed, when it is an error case.
    expects_code: str | None = None
    #: Set False only to capture what happens WITHOUT the UTF-8 mitigation (see SAFE_ENV).
    utf8_io: bool = True


# pip's `--report -` writes JSON through its vendored rich, which on Windows goes via
# legacy_windows_render and the console codec. Under cp1252 that raises UnicodeEncodeError the
# moment any report field contains a non-ASCII character, and pip exits 2 with no report at all.
# Confirmed on pip 25.0.1 AND pip 26.1.2 (spikes/README.md, SP-2). Forcing UTF-8 on the child's
# stdio is the mitigation, and it is mandatory for every pip invocation PipDock makes.
SAFE_ENV = {"PYTHONIOENCODING": "utf-8", "PYTHONUTF8": "1"}


SCENARIOS: list[Scenario] = [
    Scenario(
        name="clean-upgrade",
        seed=["idna==3.4"],
        target=["idna"],
        question="Baseline: what does an unconstrained upgrade plan look like?",
    ),
    Scenario(
        name="held-back",
        seed=["httpx==0.23.0"],
        target=["httpcore"],
        question=(
            "SP-1 core question: httpx 0.23.0 requires httpcore<0.16, so httpcore cannot reach "
            "its 1.x latest. Does the engine name httpx as the blocker, or only report the "
            "resolved version? ANSWER: neither — both engines ignore the installed httpx "
            "entirely and plan httpcore 1.0.9, silently breaking it. Exit 0."
        ),
    ),
    Scenario(
        name="held-back-constrained",
        seed=["httpx==0.23.0"],
        # The same upgrade, but with the installed distribution restated as a requirement.
        target=["httpcore", "httpx==0.23.0"],
        question=(
            "SP-1 follow-up: does restating the installed set as explicit requirements make the "
            "resolver hold httpcore back to 0.15.x instead of breaking httpx? This decides "
            "whether PipDock must pass the full installed set on every plan."
        ),
    ),
    Scenario(
        name="report-encoding-crash",
        seed=["idna==3.4"],
        target=["idna"],
        utf8_io=False,
        question=(
            "Evidence for the pip --report UnicodeEncodeError on Windows/cp1252. Kept as a "
            "fixture so a regression in the UTF-8 mitigation is recognisable."
        ),
    ),
    Scenario(
        name="impossible",
        seed=["httpx==0.23.0"],
        target=["httpcore>=1.0", "httpx==0.23.0"],
        question="Unsatisfiable set: how is ResolutionImpossible reported and is it attributable?",
        expects_code="PD-RES-001",
    ),
    Scenario(
        name="not-found",
        seed=[],
        target=["pipdock-no-such-package-9f3a2b"],
        question="Unknown distribution name.",
        expects_code="PD-PKG-002",
    ),
    Scenario(
        name="requires-python",
        seed=[],
        # scipy 1.7.3 declares Requires-Python >=3.7,<3.11 and ships wheels, so the resolver
        # rejects it on metadata rather than attempting a build.
        target=["scipy==1.7.3"],
        question="Requires-Python mismatch against the running interpreter.",
        expects_code="PD-PKG-001",
    ),
    Scenario(
        name="yanked",
        seed=[],
        # urllib3 2.0.0 is yanked upstream. PipDock must surface the yank rather than proceed.
        target=["urllib3==2.0.0"],
        question=(
            "Yanked release explicitly requested. Does the engine flag the yank, and where — "
            "structured field or stderr text?"
        ),
        expects_code="PD-PKG-003",
    ),
    Scenario(
        name="build-backend-failure",
        seed=[],
        # numpy 1.19.5 is sdist-only for this interpreter, and a modern venv ships no setuptools,
        # so the PEP 517 backend cannot even be imported. A real-world "old sdist on new Python".
        target=["numpy==1.19.5"],
        dry_run=False,
        question="sdist build backend failure (PEP 517 backend unavailable).",
        expects_code="PD-BLD-002",
    ),
]


def run(
    argv: list[str], cwd: Path | None = None, extra_env: dict[str, str] | None = None
) -> subprocess.CompletedProcess[str]:
    """Run a command with no shell involved, mirroring SECURITY §2."""
    env = None
    if extra_env:
        env = {**os.environ, **extra_env}
    return subprocess.run(
        argv,
        cwd=cwd,
        env=env,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=600,
        check=False,
    )


def make_venv(path: Path) -> Path:
    """Create a disposable venv and return its interpreter."""
    venv.create(path, with_pip=True, clear=True)
    python = path / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
    if not python.exists():
        raise RuntimeError(f"venv interpreter missing at {python}")
    return python


def pip_argv(python: Path, args: list[str]) -> list[str]:
    return [str(python), "-m", "pip", *args]


def uv_argv(uv_exe: Path, python: Path, args: list[str]) -> list[str]:
    return [str(uv_exe), "pip", *args, "--python", str(python)]


def capture(engine: str, scenario: Scenario, uv_exe: Path | None, out_root: Path) -> dict:
    """Run one scenario and write its fixture. Returns a summary record."""
    work = Path(tempfile.mkdtemp(prefix=f"pd-{engine}-{scenario.name}-"))
    env = SAFE_ENV if scenario.utf8_io else None
    try:
        python = make_venv(work / "venv")

        seed_result = None
        if scenario.seed:
            argv = (
                pip_argv(python, ["install", "--quiet", *scenario.seed])
                if engine == "pip"
                else uv_argv(uv_exe, python, ["install", *scenario.seed])
            )
            seed_result = run(argv, extra_env=SAFE_ENV)
            if seed_result.returncode != 0:
                print(f"  ! seed failed ({seed_result.returncode}); capturing anyway")

        if engine == "pip":
            args = ["install", "-U"]
            if scenario.dry_run:
                # DATA-FLOW §7: this is the exact planning command, JSON report to stdout.
                args += ["--dry-run", "--quiet", "--report", "-"]
            args += scenario.target
            argv = pip_argv(python, args)
        else:
            args = ["install", "-U"]
            if scenario.dry_run:
                args += ["--dry-run"]
            args += scenario.target
            argv = uv_argv(uv_exe, python, args)

        result = run(argv, extra_env=env)

        out_dir = out_root / engine / scenario.name
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "stdout.txt").write_text(result.stdout, encoding="utf-8")
        (out_dir / "stderr.txt").write_text(result.stderr, encoding="utf-8")
        meta = {
            "scenario": scenario.name,
            "question": scenario.question,
            "expects_code": scenario.expects_code,
            "engine": engine,
            "engine_version": engine_version(engine, python, uv_exe),
            "python": python_version(python),
            "argv": [redact(a) for a in argv],
            "extra_env": env,
            "exit_code": result.returncode,
            "seed": scenario.seed,
            "seed_exit_code": None if seed_result is None else seed_result.returncode,
            "stdout_bytes": len(result.stdout),
            "stderr_bytes": len(result.stderr),
        }
        (out_dir / "meta.json").write_text(json.dumps(meta, indent=2) + "\n", encoding="utf-8")
        return meta
    finally:
        shutil.rmtree(work, ignore_errors=True)


def redact(arg: str) -> str:
    """Replace the temp-dir path so fixtures are stable across machines and runs."""
    return arg.replace(str(Path(tempfile.gettempdir())), "<TMP>")


def engine_version(engine: str, python: Path, uv_exe: Path | None) -> str:
    argv = (
        pip_argv(python, ["--version"]) if engine == "pip" else [str(uv_exe), "--version"]
    )
    return run(argv).stdout.strip()


def python_version(python: Path) -> str:
    return run([str(python), "--version"]).stdout.strip() or run(
        [str(python), "-c", "import sys;print(sys.version.split()[0])"]
    ).stdout.strip()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", choices=["pip", "uv"], default="pip")
    parser.add_argument("--uv-exe", type=Path, help="uv executable (required for --engine uv)")
    parser.add_argument("--out", type=Path, default=FIXTURES)
    parser.add_argument("--only", action="append", help="run only these scenarios")
    parser.add_argument("--list", action="store_true", help="list scenarios and exit")
    args = parser.parse_args()

    if args.list:
        for s in SCENARIOS:
            print(f"{s.name:24} {s.expects_code or '-':12} {s.question}")
        return 0

    if args.engine == "uv" and not args.uv_exe:
        parser.error("--uv-exe is required with --engine uv")

    selected = [s for s in SCENARIOS if not args.only or s.name in args.only]
    summaries = []
    for scenario in selected:
        print(f"[{args.engine}] {scenario.name} …", flush=True)
        meta = capture(args.engine, scenario, args.uv_exe, args.out)
        summaries.append(meta)
        print(
            f"  exit={meta['exit_code']}  stdout={meta['stdout_bytes']}B  "
            f"stderr={meta['stderr_bytes']}B",
            flush=True,
        )

    print(json.dumps({"engine": args.engine, "captured": len(summaries)}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
