//! What a release requires of the interpreter — the half of PipDock's own `Requires-Python`
//! enforcement an engine's plan cannot always supply (hard invariant 10).
//!
//! The engines disagree, which is why PipDock checks at all: pip filters candidates by
//! `Requires-Python` and refuses an incompatible exact pin with the same "No matching
//! distribution" it gives a typo; uv honours the lower bound and **ignores an upper bound**, so it
//! plans `scipy==1.7.3` (`<3.10`) on Python 3.14 and the install fails later, building from
//! source. pip's report carries each planned release's `requires_python`; uv's plan is text and
//! carries nothing, so for uv — and for an exact pin under either engine, checked before the
//! engine runs — the answer comes from PyPI's per-release JSON.
//!
//! **Unknown never blocks.** A release PyPI does not know (a private index), a network that is
//! down, a specifier that will not parse: each is "do not block", because refusing an installable
//! package on PipDock's uncertainty is the worse failure, and the engine still judges.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::model::{PkgName, Version};
use crate::store::Store;

/// How many lookups may be in flight at once. A plan for "update everything" can name fifty
/// releases; PyPI is a CDN, but fifty simultaneous connections from one desktop app is rude.
pub const LOOKUP_CONCURRENCY: usize = 8;

/// Per-request watchdog. A lookup that takes longer is treated as unknown, which never blocks.
pub const LOOKUP_TIMEOUT: Duration = Duration::from_secs(15);

/// Where `Requires-Python` of a release comes from.
#[async_trait]
pub trait ReleaseMeta: Send + Sync + std::fmt::Debug {
    /// `Requires-Python` of `name==version`; `None` when it declares none **or nobody knows** —
    /// both mean "do not block".
    async fn requires_python(&self, name: &PkgName, version: &Version) -> Option<String>;
}

/// PyPI's per-release JSON, remembered in the store's `release_meta` table **forever**: an
/// uploaded release's metadata never changes, so a row is never refreshed. Only an answer is
/// remembered; a failure is asked again next time.
#[derive(Debug, Clone)]
pub struct PyPiReleases {
    app_data: PathBuf,
}

impl PyPiReleases {
    /// Lookups cached under `app_data`'s store.
    #[must_use]
    pub fn new(app_data: impl Into<PathBuf>) -> Self {
        Self {
            app_data: app_data.into(),
        }
    }

