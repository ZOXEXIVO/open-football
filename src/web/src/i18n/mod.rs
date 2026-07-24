use crate::common::default_handler::Assets;
use chrono::{Datelike, NaiveDate, NaiveDateTime};
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

pub struct I18nManager {
    translations: HashMap<String, Arc<HashMap<String, String>>>,
    country_names: HashMap<String, Arc<HashMap<String, String>>>,
    date: RwLock<NaiveDateTime>,
}

impl I18nManager {
    pub fn new() -> Self {
        let mut translations = HashMap::new();
        let mut country_names = HashMap::new();

        for &(lang, _, _) in SUPPORTED_LANGUAGES {
            let path = format!("i18n/{}.json", lang);
            let data =
                Assets::get(&path).unwrap_or_else(|| panic!("Missing translation file: {}", path));
            let json_str = std::str::from_utf8(&data.data)
                .unwrap_or_else(|_| panic!("Invalid UTF-8 in {}", path));
            // Tolerate a UTF-8 BOM at the start of the file — Windows
            // editors / PowerShell sometimes prepend one and serde_json
            // rejects it as "expected value at line 1 column 1".
            let json_str = json_str.strip_prefix('\u{feff}').unwrap_or(json_str);
            let map: HashMap<String, String> = serde_json::from_str(json_str)
                .unwrap_or_else(|e| panic!("Invalid JSON in {}: {}", path, e));
            translations.insert(lang.to_string(), Arc::new(map));

            let countries_path = format!("i18n/countries/{}.json", lang);
            if let Some(data) = Assets::get(&countries_path) {
                let json_str = std::str::from_utf8(&data.data)
                    .unwrap_or_else(|_| panic!("Invalid UTF-8 in {}", countries_path));
                let json_str = json_str.strip_prefix('\u{feff}').unwrap_or(json_str);
                let map: HashMap<String, String> = serde_json::from_str(json_str)
                    .unwrap_or_else(|e| panic!("Invalid JSON in {}: {}", countries_path, e));
                country_names.insert(lang.to_string(), Arc::new(map));
            }
        }

        I18nManager {
            translations,
            country_names,
            date: RwLock::new(NaiveDateTime::default()),
        }
    }

    pub fn set_date(&self, date: NaiveDateTime) {
        *self.date.write().unwrap() = date;
    }

    pub fn for_lang(&self, lang: &str) -> I18n {
        let lang_key = if self.translations.contains_key(lang) {
            lang
        } else {
            DEFAULT_LANGUAGE
        };

        let translations = self
            .translations
            .get(lang_key)
            .cloned()
            .unwrap_or_else(|| Arc::new(HashMap::new()));
        let fallback = if lang_key != DEFAULT_LANGUAGE {
            self.translations
                .get(DEFAULT_LANGUAGE)
                .cloned()
                .unwrap_or_else(|| Arc::new(HashMap::new()))
        } else {
            translations.clone()
        };

        let date = *self.date.read().unwrap();
        let month_key = MONTH_KEYS[date.month0() as usize];
        let day_key = DAY_KEYS[date.weekday().num_days_from_monday() as usize];

        let t = |key: &str| -> String {
            translations
                .get(key)
                .or_else(|| fallback.get(key))
                .cloned()
                .unwrap_or_else(|| key.to_string())
        };

        let date_main = format!("{} {} {}", date.day(), t(month_key), date.year());
        let date_sub = t(day_key);

        let country_names = self
            .country_names
            .get(lang_key)
            .cloned()
            .unwrap_or_else(|| Arc::new(HashMap::new()));
        let country_names_fallback = if lang_key != DEFAULT_LANGUAGE {
            self.country_names
                .get(DEFAULT_LANGUAGE)
                .cloned()
                .unwrap_or_else(|| Arc::new(HashMap::new()))
        } else {
            country_names.clone()
        };

        I18n {
            translations,
            fallback,
            country_names,
            country_names_fallback,
            lang: lang_key.to_string(),
            date_main,
            date_sub,
        }
    }

    pub fn is_supported_language(lang: &str) -> bool {
        SUPPORTED_LANG_CODES.contains(&lang)
    }
}

pub struct I18n {
    translations: Arc<HashMap<String, String>>,
    fallback: Arc<HashMap<String, String>>,
    country_names: Arc<HashMap<String, String>>,
    country_names_fallback: Arc<HashMap<String, String>>,
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
        let translations = Arc::new(map);
        let fallback = translations.clone();
        Self {
            translations,
            fallback,
            country_names: Arc::new(HashMap::new()),
            country_names_fallback: Arc::new(HashMap::new()),
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
        self.translations
            .get(key)
            .or_else(|| self.fallback.get(key))
            .map(|s| s.as_str())
            .unwrap_or(key)
    }

