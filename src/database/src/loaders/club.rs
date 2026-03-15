use serde::Deserialize;

#[derive(Deserialize)]
pub struct ClubEntity {
    pub id: u32,
    pub name: String,
    /// Populated by the loader from the directory path, not present in JSON.
    #[serde(default)]
    pub country_id: u32,
    pub location: ClubLocationEntity,
    pub finance: ClubFinanceEntity,
    pub colors: ClubColorsEntity,
    pub teams: Vec<ClubTeamEntity>,
}

#[derive(Deserialize)]
pub struct ClubColorsEntity {
    pub background: String,
    pub foreground: String,
}

#[derive(Deserialize)]
pub struct ClubLocationEntity {
    pub city_id: u32,
}

#[derive(Deserialize)]
pub struct ClubFinanceEntity {
    pub balance: i32,
}

#[derive(Deserialize)]
pub struct ClubReputationEntity {
    pub home: u16,
    pub national: u16,
    pub world: u16,
}

#[derive(Deserialize)]
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
