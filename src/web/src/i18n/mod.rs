use crate::common::default_handler::Assets;
use std::collections::HashMap;
use std::sync::Arc;

/// (lang_code, flag_code, display_name)
pub const SUPPORTED_LANGUAGES: &[(&str, &str, &str)] = &[
    ("en", "us", "English"),
    ("es", "es", "Español"),
    ("ru", "ru", "Русский"),
];

const SUPPORTED_LANG_CODES: &[&str] = &["en", "es", "ru"];

pub const DEFAULT_LANGUAGE: &str = "en";

pub struct I18nManager {
    translations: HashMap<String, Arc<HashMap<String, String>>>,
}

impl I18nManager {
    pub fn new() -> Self {
        let mut translations = HashMap::new();

        for &(lang, _, _) in SUPPORTED_LANGUAGES {
            let path = format!("i18n/{}.json", lang);
            let data = Assets::get(&path)
                .unwrap_or_else(|| panic!("Missing translation file: {}", path));
            let json_str = std::str::from_utf8(&data.data)
                .unwrap_or_else(|_| panic!("Invalid UTF-8 in {}", path));
            let map: HashMap<String, String> = serde_json::from_str(json_str)
                .unwrap_or_else(|e| panic!("Invalid JSON in {}: {}", path, e));
            translations.insert(lang.to_string(), Arc::new(map));
        }

        I18nManager { translations }
    }

    pub fn for_lang(&self, lang: &str) -> I18n {
        let lang_key = if self.translations.contains_key(lang) {
            lang
        } else {
            DEFAULT_LANGUAGE
        };

        let translations = self.translations.get(lang_key).cloned()
            .unwrap_or_else(|| Arc::new(HashMap::new()));
        let fallback = if lang_key != DEFAULT_LANGUAGE {
            self.translations.get(DEFAULT_LANGUAGE).cloned()
                .unwrap_or_else(|| Arc::new(HashMap::new()))
        } else {
            translations.clone()
        };

        I18n {
            translations,
            fallback,
            lang: lang_key.to_string(),
        }
    }

    pub fn is_supported_language(lang: &str) -> bool {
        SUPPORTED_LANG_CODES.contains(&lang)
    }
}

pub struct I18n {
    translations: Arc<HashMap<String, String>>,
    fallback: Arc<HashMap<String, String>>,
    pub lang: String,
}

impl I18n {
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.translations.get(key)
            .or_else(|| self.fallback.get(key))
            .map(|s| s.as_str())
            .unwrap_or(key)
    }
}

pub fn detect_language(accept_language: &str) -> String {
    for part in accept_language.split(',') {
        let lang = part.split(';').next().unwrap_or("").trim();
        let lang_prefix = lang.split('-').next().unwrap_or("").to_lowercase();
        if SUPPORTED_LANG_CODES.contains(&lang_prefix.as_str()) {
            return lang_prefix;
        }
    }
    DEFAULT_LANGUAGE.to_string()
}
