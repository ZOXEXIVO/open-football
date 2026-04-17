use serde::Deserialize;

#[derive(Deserialize, Clone)]
pub struct ClubEntity {
    pub id: u32,
    pub name: String,
    /// Resolved from `country_code` by the loader; zero-default in JSON.
    #[serde(default)]
    pub country_id: u32,
    /// Baked in by the compiler from the enclosing directory.
    #[serde(default)]
    pub country_code: String,
    pub location: ClubLocationEntity,
    pub finance: ClubFinanceEntity,
    pub colors: ClubColorsEntity,
    pub teams: Vec<ClubTeamEntity>,
    #[serde(default)]
    pub rivals: Vec<u32>,
    #[serde(default)]
    pub philosophy: Option<String>,
    #[serde(default)]
    pub facilities: Option<ClubFacilitiesEntity>,
    #[serde(default)]
    pub average_attendance: Option<u32>,
}

#[derive(Deserialize, Clone)]
pub struct ClubFacilitiesEntity {
    pub training: String,
    pub youth: String,
    pub academy: String,
    pub recruitment: String,
}

#[derive(Deserialize, Clone)]
pub struct ClubColorsEntity {
    pub background: String,
    pub foreground: String,
}

#[derive(Deserialize, Clone)]
pub struct ClubLocationEntity {
    pub city_id: u32,
}

#[derive(Deserialize, Clone)]
pub struct ClubFinanceEntity {
    pub balance: i32,
}

#[derive(Deserialize, Clone)]
pub struct ClubReputationEntity {
    pub home: u16,
    pub national: u16,
    pub world: u16,
}

#[derive(Deserialize, Clone)]
pub struct ClubTeamEntity {
    pub id: u32,
    pub name: String,
    pub slug: String,
    pub team_type: String,
    /// Populated by the loader from the directory context, not present in JSON.
    #[serde(default)]
    pub league_id: Option<u32>,
    pub finance: Option<ClubFinanceEntity>,
    pub reputation: ClubReputationEntity,
}
