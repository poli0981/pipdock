//! pip-audit over an installed environment — PRD P1-1, SECURITY §6.
//!
//! Runs out of **its own venv** ([`crate::health::AUDIT_TOOLS`], [`crate::health::audit_dir`]) and
//! in freeze-file mode, which SP-4 established is the only mode there is: `--path <site-packages>`
//! is rejected outright when combined with `--no-deps`, so the environment reaches the tool as a
//! `pip freeze` document rather than as a directory.
//!
//! Four things this module exists to get right, every one measured against the pinned 2.10.1
//! rather than read out of a document (`tests/fixtures/audit/`):
//!
//! 1. **The ids are `PYSEC-*`, because the default vulnerability service is PyPI.** Left as the
//!    default deliberately: PipDock querying `api.osv.dev` itself would add a second network
//!    destination and falsify `legal/PRIVACY-POLICY.md` §3's "exactly one destination". Findings
//!    still *link* to OSV, which is a visit the user's browser makes after a click — the case §3
//!    already carves out.
//! 2. **There is no severity, under either service.** The vuln object carries exactly `id`,
//!    `fix_versions`, `aliases` and `description`. PRD P1-1's "severity-sorted" and SECURITY §6's
//!    "(CVE/GHSA id, severity, fixed-in)" were written against a field that has never existed, so
//!    [`sort_advisories`] sorts by what is real instead.
//! 3. **Ids repeat.** The captured run returns ten rows for **eight** advisories; the same
//!    environment under `--vulnerability-service osv` returns sixteen rows and the *same* eight.
//!    A count taken before [`dedupe`] is wrong under either service.
//! 4. **stderr is not a failure signal.** pip-audit writes a `pip-compile`/hashes advisory on
//!    every single run. Parse stdout first and classify only if that fails — the rule SP-1 left
//!    behind for uv, which writes its plan to stderr for the same reason.

use std::collections::BTreeSet;

use crate::errors::{Code, PdError, Result};
use crate::model::{PkgName, Version};

/// Where a finding links. The id is appended after validation — never interpolated raw.
const OSV_BASE: &str = "https://osv.dev/vulnerability/";

/// One advisory against one installed distribution.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct Advisory {
    /// The installed distribution the advisory is against.
    pub pkg: PkgName,
    /// The version installed, not the version that fixes it.
    pub version: Version,
    /// pip-audit's primary id — `PYSEC-*` under the default service.
    pub id: String,
    /// Other ids for the same advisory, which is where the `CVE-*` and `GHSA-*` live.
    ///
    /// PRD P1-1 says "known CVEs"; a CVE is never the primary id here, so a screen that wants to
    /// show one reads this.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Versions that fix it. **Empty is meaningful** — an advisory with nowhere to upgrade to —
    /// and [`sort_advisories`] puts those last rather than hiding them.
    #[serde(default)]
    pub fix_versions: Vec<String>,
    /// The advisory prose, verbatim. Never translated (I18N §2) and never trimmed here.
    pub description: String,
    /// The OSV entry, when the id is shaped like one.
    ///
    /// **Built from a validated id rather than taken from the tool**, which is the whole
    /// difference between this and `PdHealthReport`'s `finding.url`.
    /// `capabilities/external-links.json` records that ruff's URL is the only one in the
    /// application derived from a third party's output; this one is derived from a string PipDock
    /// has checked, so widening the allowlist to `osv.dev` does not widen what can reach it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// The OSV entry for an advisory id, or `None` if the id is not shaped like one.
///
/// The check is a whitelist, not an escape: ASCII letters, digits and `-`, 4 to 64 characters,
/// leading letter. Nothing that passes can carry a `/`, `?`, `#`, `:` or `%`, so the result cannot
/// be anything but a path segment under [`OSV_BASE`] — which is what lets the opener allowlist
/// stay scoped to one host. SECURITY §2's obligation follows the *input*, and this input is a
/// third-party tool's JSON.
#[must_use]
pub fn advisory_url(id: &str) -> Option<String> {
    let ok = (4..=64).contains(&id.len())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && id.starts_with(|c: char| c.is_ascii_alphabetic());
    ok.then(|| format!("{OSV_BASE}{id}"))
}

