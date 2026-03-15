use serde::Deserialize;

#[derive(Deserialize)]
pub struct NamesByCountryEntity {
    /// Populated by the loader from the directory path, not present in JSON.
    #[serde(default)]
    pub country_id: u32,
    pub first_names: Vec<String>,
    pub last_names: Vec<String>,
    #[serde(default)]
    pub nicknames: Vec<String>,
}
