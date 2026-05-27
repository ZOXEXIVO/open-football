use serde::Deserialize;

/// A configured domestic club cup (FA Cup, Copa del Rey, Coppa Italia, …).
///
/// Sourced from the compiler's top-level `domestic_cups` array and matched
/// to a country by `country_slug`. Countries without an entry fall back to
/// a generated "{Country} Cup" in the runtime generator, so this only
/// carries the *named* cups for the major footballing nations.
#[derive(Deserialize, Debug, Clone)]
pub struct DomesticCupEntity {
    /// Country `slug` (as in countries.json, e.g. "england", "czech republic").
    pub country_slug: String,
    /// URL-safe competition slug (e.g. "fa-cup", "copa-del-rey").
    pub slug: String,
    /// Display name (e.g. "FA Cup", "Copa del Rey").
    pub name: String,
    /// Optional competition reputation; when absent the generator derives
    /// one from the country's primary league.
    #[serde(default)]
    pub reputation: Option<u16>,
}
