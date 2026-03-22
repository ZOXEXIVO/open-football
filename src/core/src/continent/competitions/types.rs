use chrono::NaiveDate;

/// Reserved league_id values for continental competitions.
/// Used in match result processing to identify competition type.
pub const CHAMPIONS_LEAGUE_ID: u32 = 900_000_001;
pub const EUROPA_LEAGUE_ID: u32 = 900_000_002;
pub const CONFERENCE_LEAGUE_ID: u32 = 900_000_003;

#[derive(Debug, Clone)]
pub enum CompetitionStage {
    NotStarted,
    Qualifying,
    GroupStage,
    RoundOf32,
    RoundOf16,
    QuarterFinals,
    SemiFinals,
    Final,
}

#[derive(Debug, Clone)]
pub struct ContinentalMatch {
    pub home_team: u32,
    pub away_team: u32,
    pub date: NaiveDate,
    pub stage: CompetitionStage,
}

#[derive(Debug, Clone)]
pub struct ContinentalMatchResult {
    pub home_team: u32,
    pub away_team: u32,
    pub home_score: u8,
    pub away_score: u8,
    pub competition: CompetitionTier,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CompetitionTier {
    ChampionsLeague,
    EuropaLeague,
    ConferenceLeague,
}

#[derive(Debug, Clone)]
pub struct TransferInterest {
    pub player_id: u32,
    pub source_country: u32,
    pub interest_level: f32,
}

#[derive(Debug, Clone)]
pub struct TransferNegotiation {
    pub player_id: u32,
    pub selling_club: u32,
    pub buying_club: u32,
    pub current_offer: f64,
}
