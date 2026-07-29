mod catalog;
pub mod events;
pub mod news;

use crate::i18n::catalog::{LocaleCatalog, ScopeLookup};
use chrono::{Datelike, NaiveDate, NaiveDateTime};
#[cfg(test)]
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// (lang_code, flag_code, display_name)
pub const SUPPORTED_LANGUAGES: &[(&str, &str, &str)] = &[
    ("en", "us", "English"),
    ("es", "es", "Español"),
    ("fr", "fr", "Français"),
    ("de", "de", "Deutsch"),
    ("pt", "pt", "Português"),
    ("ru", "ru", "Русский"),
    ("zh", "cn", "繁體中文"),
    ("tr", "tr", "Türkçe"),
    ("ja", "jp", "日本語"),
];

pub const SUPPORTED_LANG_CODES: &[&str] = &["en", "es", "fr", "de", "pt", "ru", "zh", "tr", "ja"];

pub const DEFAULT_LANGUAGE: &str = "en";

const MONTH_KEYS: &[&str] = &[
    "month_jan",
    "month_feb",
    "month_mar",
    "month_apr",
    "month_may",
    "month_jun",
    "month_jul",
    "month_aug",
    "month_sep",
    "month_oct",
    "month_nov",
    "month_dec",
];

const DAY_KEYS: &[&str] = &[
    "day_mon", "day_tue", "day_wed", "day_thu", "day_fri", "day_sat", "day_sun",
];

/// Owns the page-chrome vocabulary: `assets/i18n/{lang}.json` plus the
/// country-name bundles.
///
/// Deliberately *not* the whole application's dictionary. The newspaper and
/// happiness-event pages each render a vocabulary an order of magnitude
/// larger than the rest of the UI put together, and neither is reachable
/// from a normal page. They live in their own scopes, loaded by
/// [`news::NewsI18nManager`] and [`events::EventI18nManager`], so the map
/// every page clones per request stays small and a UI template cannot
/// accidentally resolve a press headline.
pub struct I18nManager {
    ui: Arc<LocaleCatalog>,
    country_names: LocaleCatalog,
    date: RwLock<NaiveDateTime>,
}

impl I18nManager {
    pub fn new() -> Self {
        I18nManager {
            ui: Arc::new(LocaleCatalog::required(None, SUPPORTED_LANG_CODES)),
            country_names: LocaleCatalog::optional("countries", SUPPORTED_LANG_CODES),
            date: RwLock::new(NaiveDateTime::default()),
        }
    }

    /// The chrome catalog, shared with scopes that fall back to it for the
    /// handful of generic labels they reuse (award names, "now", …).
    pub(crate) fn ui_catalog(&self) -> Arc<LocaleCatalog> {
        Arc::clone(&self.ui)
    }

    /// The language a request resolves under: the one asked for when it
    /// ships a bundle, the default otherwise.
    pub(crate) fn resolved_lang<'a>(catalog: &LocaleCatalog, lang: &'a str) -> &'a str {
        if catalog.has(lang) {
            lang
        } else {
            DEFAULT_LANGUAGE
        }
    }

    pub fn set_date(&self, date: NaiveDateTime) {
        *self.date.write().unwrap() = date;
    }

    pub fn for_lang(&self, lang: &str) -> I18n {
        let lang_key = Self::resolved_lang(&self.ui, lang);
        let translations = self.ui.resolve(lang_key, DEFAULT_LANGUAGE);

        let date = *self.date.read().unwrap();
        let month_key = MONTH_KEYS[date.month0() as usize];
        let day_key = DAY_KEYS[date.weekday().num_days_from_monday() as usize];

        let date_main = format!(
            "{} {} {}",
            date.day(),
            translations.t(month_key),
            date.year()
        );
        let date_sub = translations.t(day_key).to_string();

        I18n {
            translations,
            country_names: self.country_names.resolve(lang_key, DEFAULT_LANGUAGE),
            lang: lang_key.to_string(),
            date_main,
            date_sub,
        }
    }

    pub fn is_supported_language(lang: &str) -> bool {
        SUPPORTED_LANG_CODES.contains(&lang)
    }
}

impl Default for I18nManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Page-chrome translations for one request. Resolves labels, menus and
/// dates — never press copy or happiness-event copy.
pub struct I18n {
    translations: ScopeLookup,
    country_names: ScopeLookup,
    pub lang: String,
    pub date_main: String,
    pub date_sub: String,
}

pub struct LangOption {
    pub code: &'static str,
    pub flag: &'static str,
    pub name: &'static str,
}

