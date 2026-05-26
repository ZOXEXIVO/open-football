use crate::MatchTacticType;
use crate::club::facilities::FacilityLevel;

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
    /// Goal difference over the recent-form window.
    pub recent_goal_difference: i16,
    /// Season progress: matches played
    pub matches_played: u8,
    /// Season progress: total matches in a full season
    pub total_matches: u8,
    /// Average squad CA (main team)
    pub avg_squad_ability: u8,
    /// Average age of the main squad (0 when unknown).
    pub squad_avg_age: u8,
    /// Annual wage spend divided by the current wage budget. 1.0 means
    /// fully committed; >1.0 means the club is overspending its mandate.
    pub wage_budget_usage: f32,
    /// Main team's currently selected tactical formation. None when the
    /// team has no tactics configured. Used to judge style fit against
    /// `ClubVision.playing_style`.
    pub main_tactic: Option<MatchTacticType>,

    // ── Richer board inputs (component scoring / pressure / governance) ──
    /// Division tier the main team plays in (1 = top flight). Scales the
    /// board's ambition and how much a finish "should" be celebrated.
    pub league_tier: u8,
    /// Average league points per match this season (0.0 when unknown).
    pub points_per_match: f32,
    /// Full-season goal difference so far.
    pub goal_difference: i16,
    /// Positions clear of the relegation zone. 0 or negative = in it.
    pub distance_to_relegation: i16,
    /// Positions away from a European / promotion-playoff place. 0 or
    /// negative = currently in such a spot.
    pub distance_to_europe_or_playoff: i16,
    /// Crowd turnout proxy (form/standing driven), ~0.65..1.30. >1.1
    /// signals demand that can justify a stadium expansion.
    pub attendance_ratio: f32,
    /// Aggregate supporter mood 0.0 (mutinous) .. 1.0 (euphoric).
    pub supporter_mood: f32,
    /// Spent transfer budget / allocated transfer budget. 0.0 = unknown.
    /// TODO: thread real spent-this-window figure from the transfer
    /// pipeline; for now the financial score treats 0.0 as neutral.
    pub transfer_budget_usage: f32,
    /// Net debt / trailing annual revenue. 0.0 = no debt or unknown.
    pub debt_ratio: f32,
    /// Trailing-twelve-month profit (income − outcome).
    pub profit_loss_12m: i64,
    /// Academy players promoted to a senior squad this season.
    /// TODO: source the precise per-season count from the academy
    /// pipeline; defaults to 0 until then.
    pub academy_graduates_this_season: u32,
    /// Share of the main squad that is U21 — a proxy for youth minutes.
    pub u21_minutes_share: f32,
    /// Fraction of the main squad currently injured (0.0..1.0). Softens
    /// the squad-building blame when high.
    pub injury_crisis_score: f32,
    /// Months left on the head coach's contract (0 when unknown / vacant).
    pub manager_contract_months_left: i32,
    /// Count of key (senior) players currently unhappy / agitating.
    pub key_player_unrest_count: u8,

    // ── Facility levels (read by the yearly infrastructure review) ──
    pub facility_training: FacilityLevel,
    pub facility_youth: FacilityLevel,
    pub facility_academy: FacilityLevel,
    pub facility_recruitment: FacilityLevel,
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
            recent_goal_difference: 0,
            matches_played: 0,
            total_matches: 0,
            avg_squad_ability: 0,
            squad_avg_age: 0,
            wage_budget_usage: 0.0,
            main_tactic: None,
            league_tier: 1,
            points_per_match: 0.0,
            goal_difference: 0,
            distance_to_relegation: 0,
            distance_to_europe_or_playoff: 0,
            attendance_ratio: 1.0,
            supporter_mood: 0.5,
            transfer_budget_usage: 0.0,
            debt_ratio: 0.0,
            profit_loss_12m: 0,
            academy_graduates_this_season: 0,
            u21_minutes_share: 0.0,
            injury_crisis_score: 0.0,
            manager_contract_months_left: 0,
            key_player_unrest_count: 0,
            facility_training: FacilityLevel::Average,
            facility_youth: FacilityLevel::Average,
            facility_academy: FacilityLevel::Average,
            facility_recruitment: FacilityLevel::Average,
        }
    }
}
