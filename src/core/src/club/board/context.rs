use crate::MatchTacticType;

/// FFP status from the board's perspective. Drives the budget multiplier
/// so a club teetering on a breach can't sign its way deeper into one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FfpStatus {
    #[default]
    Clean,
    Watchlist,
    Breach,
}

#[derive(Clone)]
pub struct BoardContext {
    pub balance: i64,
    pub total_annual_wages: u32,
    pub reputation_score: f32,
    pub main_squad_size: usize,
    pub reserve_squad_size: usize,
    pub country_economic_factor: f32,
    pub country_price_level: f32,
    /// Total income across the trailing twelve months — read from the
    /// finance history. Drives revenue-based budget calculations.
    pub trailing_annual_income: i64,
    /// Total operating expenses across the trailing twelve months.
    pub trailing_annual_outcome: i64,
    /// FFP standing. Throttles transfer budget when breached/watchlisted.
    pub ffp_status: FfpStatus,

    // Performance tracking
    /// Current league position (1-based, 0 = unknown)
    pub league_position: u8,
    /// Total teams in the league
    pub league_size: u8,
    /// Recent form: wins in last 5 matches
    pub recent_wins: u8,
    /// Recent form: losses in last 5 matches
    pub recent_losses: u8,
    /// Season progress: matches played
    pub matches_played: u8,
    /// Season progress: total matches in a full season
    pub total_matches: u8,
    /// Average squad CA (main team)
    pub avg_squad_ability: u8,
    /// Main team's currently selected tactical formation. None when the
    /// team has no tactics configured. Used to judge style fit against
    /// `ClubVision.playing_style`.
    pub main_tactic: Option<MatchTacticType>,
}

impl BoardContext {
    pub fn new() -> Self {
        BoardContext {
            balance: 0,
            total_annual_wages: 0,
            reputation_score: 0.0,
            main_squad_size: 0,
            reserve_squad_size: 0,
            country_economic_factor: 1.0,
            country_price_level: 1.0,
            trailing_annual_income: 0,
            trailing_annual_outcome: 0,
            ffp_status: FfpStatus::Clean,
            league_position: 0,
            league_size: 0,
            recent_wins: 0,
            recent_losses: 0,
            matches_played: 0,
            total_matches: 0,
            avg_squad_ability: 0,
            main_tactic: None,
        }
    }
}
