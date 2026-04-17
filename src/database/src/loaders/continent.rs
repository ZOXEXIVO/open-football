use serde::Deserialize;

use super::compiled::compiled;

#[derive(Deserialize, Clone)]
pub struct ContinentEntity {
    pub id: u32,
    pub name: String,
}

pub struct ContinentLoader;

impl ContinentLoader {
    pub fn load() -> Vec<ContinentEntity> {
        compiled().continents.clone()
    }
}
