use crate::i18n::catalog::{LocaleCatalog, ScopeLookup};
use crate::i18n::{DEFAULT_LANGUAGE, I18nManager, SUPPORTED_LANG_CODES};
#[cfg(test)]
use std::collections::HashMap;
use std::sync::Arc;

/// Owns the happiness-event vocabulary: `assets/i18n/events/{lang}.json`.
///
/// That bundle is ~850 keys — every event headline with its cause, evidence
/// and follow-up sentences. Only the player events page renders it, so it is
/// loaded and resolved apart from the page-chrome scope.
///
/// Unlike the press scope, this one keeps a handle on the chrome catalog:
/// event copy reuses a dozen generic labels that also appear elsewhere in
/// the UI ("now", the award names, competition names). Falling back to
/// chrome for those keeps a single translation of each rather than a copy
/// per scope that drifts. The dependency runs one way only — chrome still
/// cannot resolve an event key.
pub struct EventI18nManager {
    events: LocaleCatalog,
    ui: Arc<LocaleCatalog>,
}

impl EventI18nManager {
    pub fn new(chrome: &I18nManager) -> Self {
        EventI18nManager {
            events: LocaleCatalog::required(Some("events"), SUPPORTED_LANG_CODES),
            ui: chrome.ui_catalog(),
        }
    }

    pub fn for_lang(&self, lang: &str) -> EventI18n {
        let lang_key = I18nManager::resolved_lang(&self.events, lang);
        EventI18n {
            events: self.events.resolve(lang_key, DEFAULT_LANGUAGE),
            ui: self.ui.resolve(lang_key, DEFAULT_LANGUAGE),
        }
    }
}

/// Happiness-event translations for one request.
pub struct EventI18n {
    events: ScopeLookup,
    ui: ScopeLookup,
}

impl EventI18n {
    /// Build a test-only `EventI18n` from a flat key→string map. The map
    /// answers both scopes, so a test fixture can mix event copy with the
    /// generic labels the renderers reach for.
    #[cfg(test)]
    pub fn for_test(map: HashMap<String, String>) -> Self {
        let lookup = ScopeLookup::from_map(map);
        EventI18n {
            events: lookup.clone(),
            ui: lookup,
        }
    }

    /// Resolve `key` against the event scope, then the chrome scope,
    /// returning the key itself when neither has copy for it. Renderers
    /// depend on that identity: comparing the result against the key is how
    /// they skip a sentence a locale has not translated yet.
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.events.get(key).unwrap_or_else(|| self.ui.t(key))
    }
}

#[cfg(test)]
mod tests {
    use crate::i18n::bundle_tests;
    use std::collections::BTreeSet;

    const SCOPE: &str = "i18n/events";

    const BUNDLES: &[(&str, &[u8])] = &[
        ("en", include_bytes!("../../assets/i18n/events/en.json")),
        ("de", include_bytes!("../../assets/i18n/events/de.json")),
        ("es", include_bytes!("../../assets/i18n/events/es.json")),
        ("fr", include_bytes!("../../assets/i18n/events/fr.json")),
        ("ja", include_bytes!("../../assets/i18n/events/ja.json")),
        ("pt", include_bytes!("../../assets/i18n/events/pt.json")),
        ("ru", include_bytes!("../../assets/i18n/events/ru.json")),
        ("tr", include_bytes!("../../assets/i18n/events/tr.json")),
        ("zh", include_bytes!("../../assets/i18n/events/zh.json")),
    ];

    #[test]
    fn every_locale_carries_the_full_english_key_set() {
        bundle_tests::assert_key_parity(SCOPE, BUNDLES);
    }

    #[test]
    fn locale_prose_is_actually_translated() {
        bundle_tests::assert_prose_is_translated(SCOPE, BUNDLES, &[]);
    }

    #[test]
    fn locale_placeholders_match_english() {
        bundle_tests::assert_placeholders_match(SCOPE, BUNDLES);
    }

    /// This scope falls back to chrome, so a key present in both would
    /// shadow rather than clash — silently, and only for this page. That is
    /// worse than a duplicate: the two copies drift and nobody sees it.
    #[test]
    fn the_event_scope_shares_no_key_with_page_chrome() {
        let events = bundle_tests::parse(SCOPE, "en", BUNDLES[0].1);
        let chrome = bundle_tests::parse("i18n", "en", include_bytes!("../../assets/i18n/en.json"));
        let shared: BTreeSet<&str> = events
            .keys()
            .filter(|k| chrome.contains_key(*k))
            .map(String::as_str)
            .collect();
        assert!(
            shared.is_empty(),
            "these key(s) are in both the event and chrome bundles — the event copy \
             shadows the chrome one: {:?}",
            shared
        );
    }
}
