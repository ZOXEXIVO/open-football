/// Player language proficiency system.
///
/// Players have a native language from their home country and can learn
/// new languages when playing abroad. Learning speed depends on:
/// - Adaptability (primary factor)
/// - Professionalism (study habits)
/// - Age (younger = faster)
/// - Star status (stars may resist learning — they don't "need" to)

/// Languages in the football world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    English,
    Spanish,
    Portuguese,
    French,
    German,
    Italian,
    Dutch,
    Russian,
    Turkish,
    Arabic,
    Japanese,
    Korean,
    Chinese,
    Swedish,
    Norwegian,
    Danish,
    Finnish,
    Polish,
    Czech,
    Romanian,
    Ukrainian,
    Serbian,
    Croatian,
    Greek,
    Hungarian,
    Bulgarian,
    Slovak,
    Slovenian,
    Albanian,
    Bosnian,
    Georgian,
    Armenian,
    Azerbaijani,
    Kazakh,
    Hebrew,
    Persian,
    Hindi,
    Bengali,
    Thai,
    Vietnamese,
    Indonesian,
    Malay,
    Swahili,
    Yoruba,
    Hausa,
    Amharic,
    Afrikaans,
    Icelandic,
}

impl Language {
    /// Map a country code to its official/primary language(s).
    pub fn from_country_code(code: &str) -> Vec<Language> {
        match code {
            // Western Europe
            "gb" | "ie" | "sc" => vec![Language::English],
            "es" => vec![Language::Spanish],
            "fr" => vec![Language::French],
            "de" | "at" | "ch" | "li" => vec![Language::German],
            "it" | "sm" => vec![Language::Italian],
            "pt" => vec![Language::Portuguese],
            "nl" => vec![Language::Dutch],
            "be" => vec![Language::Dutch, Language::French],
            "lu" => vec![Language::French, Language::German],
            "ad" | "mc" | "gi" => vec![Language::Spanish],
            "mt" => vec![Language::English],

            // Scandinavia
            "se" => vec![Language::Swedish],
            "no" => vec![Language::Norwegian],
            "dk" | "fo" => vec![Language::Danish],
            "fi" => vec![Language::Finnish],
            "is" => vec![Language::Icelandic],

            // Eastern Europe
            "pl" => vec![Language::Polish],
            "cz" => vec![Language::Czech],
            "sk" => vec![Language::Slovak],
            "hu" => vec![Language::Hungarian],
            "ro" | "md" => vec![Language::Romanian],
            "bg" => vec![Language::Bulgarian],
            "rs" => vec![Language::Serbian],
            "hr" => vec![Language::Croatian],
            "si" => vec![Language::Slovenian],
            "ba" => vec![Language::Bosnian],
            "al" | "mk" => vec![Language::Albanian],
            "gr" | "cy" => vec![Language::Greek],
            "ee" | "lv" | "lt" => vec![Language::Russian],
            "ua" => vec![Language::Ukrainian],
            "ru" | "by" => vec![Language::Russian],
            "ge" => vec![Language::Georgian],
            "am" => vec![Language::Armenian],
            "az" => vec![Language::Azerbaijani],
            "kz" | "kg" | "uz" | "tj" | "tm" => vec![Language::Russian],
            "me" => vec![Language::Serbian],

            // Turkey & Middle East
            "tr" => vec![Language::Turkish],
            "il" => vec![Language::Hebrew],
            "ir" => vec![Language::Persian],
            "sa" | "ae" | "qa" | "kw" | "bh" | "om" | "jo" | "lb" | "iq" | "ye" | "eg" | "dz"
            | "ma" | "tn" | "ly" | "sd" | "ps" | "mr" => vec![Language::Arabic],

            // South America
            "br" => vec![Language::Portuguese],
            "ar" | "co" | "cl" | "uy" | "py" | "pe" | "ec" | "bo" | "ve" | "mx" | "sv" | "gt"
            | "hn" | "ni" | "cr" | "pa" | "cu" | "do" | "pr" => vec![Language::Spanish],
            "gy" | "sr" | "gf" => vec![Language::English],

            // North America & Caribbean
            "us" | "ca" | "jm" | "tt" | "bb" | "bs" | "bz" | "ag" | "dm" | "gd" | "kn" | "lc"
            | "vc" | "ky" | "bm" | "vi" | "ai" | "ms" | "vg" | "tc" => vec![Language::English],
            "ht" | "gp" | "mq" | "mf" => vec![Language::French],
            "aw" => vec![Language::Dutch],

            // East Asia
            "jp" => vec![Language::Japanese],
            "kr" | "kp" => vec![Language::Korean],
            "cn" | "hk" | "mo" | "tw" => vec![Language::Chinese],

            // Southeast Asia
            "th" => vec![Language::Thai],
            "vn" => vec![Language::Vietnamese],
            "id" => vec![Language::Indonesian],
            "my" | "sg" | "bn" => vec![Language::Malay],
            "ph" | "mm" | "kh" | "la" => vec![Language::English],

            // South Asia
            "in" | "pk" | "bd" | "lk" | "np" | "mv" => vec![Language::Hindi],
            "af" => vec![Language::Persian],

            // Oceania
            "au" | "nz" | "fj" | "pg" | "sb" | "vu" | "ws" | "to" | "ck" | "as" | "gu" | "fm"
            | "ki" | "tv" | "mp" => vec![Language::English],
            "nc" | "wf" => vec![Language::French],

            // Africa — English-speaking
            "ng" | "gh" | "ke" | "tz" | "ug" | "za" | "zw" | "zm" | "bw" | "mw" | "sl" | "lr"
            | "gm" | "mu" | "sz" | "ls" | "na" | "rw" => vec![Language::English],

            // Africa — French-speaking
            "ci" | "cm" | "sn" | "ml" | "bf" | "ne" | "td" | "cf" | "cg" | "ga" | "bj" | "tg"
            | "gw" | "gq" | "dj" | "bi" | "mg" | "km" | "yt" | "re" | "st" => {
                vec![Language::French]
            }

            // Africa — Portuguese-speaking
            "ao" | "mz" | "cv" => vec![Language::Portuguese],

            // Africa — Arabic
            // (already handled above: eg, dz, ma, tn, ly, sd, mr)

            // Africa — East (Swahili region, also English)
            "et" | "so" => vec![Language::Swahili],

            // Africa — South Africa also has Afrikaans
            // (za already mapped to English above)
            _ => vec![Language::English], // fallback
        }
    }

