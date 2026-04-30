use crate::common::default_handler::Assets;
use chrono::{Datelike, NaiveDateTime};
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
            let map: HashMap<String, String> = serde_json::from_str(json_str)
                .unwrap_or_else(|e| panic!("Invalid JSON in {}: {}", path, e));
            translations.insert(lang.to_string(), Arc::new(map));

            let countries_path = format!("i18n/countries/{}.json", lang);
            if let Some(data) = Assets::get(&countries_path) {
                let json_str = std::str::from_utf8(&data.data)
                    .unwrap_or_else(|_| panic!("Invalid UTF-8 in {}", countries_path));
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
