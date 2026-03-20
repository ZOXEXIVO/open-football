use chrono::NaiveDate;

// ─── RecentMove ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecentMoveType {
    DemotedToReserves,
    RecalledFromReserves,
    PromotedToFirst,
    YouthPromoted,
    SwappedIn,
    SwappedOut,
}

#[derive(Debug, Clone, Copy)]
pub struct RecentMove {
    pub move_type: RecentMoveType,
    pub week: u32,
}

// ─── PlayerBias ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PlayerBias {
    pub quality_offset: f32,
    pub visibility: f32,
    pub sunk_cost: f32,
    pub first_impression: f32,
    pub anchored: bool,
    pub disappointments: u8,
    pub perception_drift: f32,
    pub last_observation_week: u32,
    pub overreaction_timer: u8,
    pub overreaction_magnitude: f32,
}

impl Default for PlayerBias {
    fn default() -> Self {
        PlayerBias {
            quality_offset: 0.0,
            visibility: 1.0,
            sunk_cost: 0.0,
            first_impression: 0.0,
            anchored: false,
            disappointments: 0,
            perception_drift: 0.0,
            last_observation_week: 0,
            overreaction_timer: 0,
            overreaction_magnitude: 0.0,
        }
    }
}

// ─── PlayerImpression ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PlayerImpression {
    pub player_id: u32,
    pub perceived_quality: f32,
    pub match_readiness: f32,
    pub coach_trust: f32,
    pub potential_impression: f32,
    pub training_impression: f32,
    pub last_updated: NaiveDate,
    pub weeks_in_squad: u16,
    pub recent_move: Option<RecentMove>,
    pub bias: PlayerBias,
    pub prev_red_cards: u8,
    pub prev_goals: u16,
    pub prev_avg_rating: f32,
}

impl PlayerImpression {
    pub fn new(player_id: u32, date: NaiveDate) -> Self {
        PlayerImpression {
            player_id,
            perceived_quality: 0.0,
            match_readiness: 0.0,
            coach_trust: 5.0,
            potential_impression: 0.0,
            training_impression: 0.0,
            last_updated: date,
            weeks_in_squad: 0,
            recent_move: None,
            bias: PlayerBias::default(),
            prev_red_cards: 0,
            prev_goals: 0,
            prev_avg_rating: 0.0,
        }
    }
}
