use serde::Deserialize;

const STATIC_NAME_BY_COUNTRY_JSON: &str = include_str!("../data/names/names_by_country.json");

#[derive(Deserialize)]
pub struct NamesByCountryEntity {
    pub country_id: u32,
    pub first_names: Vec<String>,
    pub last_names: Vec<String>,
    #[serde(default)]
    pub nicknames: Vec<String>,
}

pub struct NamesByCountryLoader;

impl NamesByCountryLoader {
    pub fn load() -> Vec<NamesByCountryEntity> {
        serde_json::from_str(STATIC_NAME_BY_COUNTRY_JSON).unwrap()
    }
}
