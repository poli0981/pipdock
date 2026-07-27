"""SP-3 — PEP 691 index economics.

`docs/ROADMAP.md` SP-3 asks: how big is the full PyPI name index, how long does it take to fetch
on VN broadband, how long to ingest into SQLite, and can fuzzy search stay under the 50 ms
per-keystroke budget over ~600 k names? If the answer is no, ARCHITECTURE §5 needs a delta or
compression fallback.

This measures the fetch and the SQLite side in Python. The fuzzy-search half is measured in Rust
with the real `nucleo` matcher (`spikes/sp3_search.rs` is not a thing yet — see the README), so
what is timed here is the SQLite scan that feeds the matcher, which is the part that would sink
the budget if the schema were wrong.

Usage:
    py -3.14 spikes/sp3_index.py --out spikes/out
"""

from __future__ import annotations

import argparse
import gzip
import io
import json
import sqlite3
import time
import urllib.request
from pathlib import Path

SIMPLE_INDEX = "https://pypi.org/simple/"
PEP691_ACCEPT = "application/vnd.pypi.simple.v1+json"


def normalize(name: str) -> str:
    """PEP 503 normalization; must match `PkgName::parse` in the Rust core."""
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


def fetch(url: str) -> tuple[bytes, float, int]:
    """Fetch the index, returning (body, seconds, wire_bytes)."""
    req = urllib.request.Request(
        url, headers={"Accept": PEP691_ACCEPT, "Accept-Encoding": "gzip"}
    )
    start = time.perf_counter()
    with urllib.request.urlopen(req, timeout=300) as resp:
        raw = resp.read()
        encoding = resp.headers.get("Content-Encoding", "")
    elapsed = time.perf_counter() - start
    wire = len(raw)
    if encoding == "gzip":
        raw = gzip.GzipFile(fileobj=io.BytesIO(raw)).read()
    return raw, elapsed, wire


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("spikes/out"))
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)

    print(f"fetching {SIMPLE_INDEX} …", flush=True)
    body, fetch_s, wire_bytes = fetch(SIMPLE_INDEX)

    t0 = time.perf_counter()
    doc = json.loads(body)
    parse_s = time.perf_counter() - t0
    names = [p["name"] for p in doc["projects"]]

    t0 = time.perf_counter()
    rows = [(n, normalize(n)) for n in names]
    normalize_s = time.perf_counter() - t0

    db_path = args.out / "index.db"
    db_path.unlink(missing_ok=True)
    conn = sqlite3.connect(db_path)
    t0 = time.perf_counter()
    conn.executescript(
        """
        PRAGMA journal_mode = OFF;
        PRAGMA synchronous = OFF;
        CREATE TABLE names (name TEXT NOT NULL, normalized TEXT NOT NULL);
        """
    )
    conn.executemany("INSERT INTO names (name, normalized) VALUES (?, ?)", rows)
    conn.execute("CREATE INDEX idx_names_normalized ON names (normalized)")
    conn.commit()
    ingest_s = time.perf_counter() - t0

    # The search path PipDock actually needs: pull the normalized column into memory once, then
    # let the fuzzy matcher work over it. Timing the scan separately shows whether SQLite is the
    # bottleneck or the matcher is.
    t0 = time.perf_counter()
    loaded = [r[0] for r in conn.execute("SELECT normalized FROM names")]
    scan_s = time.perf_counter() - t0

    # A prefix query is the cheapest possible search; it bounds how fast any index-backed lookup
    # can be, and shows whether the btree index is doing its job.
    t0 = time.perf_counter()
    for q in ("req", "num", "pand", "flask", "zope"):
        conn.execute(
            "SELECT name FROM names WHERE normalized GLOB ? LIMIT 50", (f"{q}*",)
        ).fetchall()
    prefix_s = (time.perf_counter() - t0) / 5

    conn.close()
    db_bytes = db_path.stat().st_size

    result = {
        "projects": len(names),
        "wire_bytes": wire_bytes,
        "wire_mib": round(wire_bytes / 1024 / 1024, 2),
        "decompressed_mib": round(len(body) / 1024 / 1024, 2),
        "fetch_seconds": round(fetch_s, 2),
        "json_parse_seconds": round(parse_s, 2),
        "normalize_seconds": round(normalize_s, 2),
        "sqlite_ingest_seconds": round(ingest_s, 2),
        "sqlite_bytes": db_bytes,
        "sqlite_mib": round(db_bytes / 1024 / 1024, 2),
        "full_scan_ms": round(scan_s * 1000, 1),
        "prefix_query_ms": round(prefix_s * 1000, 2),
        "cold_total_seconds": round(fetch_s + parse_s + normalize_s + ingest_s, 2),
        "budget_cold_refresh_seconds": 60,
        "budget_search_ms": 50,
    }
    (args.out / "sp3.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))

    verdict = "PASS" if result["cold_total_seconds"] < 60 else "FAIL"
    print(f"\ncold refresh: {result['cold_total_seconds']}s vs 60s budget -> {verdict}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
