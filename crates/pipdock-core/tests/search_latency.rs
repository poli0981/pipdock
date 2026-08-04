//! The per-keystroke search budget, against the **real** index.
//!
//! PRD §6 and the S4 exit check both say **< 50 ms per keystroke**. SP-3 measured 42.1 ms for the
//! ranking alone and warned the margin was thin on fast hardware; this re-measures it against
//! whatever the local index actually contains now, which is the number that matters — the index
//! grew from 858k to 864k projects in the week between those two measurements.
//!
//! `#[ignore]` by default: it needs `%LOCALAPPDATA%\PipDock\index.db` populated by a real
//! `pipdock index refresh`, which CI does not have and should not spend four seconds getting. Run
//! it deliberately:
//!
//! ```text
//! cargo test -p pipdock-core --test search_latency -- --ignored --nocapture
//! ```
//!
//! What this does **not** measure is the IPC round trip and the React render on top. Those are the
//! new cost S4 adds and SP-3 never saw, and they are measured in the running app — see the Stage 4
//! notes in `docs/ROADMAP.md`.
//!
//! **Run it `--release`.** A debug build measures the allocator and the bounds checks, not the
//! ranking: the same corpus and the same queries give a 176 ms worst keystroke in debug and
//! 16 ms in release, and a 572 ms index load in debug against 140 ms in release. Debug numbers
//! led to a design being justified with a figure four times too large — see the assertion below,
//! which refuses to let that happen quietly again.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]

/// Queries chosen to be the expensive shapes, not the friendly ones.
///
/// A single character matches the most names and does the most ranking work; a long exact name is
/// the cheap case. Real typing walks from the first to the second, so the worst keystroke is
/// almost always the first.
#[cfg(not(debug_assertions))]
const QUERIES: &[&str] = &[
    "r", "re", "req", "requ", "reque", "reques", "requests", "n", "nu", "num", "nump", "numpy",
    "d", "dj", "dja", "djan", "djang", "django", "z", "zz", "zzz",
];

#[test]
#[ignore = "needs a populated index; run with --ignored"]
fn every_keystroke_stays_under_the_budget() {
    // A latency budget measured in debug is not a measurement of anything the user will run, and
    // reporting one as if it were is how a design gets justified with a number four times too
    // large. Refuse rather than print something misleading.
    #[cfg(debug_assertions)]
    panic!("run this with --release; a debug build measures bounds checks, not ranking");

    #[cfg(not(debug_assertions))]
    {
        use std::time::Instant;

        use pipdock_core::index::NameIndex;
        use pipdock_core::store::{Store, default_app_data};

        let store = Store::open(&default_app_data()).expect("store opens");

        let loading = Instant::now();
        let index = match NameIndex::load(&store) {
            Ok(i) => i,
            Err(e) => {
                panic!("no index to measure — run `pipdock index refresh` first ({e:?})");
            }
        };
        let load_ms = loading.elapsed().as_secs_f64() * 1000.0;

        // Warm once: the first search touches memory the loader just wrote, and measuring that
        // measures the allocator rather than the ranking.
        let _ = index.search("warmup", 50);

        let mut worst = 0.0_f64;
        let mut worst_query = "";
        let mut total = 0.0_f64;

        for query in QUERIES {
            // Three passes, best of: this is a latency budget, so the question is what the ranking
            // costs, not what the machine was doing at the time.
            let mut best = f64::MAX;
            for _ in 0..3 {
                let started = Instant::now();
                let hits = index.search(query, 50);
                let ms = started.elapsed().as_secs_f64() * 1000.0;
                best = best.min(ms);
                // Keep the compiler from optimising the search away.
                assert!(hits.len() <= 50);
            }
            total += best;
            if best > worst {
                worst = best;
                worst_query = query;
            }
            println!("  {query:>10}  {best:6.2} ms");
        }

        println!();
        println!("index load        {load_ms:8.1} ms  (paid once, on first Search open)");
        println!("worst keystroke   {worst:8.2} ms  ({worst_query:?})");
        println!("mean keystroke    {:8.2} ms", total / QUERIES.len() as f64);

        // The budget is per keystroke and belongs to the whole round trip, so the ranking alone has to
        // come in well under it — the IPC hop and the render are still to be paid.
        assert!(
            worst < 50.0,
            "ranking alone exceeded the entire budget: {worst:.2} ms on {worst_query:?}"
        );
    }
}