    /// Display name for the language.
    pub fn name(&self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Spanish => "Spanish",
            Language::Portuguese => "Portuguese",
            Language::French => "French",
            Language::German => "German",
            Language::Italian => "Italian",
            Language::Dutch => "Dutch",
            Language::Russian => "Russian",
            Language::Turkish => "Turkish",
            Language::Arabic => "Arabic",
            Language::Japanese => "Japanese",
            Language::Korean => "Korean",
            Language::Chinese => "Chinese",
            Language::Swedish => "Swedish",
            Language::Norwegian => "Norwegian",
            Language::Danish => "Danish",
            Language::Finnish => "Finnish",
            Language::Polish => "Polish",
            Language::Czech => "Czech",
            Language::Romanian => "Romanian",
            Language::Ukrainian => "Ukrainian",
            Language::Serbian => "Serbian",
            Language::Croatian => "Croatian",
            Language::Greek => "Greek",
            Language::Hungarian => "Hungarian",
            Language::Bulgarian => "Bulgarian",
            Language::Slovak => "Slovak",
            Language::Slovenian => "Slovenian",
            Language::Albanian => "Albanian",
            Language::Bosnian => "Bosnian",
            Language::Georgian => "Georgian",
            Language::Armenian => "Armenian",
            Language::Azerbaijani => "Azerbaijani",
            Language::Kazakh => "Kazakh",
            Language::Hebrew => "Hebrew",
            Language::Persian => "Persian",
            Language::Hindi => "Hindi",
            Language::Bengali => "Bengali",
            Language::Thai => "Thai",
            Language::Vietnamese => "Vietnamese",
            Language::Indonesian => "Indonesian",
            Language::Malay => "Malay",
            Language::Swahili => "Swahili",
            Language::Yoruba => "Yoruba",
            Language::Hausa => "Hausa",
            Language::Amharic => "Amharic",
            Language::Afrikaans => "Afrikaans",
            Language::Icelandic => "Icelandic",
        }
    }

    /// i18n key for translation lookup.
    pub fn i18n_key(&self) -> &'static str {
        match self {
            Language::English => "lang_english",
            Language::Spanish => "lang_spanish",
            Language::Portuguese => "lang_portuguese",
            Language::French => "lang_french",
            Language::German => "lang_german",
            Language::Italian => "lang_italian",
            Language::Dutch => "lang_dutch",
            Language::Russian => "lang_russian",
            Language::Turkish => "lang_turkish",
            Language::Arabic => "lang_arabic",
            Language::Japanese => "lang_japanese",
            Language::Korean => "lang_korean",
            Language::Chinese => "lang_chinese",
            Language::Swedish => "lang_swedish",
            Language::Norwegian => "lang_norwegian",
            Language::Danish => "lang_danish",
            Language::Finnish => "lang_finnish",
            Language::Polish => "lang_polish",
            Language::Czech => "lang_czech",
            Language::Romanian => "lang_romanian",
            Language::Ukrainian => "lang_ukrainian",
            Language::Serbian => "lang_serbian",
            Language::Croatian => "lang_croatian",
            Language::Greek => "lang_greek",
            Language::Hungarian => "lang_hungarian",
            Language::Bulgarian => "lang_bulgarian",
            Language::Slovak => "lang_slovak",
            Language::Slovenian => "lang_slovenian",
            Language::Albanian => "lang_albanian",
            Language::Bosnian => "lang_bosnian",
            Language::Georgian => "lang_georgian",
            Language::Armenian => "lang_armenian",
            Language::Azerbaijani => "lang_azerbaijani",
            Language::Kazakh => "lang_kazakh",
            Language::Hebrew => "lang_hebrew",
            Language::Persian => "lang_persian",
            Language::Hindi => "lang_hindi",
            Language::Bengali => "lang_bengali",
            Language::Thai => "lang_thai",
            Language::Vietnamese => "lang_vietnamese",
            Language::Indonesian => "lang_indonesian",
            Language::Malay => "lang_malay",
            Language::Swahili => "lang_swahili",
            Language::Yoruba => "lang_yoruba",
            Language::Hausa => "lang_hausa",
            Language::Amharic => "lang_amharic",
            Language::Afrikaans => "lang_afrikaans",
            Language::Icelandic => "lang_icelandic",
        }
    }
}

