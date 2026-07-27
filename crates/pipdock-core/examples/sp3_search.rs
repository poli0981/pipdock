//! SP-3 — fuzzy-search latency over the full PyPI name index.
//!
//! This measures **the code that ships** — `NameIndex::search` — not a private copy of it. An
//! earlier version scored with a bare nucleo loop, which measured something real but no longer
//! measured PipDock: the production path is tiered rather than score-ordered, and skips the
//! matcher entirely for exact and prefix hits. A benchmark that drifts from the implementation
//! reports numbers nobody can act on.
//!
//! `spikes/sp3_index.py` measures the fetch and ingest side and found that a **full SQLite column
//! scan costs ~218 ms**, four times the 50 ms per-keystroke budget in PRD §6. So the question here
//! is not "is SQLite fast enough per keystroke" — it is not — but "does loading once and matching
//! in memory fit the budget?"
//!
//! ```text
//! pipdock index refresh
//! cargo run --release --example sp3_search -- "%LOCALAPPDATA%\PipDock\index.db"
//! ```

// A measurement harness reports to a human on stdout; the library-level ban does not apply.
#![allow(clippy::print_stdout)]

use std::time::Instant;

use pipdock_core::index::{NameIndex, SEARCH_LATENCY_BUDGET};
use pipdock_core::store::{Store, default_app_data};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = std::env::args().nth(1).map_or_else(
        || default_app_data().join("index.db"),
        std::path::PathBuf::from,
    );
    let store = Store::open_at(&db)?;

    // Loaded once at startup, which is the whole design: SQLite is persistence, not the search
    // path.
    let load_start = Instant::now();
    let index = NameIndex::load(&store)?;
    let load_ms = load_start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "loaded {} names in {load_ms:.0} ms (once, at startup)",
        index.len()
    );

    // Progressive typing: the budget is about the cost of each keystroke, including the
    // single-character query that matches almost everything.
    let queries = [
        "r", "re", "req", "requ", "reque", "request", "requests", "numpy", "zope",
    ];

    let mut worst: f64 = 0.0;
    for query in queries {
        let start = Instant::now();
        let hits = index.search(query, 50);
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        worst = worst.max(ms);

        let top = hits.first().map_or("-", |h| h.display.as_str());
        println!(
            "  {query:<10} {ms:>7.1} ms   hits: {:>2}   top: {top}",
            hits.len()
        );
    }

    let budget_ms = SEARCH_LATENCY_BUDGET.as_secs_f64() * 1000.0;
    println!("\nworst keystroke: {worst:.1} ms vs the {budget_ms:.0} ms budget in PRD §6");
    println!(
        "verdict: {}",
        if worst < budget_ms {
            "PASS"
        } else {
            "FAIL — needs a prefilter before the fuzzy pass"
        }
    );
    Ok(())
}
