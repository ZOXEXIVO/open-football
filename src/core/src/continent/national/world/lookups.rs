//! Cross-continent country lookup helpers.
//!
//! Squad building, stats updates and Elo all need to find a country
//! regardless of which continent it sits on. The previous
//! continent-local helpers silently returned defaults for foreign
//! lookups (reputation 0, elo 1500, empty name) — these world-aware
//! variants walk every continent so a Brazilian squad picked from
//! Spanish clubs can still resolve Brazil's reputation correctly.

use crate::Country;
use crate::continent::Continent;

pub fn world_country_reputation(continents: &[Continent], country_id: u32) -> u16 {
    continents
        .iter()
        .flat_map(|c| c.countries.iter())
        .find(|c| c.id == country_id)
        .map(|c| c.reputation)
        .unwrap_or(0)
}

pub fn world_country_elo(continents: &[Continent], country_id: u32) -> u16 {
    continents
        .iter()
        .flat_map(|c| c.countries.iter())
        .find(|c| c.id == country_id)
        .map(|c| c.national_team.elo_rating)
        .unwrap_or(1500)
}

pub fn world_country_name(continents: &[Continent], country_id: u32) -> String {
    continents
        .iter()
        .flat_map(|c| c.countries.iter())
        .find(|c| c.id == country_id)
        .map(|c| c.name.clone())
        .unwrap_or_default()
}

pub(crate) fn country_lookup(continents: &[Continent], country_id: u32) -> Option<&Country> {
    continents
        .iter()
        .flat_map(|c| c.countries.iter())
        .find(|c| c.id == country_id)
}

pub(crate) fn country_lookup_mut(
    continents: &mut [Continent],
    country_id: u32,
) -> Option<&mut Country> {
    continents
        .iter_mut()
        .flat_map(|c| c.countries.iter_mut())
        .find(|c| c.id == country_id)
}
