use serde::Deserialize;

#[derive(Deserialize)]
pub struct ForeignPlayerEntry {
    pub country_id: u32,
    pub weight: u16,
}

#[derive(Deserialize)]
pub struct LeagueEntity {
    pub id: u32,
    /// Whether this league is active in the simulation. Set to false to skip.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub slug: String,
    pub name: String,
    /// Populated by the loader from the directory path, not present in JSON.
    #[serde(default)]
    pub country_id: u32,
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

#[derive(Debug, Deserialize)]
pub struct LeagueGroupEntity {
    /// Display name of the group (e.g. "A", "B", "C", "North", "South")
    pub name: String,
    /// Parent competition name that groups belong to (e.g. "Serie C", "Regionalliga")
    pub competition: String,
    /// Number of groups in the parent competition (e.g. 3 for Serie C)
    pub total_groups: u8,
}

fn default_enabled() -> bool {
    false
}

#[derive(Deserialize)]
pub struct LeagueSettingsEntity {
    pub season_starting_half: DayMonthPeriodEntity,
    pub season_ending_half: DayMonthPeriodEntity,
}

#[derive(Debug, Deserialize)]
pub struct DayMonthPeriodEntity {
    pub from_day: u8,
    pub from_month: u8,

    pub to_day: u8,
    pub to_month: u8,
}
