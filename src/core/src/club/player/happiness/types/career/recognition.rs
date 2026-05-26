/// Identifies which trophy / public recognition the event represents.
/// Maps 1:1 to the relevant `HappinessEventType` award variants and lets
/// the renderer pick recognition-specific copy without re-deriving the
/// kind from the event-type enum at render time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecognitionEventKind {
    PlayerOfTheWeek,
    YoungPlayerOfTheWeek,
    PlayerOfTheMonth,
    YoungPlayerOfTheMonth,
    TeamOfTheMonthSelection,
    YoungTeamOfTheMonthSelection,
    PlayerOfTheSeason,
    YoungPlayerOfTheSeason,
    TeamOfTheSeasonSelection,
    TeamOfTheYearSelection,
    LeagueTopScorer,
    LeagueTopAssists,
    LeagueGoldenGlove,
    WorldPlayerOfYear,
    WorldPlayerOfYearNomination,
    NationalTeamDebut,
}

impl RecognitionEventKind {
    pub fn as_token(&self) -> &'static str {
        match self {
            RecognitionEventKind::PlayerOfTheWeek => "player_of_the_week",
            RecognitionEventKind::YoungPlayerOfTheWeek => "young_player_of_the_week",
            RecognitionEventKind::PlayerOfTheMonth => "player_of_the_month",
            RecognitionEventKind::YoungPlayerOfTheMonth => "young_player_of_the_month",
            RecognitionEventKind::TeamOfTheMonthSelection => "team_of_the_month",
            RecognitionEventKind::YoungTeamOfTheMonthSelection => "young_team_of_the_month",
            RecognitionEventKind::PlayerOfTheSeason => "player_of_the_season",
            RecognitionEventKind::YoungPlayerOfTheSeason => "young_player_of_the_season",
            RecognitionEventKind::TeamOfTheSeasonSelection => "team_of_the_season",
            RecognitionEventKind::TeamOfTheYearSelection => "team_of_the_year",
            RecognitionEventKind::LeagueTopScorer => "league_top_scorer",
            RecognitionEventKind::LeagueTopAssists => "league_top_assists",
            RecognitionEventKind::LeagueGoldenGlove => "league_golden_glove",
            RecognitionEventKind::WorldPlayerOfYear => "world_player_of_year",
            RecognitionEventKind::WorldPlayerOfYearNomination => "world_player_nominee",
            RecognitionEventKind::NationalTeamDebut => "national_team_debut",
        }
    }
}

/// Recognition / award explanation payload — captured at emit time so the
/// renderer can describe what was won, the season totals or vote
/// margin behind the award, and who the closest contender was.
/// All quantitative fields are `Option` so emit sites can attach what's
/// available without forcing missing-data placeholders.
#[derive(Debug, Clone)]
pub struct RecognitionEventContext {
    pub kind: RecognitionEventKind,
    pub league_id: Option<u32>,
    pub country_id: Option<u32>,
    pub season_goals: Option<u16>,
    pub season_assists: Option<u16>,
    pub season_clean_sheets: Option<u16>,
    pub avg_rating: Option<f32>,
    /// Quantitative gap to the closest contender — vote share for POM/POS,
    /// goals lead for top scorer, ratings gap for season selections.
    /// Renderer interprets this together with `kind` so the unit doesn't
    /// have to be encoded in the type system.
    pub margin: Option<f32>,
    pub runner_up_player_id: Option<u32>,
    pub matches_played: Option<u16>,
    pub previous_caps: Option<u16>,
    /// True for first-time achievements (first POM, first cap, etc.). Lets
    /// the renderer surface "first" framing when relevant.
    pub first_time: bool,
}

impl RecognitionEventContext {
    pub fn new(kind: RecognitionEventKind) -> Self {
        Self {
            kind,
            league_id: None,
            country_id: None,
            season_goals: None,
            season_assists: None,
            season_clean_sheets: None,
            avg_rating: None,
            margin: None,
            runner_up_player_id: None,
            matches_played: None,
            previous_caps: None,
            first_time: false,
        }
    }

    pub fn with_league(mut self, id: u32) -> Self {
        self.league_id = Some(id);
        self
    }
    pub fn with_country(mut self, id: u32) -> Self {
        self.country_id = Some(id);
        self
    }
    pub fn with_season_goals(mut self, goals: u16) -> Self {
        self.season_goals = Some(goals);
        self
    }
    pub fn with_season_assists(mut self, assists: u16) -> Self {
        self.season_assists = Some(assists);
        self
    }
    pub fn with_clean_sheets(mut self, cs: u16) -> Self {
        self.season_clean_sheets = Some(cs);
        self
    }
    pub fn with_avg_rating(mut self, rating: f32) -> Self {
        self.avg_rating = Some(rating);
        self
    }
    pub fn with_margin(mut self, margin: f32) -> Self {
        self.margin = Some(margin);
        self
    }
    pub fn with_runner_up(mut self, id: u32) -> Self {
        self.runner_up_player_id = Some(id);
        self
    }
    pub fn with_matches_played(mut self, m: u16) -> Self {
        self.matches_played = Some(m);
        self
    }
    pub fn with_previous_caps(mut self, caps: u16) -> Self {
        self.previous_caps = Some(caps);
        self
    }
    pub fn with_first_time(mut self, first: bool) -> Self {
        self.first_time = first;
        self
    }
}