/// pip-audit's `-f json` document, only the parts PipDock reads.
mod raw {
    #[derive(serde::Deserialize)]
    pub struct Doc {
        #[serde(default)]
        pub dependencies: Vec<Dep>,
    }

    #[derive(serde::Deserialize)]
    pub struct Dep {
        pub name: String,
        #[serde(default)]
        pub version: String,
        #[serde(default)]
        pub vulns: Vec<Vuln>,
    }

    #[derive(serde::Deserialize)]
    pub struct Vuln {
        pub id: String,
        #[serde(default)]
        pub fix_versions: Vec<String>,
        #[serde(default)]
        pub aliases: Vec<String>,
        #[serde(default)]
        pub description: String,
    }
}

/// How many distributions the run covered, and what it found.
///
/// The count is separate from `advisories.len()` on purpose: "audited 145 packages, found nothing"
/// and "audited nothing" are different answers, and a screen that cannot tell them apart repeats
/// P4's *no issues found before anything had run*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Audited {
    /// Distributions present in the freeze document.
    pub packages: usize,
    /// Deduplicated and sorted.
    pub advisories: Vec<Advisory>,
}

/// Parse a pip-audit `-f json` document.
///
/// # Errors
/// `PD-HLT-002` when stdout is not the document `--format json` promises. That is the only failure
/// mode here: a non-zero exit is **not** one, because findings exit 1.
pub fn parse(stdout: &str) -> Result<Audited> {
    let doc: raw::Doc = serde_json::from_str(stdout).map_err(|e| {
        PdError::new(
            Code::HltToolFailed,
            format!("pip-audit did not emit the JSON document --format json promises: {e}"),
        )
    })?;

    let mut advisories = Vec::new();
    for dep in &doc.dependencies {
        // A name pip-audit echoed back out of a freeze document PipDock wrote. Validated anyway:
        // it is the join key against the installed table, and an unparseable one would be a row
        // matching nothing rather than a row that is merely wrong.
        let Ok(pkg) = PkgName::parse(&dep.name) else {
            continue;
        };
        for vuln in &dep.vulns {
            advisories.push(Advisory {
                pkg: pkg.clone(),
                version: Version(dep.version.clone()),
                url: advisory_url(&vuln.id),
                id: vuln.id.clone(),
                aliases: vuln.aliases.clone(),
                fix_versions: vuln.fix_versions.clone(),
                description: vuln.description.clone(),
            });
        }
    }

    Ok(Audited {
        packages: doc.dependencies.len(),
        advisories: sort_advisories(dedupe(advisories)),
    })
}

/// Collapse advisories repeated for one package.
///
/// pip-audit emits the same id more than once — twice each for two of the eight in the captured
/// run, and eight times each under the OSV service. The key is `(pkg, id)` rather than `id` alone,
/// because one advisory legitimately applies to two different installed packages.
#[must_use]
pub fn dedupe(advisories: Vec<Advisory>) -> Vec<Advisory> {
    let mut seen = BTreeSet::new();
    advisories
        .into_iter()
        .filter(|a| seen.insert((a.pkg.clone(), a.id.clone())))
        .collect()
}

