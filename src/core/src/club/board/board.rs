use crate::club::board::manager_market::ManagerCandidate;
use crate::club::team::reputation::AchievementType;
use crate::club::{BoardContext, BoardMood, BoardMoodState, BoardResult, StaffClubContract};
use crate::context::{GlobalContext, SimulationContext};
use crate::MatchTacticType;
use chrono::{Datelike, NaiveDate};
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

/// Ownership personality — a simplified chairman archetype whose traits
/// shape how the board actually exercises its powers. Two knobs, each
/// with meaningful consequences downstream of board.simulate().
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChairmanAmbition {
    #[default]
    Balanced,
    /// "We want the Champions League." Budget skew +, expectations +.
    Ambitious,
    /// Sugar daddy / oil money. Budget skew ++, expectations ++,
    /// but also trigger-happy when results slip.
    Reckless,
    /// Old-money prudent. Budget skew -, stability prized.
    Conservative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChairmanPatience {
    #[default]
    Medium,
    /// Results yesterday. Sacking threshold is one bad run away.
    Low,
    /// Long-term project builder, trusts the process.
    High,
}

#[derive(Debug, Clone, Default)]
pub struct ChairmanProfile {
    pub ambition: ChairmanAmbition,
    pub patience: ChairmanPatience,
    /// 0..100 — how personally loyal the chairman is to the current manager.
    /// Rebuilt on each hire; decays with poor form, lifts with trophies.
    pub manager_loyalty: u8,
}

impl ChairmanProfile {
    pub fn new() -> Self {
        ChairmanProfile {
            ambition: ChairmanAmbition::Balanced,
            patience: ChairmanPatience::Medium,
            manager_loyalty: 50,
        }
    }

    /// Poor-mood-month threshold before patience snaps. Lower = quicker
    /// firing. High-loyalty chairmen buy their guy some extra time.
    pub fn poor_mood_threshold(&self) -> u8 {
        let base = match self.patience {
            ChairmanPatience::Low => 3,
            ChairmanPatience::Medium => 4,
            ChairmanPatience::High => 6,
        };
        // Loyal chairmen tolerate one extra poor month before acting.
        if self.manager_loyalty >= 70 {
            base + 1
        } else if self.manager_loyalty <= 20 {
            base.saturating_sub(1).max(1)
        } else {
            base
        }
    }

    /// Multiplier applied to the baseline transfer budget. Reckless owners
    /// push spend harder; conservative ones throttle it.
    pub fn budget_multiplier(&self) -> f32 {
        match self.ambition {
            ChairmanAmbition::Reckless => 1.4,
            ChairmanAmbition::Ambitious => 1.15,
            ChairmanAmbition::Balanced => 1.0,
            ChairmanAmbition::Conservative => 0.85,
        }
    }
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
/// At 0 — or after sustained Poor mood — the manager is sacked.
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
    /// Year the current vision horizon started. Populated on the first
    /// season-start tick after the vision is installed. Reset at the end
    /// of each horizon regardless of outcome.
    pub vision_start_year: Option<i32>,
    /// Set to true the first time a trophy / promotion matching the
    /// long-term goal lands in the current horizon. Tracked separately
    /// from `team.reputation` achievements because those decay after two
    /// years and horizons can extend longer.
    pub vision_goal_achieved: bool,
    /// Date the last manager was dismissed — drives the search timer.
    /// `None` when the manager seat is filled (either permanently, or
    /// an interim has been confirmed as permanent).
    pub manager_search_since: Option<NaiveDate>,
    /// Ranked free-agent (slice B) and employed-target (slice C)
    /// candidates the board is willing to appoint. Refreshed weekly
    /// while a search is open. Front of vec = top choice.
    pub manager_shortlist: Vec<ManagerCandidate>,
    /// Day the current shortlist was built. Used to decide when it's
    /// stale enough to rebuild — see `manager_market::SHORTLIST_REFRESH_DAYS`.
    pub shortlist_built_at: Option<NaiveDate>,
    /// How long the search may run before the board commits to a
    /// hire. Locked in when `manager_search_since` is set so it stays
    /// stable across the search window. Top clubs hold out longer.
    pub search_window_days: u16,
    /// Ownership archetype. Modulates budget size, sacking threshold,
    /// and long-term tolerance. Populated at club creation; stable for
    /// the lifetime of the chairman.
    pub chairman: ChairmanProfile,
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
            vision_start_year: None,
            vision_goal_achieved: false,
            manager_search_since: None,
            manager_shortlist: Vec::new(),
            shortlist_built_at: None,
            search_window_days: 0,
            chairman: ChairmanProfile::new(),
        }
    }

    /// True when the current long-term goal matches the achievement just
    /// earned. Call at trophy time to flip `vision_goal_achieved`.
    pub fn matches_long_term_goal(&self, ach: AchievementType) -> bool {
        let Some(goal) = self.vision.long_term_goal else {
            return false;
        };
        use LongTermGoal::*;
        matches!(
            (goal, ach),
            (WinLeague, AchievementType::LeagueTitle)
                | (WinDomesticCup, AchievementType::CupWin)
                | (WinContinental, AchievementType::ContinentalTrophy)
                | (PromotionToTopFlight, AchievementType::Promotion)
        )
    }

    /// Flip `vision_goal_achieved` when this achievement lands the long-term
    /// target. Returns true if the flag changed.
    pub fn on_achievement(&mut self, ach: AchievementType) -> bool {
        if !self.vision_goal_achieved && self.matches_long_term_goal(ach) {
            self.vision_goal_achieved = true;
            true
        } else {
            false
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
                let current_year = ctx.simulation.date.date().year();
                self.evaluate_long_term_vision(current_year, &mut result);
                self.calculate_season_targets(board_ctx);
                // Season-end review: if the manager's still in their seat
                // and the board is happy with them, trigger a renewal
                // offer. Carries the baseline confidence into the new
                // season so the manager keeps the momentum.
                if !result.manager_sacked
                    && self.confidence.level >= 70
                    && self.chairman.manager_loyalty >= 55
                {
                    result.offer_manager_renewal = true;
                }
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

        // Manager search: once the per-club search window elapses, signal
        // the result stage to confirm a permanent appointment. The result
        // stage tries the top free-agent shortlist first (slice B) and
        // falls back to promoting the caretaker if no candidate sticks.
        // Window length scales with reputation — top clubs hunt longer
        // because they're chasing big names; smaller clubs move faster.
        if let Some(since) = self.manager_search_since {
            let today = ctx.simulation.date.date();
            let days = (today - since).num_days();
            // Defensive: a board with `manager_search_since` set but a
            // zero search window (legacy state, or first tick after a
            // hot-reload) falls back to the previous fixed value so the
            // seat doesn't sit empty forever.
            let window = if self.search_window_days == 0 {
                30
            } else {
                self.search_window_days as i64
            };
            if days >= window {
                result.confirm_new_manager = true;
            }
        }

        result
    }

    /// Check whether the long-term horizon has elapsed and reckon with the
    /// manager against the original vision goal. Fires at the START of a
    /// season — the previous season's trophies are already banked in
    /// `vision_goal_achieved`. Horizonless visions (no `long_term_goal`)
    /// don't trigger any judgment.
    fn evaluate_long_term_vision(&mut self, current_year: i32, result: &mut BoardResult) {
        if self.vision.long_term_goal.is_none() || self.vision.long_term_horizon_seasons == 0 {
            return;
        }

        let start_year = match self.vision_start_year {
            Some(y) => y,
            None => {
                // First season under this vision — start the clock and return.
                self.vision_start_year = Some(current_year);
                return;
            }
        };

        let seasons_elapsed = (current_year - start_year).max(0) as u8;
        if seasons_elapsed < self.vision.long_term_horizon_seasons {
            return;
        }

        // Horizon reached. Judge and reset regardless of outcome.
        if !self.vision_goal_achieved {
            debug!(
                "Long-term vision failed: goal {:?} not met in {} seasons — manager sacked",
                self.vision.long_term_goal, self.vision.long_term_horizon_seasons
            );
            result.manager_sacked = true;
            self.confidence.level = 20;
            self.poor_mood_months = 0;
        } else {
            // Horizon met. Small confidence bump so the next horizon starts
            // on a positive note; board keeps the manager.
            self.confidence.level = (self.confidence.level + 10).clamp(0, 100);
        }

        self.vision_start_year = Some(current_year);
        self.vision_goal_achieved = false;
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
        // Chairman tilts the budget — reckless spends harder, conservative
        // throttles. Applied to the raw budget before hitting the sanity
        // ceiling, so a reckless owner at a mid-table club gets a real
        // war chest but can't breach the country-wide economic cap.
        let chair_mult = self.chairman.budget_multiplier() as f64;
        let tilted_budget = (raw_budget as f64 * chair_mult) as i64;
        let transfer_budget = tilted_budget.min(budget_ceiling) as i32;

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

        // ── Factor 5: Style fit ──
        // How much does the chosen formation embody the board's preferred
        // playing style? A non-default vision that the manager ignores
        // erodes confidence, even when results are decent.
        let style_drag = match board_ctx.main_tactic {
            Some(t) => style_mismatch_drag(self.vision.playing_style, t),
            None => 0,
        };

        // ── Update confidence (cumulative, carries across months) ──
        let confidence_change =
            performance_delta * 3     // League position is most important
            + form_score * 2          // Recent form matters
            + financial_health * 2    // Financial stability
            + squad_health            // Squad management
            - style_drag;             // Vision / tactics alignment

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

        // Chairman loyalty drifts with results — trophies and strong league
        // positions build personal trust, collapses erode it. Capped [0,100].
        let loyalty_delta: i16 = if performance_delta >= 3 {
            2
        } else if performance_delta >= 1 {
            1
        } else if performance_delta <= -3 {
            -3
        } else if performance_delta <= -1 {
            -1
        } else {
            0
        };
        self.chairman.manager_loyalty = ((self.chairman.manager_loyalty as i16
            + loyalty_delta)
            .clamp(0, 100)) as u8;

        // Manager's own morale tracks the board mood. A manager working
        // for a happy chairman feels secure; a manager under Poor mood
        // for months feels the pressure.
        let mood_delta = match self.mood.state {
            BoardMoodState::Excellent => 1.5,
            BoardMoodState::Good => 0.5,
            BoardMoodState::Normal => 0.0,
            BoardMoodState::Poor => {
                // Scaled by how long it's been — a second poor month lands
                // much harder than the first.
                -1.0 - (self.poor_mood_months as f32 * 0.5).min(3.0)
            }
        };
        // Style-clash friction: if the manager's preferred tactic fights
        // the board's vision, the manager also feels that tension (not
        // just the board). Even in a Good-mood run, a coach pushed into
        // a style they hate bleeds satisfaction slowly.
        let style_friction = (style_drag as f32 * 0.35).min(1.5);
        result.manager_satisfaction_delta = mood_delta - style_friction;

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

        // ── Sacking gate ──
        // Three independent triggers, any one fires:
        //   1. Confidence collapsed to zero (extreme, rare)
        //   2. Poor mood ≥N consecutive months AND underperforming (chairman patience tunes N)
        //   3. Poor mood ≥(N+2) consecutive months regardless
        // Early-season grace: need at least 10 matches played so a bad August
        // doesn't cost a job. Re-hires are out of scope here — the transfer
        // pipeline / staff search will offer a replacement next tick.
        let enough_data = board_ctx.matches_played >= 10;
        let zero_confidence = self.confidence.level <= 0;
        let patience_threshold = self.chairman.poor_mood_threshold();
        let sustained_poor_with_underperformance =
            self.poor_mood_months >= patience_threshold && result.underperforming;
        let sustained_poor_absolute = self.poor_mood_months >= patience_threshold + 2;

        if enough_data
            && (zero_confidence
                || sustained_poor_with_underperformance
                || sustained_poor_absolute)
        {
            result.manager_sacked = true;
            // Reset confidence so the successor starts from a neutral base
            // when the next board tick runs; avoids immediate re-sack.
            self.confidence.level = 50;
            self.poor_mood_months = 0;
        }
    }

    fn is_director_contract_expiring(&self, simulation_ctx: &SimulationContext) -> bool {
        match &self.director {
            Some(d) => d.is_expired(simulation_ctx),
            None => false,
        }
    }

    /// Stand up a fresh director contract — four-year term, salary
    /// indexed to board ambition. This is the board's own administrative
    /// slot, separate from the team's DoF staff member.
    fn run_director_election(&mut self, ctx: &SimulationContext) {
        use crate::{StaffPosition, StaffStatus};
        let base_salary: u32 = match self.chairman.ambition {
            ChairmanAmbition::Reckless | ChairmanAmbition::Ambitious => 200_000,
            ChairmanAmbition::Balanced => 120_000,
            ChairmanAmbition::Conservative => 80_000,
        };
        let expires = ctx.date.date()
            .with_year(ctx.date.date().year() + 4)
            .unwrap_or(ctx.date.date());
        self.director = Some(StaffClubContract::new(
            base_salary,
            expires,
            StaffPosition::Director,
            StaffStatus::Active,
        ));
    }

    fn is_sport_director_contract_expiring(&self, simulation_ctx: &SimulationContext) -> bool {
        match &self.sport_director {
            Some(d) => d.is_expired(simulation_ctx),
            None => false,
        }
    }

    /// Stand up a sport director contract — three-year term; this is a
    /// more "football-side" role so salary floor is slightly higher.
    fn run_sport_director_election(&mut self, ctx: &SimulationContext) {
        use crate::{StaffPosition, StaffStatus};
        let base_salary: u32 = match self.chairman.ambition {
            ChairmanAmbition::Reckless | ChairmanAmbition::Ambitious => 250_000,
            ChairmanAmbition::Balanced => 150_000,
            ChairmanAmbition::Conservative => 100_000,
        };
        let expires = ctx.date.date()
            .with_year(ctx.date.date().year() + 3)
            .unwrap_or(ctx.date.date());
        self.sport_director = Some(StaffClubContract::new(
            base_salary,
            expires,
            StaffPosition::DirectorOfFootball,
            StaffStatus::Active,
        ));
    }
}

