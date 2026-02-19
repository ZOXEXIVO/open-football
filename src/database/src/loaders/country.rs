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
}

#[derive(Deserialize)]
pub struct CountrySettingsEntity {
    pub pricing: CountryPricingEntity,
}

#[derive(Deserialize)]
pub struct CountryPricingEntity {
    pub price_level: f32,
}

pub struct CountryLoader;

impl CountryLoader {
    pub fn load() -> Vec<CountryEntity> {
        serde_json::from_str(STATIC_COUNTRIES_JSON).unwrap()
    }
}