/// Order: package, then whether anything fixes it, then id.
///
/// **Not severity.** PRD P1-1 asked for severity-sorted and pip-audit has no such field, so this
/// sorts by the one distinction that changes what a user can *do*: an advisory with a
/// `fix_versions` entry is one the normal Update flow can act on, and an advisory without one is
/// not. Inventing a severity — from the id's year, from the description's wording — would be a
/// number PipDock made up, shown beside real ones.
#[must_use]
pub fn sort_advisories(mut advisories: Vec<Advisory>) -> Vec<Advisory> {
    advisories.sort_by(|a, b| {
        a.pkg
            .cmp(&b.pkg)
            .then(a.fix_versions.is_empty().cmp(&b.fix_versions.is_empty()))
            .then(a.id.cmp(&b.id))
    });
    advisories
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pinned 2.10.1 over `urllib3==2.0.0` — see the directory's README for provenance.
    const CAPTURE: &str = include_str!("../tests/fixtures/audit/urllib3-2.0.0.json");

    #[test]
    fn the_capture_parses_and_collapses_to_eight() {
        let got = parse(CAPTURE).expect("the committed capture must parse");

        assert_eq!(got.packages, 1);
        // Ten rows in the document, eight advisories. Asserted as a number rather than "fewer than
        // before", because this count is what a Security tab puts on screen.
        assert_eq!(got.advisories.len(), 8, "ten rows collapse to eight");

        let ids: BTreeSet<&str> = got.advisories.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids.len(), 8, "no id survives twice");
        assert!(ids.contains("PYSEC-2023-192"));
    }

    #[test]
    fn the_cve_is_an_alias_and_never_the_id() {
        let got = parse(CAPTURE).expect("parses");

        let a = got
            .advisories
            .iter()
            .find(|a| a.id == "PYSEC-2023-192")
            .expect("the capture carries it");
        assert!(a.aliases.contains(&"CVE-2023-43804".to_owned()));
        assert!(a.aliases.iter().any(|x| x.starts_with("GHSA-")));
        assert!(
            !a.id.starts_with("CVE-"),
            "PRD says CVEs; the id is not one"
        );
        assert_eq!(a.fix_versions, ["1.26.17", "2.0.6"]);
        assert_eq!(a.version.0, "2.0.0", "the installed version, not the fix");
    }

    #[test]
    fn every_advisory_links_to_its_osv_entry() {
        let got = parse(CAPTURE).expect("parses");

        for a in &got.advisories {
            assert_eq!(
                a.url,
                Some(format!("{OSV_BASE}{}", a.id)),
                "{} lost its link",
                a.id
            );
        }
    }

    #[test]
    fn an_id_that_could_escape_the_path_segment_gets_no_link() {
        // Why the URL is built rather than taken: these are what a compromised — or merely
        // surprising — tool output could otherwise carry into `opener:allow-open-url`.
        for bad in [
            "../../evil",
            "PYSEC-2023-192/../../x",
            "PYSEC 2023 192",
            "javascript:alert(1)",
            "https://evil.test/x",
            "PYSEC-2023-192?x=1",
            "PYSEC-2023-192#f",
            "%2e%2e",
            "1PYSEC",
            "ab",
        ] {
            assert_eq!(advisory_url(bad), None, "{bad:?} must not become a URL");
        }

        assert!(advisory_url("PYSEC-2023-192").is_some());
        assert!(advisory_url("GHSA-v845-jxx5-vc9f").is_some());
        assert!(advisory_url("CVE-2023-43804").is_some());
    }

    #[test]
    fn fixable_advisories_sort_ahead_of_unfixable_ones() {
        // The order PRD P1-1 gets instead of severity, so it is asserted rather than assumed.
        let mk = |pkg: &str, id: &str, fixes: &[&str]| Advisory {
            pkg: PkgName::parse(pkg).expect("valid"),
            version: Version("1.0".to_owned()),
            id: id.to_owned(),
            aliases: vec![],
            fix_versions: fixes.iter().map(|s| (*s).to_owned()).collect(),
            description: String::new(),
            url: None,
        };
        let got = sort_advisories(vec![
            mk("zlib-wrapper", "PYSEC-1", &["1.1"]),
            mk("aiohttp", "PYSEC-3", &[]),
            mk("aiohttp", "PYSEC-2", &["2.0"]),
        ]);

        let order: Vec<&str> = got.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(order, ["PYSEC-2", "PYSEC-3", "PYSEC-1"]);
    }

    #[test]
    fn stdout_that_is_not_the_promised_document_is_a_tool_failure() {
        // Not a panic, and not a silent empty report: pip-audit printing anything else means the
        // run did not do what was asked, and PD-HLT-002 is the code that says so.
        let err = parse("Found 10 known vulnerabilities in 1 package").expect_err("not JSON");
        assert_eq!(err.code, Code::HltToolFailed);
    }

    #[test]
    fn a_clean_environment_is_zero_advisories_over_a_real_package_count() {
        let doc =
            r#"{"dependencies":[{"name":"certifi","version":"2024.2.2","vulns":[]}],"fixes":[]}"#;
        let got = parse(doc).expect("parses");

        assert_eq!((got.packages, got.advisories.len()), (1, 0));
    }
}
