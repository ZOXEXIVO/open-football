use crate::club::{BoardContext, BoardMood, BoardMoodState, BoardResult, StaffClubContract};
use crate::context::{GlobalContext, SimulationContext};
use log::debug;

/// Long-term club vision — the direction the board wants the manager to
/// take the club. Drives expectations, recruitment preferences, and
/// manager-board friction. Each item is advisory: the manager can ignore
/// it but the board will judge them against it at season's end.
#[derive(Debug, Clone, Default)]
pub struct ClubVision {
    pub playing_style: VisionPlayingStyle,
    pub youth_focus: VisionYouthFocus,
    pub signing_preference: SigningPreference,
    pub financial_stance: FinancialStance,
    pub long_term_goal: Option<LongTermGoal>,
    /// Seasons allotted for the manager to reach `long_term_goal`.
    pub long_term_horizon_seasons: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisionPlayingStyle {
    #[default]
    Balanced,
    AttackingFootball,
    Possession,
    HighPressing,
    DefensiveSolid,
    CounterAttack,
    DirectPlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisionYouthFocus {
    #[default]
    Balanced,
    /// Promote youth aggressively, prefer home-grown signings.
    DevelopYouth,
    /// Proven quality only; youth serves as backup.
    SignExperienced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SigningPreference {
    #[default]
    Anyone,
    /// Prefer home-nation or home-continent signings.
    Domestic,
    /// Actively scout cheaper regions for value gems.
    ValueHunter,
    /// Top-tier names only.
    Marquee,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FinancialStance {
    #[default]
    Balanced,
    /// Spend now, worry later.
    Ambitious,
    /// Live within wage budget; no loans.
    Conservative,
    /// Cost-cutting mode — sell high, minimise outgoings.
    Austerity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LongTermGoal {
    WinLeague,
    WinDomesticCup,
    WinContinental,
    PromotionToTopFlight,
    EstablishTopHalf,
    Survive,
}

#[derive(Debug, Clone)]
pub struct SeasonTargets {
    pub transfer_budget: i32,
    pub wage_budget: i32,
    pub max_squad_size: u8,
    pub min_squad_size: u8,
    /// Expected league finish position (1-based). Board judges performance against this.
    pub expected_position: u8,
    /// Minimum acceptable position before board becomes unhappy
    pub min_acceptable_position: u8,
}

/// Board confidence in the current management (0-100).
/// Drops when results are poor, recovers when exceeding expectations.
/// At 0: board sacks the manager (not yet implemented — future).
#[derive(Debug, Clone)]
pub struct BoardConfidence {
    pub level: i32,
}

impl Default for BoardConfidence {
    fn default() -> Self {
        BoardConfidence { level: 65 }
    }
}

#[derive(Debug, Clone)]
pub struct ClubBoard {
    pub mood: BoardMood,
    pub confidence: BoardConfidence,
    pub director: Option<StaffClubContract>,
    pub sport_director: Option<StaffClubContract>,
    pub season_targets: Option<SeasonTargets>,
    /// Consecutive months the board has been in Poor mood
    pub poor_mood_months: u8,
    /// Long-term vision — the "contract" the board expects the manager
    /// to honour across multiple seasons.
    pub vision: ClubVision,
}

impl ClubBoard {
    pub fn new() -> Self {
        ClubBoard {
            mood: BoardMood::default(),
            confidence: BoardConfidence::default(),
            director: None,
            sport_director: None,
            season_targets: None,
            poor_mood_months: 0,
            vision: ClubVision::default(),
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> BoardResult {
        let mut result = BoardResult::new();
        result.club_id = ctx.club.as_ref().map(|c| c.id).unwrap_or(0);

        if self.director.is_none() {
            self.run_director_election(&ctx.simulation);
        }

        if self.sport_director.is_none() {
            self.run_sport_director_election(&ctx.simulation);
        }

        if ctx.simulation.check_contract_expiration() {
            if self.is_director_contract_expiring(&ctx.simulation) {}
            if self.is_sport_director_contract_expiring(&ctx.simulation) {}
        }

        // Season start: calculate season targets and expectations
        let season = ctx.country.as_ref().map(|c| c.season_dates).unwrap_or_default();
        if ctx.simulation.is_season_start(&season) {
            if let Some(board_ctx) = &ctx.board {
                self.calculate_season_targets(board_ctx);
                self.confidence.level = 65; // Reset confidence at season start
                self.poor_mood_months = 0;
            }
        }

        // Monthly: evaluate performance, mood, confidence
        if ctx.simulation.is_month_beginning() {
            if let Some(board_ctx) = &ctx.board {
                self.evaluate_performance(board_ctx, &mut result);
            }
        }

        result
    }

    fn calculate_season_targets(&mut self, board_ctx: &BoardContext) {
        let rep = board_ctx.reputation_score;

        // Transfer budget: % of balance based on reputation tier
        let budget_pct = if rep >= 0.8 {
            0.40
        } else if rep >= 0.6 {
            0.35
        } else if rep >= 0.4 {
            0.30
        } else if rep >= 0.2 {
            0.25
        } else {
            0.20
        };

        let raw_budget = if board_ctx.balance > 0 {
            (board_ctx.balance as f64 * budget_pct) as i64
        } else {
            0
        };

        let eco = board_ctx.country_economic_factor as f64;
        let price = board_ctx.country_price_level as f64;
        let price_ceiling = price * price * 80_000_000.0;
        let eco_ceiling = eco * eco * 300_000_000.0;
        let budget_ceiling = price_ceiling.min(eco_ceiling) as i64;
        let transfer_budget = raw_budget.min(budget_ceiling) as i32;

        // Wage budget: current annual wages * growth factor
        let wage_growth = if rep >= 0.7 {
            1.10
        } else if rep >= 0.4 {
            1.05
        } else {
            1.00
        };
        let annual_wages = board_ctx.total_annual_wages as f64;
        let wage_budget = (annual_wages * wage_growth) as i32;

        // Squad size limits based on reputation
        let (min_squad, max_squad) = if rep >= 0.8 {
            (25u8, 50u8)
        } else if rep >= 0.6 {
            (23, 45)
        } else if rep >= 0.4 {
            (20, 38)
        } else if rep >= 0.2 {
            (18, 30)
        } else {
            (16, 25)
        };

        // Expected league position based on reputation within the league.
        // Higher reputation = higher expectations.
        let (expected, min_acceptable) = if board_ctx.league_size > 0 {
            let league_sz = board_ctx.league_size as f32;
            // Expected: reputation maps to position (top rep = 1st, low rep = bottom)
            let expected_pct = 1.0 - rep; // 0.8 rep → top 20%
            let expected = ((expected_pct * league_sz) as u8).clamp(1, board_ctx.league_size);
            // Min acceptable: 50% further down from expected (e.g. expected 3rd → acceptable 8th in 20-team)
            let buffer = (league_sz * 0.25) as u8;
            let min_acceptable = (expected + buffer).min(board_ctx.league_size);
            (expected, min_acceptable)
        } else {
            (1, 1)
        };

        self.season_targets = Some(SeasonTargets {
            transfer_budget,
            wage_budget,
            max_squad_size: max_squad,
            min_squad_size: min_squad,
            expected_position: expected,
            min_acceptable_position: min_acceptable,
        });
    }

    /// Monthly performance evaluation — the core of board behavior.
    /// Considers: league position vs expectations, recent form, finances, squad state.
    fn evaluate_performance(&mut self, board_ctx: &BoardContext, result: &mut BoardResult) {
        let targets = match &self.season_targets {
            Some(t) => t,
            None => return,
        };

        // ── Factor 1: League performance vs expectations ──
        let performance_delta = if board_ctx.league_position > 0 && board_ctx.matches_played >= 5 {
            // Positive = above expectations, negative = below
            let expected = targets.expected_position as i32;
            let actual = board_ctx.league_position as i32;
            expected - actual // e.g. expected 5th, actual 3rd → +2 (good)
        } else {
            0 // Not enough data yet
        };

        // ── Factor 2: Recent form (last 5 matches) ──
        let form_score: i32 = board_ctx.recent_wins as i32 - board_ctx.recent_losses as i32;
        // form_score: -5 (all losses) to +5 (all wins)

        // ── Factor 3: Financial health ──
        let financial_health = if board_ctx.balance > 0 {
            1 // Healthy
        } else if board_ctx.balance > -(board_ctx.total_annual_wages as i64 / 4) {
            0 // Minor concern
        } else {
            -2 // Serious financial trouble
        };

        // ── Factor 4: Squad state ──
        let total_squad = board_ctx.main_squad_size + board_ctx.reserve_squad_size;
        let squad_bloated = total_squad > (targets.max_squad_size as usize + 5);
        let squad_thin = board_ctx.main_squad_size < targets.min_squad_size as usize;

        let squad_health: i32 = if squad_bloated { -1 } else if squad_thin { -1 } else { 0 };

        // ── Update confidence (cumulative, carries across months) ──
        let confidence_change =
            performance_delta * 3     // League position is most important
            + form_score * 2          // Recent form matters
            + financial_health * 2    // Financial stability
            + squad_health;           // Squad management

        self.confidence.level = (self.confidence.level + confidence_change).clamp(0, 100);

        // ── Determine board mood from confidence level ──
        let new_mood = if self.confidence.level >= 80 {
            BoardMoodState::Excellent
        } else if self.confidence.level >= 55 {
            BoardMoodState::Good
        } else if self.confidence.level >= 30 {
            BoardMoodState::Normal
        } else {
            BoardMoodState::Poor
        };

        // Track consecutive poor months
        if matches!(new_mood, BoardMoodState::Poor) {
            self.poor_mood_months += 1;
        } else {
            self.poor_mood_months = 0;
        }

        self.mood.state = new_mood;

        // ── Board actions based on mood ──

        // Poor mood: cut transfer budget
        if matches!(self.mood.state, BoardMoodState::Poor) {
            result.cut_transfer_budget = true;
        }

        // Excellent mood + overperforming: board releases extra transfer funds
        if matches!(self.mood.state, BoardMoodState::Excellent) && performance_delta > 3 {
            result.bonus_transfer_funds = true;
        }

        // Squad issues
        if squad_bloated {
            result.squad_over_limit = true;
            result.squad_excess = total_squad.saturating_sub(targets.max_squad_size as usize);
        }

        if squad_thin {
            result.squad_under_limit = true;
        }

        // Position alarm: underperforming significantly
        if board_ctx.league_position > 0
            && board_ctx.league_position > targets.min_acceptable_position
            && board_ctx.matches_played >= 10
        {
            result.underperforming = true;
        }

        result.mood = self.mood.state.clone();
        result.confidence = self.confidence.level;

        if result.underperforming || matches!(self.mood.state, BoardMoodState::Poor) {
            debug!(
                "Board unhappy (confidence: {}, position: {}/{}, expected: {})",
                self.confidence.level,
                board_ctx.league_position,
                board_ctx.league_size,
                targets.expected_position
            );
        }
    }

    fn is_director_contract_expiring(&self, simulation_ctx: &SimulationContext) -> bool {
        match &self.director {
            Some(d) => d.is_expired(simulation_ctx),
            None => false,
        }
    }

    fn run_director_election(&mut self, _: &SimulationContext) {}

    fn is_sport_director_contract_expiring(&self, simulation_ctx: &SimulationContext) -> bool {
        match &self.sport_director {
            Some(d) => d.is_expired(simulation_ctx),
            None => false,
        }
    }

    fn run_sport_director_election(&mut self, _: &SimulationContext) {}
}
