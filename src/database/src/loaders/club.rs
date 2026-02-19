use serde::Deserialize;

const STATIC_CLUB_JSON: &'static str = include_str!("../data/clubs.json");

#[derive(Deserialize)]
pub struct ClubEntity {
    pub id: u32,
    pub name: String,
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
    pub league_id: Option<u32>,
    pub finance: Option<ClubFinanceEntity>,
    pub reputation: ClubReputationEntity,
}

pub struct ClubLoader;

impl ClubLoader {
    pub fn load() -> Vec<ClubEntity> {
        serde_json::from_str(STATIC_CLUB_JSON).unwrap()
    }
}
