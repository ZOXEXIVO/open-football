use serde::Deserialize;

#[derive(Deserialize, Clone)]
pub struct ForeignPlayerEntry {
    pub country_id: u32,
    pub weight: u16,
}

#[derive(Deserialize, Clone)]
pub struct LeagueEntity {
    pub id: u32,
    /// Whether this league is active in the simulation. Set to false to skip.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub slug: String,
    pub name: String,
    /// Resolved from `country_code` by the loader; zero-default in JSON.
    #[serde(default)]
    pub country_id: u32,
    /// Baked in by the compiler from the enclosing directory; used to derive
    /// `country_id` at load time.
    #[serde(default)]
    pub country_code: String,
    pub settings: LeagueSettingsEntity,
    pub reputation: u16,
    #[serde(default)]
    pub tier: u8,
    #[serde(default)]
    pub promotion_spots: u8,
    #[serde(default)]
    pub relegation_spots: u8,
    #[serde(default)]
    pub foreign_players: Vec<ForeignPlayerEntry>,
    #[serde(default)]
    pub sub_leagues_competitions: Vec<String>,
    /// Optional group configuration for multi-group leagues (e.g. Serie C Group A/B/C).
    /// When set, multiple leagues share the same tier but are treated as separate groups
    /// within the same competition.
    #[serde(default)]
    pub league_group: Option<LeagueGroupEntity>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LeagueGroupEntity {
    /// Display name of the group (e.g. "A", "B", "C", "North", "South")
    pub name: String,
    /// Parent competition name that groups belong to (e.g. "Serie C", "Regionalliga")
    pub competition: String,
    /// Number of groups in the parent competition (e.g. 3 for Serie C)
    pub total_groups: u8,
    /// Optional end-of-season playoff. When present, the competition crowns
    /// a single champion via a knockout bracket seeded from every group's
    /// final standings (MLS Cup, Serie C promotion playoff, …).
    #[serde(default)]
    pub playoff: Option<PlayoffConfigEntity>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PlayoffConfigEntity {
    /// Top N of each group's table that enter the knockout bracket
    /// (e.g. 9 for MLS: seven direct + the two wild-card sides).
    pub qualifiers_per_group: u8,
    /// Bracket shape: "mls" (per-conference wild card + best-of-3 round
    /// one + cross-conference final), "cross_group" (Argentine fixed
    /// cross-zone bracket), or unset for generic single elimination.
    #[serde(default)]
    pub format: Option<String>,
    /// Display name of the playoff competition (e.g. "MLS Cup Playoffs").
    #[serde(default)]
    pub name: Option<String>,
    /// Split-season tournament names, first then second (e.g.
    /// ["Torneo Apertura", "Torneo Clausura"]).
    #[serde(default)]
    pub stage_names: Vec<String>,
}

fn default_enabled() -> bool {
    false
}

#[derive(Deserialize, Clone)]
pub struct LeagueSettingsEntity {
    pub season_starting_half: DayMonthPeriodEntity,
    pub season_ending_half: DayMonthPeriodEntity,
    /// Argentine-style split season: the two halves are separate
    /// tournaments (Apertura/Clausura), each a single round-robin with
    /// its own table, playoff and champion.
    #[serde(default)]
    pub split_season: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DayMonthPeriodEntity {
    pub from_day: u8,
    pub from_month: u8,

    pub to_day: u8,
    pub to_month: u8,
}
