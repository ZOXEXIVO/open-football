#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeasonOutcomeKind {
    Relegated,
    RelegationFear,
    SurvivedRelegation,
}

impl SeasonOutcomeKind {
    pub fn as_token(&self) -> &'static str {
        match self {
            SeasonOutcomeKind::Relegated => "relegated",
            SeasonOutcomeKind::RelegationFear => "relegation_fear",
            SeasonOutcomeKind::SurvivedRelegation => "survived_relegation",
        }
    }
}

/// Season-outcome context for relegation-adjacent events. Carries the
/// final / current league standing, points gap to safety, and matches
/// remaining when the worry crystallised, so the renderer can describe
/// "10th in the table, 4 points clear of the drop with 6 to play"
/// instead of a generic "Relegation fear".
#[derive(Debug, Clone)]
pub struct SeasonOutcomeContext {
    pub kind: SeasonOutcomeKind,
    pub league_id: Option<u32>,
    pub final_position: Option<u8>,
    pub points: Option<u16>,
    /// Points gap to the safety line (positive = clear of the drop,
    /// negative = below it). For `Relegated`, captures how far short.
    pub points_to_safety: Option<i16>,
    pub matches_remaining: Option<u8>,
    pub season_participation: Option<f32>,
}

impl SeasonOutcomeContext {
    pub fn new(kind: SeasonOutcomeKind) -> Self {
        Self {
            kind,
            league_id: None,
            final_position: None,
            points: None,
            points_to_safety: None,
            matches_remaining: None,
            season_participation: None,
        }
    }

    pub fn with_league(mut self, id: u32) -> Self {
        self.league_id = Some(id);
        self
    }
    pub fn with_final_position(mut self, pos: u8) -> Self {
        self.final_position = Some(pos);
        self
    }
    pub fn with_points(mut self, points: u16) -> Self {
        self.points = Some(points);
        self
    }
    pub fn with_points_to_safety(mut self, gap: i16) -> Self {
        self.points_to_safety = Some(gap);
        self
    }
    pub fn with_matches_remaining(mut self, n: u8) -> Self {
        self.matches_remaining = Some(n);
        self
    }
    pub fn with_participation(mut self, p: f32) -> Self {
        self.season_participation = Some(p);
        self
    }
}
