use crate::i18n::catalog::{LocaleCatalog, ScopeLookup};
use crate::i18n::{DEFAULT_LANGUAGE, I18nManager, SUPPORTED_LANG_CODES};
#[cfg(test)]
use std::collections::HashMap;

/// Owns the press vocabulary: `assets/i18n/news/{lang}.json`.
///
/// That bundle is a thousand keys and more — every story kind with its
/// phrasing variants, the desk kickers, the mastheads, the mood straps
/// and the paper chrome. Only the three newspaper pages render any of
/// it, so it is loaded and resolved apart from the page-chrome scope
/// rather than inflating the map every other page clones per request.
pub struct NewsI18nManager {
    news: LocaleCatalog,
}

impl NewsI18nManager {
    pub fn new() -> Self {
        NewsI18nManager {
            news: LocaleCatalog::required(Some("news"), SUPPORTED_LANG_CODES),
        }
    }

    pub fn for_lang(&self, lang: &str) -> NewsI18n {
        let lang_key = I18nManager::resolved_lang(&self.news, lang);
        NewsI18n {
            news: self.news.resolve(lang_key, DEFAULT_LANGUAGE),
        }
    }
}

impl Default for NewsI18nManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Press translations for one request. Resolves story copy and paper
/// chrome only; page chrome stays on [`crate::I18n`], which the newspaper
/// templates carry alongside this.
pub struct NewsI18n {
    news: ScopeLookup,
}

impl NewsI18n {
    /// Build a test-only `NewsI18n` from a flat key→string map, for renderer
    /// tests that exercise headline / body / variant selection without
    /// standing up the bundle loader.
    #[cfg(test)]
    pub fn for_test(map: HashMap<String, String>) -> Self {
        NewsI18n {
            news: ScopeLookup::from_map(map),
        }
    }

    /// Resolve `key`, returning the key itself when this locale has no copy
    /// for it. The story composer depends on that identity: probing
    /// `news_h_<stem>_v2` and comparing against the key is how it discovers
    /// how many phrasing variants a story kind ships.
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.news.t(key)
    }
}

#[cfg(test)]
mod tests {
    use crate::i18n::bundle_tests;
    use std::collections::BTreeSet;

    const SCOPE: &str = "i18n/news";

    const BUNDLES: &[(&str, &[u8])] = &[
        ("en", include_bytes!("../../assets/i18n/news/en.json")),
        ("de", include_bytes!("../../assets/i18n/news/de.json")),
        ("es", include_bytes!("../../assets/i18n/news/es.json")),
        ("fr", include_bytes!("../../assets/i18n/news/fr.json")),
        ("ja", include_bytes!("../../assets/i18n/news/ja.json")),
        ("pt", include_bytes!("../../assets/i18n/news/pt.json")),
        ("ru", include_bytes!("../../assets/i18n/news/ru.json")),
        ("tr", include_bytes!("../../assets/i18n/news/tr.json")),
        ("zh", include_bytes!("../../assets/i18n/news/zh.json")),
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

    /// The press scope resolves on its own, with no chrome behind it — so a
    /// key that exists in both bundles is not a harmless duplicate, it is
    /// two translations of one string waiting to drift apart.
    #[test]
    fn the_press_scope_shares_no_key_with_page_chrome() {
        let news = bundle_tests::parse(SCOPE, "en", BUNDLES[0].1);
        let chrome = bundle_tests::parse("i18n", "en", include_bytes!("../../assets/i18n/en.json"));
        let shared: BTreeSet<&str> = news
            .keys()
            .filter(|k| chrome.contains_key(*k))
            .map(String::as_str)
            .collect();
        assert!(
            shared.is_empty(),
            "these key(s) are in both the press and chrome bundles: {:?}",
            shared
        );
    }
}
