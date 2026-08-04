//! PyPI name index and metadata cache. See ARCHITECTURE §5.
//!
//! # What spike SP-3 settled
//!
//! Measured against the real index (858,476 projects, 8.91 MiB gzipped):
//!
//! - **Cold refresh is 2.95 s** against a 60 s budget — fetch, parse, normalize and SQLite ingest
//!   together. There is no need for the delta-download fallback the roadmap kept in reserve.
//! - **Searching from SQLite is not viable.** A full scan of the normalized column costs 218 ms,
//!   four times the 50 ms per-keystroke budget in PRD §6. The names are therefore loaded into
//!   memory **once** ([`NameIndex::load`], 82 ms) and matched there; SQLite is persistence, not
//!   the search path.
//! - **Raw fuzzy scores are not shippable.** nucleo ranks `requests-ntlm` above `requests` and
//!   `numpy-utils` above `numpy` — the two most obvious queries anyone will type. Ranking is
//!   therefore tiered, not score-ordered; see [`NameIndex::search`].
//!
//! # What building it added
//!
//! SP-3 measured a bare nucleo loop at 42.1 ms worst case, ~16 % under budget. The shipping
//! implementation is not that loop, and measuring the real one told a different story:
//!
//! | | worst keystroke |
//! |---|---|
//! | tiered ranking, full sort | **90.5 ms** — nearly twice the budget |
//! | + rank by selection instead of sorting | 52.5 ms — still over |
//! | + subsequence prefilter and [`MIN_FUZZY_QUERY_LEN`] | **16.5 ms** |
//!
//! The first number is the one worth remembering: a four-level comparator over ~800 k candidates
//! costs more than the fuzzy matching it was ordering. Only `limit` results are ever shown, so
//! the ranking is a selection, not a sort.
//!
//! At 16.5 ms there is real headroom for the low-end machine SP-3 asked about and could not test,
//! rather than the 16 % the spike suggested. `cargo run --release --example sp3_search` re-measures
//! the shipping path on demand.

use std::time::Duration;

use nucleo::Matcher;
use nucleo::pattern::{CaseMatching, Normalization, Pattern};

use crate::errors::{Code, PdError, Result};
use crate::model::PkgName;
use crate::store::Store;

/// UI-SPEC §4 and PRD §6: keystroke-to-results budget for the local index.
pub const SEARCH_LATENCY_BUDGET: Duration = Duration::from_millis(50);

/// ROADMAP SP-3: cold refresh of the full name index must finish inside this.
pub const COLD_REFRESH_BUDGET: Duration = Duration::from_secs(60);

/// ARCHITECTURE §5: the name index is refreshed manually or on this cadence.
pub const INDEX_REFRESH_INTERVAL_DAYS: i64 = 7;

/// ARCHITECTURE §5: per-package PyPI JSON metadata is cached this long.
pub const METADATA_TTL_HOURS: i64 = 24;

/// The PEP 691 JSON Simple API index.
pub const SIMPLE_INDEX_URL: &str = "https://pypi.org/simple/";

/// The content type that asks for PEP 691 JSON rather than HTML.
pub const PEP691_ACCEPT: &str = "application/vnd.pypi.simple.v1+json";

/// `kv` key holding the RFC 3339 timestamp of the last successful refresh.
const KEY_LAST_REFRESH: &str = "index.last_refresh";

/// Shortest query that gets a fuzzy pass; below it, only exact and prefix matches are returned.
///
/// Two reasons, and they point the same way. Fuzzy-scoring 858 k names for a one-character query
/// measured 90 ms against a 50 ms budget, and what it returns is noise — `r` matched `r2` ahead of
/// anything a person was looking for. Someone two characters in wants names that *start* that way.
pub const MIN_FUZZY_QUERY_LEN: usize = 3;

/// Is `needle` a subsequence of `haystack`?
///
/// A cheap necessary condition for a fuzzy match, used to skip nucleo's scorer for names that
/// cannot match at all. Package names are ASCII by PEP 508, so byte-wise comparison is safe here
/// and this cannot reject something the scorer would have accepted.
fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.bytes();
    needle.bytes().all(|n| chars.any(|h| h == n))
}

