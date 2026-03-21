use serde::Deserialize;

const STATIC_COUNTRIES_JSON: &str = include_str!("../data/countries.json");

#[derive(Deserialize)]
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
}

#[derive(Deserialize)]
pub struct CountrySettingsEntity {
    pub pricing: CountryPricingEntity,
}

#[derive(Deserialize)]
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
        SkinColorsEntity { white: 50, black: 20, metis: 30 }
    }
}

pub struct CountryLoader;

impl CountryLoader {
    pub fn load() -> Vec<CountryEntity> {
        serde_json::from_str(STATIC_COUNTRIES_JSON).unwrap()
    }

    /// Look up a country code by its ID. Used during player generation
    /// to assign native languages.
    pub fn code_for_id(country_id: u32) -> String {
        // Parse once per call — acceptable at generation time (not per-tick)
        let countries: Vec<CountryEntity> = serde_json::from_str(STATIC_COUNTRIES_JSON).unwrap();
        countries
            .iter()
            .find(|c| c.id == country_id)
            .map(|c| c.code.clone())
            .unwrap_or_default()
    }
}
