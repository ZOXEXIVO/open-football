use crate::common::default_handler::Assets;
use std::collections::HashMap;
use std::sync::Arc;

/// One language's key→text map within a single scope.
pub(crate) type Bundle = Arc<HashMap<String, String>>;

/// Every language bundle for one translation scope — the JSON files that sit
/// together in a directory under `assets/i18n`.
///
/// Scopes load and resolve independently, so a scope-specific view can only
/// ever see its own vocabulary. That is the point of the split: the page
/// chrome bundle stays free of the ~1700 press and happiness-event strings
/// that only the newspaper and events pages ever render.
pub(crate) struct LocaleCatalog {
    bundles: HashMap<String, Bundle>,
}

impl LocaleCatalog {
    /// Load a scope whose bundle must exist for every supported language.
    /// A missing or malformed file is a packaging error, so it panics at
    /// startup rather than leaving pages silently half-translated.
    pub(crate) fn required(dir: Option<&str>, languages: &[&str]) -> Self {
        Self::load(dir, languages, true)
    }

    /// Load a scope that is allowed to be missing for some languages; those
    /// languages then resolve entirely through the default-language fallback.
    pub(crate) fn optional(dir: &str, languages: &[&str]) -> Self {
        Self::load(Some(dir), languages, false)
    }

    fn load(dir: Option<&str>, languages: &[&str], required: bool) -> Self {
        let mut bundles = HashMap::with_capacity(languages.len());

        for &lang in languages {
            let path = match dir {
                Some(dir) => format!("i18n/{}/{}.json", dir, lang),
                None => format!("i18n/{}.json", lang),
            };

            let Some(data) = Assets::get(&path) else {
                if required {
                    panic!("Missing translation file: {}", path);
                }
                continue;
            };

            let json_str = std::str::from_utf8(&data.data)
                .unwrap_or_else(|_| panic!("Invalid UTF-8 in {}", path));
            // Tolerate a UTF-8 BOM at the start of the file — Windows
            // editors / PowerShell sometimes prepend one and serde_json
            // rejects it as "expected value at line 1 column 1".
            let json_str = json_str.strip_prefix('\u{feff}').unwrap_or(json_str);
            let map: HashMap<String, String> = serde_json::from_str(json_str)
                .unwrap_or_else(|e| panic!("Invalid JSON in {}: {}", path, e));

            bundles.insert(lang.to_string(), Arc::new(map));
        }

        LocaleCatalog { bundles }
    }

    /// Whether this scope ships a bundle for `lang`. Used to pick the key a
    /// request resolves under before any bundle is cloned.
    pub(crate) fn has(&self, lang: &str) -> bool {
        self.bundles.contains_key(lang)
    }

    /// Snapshot the bundles a request needs: `lang` first, the default
    /// language behind it.
    pub(crate) fn resolve(&self, lang: &str, default_lang: &str) -> ScopeLookup {
        let fallback = self.bundle(default_lang);
        if lang == default_lang {
            return ScopeLookup {
                primary: Arc::clone(&fallback),
                fallback,
            };
        }
        ScopeLookup {
            primary: self.bundle(lang),
            fallback,
        }
    }

    fn bundle(&self, lang: &str) -> Bundle {
        self.bundles
            .get(lang)
            .cloned()
            .unwrap_or_else(|| Arc::new(HashMap::new()))
    }
}

/// The bundles one request resolves against inside a single scope: the
/// requested language, then the default language behind it.
#[derive(Clone)]
pub(crate) struct ScopeLookup {
    primary: Bundle,
    fallback: Bundle,
}

impl ScopeLookup {
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        let empty: Bundle = Arc::new(HashMap::new());
        ScopeLookup {
            primary: Arc::clone(&empty),
            fallback: empty,
        }
    }

    /// Build a lookup from a literal map. Renderer unit tests use this to
    /// exercise copy branches without standing up the bundle loader.
    #[cfg(test)]
    pub(crate) fn from_map(map: HashMap<String, String>) -> Self {
        let primary: Bundle = Arc::new(map);
        ScopeLookup {
            fallback: Arc::clone(&primary),
            primary,
        }
    }

    pub(crate) fn get<'a>(&'a self, key: &str) -> Option<&'a str> {
        self.primary
            .get(key)
            .or_else(|| self.fallback.get(key))
            .map(|s| s.as_str())
    }

    /// Resolve against the default-language bundle only, skipping the
    /// requested language. Used where the English spelling is the value —
    /// country codes that address flag assets, for instance.
    pub(crate) fn default_get<'a>(&'a self, key: &str) -> Option<&'a str> {
        self.fallback.get(key).map(|s| s.as_str())
    }

    /// Resolve `key`, falling back to the key itself when the scope has no
    /// copy for it. Callers rely on that identity: comparing the result
    /// against the key is how they detect missing copy.
    pub(crate) fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.get(key).unwrap_or(key)
    }
}