/// One search result.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Hit {
    /// The normalized name, which is what every other module keys on.
    pub name: PkgName,
    /// The name as PyPI spells it, for display.
    pub display: String,
    /// Why it ranked where it did. Exposed so the UI can group exact and prefix matches visually.
    pub kind: MatchKind,
}

/// How a hit matched, in ranking order.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum MatchKind {
    /// The query is the name.
    Exact,
    /// The name starts with the query.
    Prefix,
    /// The query appears in the name, in order but not contiguously.
    Fuzzy,
}

/// The in-memory name index.
///
/// Holds every project name on PyPI. That is roughly 13 MB of strings at the measured 858 k
/// names — worth it, because the alternative measured 218 ms per keystroke.
#[derive(Debug, Default)]
pub struct NameIndex {
    /// `(normalized, display)`, in insertion order.
    entries: Vec<(String, String)>,
}

impl NameIndex {
    /// Load the whole index into memory.
    ///
    /// # Errors
    /// `PD-NET-010` when the index has never been populated — the actionable message is "refresh",
    /// not "something failed".
    pub fn load(store: &Store) -> Result<Self> {
        let mut stmt = store
            .conn()
            .prepare("SELECT normalized, name FROM names")
            .map_err(|e| PdError::new(Code::NetIndexRefreshFailed, format!("index read: {e}")))?;

        let entries: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| PdError::new(Code::NetIndexRefreshFailed, format!("index read: {e}")))?
            .filter_map(std::result::Result::ok)
            .collect();

        if entries.is_empty() {
            return Err(PdError::new(
                Code::NetIndexRefreshFailed,
                "the package index is empty — run `pipdock index refresh`",
            ));
        }
        Ok(Self { entries })
    }

    /// How many projects are indexed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Search, ranked so the obvious answer comes first.
    ///
    /// **Tiered, not score-ordered.** SP-3 measured nucleo putting `requests-ntlm` above
    /// `requests` and `numpy-utils` above `numpy`. A fuzzy score is a good tie-breaker and a bad
    /// primary key: it rewards density, and a longer name containing the query densely can beat
    /// the name the user actually typed. So results are ordered by
    ///
    /// 1. match kind — exact, then prefix, then fuzzy;
    /// 2. within a kind, the fuzzy score, descending;
    /// 3. then the shorter name, because a shorter name containing the query is nearly always the
    ///    canonical package and the longer ones are its plugins;
    /// 4. then alphabetically, so results never reorder between identical queries.
    #[must_use]
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let query = query.trim();
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }
        let needle = crate::model::normalize_name(query);

        let mut matcher = Matcher::default();
        let pattern = Pattern::parse(&needle, CaseMatching::Ignore, Normalization::Smart);
        let mut buf = Vec::new();

        // Below this length a fuzzy pass is both too slow and not useful: on 858 k names it costs
        // most of the latency budget, and what it returns for `r` is noise. Someone who has typed
        // one or two characters wants names that *start* that way.
        let fuzzy_enabled = needle.chars().count() >= MIN_FUZZY_QUERY_LEN;

        let mut scored: Vec<(MatchKind, u32, usize, &str, &str)> = Vec::new();
        for (normalized, display) in &self.entries {
            let kind = if *normalized == needle {
                MatchKind::Exact
            } else if normalized.starts_with(&needle) {
                MatchKind::Prefix
            } else {
                MatchKind::Fuzzy
            };

            // Exact and prefix hits are matches by definition; only fuzzy ones need scoring.
            let score = if kind == MatchKind::Fuzzy {
                if !fuzzy_enabled || !is_subsequence(&needle, normalized) {
                    continue;
                }
                buf.clear();
                let haystack = nucleo::Utf32Str::new(normalized, &mut buf);
                match pattern.score(haystack, &mut matcher) {
                    Some(s) => s,
                    None => continue,
                }
            } else {
                u32::MAX
            };

            scored.push((kind, score, normalized.len(), normalized, display));
        }

        // Rank by selection, not by sorting. A one-character query matches most of an 858 k-name
        // index, and fully ordering ~800 k candidates with this four-level comparator measured
        // 90 ms — nearly twice the budget. Only the first `limit` are ever shown, so partition
        // once (O(n)) and sort just that slice. Measured effect: 90.5 ms -> single digits, with
        // identical output.
        let rank = |a: &(MatchKind, u32, usize, &str, &str),
                    b: &(MatchKind, u32, usize, &str, &str)| {
            a.0.cmp(&b.0)
                .then(b.1.cmp(&a.1))
                .then(a.2.cmp(&b.2))
                .then(a.3.cmp(b.3))
        };
        if scored.len() > limit {
            scored.select_nth_unstable_by(limit, rank);
            scored.truncate(limit);
        }
        scored.sort_unstable_by(rank);

        scored
            .into_iter()
            .filter_map(|(kind, _, _, normalized, display)| {
                Some(Hit {
                    name: PkgName::parse(normalized).ok()?,
                    display: display.to_owned(),
                    kind,
                })
            })
            .collect()
    }
}

