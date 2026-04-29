
use serde::Deserialize;

use super::compiled::compiled;

#[derive(Deserialize, Debug, Clone)]
pub struct NationalCompetitionEntity {
    pub id: u32,
    pub name: String,
    pub short_name: String,
    pub scope: String,
    pub continent_id: Option<u32>,
    pub cycle_years: u32,
    pub cycle_offset: u32,
    pub qualifying: QualifyingEntity,
    pub tournament: TournamentEntity,
    pub schedule: ScheduleEntity,
}

#[derive(Deserialize, Debug, Clone)]
pub struct QualifyingEntity {
    pub zones: Vec<QualifyingZoneEntity>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct QualifyingZoneEntity {
    pub continent_id: u32,
    pub spots: u32,
    pub max_groups: u32,
    pub teams_per_group_target: u32,
    pub qualifiers_per_group: Vec<String>,
    pub best_runners_up: u32,
    pub best_third_placed: u32,
}

#[derive(Deserialize, Debug, Clone)]
pub struct TournamentEntity {
    pub total_teams: u32,
    pub group_count: u32,
    pub teams_per_group: u32,
    pub advance_per_group: u32,
    pub best_third_placed: u32,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ScheduleEntity {
    pub qualifying_dates: Vec<ScheduleDateEntity>,
    pub tournament_group_dates: Vec<ScheduleDateEntity>,
    pub tournament_knockout_dates: Vec<ScheduleDateEntity>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ScheduleDateEntity {
    pub month: u32,
    pub day: u32,
    pub year_offset: i32,
}

pub struct NationalCompetitionLoader;

impl NationalCompetitionLoader {
    pub fn load() -> Vec<NationalCompetitionEntity> {
        compiled().national_competitions.clone()
    }
}
