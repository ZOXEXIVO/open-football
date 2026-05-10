/// Lightweight country info for nationality lookups.
/// Covers ALL countries (not just simulation participants).
#[derive(Clone, Debug)]
pub struct CountryInfo {
    pub id: u32,
    pub code: String,
    pub slug: String,
    pub name: String,
    /// Continent the country sits on. Carried here so the region-prestige
    /// gate used by the loan market, scouting, and personal-terms
    /// negotiation can resolve a `ScoutingRegion` even for nationalities
    /// whose country has no active leagues in this save.
    pub continent_id: u32,
    /// Football reputation (0..10000). Mirrors the same field on `Country`
    /// so the country-reputation realism gate keeps working when the
    /// nationality's leagues aren't loaded — without this it falls back to
    /// `0` and an Argentinian free agent slips through to a Mali buyer.
    pub reputation: u16,
}