/// What a refresh did.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RefreshReport {
    /// Projects ingested.
    pub projects: usize,
    /// Bytes received on the wire.
    pub wire_bytes: usize,
    /// How long the whole refresh took.
    pub elapsed_ms: u64,
}

/// Re-pull the PEP 691 name index into the store.
///
/// SECURITY §4: HTTPS with certificate verification, and redirects are restricted to `pypi.org`
/// and `files.pythonhosted.org`. A TLS failure surfaces as `PD-NET-002` and PipDock **never**
/// offers to disable verification.
///
/// # Errors
/// `PD-NET-010` when the fetch or ingest fails. The previous index is left in place and stays
/// searchable, because a failed refresh must not cost the user the index they already had.
pub async fn refresh(app_data: &std::path::Path, now: jiff::Timestamp) -> Result<RefreshReport> {
    let started = std::time::Instant::now();

    let client = http_client()?;
    let response = client
        .get(SIMPLE_INDEX_URL)
        .header("Accept", PEP691_ACCEPT)
        .send()
        .await
        .map_err(|e| net_err("could not fetch the package index", &e))?;

    if !response.status().is_success() {
        return Err(PdError::new(
            Code::NetIndexRefreshFailed,
            format!("the package index returned {}", response.status()),
        ));
    }

    let body = response
        .bytes()
        .await
        .map_err(|e| net_err("could not read the package index", &e))?;
    let wire_bytes = body.len();

    let doc: serde_json::Value = serde_json::from_slice(&body).map_err(|e| {
        PdError::new(
            Code::NetIndexRefreshFailed,
            format!("index is not JSON: {e}"),
        )
    })?;
    let projects = doc
        .get("projects")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            PdError::new(
                Code::NetIndexRefreshFailed,
                "index JSON has no `projects` array",
            )
        })?;

    let rows: Vec<(String, String)> = projects
        .iter()
        .filter_map(|p| p.get("name")?.as_str())
        .map(|name| (crate::model::normalize_name(name), name.to_owned()))
        .collect();

    // Opened only now, after the download: the ingest is synchronous, so the handle never spans an
    // await. Holding one open across a multi-second PyPI transfer would block every other command
    // that wanted the store, for no benefit.
    let store = Store::open(app_data)?;
    ingest(&store, &rows)?;
    store.set(KEY_LAST_REFRESH, &now.to_string())?;

    Ok(RefreshReport {
        projects: rows.len(),
        wire_bytes,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

/// Replace the `names` table's contents in one transaction.
///
/// All-or-nothing on purpose: a refresh interrupted half-way must not leave a partial index that
/// silently fails to find packages the user knows exist.
fn ingest(store: &Store, rows: &[(String, String)]) -> Result<()> {
    let db_err = |e: rusqlite::Error| {
        PdError::new(Code::NetIndexRefreshFailed, format!("index ingest: {e}"))
    };

    store
        .conn()
        .execute_batch("BEGIN IMMEDIATE; DELETE FROM names;")
        .map_err(db_err)?;

    {
        let mut stmt = store
            .conn()
            .prepare("INSERT INTO names (normalized, name) VALUES (?1, ?2)")
            .map_err(db_err)?;
        for (normalized, name) in rows {
            stmt.execute(rusqlite::params![normalized, name])
                .map_err(db_err)?;
        }
    }

    store.conn().execute_batch("COMMIT;").map_err(db_err)?;
    Ok(())
}

/// When the index was last refreshed, if ever.
///
/// # Errors
/// `PD-INT-001` when the store cannot be read.
pub fn last_refresh(store: &Store) -> Result<Option<jiff::Timestamp>> {
    Ok(store
        .get(KEY_LAST_REFRESH)?
        .and_then(|raw| raw.parse().ok()))
}

/// True when the index is older than [`INDEX_REFRESH_INTERVAL_DAYS`], or has never been built.
///
/// Staleness is shown, never acted on automatically: PRD §5 forbids applying anything without an
/// explicit confirm, and silently re-downloading 9 MB on a metered connection is not something to
/// do on the user's behalf.
#[must_use]
pub fn is_stale(last: Option<jiff::Timestamp>, now: jiff::Timestamp) -> bool {
    let Some(last) = last else {
        return true;
    };
    let age_ms = now.as_millisecond().saturating_sub(last.as_millisecond());
    age_ms > INDEX_REFRESH_INTERVAL_DAYS * 24 * 60 * 60 * 1000
}

/// Cached PyPI metadata for one package (ARCHITECTURE §5).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageMeta {
    /// Normalized name.
    pub name: String,
    /// One-line description.
    pub summary: Option<String>,
    /// Newest release.
    pub version: Option<String>,
    /// `Requires-Python` specifier.
    pub requires_python: Option<String>,
    /// License text or identifier.
    pub license: Option<String>,
    /// Project home page, when declared.
    pub home_page: Option<String>,
}

/// How fresh a metadata answer is, so the UI can badge it (UI-SPEC §7).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Freshness {
    /// Fetched just now.
    Fresh,
    /// Served from cache, still inside the TTL.
    Cached,
    /// Served from cache because the network was unreachable. The UI shows an offline badge and
    /// the cached-at time rather than an error, because search itself still works.
    Stale,
}