/// How poorly does `tactic` embody `style`? 0 = fine, up to 2 = strong
/// clash. Used as a monthly confidence drag so the board slowly loses
/// patience with a manager whose football doesn't match what they were
/// hired to deliver. `Balanced` never drags.
fn style_mismatch_drag(style: VisionPlayingStyle, tactic: MatchTacticType) -> i32 {
    use MatchTacticType::*;
    use VisionPlayingStyle::*;

    // Bias each formation on two axes: attacking weight (more forwards)
    // and possession weight (tight midfield). Hand-tuned from conventional
    // football wisdom rather than derived from match-engine values.
    let (attacking, possession) = match tactic {
        T343 => (2, 0),
        T4222 => (2, 1),
        T433 => (1, 2),
        T4231 => (1, 2),
        T4312 => (1, 1),
        T442 => (0, 0),
        T442Diamond | T442Narrow | T442DiamondWide => (0, 1),
        T352 => (0, 0),
        T4411 => (-1, 0),
        T4141 => (-1, 1),
        T451 => (-2, 0),
        T1333 => (-2, -1),
    };

    match style {
        Balanced => 0,
        AttackingFootball => (1 - attacking).max(0),
        DefensiveSolid => (1 + attacking).max(0),
        Possession => (1 - possession).max(0),
        DirectPlay => (possession).max(0),
        HighPressing => (1 - possession).max(0) + (0 - attacking).max(0),
        CounterAttack => (attacking - 1).max(0),
    }
}

