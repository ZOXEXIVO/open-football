use chrono::NaiveDate;

/// Scope of a national team competition
#[derive(Debug, Clone, PartialEq)]
pub enum CompetitionScope {
    Global,
    Continental,
}

/// Runtime configuration for a national team competition, converted from database entities
#[derive(Debug, Clone)]
pub struct NationalCompetitionConfig {
    pub id: u32,
    pub name: String,
    pub short_name: String,
    pub scope: CompetitionScope,
    pub continent_id: Option<u32>,
    pub cycle_years: u32,
    pub cycle_offset: u32,
    pub qualifying: QualifyingConfig,
    pub tournament: TournamentConfig,
    pub schedule: ScheduleConfig,
}

impl NationalCompetitionConfig {
    /// Check if a new qualifying cycle should start in the given year
    pub fn should_start_cycle(&self, year: i32) -> bool {
        // cycle_offset determines when qualifying starts relative to modular arithmetic
        // For WC: cycle_offset=2, cycle_years=4 -> qualifying in years where (year+2)%4==2
        //   i.e., 2024 qualifying for 2026 WC
        // For Euro: cycle_offset=0, cycle_years=4 -> qualifying in years where (year+2)%4==0
        //   i.e., 2026 qualifying for 2028 Euro
        let cycle = self.cycle_years as i32;
        let offset = self.cycle_offset as i32;
        (year + 2) % cycle == offset
    }

    /// Get the tournament year for a qualifying start year
    pub fn tournament_year_for(&self, qualifying_start_year: i32) -> u16 {
        (qualifying_start_year + 2) as u16
    }

    /// Get the qualifying zone config for a specific continent
    pub fn qualifying_zone_for(&self, continent_id: u32) -> Option<&QualifyingZoneConfig> {
        self.qualifying
            .zones
            .iter()
            .find(|z| z.continent_id == continent_id)
    }
}

/// Configuration for qualifying rounds
#[derive(Debug, Clone)]
pub struct QualifyingConfig {
    pub zones: Vec<QualifyingZoneConfig>,
}

/// Which positions in a group qualify
#[derive(Debug, Clone, PartialEq)]
pub enum QualifyingPosition {
    Winner,
    RunnerUp,
}

/// Configuration for a qualifying zone (per continent)
#[derive(Debug, Clone)]
pub struct QualifyingZoneConfig {
    pub continent_id: u32,
    pub spots: u32,
    pub max_groups: u32,
    pub teams_per_group_target: u32,
    pub qualifiers_per_group: Vec<QualifyingPosition>,
    pub best_runners_up: u32,
    pub best_third_placed: u32,
}

/// Configuration for the tournament phase
#[derive(Debug, Clone)]
pub struct TournamentConfig {
    pub total_teams: u32,
    pub group_count: u32,
    pub teams_per_group: u32,
    pub advance_per_group: u32,
    pub best_third_placed: u32,
}

/// Schedule configuration with date templates
#[derive(Debug, Clone)]
pub struct ScheduleConfig {
    pub qualifying_dates: Vec<ScheduleDate>,
    pub tournament_group_dates: Vec<ScheduleDate>,
    pub tournament_knockout_dates: Vec<ScheduleDate>,
}

/// A date template with month, day, and year offset from qualifying start year
#[derive(Debug, Clone)]
pub struct ScheduleDate {
    pub month: u32,
    pub day: u32,
    pub year_offset: i32,
}

impl ScheduleConfig {
    /// Generate actual qualifying dates from a start year
    pub fn generate_qualifying_dates(&self, start_year: i32) -> Vec<(u8, NaiveDate)> {
        self.qualifying_dates
            .iter()
            .enumerate()
            .filter_map(|(idx, sd)| {
                let year = start_year + sd.year_offset;
                NaiveDate::from_ymd_opt(year, sd.month, sd.day)
                    .map(|date| ((idx + 1) as u8, date))
            })
            .collect()
    }

    /// Generate tournament group stage dates from a start year
    pub fn generate_tournament_group_dates(&self, start_year: i32) -> Vec<NaiveDate> {
        self.tournament_group_dates
            .iter()
            .filter_map(|sd| {
                let year = start_year + sd.year_offset;
                NaiveDate::from_ymd_opt(year, sd.month, sd.day)
            })
            .collect()
    }

    /// Generate tournament knockout dates from a start year
    pub fn generate_tournament_knockout_dates(&self, start_year: i32) -> Vec<NaiveDate> {
        self.tournament_knockout_dates
            .iter()
            .filter_map(|sd| {
                let year = start_year + sd.year_offset;
                NaiveDate::from_ymd_opt(year, sd.month, sd.day)
            })
            .collect()
    }
}