    /// `Some(answer)` when this release has been asked about before.
    ///
    /// The store is opened for this one read and dropped: `Store` is not `Sync`, and holding it
    /// across the network call would make every caller's future not `Send`.
    fn cached(&self, name: &PkgName, version: &Version) -> Option<Option<String>> {
        let store = Store::open(&self.app_data).ok()?;
        store
            .conn()
            .query_row(
                "SELECT requires_python FROM release_meta WHERE normalized = ?1 AND version = ?2",
                rusqlite::params![name.as_str(), version.0],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
    }

    fn remember(&self, name: &PkgName, version: &Version, requires_python: Option<&str>) {
        // A cache that cannot be written only costs the next run a request.
        if let Ok(store) = Store::open(&self.app_data) {
            let _ = store.conn().execute(
                "INSERT INTO release_meta (normalized, version, requires_python)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (normalized, version) DO UPDATE SET
                    requires_python = excluded.requires_python",
                rusqlite::params![name.as_str(), version.0, requires_python],
            );
        }
    }
}

#[async_trait]
impl ReleaseMeta for PyPiReleases {
    async fn requires_python(&self, name: &PkgName, version: &Version) -> Option<String> {
        if let Some(answer) = self.cached(name, version) {
            return answer;
        }
        let answer = fetch(name, version).await?;
        self.remember(name, version, answer.as_deref());
        answer
    }
}

/// `GET https://pypi.org/pypi/{name}/{version}/json` → `info.requires_python`.
///
/// `Some(None)` is a real answer — the release declares no constraint. `None` is "could not
/// find out": not on PyPI, unreachable, slow, unreadable.
async fn fetch(name: &PkgName, version: &Version) -> Option<Option<String>> {
    // The version goes into a URL path; only a PEP 440 version is allowed to.
    if !crate::pep440::is_version(&version.0) {
        return None;
    }
    let url = format!("https://pypi.org/pypi/{}/{}/json", name.as_str(), version.0);
    let response = crate::index::shared_client()
        .ok()?
        .get(&url)
        .timeout(LOOKUP_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let doc: serde_json::Value = response.json().await.ok()?;
    Some(declared_in(&doc))
}

/// `info.requires_python` of a release document, `None` when it is absent, null or blank — all
/// three of which PyPI serves for a release that declares nothing.
fn declared_in(doc: &serde_json::Value) -> Option<String> {
    doc.get("info")
        .and_then(|i| i.get("requires_python"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Look up many releases at once, at most [`LOOKUP_CONCURRENCY`] in flight. Releases with no
/// known constraint are absent from the answer.
pub async fn requires_python_of(
    meta: &Arc<dyn ReleaseMeta>,
    releases: Vec<(PkgName, Version)>,
) -> BTreeMap<(PkgName, String), String> {
    let gate = Arc::new(tokio::sync::Semaphore::new(LOOKUP_CONCURRENCY));
    let mut set = tokio::task::JoinSet::new();
    for (name, version) in releases {
        let meta = Arc::clone(meta);
        let gate = Arc::clone(&gate);
        set.spawn(async move {
            let _permit = gate.acquire_owned().await.ok();
            let answer = meta.requires_python(&name, &version).await;
            (name, version, answer)
        });
    }
    let mut out = BTreeMap::new();
    while let Some(joined) = set.join_next().await {
        if let Ok((name, version, Some(requires))) = joined {
            out.insert((name, version.0), requires);
        }
    }
    out
}

/// `Some` when `requires_python` excludes the interpreter at `python` (e.g. `"3.14.7"`).
///
/// An unparseable interpreter version or specifier is not a mismatch — see the module note.
#[must_use]
pub fn mismatch(
    pkg: &PkgName,
    version: &Version,
    requires_python: Option<&str>,
    python: &str,
) -> Option<crate::plan::PythonMismatch> {
    let interpreter = crate::compat::PyVersion::parse(python).ok()?;
    match crate::compat::check(requires_python, &interpreter) {
        crate::compat::Compatibility::RequiresPython { required, .. } => {
            Some(crate::plan::PythonMismatch {
                pkg: pkg.clone(),
                version: version.clone(),
                requires_python: required,
                python: python.to_owned(),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(n: &str) -> PkgName {
        PkgName::parse(n).unwrap()
    }

    #[test]
    fn a_capped_release_on_a_newer_python_is_a_mismatch() {
        let m = mismatch(
            &pkg("scipy"),
            &Version("1.7.3".into()),
            Some(">=3.7,<3.10"),
            "3.14.7",
        )
        .expect("excluded");
        assert_eq!(m.requires_python, ">=3.7,<3.10");
        assert_eq!(m.python, "3.14.7");
        assert!(
            mismatch(
                &pkg("scipy"),
                &Version("1.16.0".into()),
                Some(">=3.11"),
                "3.14.7"
            )
            .is_none()
        );
    }

    #[test]
    fn what_cannot_be_read_never_blocks() {
        let v = Version("1.0".into());
        assert!(mismatch(&pkg("a"), &v, None, "3.14.7").is_none());
        assert!(mismatch(&pkg("a"), &v, Some(""), "3.14.7").is_none());
        assert!(mismatch(&pkg("a"), &v, Some("not a specifier"), "3.14.7").is_none());
        assert!(mismatch(&pkg("a"), &v, Some("<3.10"), "not a version").is_none());
    }

    #[test]
    fn an_answer_is_remembered_and_a_failure_is_not() {
        let dir = std::env::temp_dir().join(format!("pd-release-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let meta = PyPiReleases::new(&dir);
        let (name, v) = (pkg("scipy"), Version("1.7.3".into()));
        assert_eq!(meta.cached(&name, &v), None, "never asked");
        meta.remember(&name, &v, Some(">=3.7,<3.10"));
        assert_eq!(meta.cached(&name, &v), Some(Some(">=3.7,<3.10".to_owned())));
        // "Declares nothing" is an answer too, and is not asked again.
        let free = Version("1.0".into());
        meta.remember(&name, &free, None);
        assert_eq!(meta.cached(&name, &free), Some(None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_remembered_release_is_answered_without_asking_pypi() {
        // Both answers come from the store: a constraint, and "declares nothing". Neither
        // reaches the network, which this test has no way to offer.
        let dir = std::env::temp_dir().join(format!("pd-release-cached-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let meta = PyPiReleases::new(&dir);
        let name = pkg("scipy");
        meta.remember(&name, &Version("1.7.3".into()), Some(">=3.7,<3.10"));
        meta.remember(&name, &Version("1.16.0".into()), None);

        let capped = meta.requires_python(&name, &Version("1.7.3".into())).await;
        let free = meta.requires_python(&name, &Version("1.16.0".into())).await;
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(capped.as_deref(), Some(">=3.7,<3.10"));
        assert_eq!(free, None);
    }

    #[tokio::test]
    async fn a_version_that_is_not_pep_440_never_reaches_a_url() {
        // `fetch` puts the version into a URL path; anything else is unknown, which never blocks,
        // and is not remembered as if it were an answer.
        let dir = std::env::temp_dir().join(format!("pd-release-url-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let meta = PyPiReleases::new(&dir);
        let (name, odd) = (pkg("scipy"), Version("1.0/../../evil".into()));
        assert_eq!(meta.requires_python(&name, &odd).await, None);
        assert_eq!(meta.cached(&name, &odd), None, "a failure is not an answer");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_release_document_s_constraint_is_read_trimmed_and_blank_is_none() {
        let doc = |v: serde_json::Value| serde_json::json!({ "info": { "requires_python": v } });
        assert_eq!(
            declared_in(&doc(" >=3.9 ".into())).as_deref(),
            Some(">=3.9")
        );
        assert_eq!(declared_in(&doc("".into())), None);
        assert_eq!(declared_in(&doc(serde_json::Value::Null)), None);
        assert_eq!(declared_in(&serde_json::json!({ "info": {} })), None);
        assert_eq!(declared_in(&serde_json::json!({ "urls": [] })), None);
    }
}
