use core::SimulatorData;
use database::CountryLoader;
use shared::{Region, SkinDist};
use std::sync::OnceLock;

/// The per-country ancestry percentages, read out of the database once.
///
/// The DATA half of what a player looks like — the deciding is
/// `shared::Appearance`'s, and deliberately lives in a crate with no
/// database behind it so the WebAssembly viewer can run the same code. This
/// side is the part that cannot: it needs the country table, so both the
/// portrait route and the match page come through here for a [`SkinDist`]
/// before either of them asks what it means.
pub struct CountrySkin;

static SKIN_MAP: OnceLock<Vec<(String, SkinDist)>> = OnceLock::new();

impl CountrySkin {
    /// By two-letter country code. An unknown or empty code falls back to the
    /// mixed-society default rather than to a nationality of its own.
    pub fn of(code: &str) -> SkinDist {
        if code.is_empty() {
            return SkinDist::default();
        }
        let map = SKIN_MAP.get_or_init(Self::load);
        map.iter()
            .find(|(c, _)| c == code)
            .map(|(_, d)| *d)
            .unwrap_or_default()
    }

    /// By nationality id, which is what a player record actually carries.
    ///
    /// Countries with no active leagues are absent from `continents` but
    /// present in `country_info`, and plenty of players hold one of those
    /// passports — so both are consulted, same as the portrait route.
    pub fn for_country(data: &SimulatorData, country_id: u32) -> SkinDist {
        let code = data
            .country(country_id)
            .map(|c| c.code.clone())
            .or_else(|| data.country_info.get(&country_id).map(|i| i.code.clone()))
            .unwrap_or_default();
        Self::of(&code)
    }

    fn load() -> Vec<(String, SkinDist)> {
        CountryLoader::load()
            .into_iter()
            .map(|c| {
                let d = SkinDist {
                    white: c.skin_colors.white,
                    black: c.skin_colors.black,
                    metis: c.skin_colors.metis,
                    region: Region::for_code(&c.code),
                };
                (c.code, d)
            })
            .collect()
    }
}