/// A player's proficiency in a specific language.
#[derive(Debug, Clone)]
pub struct PlayerLanguage {
    pub language: Language,
    /// 0 = no knowledge, 100 = fully fluent (native).
    pub proficiency: u8,
    /// Whether this is the player's native language.
    pub is_native: bool,
}

impl PlayerLanguage {
    pub fn native(language: Language) -> Self {
        PlayerLanguage {
            language,
            proficiency: 100,
            is_native: true,
        }
    }

    pub fn learning(language: Language, proficiency: u8) -> Self {
        PlayerLanguage {
            language,
            proficiency,
            is_native: false,
        }
    }

    /// Descriptive proficiency level label key.
    pub fn level_key(&self) -> &'static str {
        if self.is_native {
            "lang_level_native"
        } else if self.proficiency >= 90 {
            "lang_level_fluent"
        } else if self.proficiency >= 70 {
            "lang_level_good"
        } else if self.proficiency >= 40 {
            "lang_level_basic"
        } else if self.proficiency >= 10 {
            "lang_level_beginner"
        } else {
            "lang_level_none"
        }
    }
}

impl Language {
    /// Bit position for the compact language masks below (48 variants,
    /// so every language fits a `u64`).
    fn mask_bit(self) -> u64 {
        1u64 << (self as u64)
    }

    /// The bridge languages of world football — a dressing room can
    /// usually run day-to-day communication in English or Spanish even
    /// where neither is the local language, so speaking one of them
    /// partially substitutes for the club country's own language.
    const BRIDGE_LANGUAGES: [Language; 2] = [Language::English, Language::Spanish];

    /// Combined mask of the official/primary language(s) of a country —
    /// [`Self::from_country_code`] folded into one `u64`.
    pub fn country_language_mask(code: &str) -> u64 {
        Self::from_country_code(code)
            .into_iter()
            .fold(0u64, |mask, lang| mask | lang.mask_bit())
    }

