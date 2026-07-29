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
import re
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
    #: Per-engine override of `expects_code`; the engines do not always agree (see SP-2).
    #: Map an engine to None to say "this engine does not fail here at all".
    expects_by_engine: dict[str, str | None] = field(default_factory=dict)
    #: Set False only to capture what happens WITHOUT the UTF-8 mitigation (see SAFE_ENV).
    utf8_io: bool = True
    #: Extra argv inserted before the requirements, e.g. an unreachable index URL.
    extra_args: list[str] = field(default_factory=list)

    def code_for(self, engine: str) -> str | None:
        return self.expects_by_engine.get(engine, self.expects_code)


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
        question=(
            "Requires-Python mismatch against the running interpreter. ANSWER: pip refuses with "
            "the same 'No matching distribution found' text it uses for an unknown name, so the "
            "two are indistinguishable from stderr; uv ignores the constraint and plans the "
            "install. Hence PD-PKG-001 is raised by pipdock-core's compat module before either "
            "engine runs, and these fixtures record what the engines do instead."
        ),
        expects_by_engine={"pip": "PD-PKG-002", "uv": None},
    ),
    # No "externally-managed" scenario. PEP 668 applies only OUTSIDE a virtual environment — pip
    # deliberately ignores the marker inside one — and deleting pyvenv.cfg to fake a base install
    # merely breaks the venv launcher ("No pyvenv.cfg file", exit 106). Reproducing it properly
    # would mean planting the marker in a real system Python, which a fixture script has no
    # business doing to a developer's machine. It does not matter: PipDock blocks such
    # environments from probe.py's `externally_managed` field at step zero (DATA-FLOW §2), before
    # any engine command runs, so PD-ENV-002 is raised internally rather than classified.
    Scenario(
        name="network-unreachable",
        seed=[],
        target=["idna"],
        # Nothing listens on port 1; deterministic and works offline.
        extra_args=["--index-url", "http://127.0.0.1:1/simple"],
        question="Index unreachable — connection refused rather than DNS or TLS failure.",
        expects_code="PD-NET-001",
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

        # Cache-free on purpose. A warm cache turns "Downloading numpy-1.19.5.zip (7.3 MB)" into
        # "Using cached numpy-1.19.5.zip", so the same scenario captures differently depending on
        # what the machine happened to have — and a CI runner is always cold while a dev machine
        # is usually warm. The download path is the one both agree on. It costs bandwidth on a
        # weekly job and buys a fixture that can actually be compared.
        if engine == "pip":
            args = ["install", "-U", "--no-cache-dir"]
            if scenario.dry_run:
                # DATA-FLOW §7: this is the exact planning command, JSON report to stdout.
                args += ["--dry-run", "--quiet", "--report", "-"]
            args += scenario.extra_args + scenario.target
            argv = pip_argv(python, args)
        else:
            args = ["install", "-U", "--no-cache"]
            if scenario.dry_run:
                args += ["--dry-run"]
            args += scenario.extra_args + scenario.target
            argv = uv_argv(uv_exe, python, args)

        result = run(argv, extra_env=env)

        out_dir = out_root / engine / scenario.name
        out_dir.mkdir(parents=True, exist_ok=True)
        # The captured streams are redacted for the same reason argv is, and it matters more
        # here: pip's "[notice] To update, run: <venv>\Scripts\python.exe -m pip install
        # --upgrade pip" embeds the disposable venv's path, which carries mkdtemp's random
        # suffix. Left in, every pip stderr.txt differs on every capture — so the drift job can
        # never go green — and the capturing user's home directory ships in a public repo.
        #
        # This is the one place the fixtures are not literal engine bytes, and it is safe
        # because no classifier in errors/catalog.rs matches on a filesystem path; they all key
        # off message text. Line endings are untouched: `text=True` gave us LF in memory and
        # write_text puts CRLF back, exactly as before.
        (out_dir / "stdout.txt").write_text(redact(result.stdout, work), encoding="utf-8")
        (out_dir / "stderr.txt").write_text(redact(result.stderr, work), encoding="utf-8")

        # The sidecar is split by what each half is FOR, because the weekly drift job diffs the
        # fixture tree and fails on any change (ci-integration.yml, "Fail if the captured shapes
        # moved"). Anything that churns for reasons unrelated to engine output would make that
        # job fire on every run and mean nothing — which is exactly what it used to do.
        #
        #   meta.json               the contract. A diff here is a real change in engine
        #                           behaviour, and the drift job is right to fail on it.
        #   capture-provenance.json how these bytes were obtained. Rewritten every capture and
        #                           excluded from the drift gate, but still committed so you can
        #                           always tell which engine produced the current fixtures.
        meta = {
            "scenario": scenario.name,
            "question": scenario.question,
            "expects_code": scenario.code_for(engine),
            "engine": engine,
            "extra_env": env,
            "exit_code": result.returncode,
            "seed": scenario.seed,
            "seed_exit_code": None if seed_result is None else seed_result.returncode,
        }
        provenance = {
            "scenario": scenario.name,
            "engine_version": engine_version(engine, python, uv_exe),
            "python": python_version(python),
            "argv": [redact(a, work) for a in argv],
            "stdout_bytes": len(result.stdout),
            "stderr_bytes": len(result.stderr),
        }
        (out_dir / "meta.json").write_text(json.dumps(meta, indent=2) + "\n", encoding="utf-8")
        (out_dir / "capture-provenance.json").write_text(
            json.dumps(provenance, indent=2) + "\n", encoding="utf-8"
        )
        return {**meta, **provenance}
    finally:
        shutil.rmtree(work, ignore_errors=True)


# Any drive-absolute Windows path, in both the raw form engines print to a console and the
# backslash-escaped form they emit inside JSON. Stops at quotes and whitespace so a path
# embedded in a JSON string cannot swallow the closing quote.
_ABS_PATH = re.compile(r'[A-Za-z]:(?:\\\\[^\\/:*?"<>|\r\n]+)+|[A-Za-z]:(?:\\[^\\/:*?"<>|\r\n]+)+')


def redact(text: str, work: Path) -> str:
    """Strip machine- and run-specific paths so a capture is comparable across machines.

    Three distinct sources of churn, all of which used to land in committed fixtures:

    * `work` is a fresh `mkdtemp`, so its name carries a random suffix. pip's "[notice] To
      update, run: <venv>\\Scripts\\python.exe" embeds it, so every pip stderr.txt differed on
      every capture.
    * uv's build cache uses its own random `.tmpXXXXXX` directories, which appear in build
      backend tracebacks.
    * Absolute interpreter paths differ by machine — `C:\\Users\\<name>\\AppData\\Local\\Programs`
      locally versus `C:\\hostedtoolcache\\...` on a CI runner — so a fixture captured on one
      could never match a re-capture on the other.

    Together those meant the drift job could not go green, and they shipped a real person's home
    directory to a public repo. The final path component is kept because it is the part that
    carries diagnostic meaning (`cp1252.py`); the directory prefix is not.

    This is the one place fixtures are not literal engine bytes. It is safe because nothing reads
    a path out of them: every entry in `CLASSIFIERS` (errors/catalog.rs) keys off message text,
    and the plan parsers read pip's JSON report, whose URLs are https rather than filesystem.
    """
    text = text.replace(str(work), "<WORK>")
    text = text.replace(str(work).replace("\\", "\\\\"), "<WORK>")
    text = re.sub(r"\.tmp[A-Za-z0-9_]{6,}", ".tmpXXXXXX", text)

    def keep_basename(m: re.Match[str]) -> str:
        sep = "\\\\" if "\\\\" in m.group(0) else "\\"
        return "<PATH>" + sep + m.group(0).split(sep)[-1]

    text = _ABS_PATH.sub(keep_basename, text)
    text = text.replace(str(Path(tempfile.gettempdir())), "<TMP>")

    # CPython object addresses. pip's retry warnings embed the urllib3 connection object's repr
    # ("<...HTTPConnection object at 0x000001A92FBCFBC0>"), and ASLR moves it every process.
    text = re.sub(r"0x[0-9A-Fa-f]{6,}", "0xADDR", text)

    # pip's download progress bar carries a transfer rate and an ETA, so it is different on
    # every run and wildly different between a dev machine and a CI runner.
    text = re.sub(r"(?m)^\s*-+\s+[^\r\n]*?eta [0-9:]+[ \t]*$", "     <PROGRESS>", text)

    # uv times every phase — "Resolved 3 packages in 775ms", "Checked 8 packages in 1ms". The
    # counts are signal and stay; the durations are never the same twice. Safe to drop: the uv
    # adapter only tests `text.contains("Resolved ")` and reads the +/- lines (engine/parse.rs).
    return re.sub(r"\bin \d+(?:\.\d+)?(?:ms|µs|us|s)\b", "in <DURATION>", text)


def engine_version(engine: str, python: Path, uv_exe: Path | None) -> str:
    argv = (
        pip_argv(python, ["--version"]) if engine == "pip" else [str(uv_exe), "--version"]
    )
    raw = run(argv).stdout.strip()
    # `pip --version` reports its own install path: "pip 25.0.1 from C:\Users\<name>\... (python
    # 3.12)". That is the capturing user's home directory, and these fixtures are committed to a
    # public repo. Drop the clause; the version and the interpreter are the parts that matter.
    return re.sub(r"^(pip \S+) from .*? (\(python .*\))$", r"\1 \2", raw)


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
            print(f"{s.name:26} {s.expects_code or '-':12} {s.question[:70]}")
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