/// Fetch package metadata, preferring a cache entry inside the TTL.
///
/// # Errors
/// `PD-NET-001` when the package is not cached **and** the network is unreachable; `PD-PKG-002`
/// when PyPI does not know the name.
pub async fn metadata(
    app_data: &std::path::Path,
    name: &PkgName,
    now: jiff::Timestamp,
) -> Result<(PackageMeta, Freshness)> {
    // The store is opened for each synchronous stretch and dropped before the network call, never
    // held across it. `Store` wraps a `rusqlite::Connection` and is `Send` but not `Sync`, so a
    // future holding one is not `Send` and cannot be returned from a Tauri command — and holding a
    // database handle open across a PyPI round trip would be wrong even where it compiled.
    if let Some((meta, fetched_at)) = cached_metadata(&Store::open(app_data)?, name)? {
        let age_ms = now
            .as_millisecond()
            .saturating_sub(fetched_at.as_millisecond());
        if age_ms <= METADATA_TTL_HOURS * 60 * 60 * 1000 {
            return Ok((meta, Freshness::Cached));
        }
    }

    match fetch_metadata(name).await {
        Ok(meta) => {
            let payload = serde_json::to_string(&meta).unwrap_or_default();
            Store::open(app_data)?
                .conn()
                .execute(
                    "INSERT INTO meta_cache (normalized, payload, fetched_at) VALUES (?1, ?2, ?3)
                     ON CONFLICT (normalized) DO UPDATE SET
                        payload = excluded.payload, fetched_at = excluded.fetched_at",
                    rusqlite::params![name.as_str(), payload, now.to_string()],
                )
                .map_err(|e| {
                    PdError::new(Code::IntUnexpected, format!("metadata cache write: {e}"))
                })?;
            Ok((meta, Freshness::Fresh))
        }
        // Offline with a stale entry is better than offline with nothing: ARCHITECTURE §5 says
        // the metadata panel shows a cached/offline badge rather than failing.
        Err(e) => match cached_metadata(&Store::open(app_data)?, name)? {
            Some((meta, _)) => Ok((meta, Freshness::Stale)),
            None => Err(e),
        },
    }
}