    pub fn country<'a>(&'a self, code: &'a str) -> &'a str {
        self.country_names
            .get(code)
            .or_else(|| self.country_names_fallback.get(code))
            .map(|s| s.as_str())
            .unwrap_or(code)
    }

    pub fn country_en<'a>(&'a self, code: &'a str) -> &'a str {
        self.country_names_fallback
            .get(code)
            .map(|s| s.as_str())
            .unwrap_or(code)
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
mod tests {
    use super::{DEFAULT_LANGUAGE, detect_language};
    use std::collections::{BTreeMap, BTreeSet};

    /// Every shipped bundle, paired with its language code. `en.json` is the
    /// reference the others are measured against.
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

    /// A value is treated as prose — and therefore must be localised — once it
    /// is long enough to be a phrase rather than a code. Short labels ("Pts",
    /// "GK", "Final", "Port") legitimately coincide with English in several
    /// languages, so they are exempt by construction rather than by list.
    const PROSE_MIN_LEN: usize = 12;

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

    fn bundle(lang: &str) -> BTreeMap<String, String> {
        let (_, bytes) = BUNDLES
            .iter()
            .find(|(code, _)| *code == lang)
            .unwrap_or_else(|| panic!("no bundle for {}", lang));
        let text =
            std::str::from_utf8(bytes).unwrap_or_else(|_| panic!("{}.json is not UTF-8", lang));
        assert!(
            !text.starts_with('\u{feff}'),
            "{}.json starts with a UTF-8 BOM — strip it, Windows editors add it back on save",
            lang
        );
        serde_json::from_str(text)
            .unwrap_or_else(|e| panic!("{}.json is not valid JSON: {}", lang, e))
    }

    #[test]
    fn every_locale_carries_the_full_english_key_set() {
        // A key missing from a locale silently falls back to English, so the
        // page renders half-translated instead of failing loudly. Key parity
        // is what keeps that from happening unnoticed.
        let en = bundle(DEFAULT_LANGUAGE);
        for (lang, _) in BUNDLES.iter().filter(|(c, _)| *c != DEFAULT_LANGUAGE) {
            let loc = bundle(lang);
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
                "{}.json is missing {} key(s) present in en.json: {:?}",
                lang,
                missing.len(),
                missing
            );
            assert!(
                extra.is_empty(),
                "{}.json has {} key(s) absent from en.json: {:?}",
                lang,
                extra.len(),
                extra
            );
        }
    }

    #[test]
    fn locale_prose_is_actually_translated() {
        // Presence alone is not translation: a key can be added to a locale
        // with the English sentence pasted in to satisfy a key-coverage test,
        // and the reader then sees English inside an otherwise localised page.
        let en = bundle(DEFAULT_LANGUAGE);
        for (lang, _) in BUNDLES.iter().filter(|(c, _)| *c != DEFAULT_LANGUAGE) {
            let loc = bundle(lang);
            let untranslated: Vec<&str> = en
                .iter()
                .filter(|(key, value)| {
                    value.chars().count() >= PROSE_MIN_LEN
                        && value.contains(char::is_whitespace)
                        && !PROSE_EXEMPT.contains(&key.as_str())
                        && loc.get(*key) == Some(*value)
                })
                .map(|(key, _)| key.as_str())
                .collect();
            assert!(
                untranslated.is_empty(),
                "{}.json still carries the English text for {} key(s): {:?}",
                lang,
                untranslated.len(),
                untranslated
            );
        }
    }

    #[test]
    fn locale_placeholders_match_english() {
        // `{min}` / `{rating}` are substituted by the renderer. A translation
        // that drops or renames one leaves a literal brace on the page.
        // Compared as a set, not a count: English strings that carry a
        // `singular|plural` pair repeat their placeholder, and languages
        // without a plural form legitimately name it once.
        let placeholders = |s: &str| -> BTreeSet<String> {
            s.split('{')
                .skip(1)
                .filter_map(|part| part.split_once('}'))
                .map(|(name, _)| name.to_string())
                .collect()
        };
        let en = bundle(DEFAULT_LANGUAGE);
        for (lang, _) in BUNDLES.iter().filter(|(c, _)| *c != DEFAULT_LANGUAGE) {
            for (key, value) in bundle(lang) {
                let Some(reference) = en.get(&key) else {
                    continue;
                };
                assert_eq!(
                    placeholders(&value),
                    placeholders(reference),
                    "{}.json key {} has different placeholders than en.json",
                    lang,
                    key
                );
            }
        }
    }

    #[test]
    fn accept_language_header_picks_the_highest_quality_supported_tag() {
        assert_eq!(detect_language("fr-FR,fr;q=0.9,en;q=0.8"), "fr");
        assert_eq!(detect_language("en;q=0.5, de;q=0.8, fr;q=0.9"), "fr");
        assert_eq!(detect_language("kl-GL,kl"), DEFAULT_LANGUAGE);
    }
}
