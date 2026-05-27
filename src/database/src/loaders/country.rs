use serde::Deserialize;

use super::compiled::compiled;
use super::domestic_cup::DomesticCupEntity;

#[derive(Deserialize, Clone)]
pub struct CountryEntity {
    pub id: u32,
    pub code: String,
    pub slug: String,
    pub name: String,
    pub background_color: String,
    pub foreground_color: String,
    pub continent_id: u32,
    pub reputation: u16,
    pub settings: CountrySettingsEntity,
    #[serde(default)]
    pub skin_colors: SkinColorsEntity,
    /// The country's named domestic cup, resolved from the compiled
    /// `domestic_cups` table by `CountryLoader::load`. Not present in
    /// countries.json (hence `skip_deserializing`); `None` means the
    /// runtime generator falls back to a "{Country} Cup".
    #[serde(skip_deserializing, default)]
    pub domestic_cup: Option<DomesticCupEntity>,
}

#[derive(Deserialize, Clone)]
pub struct CountrySettingsEntity {
    pub pricing: CountryPricingEntity,
}

#[derive(Deserialize, Clone)]
pub struct CountryPricingEntity {
    pub price_level: f32,
}

#[derive(Deserialize, Clone)]
pub struct SkinColorsEntity {
    pub white: u8,
    pub black: u8,
    pub metis: u8,
}

impl Default for SkinColorsEntity {
    fn default() -> Self {
        SkinColorsEntity {
            white: 50,
            black: 20,
            metis: 30,
        }
    }
}

pub struct CountryLoader;

impl CountryLoader {
    pub fn load() -> Vec<CountryEntity> {
        let db = compiled();
        db.countries
            .iter()
            .cloned()
            .map(|mut country| {
                // Attach the named cup (if configured) by country slug.
                // Matching is case-insensitive on the trimmed slug so a
                // stray space or capitalisation in the data doesn't drop
                // the cup — the fallback generator covers any misses.
                let key = country.slug.trim().to_ascii_lowercase();
                country.domestic_cup = db
                    .domestic_cups
                    .iter()
                    .find(|c| c.country_slug.trim().to_ascii_lowercase() == key)
                    .cloned();
                country
            })
            .collect()
    }

    /// Look up a country code by its ID. Always returns lowercase ASCII —
    /// the loaded data is lowercase ("br", "nl", "jp"), and consumers
    /// (`Language::from_country_code`, `country_skill_bias`,
    /// `PhysicalProfile::country_height_offset`) all match on lowercase.
    /// Forcing the cast here makes the contract explicit and immune to any
    /// future data file accidentally storing mixed case.
    pub fn code_for_id(country_id: u32) -> String {
        compiled()
            .countries
            .iter()
            .find(|c| c.id == country_id)
            .map(|c| c.code.to_ascii_lowercase())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::CountryLoader;

    #[test]
    fn named_domestic_cups_resolve_onto_countries() {
        let countries = CountryLoader::load();
        let cup_name = |slug: &str| {
            countries
                .iter()
                .find(|c| c.slug == slug)
                .unwrap_or_else(|| panic!("country {slug} missing"))
                .domestic_cup
                .as_ref()
                .map(|c| c.name.as_str())
        };

        assert_eq!(cup_name("england"), Some("FA Cup"));
        assert_eq!(cup_name("spain"), Some("Copa del Rey"));
        assert_eq!(cup_name("italy"), Some("Coppa Italia"));
        assert_eq!(cup_name("germany"), Some("DFB-Pokal"));

        // A country with no configured cup resolves to `None`; the runtime
        // generator gives it a "{Country} Cup" fallback.
        assert_eq!(cup_name("afghanistan"), None);
    }
}