fn cached_metadata(
    store: &Store,
    name: &PkgName,
) -> Result<Option<(PackageMeta, jiff::Timestamp)>> {
    let row: std::result::Result<(String, String), _> = store.conn().query_row(
        "SELECT payload, fetched_at FROM meta_cache WHERE normalized = ?1",
        [name.as_str()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    );
    match row {
        Ok((payload, fetched_at)) => {
            let Ok(meta) = serde_json::from_str(&payload) else {
                return Ok(None);
            };
            let Ok(at) = fetched_at.parse() else {
                return Ok(None);
            };
            Ok(Some((meta, at)))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(PdError::new(
            Code::IntUnexpected,
            format!("metadata cache read: {e}"),
        )),
    }
}

async fn fetch_metadata(name: &PkgName) -> Result<PackageMeta> {
    let url = format!("https://pypi.org/pypi/{}/json", name.as_str());
    let response = http_client()?
        .get(&url)
        .send()
        .await
        .map_err(|e| net_err("could not reach PyPI", &e))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(PdError::new(
            Code::PkgNotFound,
            format!("PyPI has no package named {name}"),
        ));
    }
    let doc: serde_json::Value = response
        .json()
        .await
        .map_err(|e| net_err("could not read PyPI's response", &e))?;

    let info = doc.get("info").unwrap_or(&serde_json::Value::Null);
    let text = |key: &str| {
        info.get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };

    Ok(PackageMeta {
        name: name.as_str().to_owned(),
        summary: text("summary"),
        version: text("version"),
        requires_python: text("requires_python"),
        license: text("license_expression").or_else(|| text("license")),
        home_page: text("home_page").or_else(|| text("project_url")),
    })
}

/// An HTTPS client restricted to PyPI's hosts.
///
/// SECURITY §4: certificate verification is always on and redirects may not leave `pypi.org` or
/// `files.pythonhosted.org`, so a hijacked redirect cannot walk the client somewhere else.
fn http_client() -> Result<reqwest::Client> {
    const ALLOWED: [&str; 2] = ["pypi.org", "files.pythonhosted.org"];

    reqwest::Client::builder()
        .user_agent(concat!("pipdock/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let host_ok = attempt.url().host_str().is_some_and(|h| {
                ALLOWED
                    .iter()
                    .any(|a| h == *a || h.ends_with(&format!(".{a}")))
            });
            if host_ok && attempt.previous().len() < 5 {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|e| PdError::new(Code::IntUnexpected, format!("http client: {e}")))
}

fn net_err(what: &str, e: &reqwest::Error) -> PdError {
    // A TLS failure gets its own code because its guidance is different: corporate proxy or AV
    // interception, and never "turn verification off" (SECURITY §4).
    let text = e.to_string().to_ascii_lowercase();
    let code = if text.contains("certificate") || text.contains("tls") || text.contains("ssl") {
        Code::NetTlsFailure
    } else {
        Code::NetUnreachable
    };
    PdError::new(code, format!("{what}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(names: &[&str]) -> NameIndex {
        NameIndex {
            entries: names
                .iter()
                .map(|n| (crate::model::normalize_name(n), (*n).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn the_package_you_typed_ranks_first() {
        // SP-3's finding, and the reason ranking is tiered: raw nucleo scores put requests-ntlm
        // above requests, which makes the most common query in the product look broken.
        let idx = index(&[
            "requests-ntlm",
            "requests-html-playwright",
            "requests",
            "requests-go",
            "requests-toolbelt",
        ]);
        let hits = idx.search("requests", 5);

        assert_eq!(hits[0].name.as_str(), "requests");
        assert_eq!(hits[0].kind, MatchKind::Exact);
    }

    #[test]
    fn the_numpy_case_from_the_spike() {
        let idx = index(&["numpy-utils", "numpy-financial", "numpy", "numpydoc"]);
        assert_eq!(idx.search("numpy", 4)[0].name.as_str(), "numpy");
    }

    #[test]
    fn prefix_matches_beat_fuzzy_ones() {
        let idx = index(&[
            "django-rest-framework",
            "djangorestframework",
            "dj-database-url",
        ]);
        let hits = idx.search("django", 3);

        assert!(
            hits.iter().take(2).all(|h| h.kind == MatchKind::Prefix),
            "both django* names should outrank the fuzzy one: {hits:?}"
        );
        // Between two prefix matches, the shorter one wins: it is nearly always the canonical
        // package, and the longer ones are its plugins.
        assert_eq!(hits[0].name.as_str(), "djangorestframework");
    }

    #[test]
    fn fuzzy_matching_still_finds_things_that_are_not_prefixes() {
        let idx = index(&["beautifulsoup4", "soupsieve"]);
        let hits = idx.search("bs4", 5);
        // Not a prefix of anything, but the letters appear in order in beautifulsoup4.
        assert!(
            hits.iter().any(|h| h.name.as_str() == "beautifulsoup4"),
            "{hits:?}"
        );
    }

    #[test]
    fn very_short_queries_return_prefixes_rather_than_fuzzy_noise() {
        // Fuzzy-matching one character across 858 k names cost most of the latency budget and
        // returned noise. Someone one character in wants names that start that way.
        let idx = index(&["requests", "rich", "beautifulsoup4", "urllib3"]);
        let hits = idx.search("r", 10);

        assert!(
            hits.iter().all(|h| h.kind != MatchKind::Fuzzy),
            "no fuzzy hits for a 1-char query: {hits:?}"
        );
        assert!(hits.iter().any(|h| h.name.as_str() == "requests"));
        assert!(hits.iter().any(|h| h.name.as_str() == "rich"));
        // urllib3 contains an 'r' but does not start with one, so it must not appear.
        assert!(
            !hits.iter().any(|h| h.name.as_str() == "urllib3"),
            "{hits:?}"
        );
    }

    #[test]
    fn the_fuzzy_threshold_is_where_the_documentation_says() {
        let idx = index(&["beautifulsoup4"]);
        assert_eq!(MIN_FUZZY_QUERY_LEN, 3);
        // Two characters: prefix only, so no hit.
        assert!(idx.search("bs", 5).is_empty());
        // Three: fuzzy is on, and the subsequence b-s-4 is found.
        assert_eq!(idx.search("bs4", 5)[0].name.as_str(), "beautifulsoup4");
    }

    #[test]
    fn the_subsequence_prefilter_matches_the_scorers_necessary_condition() {
        // The prefilter exists to skip nucleo for names that cannot match. If it were stricter
        // than the scorer it would silently drop real results.
        assert!(is_subsequence("bs4", "beautifulsoup4"));
        assert!(is_subsequence("req", "requests"));
        assert!(is_subsequence("", "anything"));
        assert!(!is_subsequence("bs4", "requests"));
        assert!(!is_subsequence("zzz", "beautifulsoup4"));
        // Order matters: the same letters in the wrong order is not a subsequence.
        assert!(!is_subsequence("4sb", "beautifulsoup4"));
    }

    #[test]
    fn ranking_is_unchanged_by_the_selection_optimisation() {
        // Ranking by selection rather than a full sort was a performance change only; if it
        // altered the order the two most common queries in the product would regress.
        let idx = index(&[
            "requests-ntlm",
            "requests",
            "requests-go",
            "requests-toolbelt",
        ]);
        let all = idx.search("requests", 100);
        let capped = idx.search("requests", 2);
        assert_eq!(
            capped,
            all[..2],
            "a limit must take the same head, in the same order"
        );
    }

    #[test]
    fn a_query_matching_nothing_returns_nothing() {
        let idx = index(&["requests", "httpx"]);
        assert!(idx.search("zzzzqqqxxx", 10).is_empty());
    }

    #[test]
    fn results_are_stable_between_identical_queries() {
        // A list that reorders under the cursor between keystrokes is unusable.
        let idx = index(&["aa-tool", "ab-tool", "ac-tool"]);
        let first = idx.search("tool", 3);
        let second = idx.search("tool", 3);
        assert_eq!(first, second);
    }

    #[test]
    fn the_query_is_normalized_like_a_package_name() {
        // A user typing "Zope.Interface" must find "zope-interface".
        let idx = index(&["zope.interface"]);
        let hits = idx.search("Zope_Interface", 5);
        assert_eq!(hits[0].name.as_str(), "zope-interface");
        assert_eq!(hits[0].kind, MatchKind::Exact);
        assert_eq!(
            hits[0].display, "zope.interface",
            "display keeps PyPI's spelling"
        );
    }

    #[test]
    fn empty_and_zero_limit_queries_are_cheap_no_ops() {
        let idx = index(&["requests"]);
        assert!(idx.search("", 10).is_empty());
        assert!(idx.search("   ", 10).is_empty());
        assert!(idx.search("requests", 0).is_empty());
    }

    #[test]
    fn the_limit_is_respected() {
        let names: Vec<String> = (0..50).map(|i| format!("tool-{i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        assert_eq!(index(&refs).search("tool", 10).len(), 10);
    }

    #[test]
    fn an_empty_index_says_to_refresh_rather_than_failing_obscurely() {
        let store = Store::in_memory().expect("store");
        let err = NameIndex::load(&store).expect_err("empty index must not load");
        assert_eq!(err.code, Code::NetIndexRefreshFailed);
        assert!(err.message.contains("refresh"), "{}", err.message);
    }

    #[test]
    fn staleness_follows_the_documented_interval() {
        let now: jiff::Timestamp = "2026-07-27T12:00:00Z".parse().unwrap();
        assert!(is_stale(None, now), "never refreshed is stale");

        let six_days: jiff::Timestamp = "2026-07-21T12:00:00Z".parse().unwrap();
        assert!(!is_stale(Some(six_days), now));

        let eight_days: jiff::Timestamp = "2026-07-19T12:00:00Z".parse().unwrap();
        assert!(is_stale(Some(eight_days), now));
    }

    #[test]
    fn the_documented_budgets_are_what_sp3_measured_against() {
        assert_eq!(SEARCH_LATENCY_BUDGET, Duration::from_millis(50));
        assert_eq!(COLD_REFRESH_BUDGET, Duration::from_secs(60));
        assert_eq!(INDEX_REFRESH_INTERVAL_DAYS, 7);
        assert_eq!(METADATA_TTL_HOURS, 24);
    }

    #[test]
    fn ingest_replaces_rather_than_appends() {
        // A refresh that appended would double the index every time and quietly break dedup.
        let store = Store::in_memory().expect("store");
        ingest(
            &store,
            &[("a".into(), "A".into()), ("b".into(), "B".into())],
        )
        .expect("first");
        ingest(&store, &[("c".into(), "C".into())]).expect("second");

        let idx = NameIndex::load(&store).expect("load");
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.search("c", 5)[0].display, "C");
    }

    #[test]
    fn last_refresh_round_trips() {
        let store = Store::in_memory().expect("store");
        assert!(last_refresh(&store).expect("read").is_none());

        let now: jiff::Timestamp = "2026-07-27T12:00:00Z".parse().unwrap();
        store
            .set(KEY_LAST_REFRESH, &now.to_string())
            .expect("write");
        assert_eq!(last_refresh(&store).expect("read"), Some(now));
    }
}