    /// Combined mask of [`Self::BRIDGE_LANGUAGES`].
    pub fn bridge_language_mask() -> u64 {
        Self::BRIDGE_LANGUAGES
            .into_iter()
            .fold(0u64, |mask, lang| mask | lang.mask_bit())
    }
}

/// Compact two-tier snapshot of what a player speaks, built once from his
/// [`PlayerLanguage`] list and cheap to carry on transfer-market summaries
/// (two `u64`s, `Copy`, no allocation). The tier cutoffs mirror the
/// adaptation model: `fluent` = native or proficiency ≥ 70, `conversational`
/// = proficiency ≥ 40 (fluent languages are in both masks).
///
/// Recruitment reads it through [`Self::affinity_for`]: clubs lean toward
/// players who can walk into the dressing room and be understood — the
/// club country's own language first, the football bridge languages
/// (English, Spanish) as a partial substitute.
#[derive(Debug, Clone, Copy, Default)]
pub struct LanguageProfile {
    pub fluent_mask: u64,
    pub conversational_mask: u64,
}

impl LanguageProfile {
    pub fn from_languages(languages: &[PlayerLanguage]) -> Self {
        let mut profile = LanguageProfile::default();
        for entry in languages {
            let bit = entry.language.mask_bit();
            if entry.is_native || entry.proficiency >= 70 {
                profile.fluent_mask |= bit;
                profile.conversational_mask |= bit;
            } else if entry.proficiency >= 40 {
                profile.conversational_mask |= bit;
            }
        }
        profile
    }

    /// How well this player fits a club whose country speaks
    /// `club_language_mask` (see [`Language::country_language_mask`]),
    /// 0.0..=1.0. Graded, never a hard gate:
    ///
    /// - fluent in a club-country language → 1.0
    /// - conversational in one → 0.65
    /// - fluent in a bridge language (English/Spanish) → 0.45
    /// - conversational in a bridge language → 0.25
    /// - no common language → 0.0
    ///
    /// An empty club mask or an empty profile reads as neutral (1.0) —
    /// missing data must never penalise a candidate.
    pub fn affinity_for(&self, club_language_mask: u64) -> f32 {
        if club_language_mask == 0 {
            return 1.0;
        }
        if self.fluent_mask == 0 && self.conversational_mask == 0 {
            return 1.0;
        }
        if self.fluent_mask & club_language_mask != 0 {
            return 1.0;
        }
        if self.conversational_mask & club_language_mask != 0 {
            return 0.65;
        }
        let bridge = Language::bridge_language_mask();
        if self.fluent_mask & bridge != 0 {
            return 0.45;
        }
        if self.conversational_mask & bridge != 0 {
            return 0.25;
        }
        0.0
    }

    /// Affinity approximated from nationality alone — for call sites that
    /// only carry country codes (free-agent snapshots). The player is
    /// assumed fluent in his home country's language(s); languages learned
    /// abroad are invisible here, so this under- rather than over-states
    /// the fit.
    pub fn nationality_affinity(player_country_code: &str, club_country_code: &str) -> f32 {
        let native = Language::country_language_mask(player_country_code);
        LanguageProfile {
            fluent_mask: native,
            conversational_mask: native,
        }
        .affinity_for(Language::country_language_mask(club_country_code))
    }
}