impl I18n {
    /// Build a test-only `I18n` from a flat key→string map. Renderer
    /// unit tests use this to exercise the cause / headline / evidence
    /// branches without standing up the full bundle loader. Translations
    /// fall back to the same map for missing keys (mirroring production
    /// `t()` semantics: unknown keys return the key itself).
    #[cfg(test)]
    pub fn for_test(map: HashMap<String, String>) -> Self {
        Self {
            translations: ScopeLookup::from_map(map),
            country_names: ScopeLookup::empty(),
            lang: "en".to_string(),
            date_main: String::new(),
            date_sub: String::new(),
        }
    }

    pub fn format_date(&self, date: NaiveDate) -> String {
        let d = date.day();
        let m = date.month();
        let y = date.year();
        let month_name = self.t(MONTH_KEYS[date.month0() as usize]);
        match self.lang.as_str() {
            "en" => format!("{} {} {}", d, month_name, y),
            "es" | "fr" | "pt" => format!("{:02}/{:02}/{}", d, m, y),
            "de" | "ru" | "tr" => format!("{:02}.{:02}.{}", d, m, y),
            "zh" | "ja" => format!("{}年{:02}月{:02}日", y, m, d),
            _ => format!("{:02}.{:02}.{}", d, m, y),
        }
    }

    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.translations.t(key)
    }

    pub fn country<'a>(&'a self, code: &'a str) -> &'a str {
        self.country_names.t(code)
    }

    pub fn country_en<'a>(&'a self, code: &'a str) -> &'a str {
        self.country_names.default_get(code).unwrap_or(code)
    }

    pub fn current_flag(&self) -> &'static str {
        SUPPORTED_LANGUAGES
            .iter()
            .find(|(code, _, _)| *code == self.lang)
            .map(|(_, flag, _)| *flag)
            .unwrap_or("us")
    }

    pub fn current_name(&self) -> &'static str {
        SUPPORTED_LANGUAGES
            .iter()
            .find(|(code, _, _)| *code == self.lang)
            .map(|(_, _, name)| *name)
            .unwrap_or("English")
    }

    pub fn languages(&self) -> Vec<LangOption> {
        SUPPORTED_LANGUAGES
            .iter()
            .map(|(code, flag, name)| LangOption { code, flag, name })
            .collect()
    }
}

/// Parse the `Accept-Language` header and return the best supported language.
///
/// Respects quality weights (e.g. `fr;q=0.9, de;q=0.8, en;q=0.5`).
/// Falls back to `DEFAULT_LANGUAGE` if nothing matches.
pub fn detect_language(accept_language: &str) -> String {
    let mut candidates: Vec<(&str, f32)> = Vec::new();

    for part in accept_language.split(',') {
        let mut sections = part.split(';');
        let lang_tag = sections.next().unwrap_or("").trim();
        let lang_prefix = lang_tag.split('-').next().unwrap_or("").trim();

        // Parse quality value: "q=0.8" → 0.8, absent → 1.0
        let quality = sections
            .find_map(|s| {
                let s = s.trim();
                s.strip_prefix("q=").and_then(|v| v.parse::<f32>().ok())
            })
            .unwrap_or(1.0);

        if let Some(&code) = SUPPORTED_LANG_CODES
            .iter()
            .find(|&&c| c.eq_ignore_ascii_case(lang_prefix))
        {
            candidates.push((code, quality));
        }
    }

    // Highest quality first; on tie, keep original order (already stable)
    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    candidates
        .first()
        .map(|(code, _)| code.to_string())
        .unwrap_or_else(|| DEFAULT_LANGUAGE.to_string())
}

#[cfg(test)]
pub(crate) mod bundle_tests {
    use super::DEFAULT_LANGUAGE;
    use std::collections::{BTreeMap, BTreeSet};

    /// A value is treated as prose — and therefore must be localised — once it
    /// is long enough to be a phrase rather than a code. Short labels ("Pts",
    /// "GK", "Final", "Port") legitimately coincide with English in several
    /// languages, so they are exempt by construction rather than by list.
    const PROSE_MIN_LEN: usize = 12;

    pub(crate) fn parse(scope: &str, lang: &str, bytes: &[u8]) -> BTreeMap<String, String> {
        let text = std::str::from_utf8(bytes)
            .unwrap_or_else(|_| panic!("{}/{}.json is not UTF-8", scope, lang));
        assert!(
            !text.starts_with('\u{feff}'),
            "{}/{}.json starts with a UTF-8 BOM — strip it, Windows editors add it back on save",
            scope,
            lang
        );
        serde_json::from_str(text)
            .unwrap_or_else(|e| panic!("{}/{}.json is not valid JSON: {}", scope, lang, e))
    }

