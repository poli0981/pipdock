//! User settings, shared by both heads.
//!
//! Engine choice used to live in `pipdock-cli` behind a private `KEY_ENGINE` constant, which
//! meant the Settings screen would have had to guess the same key and the same encoding. A
//! setting written by one head and unreadable by the other is worse than no setting.
//!
//! Stored in the `kv` table of `index.db` rather than the `config.json` ARCHITECTURE §6 mentions:
//! the store already exists, already handles concurrent access, and a second on-disk format is a
//! second thing to migrate.

use crate::errors::Result;
use crate::model::EngineId;
use crate::store::Store;

/// `kv` keys. Namespaced so a future setting cannot collide with the index cache's own rows.
const KEY_ENGINE: &str = "settings.engine";
const KEY_LOCALE: &str = "settings.locale";
const KEY_ALLOW_EXTERNALLY_MANAGED: &str = "settings.allowExternallyManaged";
const KEY_PIN_SUGGEST_THRESHOLD: &str = "settings.pinSuggestThreshold";
const KEY_CONSENT_HASH: &str = "legal.consentDocsHash";
const KEY_CONSENT_AT: &str = "legal.consentAt";

/// Everything the Settings screen reads and writes (UI-SPEC §4).
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Which resolver to drive.
    pub engine: EngineId,
    /// UI language. `None` means "follow the OS", which is what a fresh install does.
    #[serde(default)]
    pub locale: Option<String>,
    /// The PEP 668 override (SECURITY §3).
    ///
    /// **Off by default, and never inferred.** When on, every mutating screen for such an
    /// environment shows a persistent warning chip, and only then may the
    /// `--break-system-packages` equivalent be passed. PipDock never adds that flag silently —
    /// hard invariant 5.
    pub allow_externally_managed: bool,
    /// How many reverse dependencies qualify a package for a pin suggestion (PRD P1-2).
    ///
    /// UI-SPEC §4 lists this under Settings' thresholds. Zero means off — see
    /// [`crate::pins::suggest`], which would otherwise offer to pin every leaf in the environment.
    pub pin_suggest_threshold: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            // pip is the safe default: it exists wherever Python does. ARCHITECTURE §3 has first
            // run preselect uv when it is on PATH, but that is a Settings-screen decision made
            // visibly, not something a read of an empty store should do behind the user's back.
            engine: EngineId::Pip,
            locale: None,
            allow_externally_managed: false,
            // The const, not a literal 5. It exists so the default has one home, and it lived in
            // `graph` unread from M2 until P1-A gave it a caller.
            pin_suggest_threshold: crate::graph::PIN_SUGGEST_THRESHOLD,
        }
    }
}

/// Read the stored settings, falling back to defaults for anything unset or unreadable.
///
/// # Errors
/// Propagates store failures. A *missing* value is not one — that is a fresh install.
pub fn load(store: &Store) -> Result<Settings> {
    let mut out = Settings::default();
    if let Some(raw) = store.get(KEY_ENGINE)? {
        // An unrecognised value means a downgrade or a hand-edited database. Keeping the default
        // is better than failing to open Settings, which is where it would be fixed.
        match raw.as_str() {
            "uv" => out.engine = EngineId::Uv,
            "pip" => out.engine = EngineId::Pip,
            _ => {}
        }
    }
    out.locale = store.get(KEY_LOCALE)?.filter(|s| !s.is_empty());
    if let Some(raw) = store.get(KEY_ALLOW_EXTERNALLY_MANAGED)? {
        out.allow_externally_managed = raw == "true";
    }
    if let Some(raw) = store.get(KEY_PIN_SUGGEST_THRESHOLD)? {
        // Same rule as the engine key above: an unparseable value keeps the default rather than
        // failing the load. Settings is where the user would fix it, so it has to open.
        if let Ok(n) = raw.parse::<usize>() {
            out.pin_suggest_threshold = n;
        }
    }
    Ok(out)
}

/// Persist settings.
///
/// # Errors
/// Propagates store failures.
pub fn save(store: &Store, settings: &Settings) -> Result<()> {
    store.set(KEY_ENGINE, settings.engine.as_str())?;
    store.set(KEY_LOCALE, settings.locale.as_deref().unwrap_or(""))?;
    store.set(
        KEY_ALLOW_EXTERNALLY_MANAGED,
        if settings.allow_externally_managed {
            "true"
        } else {
            "false"
        },
    )?;
    store.set(
        KEY_PIN_SUGGEST_THRESHOLD,
        &settings.pin_suggest_threshold.to_string(),
    )
}

/// The recorded acceptance of the legal documents (UI-SPEC §4).
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct Consent {
    /// Hash of the documents that were accepted.
    pub docs_hash: String,
    /// When, RFC 3339.
    pub accepted_at: String,
}

/// Hash of the legal documents this build ships against.
///
/// Computed at **build time** from the tracked files (see `build.rs`) rather than at runtime:
/// runtime hashing would mean shipping copies of documents that are deliberately hosted on
/// GitHub, and a hash of files that are not there is a hash of nothing.
///
/// UI-SPEC §4: a bump re-triggers the gate. That is the point — it is a re-consent trigger, not
/// an integrity check.
#[must_use]
pub const fn docs_hash() -> &'static str {
    env!("PIPDOCK_LEGAL_DOCS_HASH")
}

