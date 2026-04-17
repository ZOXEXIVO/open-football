use serde::Deserialize;

#[derive(Deserialize, Clone)]
pub struct NamesByCountryEntity {
    /// Resolved from `country_code` by the loader; zero-default in JSON.
    #[serde(default)]
    pub country_id: u32,
    /// Baked in by the compiler from the enclosing directory.
    #[serde(default)]
    pub country_code: String,
    pub first_names: Vec<String>,
    pub last_names: Vec<String>,
    #[serde(default)]
    pub nicknames: Vec<String>,
}