    /// A key missing from a locale silently falls back to English, so the page
    /// renders half-translated instead of failing loudly. Key parity is what
    /// keeps that from happening unnoticed — and it is checked in both
    /// directions, so a key that moves between scopes has to move in every
    /// locale at once.
    pub(crate) fn assert_key_parity(scope: &str, bundles: &[(&str, &[u8])]) {
        let en = parse(scope, DEFAULT_LANGUAGE, lookup(bundles, DEFAULT_LANGUAGE));
        for (lang, bytes) in bundles.iter().filter(|(c, _)| *c != DEFAULT_LANGUAGE) {
            let loc = parse(scope, lang, bytes);
            let missing: Vec<&str> = en
                .keys()
                .filter(|k| !loc.contains_key(*k))
                .map(String::as_str)
                .collect();
            let extra: Vec<&str> = loc
                .keys()
                .filter(|k| !en.contains_key(*k))
                .map(String::as_str)
                .collect();
            assert!(
                missing.is_empty(),
                "{}/{}.json is missing {} key(s) present in en.json: {:?}",
                scope,
                lang,
                missing.len(),
                missing
            );
            assert!(
                extra.is_empty(),
                "{}/{}.json has {} key(s) absent from en.json: {:?}",
                scope,
                lang,
                extra.len(),
                extra
            );
        }
    }

    /// Presence alone is not translation: a key can be added to a locale with
    /// the English sentence pasted in to satisfy a key-coverage test, and the
    /// reader then sees English inside an otherwise localised page.
    pub(crate) fn assert_prose_is_translated(
        scope: &str,
        bundles: &[(&str, &[u8])],
        exempt: &[&str],
    ) {
        let en = parse(scope, DEFAULT_LANGUAGE, lookup(bundles, DEFAULT_LANGUAGE));
        for (lang, bytes) in bundles.iter().filter(|(c, _)| *c != DEFAULT_LANGUAGE) {
            let loc = parse(scope, lang, bytes);
            let untranslated: Vec<&str> = en
                .iter()
                .filter(|(key, value)| {
                    value.chars().count() >= PROSE_MIN_LEN
                        && value.contains(char::is_whitespace)
                        && !exempt.contains(&key.as_str())
                        && loc.get(*key) == Some(*value)
                })
                .map(|(key, _)| key.as_str())
                .collect();
            assert!(
                untranslated.is_empty(),
                "{}/{}.json still carries the English text for {} key(s): {:?}",
                scope,
                lang,
                untranslated.len(),
                untranslated
            );
        }
    }

    /// `{min}` / `{rating}` are substituted by the renderer. A translation
    /// that drops or renames one leaves a literal brace on the page.
    /// Compared as a set, not a count: English strings that carry a
    /// `singular|plural` pair repeat their placeholder, and languages
    /// without a plural form legitimately name it once.
    pub(crate) fn assert_placeholders_match(scope: &str, bundles: &[(&str, &[u8])]) {
        let placeholders = |s: &str| -> BTreeSet<String> {
            s.split('{')
                .skip(1)
                .filter_map(|part| part.split_once('}'))
                .map(|(name, _)| name.to_string())
                .collect()
        };
        let en = parse(scope, DEFAULT_LANGUAGE, lookup(bundles, DEFAULT_LANGUAGE));
        for (lang, bytes) in bundles.iter().filter(|(c, _)| *c != DEFAULT_LANGUAGE) {
            for (key, value) in parse(scope, lang, bytes) {
                let Some(reference) = en.get(&key) else {
                    continue;
                };
                assert_eq!(
                    placeholders(&value),
                    placeholders(reference),
                    "{}/{}.json key {} has different placeholders than en.json",
                    scope,
                    lang,
                    key
                );
            }
        }
    }

