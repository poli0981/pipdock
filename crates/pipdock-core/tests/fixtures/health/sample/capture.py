"""Re-capture `../ruff/*.json` from this directory's sample. See README.md.

Deliberately not part of `spikes/capture.py`: that one knows pip and uv, redacts object addresses
and progress bars, and is driven by the drift job. This is four `ruff check` invocations and one
path substitution, and folding it in would give the drift job a fifth engine it does not have.

Usage:  python capture.py <path to ruff.exe> [output dir, default ../ruff]
"""

import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).parent

# target, relative to HERE -> fixture name
CASES = {
    "pkg/app.py": "findings.json",
    "nofix.py": "nofix.json",
    "broken.py": "syntax-error.json",
    "clean.py": "clean.json",
}


def capture(ruff: pathlib.Path, target: str) -> str:
    # `--no-cache` so a previous run cannot answer for this one. stderr is kept separate rather
    # than merged: ruff writes warnings there, and merging them corrupted the first capture of
    # this corpus (see `ruffs_warnings_on_stderr_would_break_a_merged_capture`).
    out = subprocess.run(
        [str(ruff), "check", target, "--output-format", "json", "--no-cache"],
        cwd=HERE,
        capture_output=True,
        text=True,
        check=False,
    )
    if out.stderr.strip():
        print(f"  note: {target} wrote to stderr: {out.stderr.strip()[:200]}")
    # ruff reports absolute paths. The committed documents say `C:\proj` so they do not carry the
    # capturing machine's home directory into a public repo — the same reason spikes/capture.py
    # redacts pip's "[notice] To update, run: ..." line.
    return out.stdout.replace(str(HERE).replace("\\", "\\\\"), "C:\\\\proj")


def main() -> int:
    ruff = pathlib.Path(sys.argv[1])
    out_dir = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else HERE.parent / "ruff"

    changed = False
    for target, name in CASES.items():
        body = capture(ruff, target)
        if str(HERE.name) in body or "Users" in body:
            raise SystemExit(f"{name}: redaction did not take — refusing to write a real path")
        json.loads(body)

        path = out_dir / name
        old = path.read_bytes() if path.exists() else b""
        new = body.encode("utf-8")
        # Bytes, not text: `.gitattributes` marks this tree `-text` so a captured document keeps
        # the line endings the tool actually emitted.
        path.write_bytes(new)
        changed = changed or old != new
        print(f"{name:20} {'unchanged' if old == new else 'CHANGED'}  ({len(new)} bytes)")

    if changed:
        print("\nSomething moved. Install the PREVIOUS ruff and run it against this same sample")
        print("before concluding the tool changed — see README.md.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
