//! SP-3 — fuzzy-search latency over the full PyPI name index.
//!
//! `spikes/sp3_index.py` measures the fetch and SQLite side and found that a **full column scan
//! costs ~218 ms**, four times the 50 ms per-keystroke budget in PRD §6. So the question this
//! example answers is not "is SQLite fast enough per keystroke" — it is not — but "does loading
//! the names into memory once and matching there fit the budget?"
//!
//! Run after `py -3.14 spikes/sp3_index.py`:
//!
//! ```text
//! cargo run --release --example sp3_search -- spikes/out/index.db
//! ```

// A measurement harness reports to a human on stdout; the library-level ban does not apply.
#![allow(clippy::print_stdout)]

use std::time::Instant;

use nucleo::Matcher;
use nucleo::pattern::{CaseMatching, Normalization, Pattern};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "spikes/out/index.db".into());

    let conn = rusqlite::Connection::open(&db_path)?;

    // Load once at startup, the way the real index module will.
    let load_start = Instant::now();
    let mut stmt = conn.prepare("SELECT normalized FROM names")?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let load_ms = load_start.elapsed().as_secs_f64() * 1000.0;

    println!(
        "loaded {} names in {load_ms:.0} ms (once, at startup)",
        names.len()
    );

    let mut matcher = Matcher::default();
    let mut buf = Vec::new();

    // Progressive typing: this is what the budget is actually about — the cost of each keystroke,
    // including the single-character query that matches almost everything.
    let queries = [
        "r", "re", "req", "requ", "reque", "request", "requests", "numpy", "zope",
    ];

    let mut worst: f64 = 0.0;
    for query in queries {
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);

        let start = Instant::now();
        let mut scored: Vec<(u32, &str)> = names
            .iter()
            .filter_map(|name| {
                buf.clear();
                let haystack = nucleo::Utf32Str::new(name, &mut buf);
                pattern
                    .score(haystack, &mut matcher)
                    .map(|s| (s, name.as_str()))
            })
            .collect();
        // The UI shows a bounded list, so only the top slice is ordered.
        scored.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        scored.truncate(50);
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        worst = worst.max(ms);

        let top = scored.first().map(|(_, n)| *n).unwrap_or("-");
        println!(
            "  {query:<10} {ms:>7.1} ms   matches shown: {:>2}   top: {top}",
            scored.len()
        );
    }

    println!("\nworst keystroke: {worst:.1} ms vs the 50 ms budget in PRD §6");
    println!(
        "verdict: {}",
        if worst < 50.0 {
            "PASS"
        } else {
            "FAIL — needs prefilter"
        }
    );
    Ok(())
}