    fn lookup<'a>(bundles: &'a [(&str, &'a [u8])], lang: &str) -> &'a [u8] {
        bundles
            .iter()
            .find(|(code, _)| *code == lang)
            .map(|(_, bytes)| *bytes)
            .unwrap_or_else(|| panic!("no bundle for {}", lang))
    }
}

#[cfg(test)]
mod tests {
    use super::bundle_tests;
    use super::{DEFAULT_LANGUAGE, detect_language};

    /// Every shipped chrome bundle, paired with its language code.
    /// `en.json` is the reference the others are measured against.
    const BUNDLES: &[(&str, &[u8])] = &[
        ("en", include_bytes!("../../assets/i18n/en.json")),
        ("de", include_bytes!("../../assets/i18n/de.json")),
        ("es", include_bytes!("../../assets/i18n/es.json")),
        ("fr", include_bytes!("../../assets/i18n/fr.json")),
        ("ja", include_bytes!("../../assets/i18n/ja.json")),
        ("pt", include_bytes!("../../assets/i18n/pt.json")),
        ("ru", include_bytes!("../../assets/i18n/ru.json")),
        ("tr", include_bytes!("../../assets/i18n/tr.json")),
        ("zh", include_bytes!("../../assets/i18n/zh.json")),
    ];

    /// Prose-length values that are proper nouns: competition brands and
    /// coaching-licence tiers that are written the same way in every language.
    const PROSE_EXEMPT: &[&str] = &[
        "supporters_shield",
        "champions_league",
        "europa_league",
        "conference_league",
        "copa_libertadores",
        "license_continental_a",
        "license_continental_b",
        "license_continental_c",
        "license_continental_pro",
    ];

    #[test]
    fn every_locale_carries_the_full_english_key_set() {
        bundle_tests::assert_key_parity("i18n", BUNDLES);
    }

    #[test]
    fn locale_prose_is_actually_translated() {
        bundle_tests::assert_prose_is_translated("i18n", BUNDLES, PROSE_EXEMPT);
    }

    #[test]
    fn locale_placeholders_match_english() {
        bundle_tests::assert_placeholders_match("i18n", BUNDLES);
    }

    /// The chrome bundle must not regrow the vocabularies that were split out
    /// into their own scopes — that is the whole point of the split, and a
    /// stray `news_*` key here would be resolvable from any page.
    #[test]
    fn chrome_bundle_carries_no_scoped_vocabulary() {
        let en = bundle_tests::parse("i18n", DEFAULT_LANGUAGE, BUNDLES[0].1);
        let strays: Vec<&str> = en
            .keys()
            .filter(|k| {
                k.starts_with("news_")
                    || k.starts_with("newspaper_")
                    || k.starts_with("masthead_")
                    || k.starts_with("press_mood_")
                    || k.starts_with("event_")
            })
            .map(String::as_str)
            .collect();
        assert!(
            strays.is_empty(),
            "en.json carries {} key(s) that belong in the news / events scope: {:?}",
            strays.len(),
            strays
        );
    }

    /// Loads all three scopes the way startup does. A bundle can ship in
    /// `assets/` and still never be read — the loader names its paths in
    /// code — so this walks every supported language through every scope
    /// and resolves a key that only that scope carries.
    ///
    /// It also pins the split itself: page chrome must come back empty for
    /// a press or event key, whichever page is asking.
    #[test]
    fn every_scope_loads_and_answers_only_for_itself() {
        use crate::i18n::events::EventI18nManager;
        use crate::i18n::news::NewsI18nManager;
        use crate::i18n::{I18nManager, SUPPORTED_LANG_CODES};

        let chrome = I18nManager::new();
        let news = NewsI18nManager::new();
        let events = EventI18nManager::new(&chrome);

        for &lang in SUPPORTED_LANG_CODES {
            let (chrome, news, events) = (
                chrome.for_lang(lang),
                news.for_lang(lang),
                events.for_lang(lang),
            );

            assert_ne!(chrome.t("site_name"), "site_name", "{lang} chrome");
            assert_ne!(news.t("news_desk_match"), "news_desk_match", "{lang} news");
            assert_ne!(
                events.t("event_label_cause"),
                "event_label_cause",
                "{lang} events"
            );

            // Event copy reaches chrome for the generic labels it shares.
            assert_ne!(
                events.t("player_of_the_week"),
                "player_of_the_week",
                "{lang} events → chrome fallback"
            );

            // …and nothing reaches back the other way.
            assert_eq!(chrome.t("news_desk_match"), "news_desk_match", "{lang}");
            assert_eq!(chrome.t("event_label_cause"), "event_label_cause", "{lang}");
        }
    }

    #[test]
    fn accept_language_header_picks_the_highest_quality_supported_tag() {
        assert_eq!(detect_language("fr-FR,fr;q=0.9,en;q=0.8"), "fr");
        assert_eq!(detect_language("en;q=0.5, de;q=0.8, fr;q=0.9"), "fr");
        assert_eq!(detect_language("kl-GL,kl"), DEFAULT_LANGUAGE);
    }
}