/// Whether the gate should be shown.
///
/// # Errors
/// Propagates store failures.
pub fn consent(store: &Store) -> Result<Option<Consent>> {
    let Some(docs_hash) = store.get(KEY_CONSENT_HASH)? else {
        return Ok(None);
    };
    let accepted_at = store.get(KEY_CONSENT_AT)?.unwrap_or_default();
    Ok(Some(Consent {
        docs_hash,
        accepted_at,
    }))
}

/// Record acceptance of the documents this build ships against.
///
/// # Errors
/// Propagates store failures.
pub fn accept_consent(store: &Store, now: jiff::Timestamp) -> Result<Consent> {
    let record = Consent {
        docs_hash: docs_hash().to_owned(),
        accepted_at: now.to_string(),
    };
    store.set(KEY_CONSENT_HASH, &record.docs_hash)?;
    store.set(KEY_CONSENT_AT, &record.accepted_at)?;
    Ok(record)
}

/// True when the user has accepted *these* documents.
///
/// # Errors
/// Propagates store failures.
pub fn consent_is_current(store: &Store) -> Result<bool> {
    Ok(consent(store)?.is_some_and(|c| c.docs_hash == docs_hash()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::in_memory().expect("in-memory store")
    }

    #[test]
    fn a_fresh_install_reads_defaults() {
        let s = load(&store()).expect("load");
        assert_eq!(s, Settings::default());
        assert_eq!(s.engine, EngineId::Pip);
        assert!(!s.allow_externally_managed, "SECURITY §3: off by default");
        assert_eq!(s.locale, None, "None means follow the OS");
        assert_eq!(
            s.pin_suggest_threshold,
            crate::graph::PIN_SUGGEST_THRESHOLD,
            "PRD P1-2's default of 5, from the const rather than a second copy of it"
        );
    }

    #[test]
    fn an_unparseable_threshold_falls_back_rather_than_failing() {
        // Same rule as the engine key: Settings is where a bad value gets fixed, so it must open.
        let store = store();
        store
            .set("settings.pinSuggestThreshold", "lots")
            .expect("set");
        assert_eq!(
            load(&store).expect("load").pin_suggest_threshold,
            crate::graph::PIN_SUGGEST_THRESHOLD
        );
    }

    #[test]
    fn a_threshold_of_zero_survives_a_round_trip() {
        // Zero is the "off" setting, so it must not be mistaken for absent and replaced by the
        // default — which is exactly what a `filter(|n| *n > 0)` in `load` would do.
        let store = store();
        let want = Settings {
            pin_suggest_threshold: 0,
            ..Settings::default()
        };
        save(&store, &want).expect("save");
        assert_eq!(load(&store).expect("load").pin_suggest_threshold, 0);
    }

    #[test]
    fn settings_round_trip() {
        let store = store();
        let want = Settings {
            engine: EngineId::Uv,
            locale: Some("vi".into()),
            allow_externally_managed: true,
            pin_suggest_threshold: 12,
        };
        save(&store, &want).expect("save");
        assert_eq!(load(&store).expect("load"), want);
    }

    #[test]
    fn an_unreadable_engine_value_falls_back_rather_than_failing() {
        // A downgrade or a hand-edited database must not make the Settings screen unopenable —
        // that is where the user would fix it.
        let store = store();
        store.set("settings.engine", "conda").expect("set");
        assert_eq!(load(&store).expect("load").engine, EngineId::Pip);
    }

    #[test]
    fn an_empty_locale_reads_as_follow_the_os() {
        // `save` writes "" for None, so loading it back must not produce Some("").
        let store = store();
        save(&store, &Settings::default()).expect("save");
        assert_eq!(load(&store).expect("load").locale, None);
    }

    #[test]
    fn consent_is_recorded_against_the_documents_it_accepted() {
        let store = store();
        assert!(!consent_is_current(&store).expect("check"), "fresh install");
        assert_eq!(consent(&store).expect("read"), None);

        let now: jiff::Timestamp = "2026-07-29T12:00:00Z".parse().expect("timestamp");
        let record = accept_consent(&store, now).expect("accept");
        assert_eq!(record.docs_hash, docs_hash());
        assert!(consent_is_current(&store).expect("check"));
    }

    #[test]
    fn consent_for_older_documents_is_not_current() {
        // UI-SPEC §4: a docs-hash bump re-triggers the gate. Stale consent must read as stale
        // rather than as "close enough".
        let store = store();
        store
            .set("legal.consentDocsHash", "an-older-build")
            .expect("set");
        assert!(consent(&store).expect("read").is_some());
        assert!(!consent_is_current(&store).expect("check"));
    }

    #[test]
    fn the_docs_hash_is_baked_in_and_stable() {
        let hash = docs_hash();
        assert_eq!(hash.len(), 64, "sha-256 hex: {hash}");
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "{hash}");
        assert_eq!(hash, docs_hash(), "must not vary between calls");
    }
}