#[cfg(test)]
mod style_fit_tests {
    use super::*;

    #[test]
    fn balanced_vision_never_drags() {
        for t in MatchTacticType::all() {
            assert_eq!(style_mismatch_drag(VisionPlayingStyle::Balanced, t), 0);
        }
    }

    #[test]
    fn attacking_vision_punishes_defensive_formations() {
        assert!(style_mismatch_drag(VisionPlayingStyle::AttackingFootball, MatchTacticType::T451) > 0);
        assert!(style_mismatch_drag(VisionPlayingStyle::AttackingFootball, MatchTacticType::T1333) > 0);
    }

    #[test]
    fn attacking_vision_accepts_attacking_formations() {
        assert_eq!(style_mismatch_drag(VisionPlayingStyle::AttackingFootball, MatchTacticType::T343), 0);
        assert_eq!(style_mismatch_drag(VisionPlayingStyle::AttackingFootball, MatchTacticType::T4222), 0);
    }

    #[test]
    fn defensive_vision_punishes_attacking_formations() {
        assert!(style_mismatch_drag(VisionPlayingStyle::DefensiveSolid, MatchTacticType::T343) > 0);
        assert!(style_mismatch_drag(VisionPlayingStyle::DefensiveSolid, MatchTacticType::T4222) > 0);
    }

    #[test]
    fn possession_vision_accepts_possession_formations() {
        assert_eq!(style_mismatch_drag(VisionPlayingStyle::Possession, MatchTacticType::T433), 0);
        assert_eq!(style_mismatch_drag(VisionPlayingStyle::Possession, MatchTacticType::T4231), 0);
    }

    #[test]
    fn counter_attack_vision_prefers_modest_formations() {
        // T442 = balanced → fits counter-attack fine.
        assert_eq!(style_mismatch_drag(VisionPlayingStyle::CounterAttack, MatchTacticType::T442), 0);
        // T343 = all-out attack → clashes with counter-attack's defensive base.
        assert!(style_mismatch_drag(VisionPlayingStyle::CounterAttack, MatchTacticType::T343) > 0);
    }
}