/// Weekly language learning processing.
///
/// When a player plays in a foreign country, they gradually learn the local
/// language. The learning rate depends on:
///
/// - **Adaptability** (0-20): Primary factor. High adaptability = fast learner.
/// - **Professionalism** (0-20): Professional players study harder.
/// - **Age**: Younger players learn faster (< 25 bonus, > 30 penalty).
/// - **Star status**: Stars (current_ability > 160) may resist learning —
///   they have translators, staff speak their language, less motivation.
/// - **Already fluent**: Diminishing returns near 100%.
///
/// A typical player with adaptability 12, professionalism 12, age 24 reaches
/// basic conversational (~40%) in about 1.5 years, functional fluency (~70%)
/// in about 3 years, and near-fluency (~90%) in 4-5 years.
/// Exceptionally adaptable young players may learn faster; older stars much slower.
pub fn weekly_language_progress(
    adaptability: f32,
    professionalism: f32,
    age: u8,
    current_ability: u8,
    current_proficiency: u8,
) -> u8 {
    if current_proficiency >= 100 {
        return 0;
    }

    // Base weekly rate: ~0.35% per week → ~18% per year at baseline
    let base_rate: f32 = 0.35;

    // Adaptability factor (0.3 at 0, 1.0 at 10, 1.8 at 20)
    let adapt_factor = 0.3 + (adaptability / 20.0) * 1.5;

    // Professionalism factor (0.6 at 0, 1.0 at 10, 1.3 at 20)
    let prof_factor = 0.6 + (professionalism / 20.0) * 0.7;

    // Age factor: young players learn faster
    let age_factor = if age <= 22 {
        1.3
    } else if age <= 25 {
        1.15
    } else if age <= 28 {
        1.0
    } else if age <= 32 {
        0.85
    } else {
        0.7
    };

    // Star penalty: top players resist learning (they have translators, etc.)
    // CA > 160 = superstar, 140-160 = star, < 140 = normal
    let star_factor = if current_ability > 160 {
        0.4 // Superstars barely bother
    } else if current_ability > 145 {
        0.65 // Stars learn slowly
    } else if current_ability > 130 {
        0.85 // Good players slightly slower
    } else {
        1.0 // Normal players fully motivated
    };

    // Diminishing returns near fluency (harder to go from 80→100 than 0→20)
    let progress_factor = if current_proficiency >= 85 {
        0.4
    } else if current_proficiency >= 70 {
        0.6
    } else if current_proficiency >= 50 {
        0.8
    } else {
        1.0
    };

    let weekly_gain =
        base_rate * adapt_factor * prof_factor * age_factor * star_factor * progress_factor;

    // Convert to integer progress, minimum 0
    let gain = weekly_gain.round().max(0.0) as u8;

    // Clamp so we don't exceed 100
    gain.min(100 - current_proficiency)
}

#[cfg(test)]
mod affinity_tests {
    use super::*;

    #[test]
    fn fluent_local_language_is_full_affinity() {
        let profile = LanguageProfile::from_languages(&[PlayerLanguage::native(Language::Spanish)]);
        assert_eq!(
            profile.affinity_for(Language::country_language_mask("es")),
            1.0
        );
    }

    #[test]
    fn conversational_local_language_is_partial() {
        let profile =
            LanguageProfile::from_languages(&[PlayerLanguage::learning(Language::German, 50)]);
        assert_eq!(
            profile.affinity_for(Language::country_language_mask("de")),
            0.65
        );
    }

    #[test]
    fn bridge_language_substitutes_partially() {
        // Fluent English going to Germany: no local language, but the
        // dressing room can run in English.
        let profile = LanguageProfile::from_languages(&[PlayerLanguage::native(Language::English)]);
        assert_eq!(
            profile.affinity_for(Language::country_language_mask("de")),
            0.45
        );
    }

    #[test]
    fn no_common_language_is_zero() {
        let profile =
            LanguageProfile::from_languages(&[PlayerLanguage::native(Language::Japanese)]);
        assert_eq!(
            profile.affinity_for(Language::country_language_mask("es")),
            0.0
        );
    }

    #[test]
    fn missing_data_reads_neutral() {
        // Empty profile (no language data) and empty club mask both must
        // never penalise a candidate.
        let empty = LanguageProfile::default();
        assert_eq!(
            empty.affinity_for(Language::country_language_mask("es")),
            1.0
        );
        let profile =
            LanguageProfile::from_languages(&[PlayerLanguage::native(Language::Japanese)]);
        assert_eq!(profile.affinity_for(0), 1.0);
    }

    #[test]
    fn nationality_affinity_follows_country_languages() {
        // Portuguese-speaking nation to Portugal: full fit.
        assert_eq!(LanguageProfile::nationality_affinity("br", "pt"), 1.0);
        // English nationality to Germany: bridge-language fit.
        assert_eq!(LanguageProfile::nationality_affinity("gb", "de"), 0.45);
    }

    #[test]
    fn multilingual_country_matches_any_official_language() {
        // Belgium speaks Dutch and French — a French speaker fits.
        let profile = LanguageProfile::from_languages(&[PlayerLanguage::native(Language::French)]);
        assert_eq!(
            profile.affinity_for(Language::country_language_mask("be")),
            1.0
        );
    }
}
