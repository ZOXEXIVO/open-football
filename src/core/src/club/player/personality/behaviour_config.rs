//! Tunables for player behaviour subsystems.
//!
//! Three sub-configs grouped by concern:
//!
//! - [`AdaptationConfig`] — settlement-shock thresholds (ambition, dream
//!   move, elite-club, salary), settlement window length, language and
//!   personality multipliers, step-up development bonus.
//! - [`HappinessConfig`] — morale framework constants (decay halflife,
//!   event retention cap, default morale, happy threshold).
//! - [`PassEvaluatorConfig`] — match-AI pass evaluator tunables
//!   (distance/vision/range bands, ultra-long thresholds, recommendation
//!   risk gates, pressure radius).
//!
//! Centralising these mirrors the `TransferConfig` and `ScoutingConfig`
//! pattern: one place to tune, unit-testable helpers, an obvious hook
//! for per-difficulty / per-save overrides later.

// ============================================================
// AdaptationConfig — post-transfer settlement & shock events
// ============================================================

#[derive(Debug, Clone)]
pub struct AdaptationConfig {
    /// Window (in days) over which settlement form / shock events apply.
    /// Match rating is dampened linearly across this band, then recovers.
    pub settlement_window_days: i64,

    /// Window (days from transfer) inside which weekly integration ticks
    /// fire (bonding / isolation events). Outliving the form penalty by
    /// design — squad chemistry takes longer than match form to settle.
    pub integration_window_days: i64,

    /// Base form-rating penalty applied at day 0 of the settlement window
    /// (before language / personality / step-up adjustments). Recovers
    /// linearly to 0 by `settlement_window_days`.
    pub settlement_base_penalty: f32,
    /// Floor on the settlement multiplier — even worst-case it never goes
    /// below this fraction of the player's normal rating.
    pub settlement_multiplier_floor: f32,
    /// Multiplier applied to penalty when the player speaks the local
    /// language fluently. < 1.0 → softer landing.
    pub language_penalty_factor: f32,
    /// At max adaptability (20), penalty is reduced by this fraction.
    /// Linear scaling toward 0 at adaptability=0.
    pub adaptability_max_penalty_reduction: f32,
    /// Step-up moves reduce the penalty by this factor (excitement → less
    /// rating drop while settling).
    pub step_up_penalty_factor: f32,

    // ── Shock-event thresholds ──────────────────────────────────
    /// Reputation gap (`expected - actual`) past which the player notices
    /// he joined below his ambition. 0–10000 scale.
    pub ambition_shock_threshold: f32,
    /// Reputation gap below which an ambition gap doesn't fire at all.
    /// (Ambition <= this value → never trigger ambition shock regardless
    /// of club rep.)
    pub ambition_shock_min_ambition: f32,
    /// Linear projection from ambition (0–20) to expected club reputation
    /// (0–10000). Used for the gap calculation in ambition shock.
    pub ambition_to_expected_rep_factor: f32,
    /// Subtracted from raw ambition before scaling. Different defaults for
    /// the dream-move calculation vs the ambition-shock calculation
    /// (ambition shock uses a higher floor — only the very ambitious feel it).
    pub ambition_dream_floor: f32,
    pub ambition_shock_floor: f32,

    /// Reputation surplus (club rep above expected) past which the move
    /// is felt as a clear step up.
    pub dream_move_threshold: f32,

    /// Club reputation (0–10000) above which a signing carries elite prestige.
    pub elite_club_reputation: f32,
    /// Required gap between elite club reputation and the player's own
    /// reputation before `JoiningElite` fires. Prevents stars feeling
    /// "joined elite" when they're already at that level.
    pub elite_club_min_player_gap: f32,

    /// New/old salary ratio below which `SalaryShock` fires.
    pub salary_shock_ratio: f32,
    /// New/old salary ratio above which `SalaryBoost` fires.
    pub salary_boost_ratio: f32,

    /// Loans dampen all shock magnitudes by this factor (0..1).
    pub loan_damp_factor: f32,

    /// Fee in dollars at which a permanent signing carries an implicit
    /// playing-time promise.
    pub big_money_signing_fee: f64,
    /// Promise horizon (days) for permanent moves above the big-money threshold.
    pub permanent_promise_horizon_days: i64,
    /// Promise horizon (days) for any loan move (always implicit).
    pub loan_promise_horizon_days: i64,

    // ── Step-up development multiplier ──────────────────────────
    /// Player's `world_reputation` to club rep gap below which no
    /// development bonus applies. Above it, gap_factor scales linearly.
    pub step_up_dev_min_gap: f32,
    /// Reputation gap normaliser — `gap / this` clamped to
    /// `step_up_dev_max_gap_factor` gives the size of the bonus.
    pub step_up_dev_gap_normaliser: f32,
    pub step_up_dev_max_gap_factor: f32,
    /// Multiplier ceiling — final result is clamped to `[1.0, this]`.
    pub step_up_dev_multiplier_ceiling: f32,

    // ── Integration tick ─────────────────────────────────────────
    /// Adaptability + professionalism, both 0–20, summed and divided by 40
    /// gives `pull_toward_bonding` in [0,1]. Above this threshold the
    /// player bonds with teammates.
    pub bonding_pull_threshold: f32,
    /// Above this threshold (and after `settled_min_weeks`) the player
    /// also feels "settled into squad" — distinct from initial bonding.
    pub settled_pull_threshold: f32,
    pub settled_min_weeks: i64,
    /// Adaptability below this value, with no local language and within
    /// the early window, fires `FeelingIsolated`.
    pub early_isolation_max_adaptability: f32,
    pub early_isolation_max_weeks: i64,
    /// Pull threshold above which chronic monthly isolation is suppressed
    /// for foreign players who never learned the language.
    pub chronic_isolation_suppress_threshold: f32,
}

impl Default for AdaptationConfig {
    fn default() -> Self {
        AdaptationConfig {
            settlement_window_days: 84,
            integration_window_days: 168,
            settlement_base_penalty: 0.15,
            settlement_multiplier_floor: 0.80,
            language_penalty_factor: 0.4,
            adaptability_max_penalty_reduction: 0.6,
            step_up_penalty_factor: 0.6,

            ambition_shock_threshold: 4000.0,
            ambition_shock_min_ambition: 10.0,
            ambition_to_expected_rep_factor: 800.0,
            ambition_dream_floor: 5.0,
            ambition_shock_floor: 10.0,

            dream_move_threshold: 1500.0,

            elite_club_reputation: 7500.0,
            elite_club_min_player_gap: 1500.0,

            salary_shock_ratio: 0.4,
            salary_boost_ratio: 1.8,

            loan_damp_factor: 0.7,

            big_money_signing_fee: 5_000_000.0,
            permanent_promise_horizon_days: 90,
            loan_promise_horizon_days: 60,

            step_up_dev_min_gap: 1000.0,
            step_up_dev_gap_normaliser: 8000.0,
            step_up_dev_max_gap_factor: 0.25,
            step_up_dev_multiplier_ceiling: 1.25,

            bonding_pull_threshold: 0.55,
            settled_pull_threshold: 0.5,
            settled_min_weeks: 8,
            early_isolation_max_adaptability: 12.0,
            early_isolation_max_weeks: 4,
            chronic_isolation_suppress_threshold: 0.7,
        }
    }
}

impl AdaptationConfig {
    /// Settlement multiplier for a player `days_since_transfer` into a new
    /// club. Returns 1.0 if outside the settlement window.
    pub fn settlement_multiplier(
        &self,
        days_since_transfer: Option<i64>,
        speaks_local_language: bool,
        adaptability: f32,
        is_step_up: bool,
    ) -> f32 {
        let days = match days_since_transfer {
            Some(d) if d >= 0 && d < self.settlement_window_days => d as f32,
            _ => return 1.0,
        };
        let recovery = days / self.settlement_window_days as f32;
        let mut penalty = (1.0 - recovery) * self.settlement_base_penalty;
        if speaks_local_language {
            penalty *= self.language_penalty_factor;
        }
        let adapt = adaptability.clamp(0.0, 20.0);
        let adapt_factor = 1.0 - (adapt / 20.0) * self.adaptability_max_penalty_reduction;
        penalty *= adapt_factor;
        if is_step_up {
            penalty *= self.step_up_penalty_factor;
        }
        (1.0 - penalty).clamp(self.settlement_multiplier_floor, 1.0)
    }

    /// Step-up move predicate. Compares club rep (0–1 normalised) to the
    /// player's ambition-derived expectation; returns true when the gap
    /// crosses `dream_move_threshold`.
    pub fn is_step_up_move(&self, ambition: f32, club_rep_0_to_1: f32) -> bool {
        let expected_rep =
            (ambition - self.ambition_dream_floor).max(0.0) * self.ambition_to_expected_rep_factor;
        let club_rep = club_rep_0_to_1 * 10000.0;
        club_rep - expected_rep >= self.dream_move_threshold
    }

    /// Implicit playing-time promise horizon in days. Returns 0 if no
    /// promise should be recorded.
    pub fn promise_horizon_days(&self, is_loan: bool, fee: f64) -> i64 {
        if is_loan {
            self.loan_promise_horizon_days
        } else if fee >= self.big_money_signing_fee {
            self.permanent_promise_horizon_days
        } else {
            0
        }
    }

    /// Multiplier (≥ 1.0) applied to skill development while the player
    /// is settling at a meaningfully bigger club. Decays over the
    /// settlement window.
    pub fn step_up_dev_multiplier(
        &self,
        days_since_transfer: Option<i64>,
        club_rep_0_to_1: f32,
        player_world_reputation: f32,
    ) -> f32 {
        let days = match days_since_transfer {
            Some(d) if d >= 0 && d < self.settlement_window_days => d as f32,
            _ => return 1.0,
        };
        let club_rep = club_rep_0_to_1 * 10000.0;
        let gap = club_rep - player_world_reputation;
        if gap <= self.step_up_dev_min_gap {
            return 1.0;
        }
        let gap_factor =
            (gap / self.step_up_dev_gap_normaliser).clamp(0.0, self.step_up_dev_max_gap_factor);
        let recency = 1.0 - (days / self.settlement_window_days as f32);
        (1.0 + gap_factor * recency).clamp(1.0, self.step_up_dev_multiplier_ceiling)
    }
}

// ============================================================
// HappinessConfig — morale framework constants
// ============================================================

#[derive(Debug, Clone)]
pub struct HappinessConfig {
    /// Default morale on construction / clear.
    pub default_morale: f32,
    /// Morale ≥ this value counts as "happy" (not visibly unhappy).
    pub happy_threshold: f32,
    /// Hard clamp on morale.
    pub morale_min: f32,
    pub morale_max: f32,
    /// Days over which a recent event linearly decays to zero contribution.
    pub event_decay_halflife_days: f32,
    /// Per-tick `days_ago` increment for `decay_events` (default: weekly tick).
    pub decay_step_days: u16,
    /// Events older than this are dropped on decay.
    pub event_retention_days: u16,
    /// Maximum number of events kept in the recent_events buffer.
    pub recent_events_cap: usize,
    /// Default magnitudes per event source. The audit flagged 31 inline
    /// hardcoded magnitudes scattered across emit sites — this catalog
    /// pulls the canonical default for each into one place. Sites that
    /// scale by context (severity / damp / loan-vs-permanent) still call
    /// `add_event` directly, but can use `magnitude(source)` here as the
    /// base value. Single-magnitude sites use `add_event_default`.
    pub catalog: MoraleEventCatalog,
}

/// Canonical default morale magnitudes, one per `HappinessEventType`.
/// Field naming mirrors the enum variant in snake_case so callers can read
/// or override individual values via `cfg.catalog.player_of_the_match = 5.0`.
#[derive(Debug, Clone)]
pub struct MoraleEventCatalog {
    // Manager interactions
    pub manager_praise: f32,
    pub manager_discipline: f32,
    pub manager_playing_time_promise: f32,
    pub manager_criticism: f32,
    pub manager_encouragement: f32,
    pub manager_tactical_instruction: f32,
    // Training
    pub good_training: f32,
    pub poor_training: f32,
    // Match selection
    pub match_dropped: f32,
    // Contract & transfers
    pub contract_offer: f32,
    pub contract_renewal: f32,
    pub squad_status_change: f32,
    pub lack_of_playing_time: f32,
    pub loan_listing_accepted: f32,
    // Injury
    pub injury_return: f32,
    // Match performance
    pub player_of_the_match: f32,
    pub player_of_the_week: f32,
    // Squad relationships
    pub teammate_bonding: f32,
    pub conflict_with_teammate: f32,
    pub dressing_room_speech: f32,
    pub settled_into_squad: f32,
    pub feeling_isolated: f32,
    pub salary_gap_noticed: f32,
    // Promises
    pub promise_kept: f32,
    pub promise_broken: f32,
    // Transfer shocks
    pub ambition_shock: f32,
    pub salary_shock: f32,
    pub role_mismatch: f32,
    pub dream_move: f32,
    pub salary_boost: f32,
    pub joining_elite: f32,
    // Lifecycle
    pub contract_terminated: f32,
    pub manager_departure: f32,
    pub national_team_callup: f32,
    pub national_team_dropped: f32,
    pub shirt_number_promotion: f32,
    pub controversy_incident: f32,
    // Match performance
    pub first_club_goal: f32,
    pub decisive_goal: f32,
    pub substitute_impact: f32,
    pub clean_sheet_pride: f32,
    pub costly_mistake: f32,
    pub red_card_fallout: f32,
    pub derby_hero: f32,
    pub derby_win: f32,
    pub derby_defeat: f32,
    // Team season events
    pub trophy_won: f32,
    pub cup_final_defeat: f32,
    pub promotion_celebration: f32,
    pub relegation_fear: f32,
    pub relegated: f32,
    pub qualified_for_europe: f32,
    // Role / status
    pub won_starting_place: f32,
    pub lost_starting_place: f32,
    pub captaincy_awarded: f32,
    pub captaincy_removed: f32,
    pub youth_breakthrough: f32,
    pub squad_registration_omitted: f32,
    // Transfer / media
    pub wanted_by_bigger_club: f32,
    pub transfer_bid_rejected: f32,
    pub dream_move_collapsed: f32,
    pub fan_praise: f32,
    pub fan_criticism: f32,
    pub media_praise: f32,
    pub media_criticism: f32,
    // Social / culture
    pub close_friend_sold: f32,
    pub compatriot_joined: f32,
    pub mentor_departed: f32,
    pub language_progress: f32,
    // Awards / nominations
    pub player_of_the_month: f32,
    pub young_player_of_the_month: f32,
    pub team_of_the_week_selection: f32,
    pub team_of_the_season_selection: f32,
    pub player_of_the_season: f32,
    pub young_player_of_the_season: f32,
    pub league_top_scorer: f32,
    pub league_top_assists: f32,
    pub league_golden_glove: f32,
    pub continental_player_of_year_nomination: f32,
    pub continental_player_of_year: f32,
    pub world_player_of_year_nomination: f32,
    pub world_player_of_year: f32,
    // Real-life football events
    pub senior_debut: f32,
    pub national_team_debut: f32,
    pub hat_trick: f32,
    pub assist_hat_trick: f32,
    pub goal_drought_ended: f32,
    pub scoring_drought_concern: f32,
    pub appearance_milestone: f32,
    pub goal_milestone: f32,
    pub clean_sheet_milestone: f32,
    pub training_ground_bust_up: f32,
    pub public_apology: f32,
    pub fans_chant_player_name: f32,
    pub media_pressure_mounting: f32,
    pub leadership_emergence: f32,
}

impl Default for MoraleEventCatalog {
    fn default() -> Self {
        // Magnitudes match the canonical (most common) inline value at each
        // emit site at the time of extraction. Multi-magnitude sites that
        // scale by context (e.g. AmbitionShock = -8 * severity * damp) keep
        // these as their *base* — the scaling happens at the call site.
        MoraleEventCatalog {
            manager_praise: 3.0,
            manager_discipline: -3.0,
            manager_playing_time_promise: 4.0,
            manager_criticism: -2.0,
            manager_encouragement: 2.0,
            manager_tactical_instruction: 1.0,
            good_training: 2.0,
            poor_training: -2.0,
            match_dropped: -1.5,
            contract_offer: 2.0,
            contract_renewal: 5.0,
            squad_status_change: 2.0,
            lack_of_playing_time: -3.0,
            loan_listing_accepted: -2.0,
            injury_return: 3.0,
            player_of_the_match: 4.0,
            // Career-memory weekly award. Larger than POM because the
            // recipient outperformed every other player in the league for
            // a full week of fixtures, not just a single ninety minutes.
            player_of_the_week: 6.0,
            teammate_bonding: 1.5,
            conflict_with_teammate: -2.0,
            dressing_room_speech: 1.5,
            settled_into_squad: 1.0,
            feeling_isolated: -2.0,
            salary_gap_noticed: -3.0,
            promise_kept: 4.0,
            promise_broken: -6.0,
            ambition_shock: -8.0,
            salary_shock: -6.0,
            role_mismatch: -6.0,
            dream_move: 10.0,
            salary_boost: 4.0,
            joining_elite: 6.0,
            contract_terminated: -3.0,
            manager_departure: 0.0,
            national_team_callup: 6.0,
            national_team_dropped: -4.0,
            shirt_number_promotion: 2.0,
            controversy_incident: -3.0,
            // Match performance — small/medium routine events plus a
            // career milestone (first_club_goal). Magnitudes here are
            // *base* values; emit sites may scale by rating, derby
            // multiplier, role or starter-vs-sub.
            first_club_goal: 6.0,
            decisive_goal: 3.0,
            substitute_impact: 2.0,
            clean_sheet_pride: 1.5,
            costly_mistake: -3.5,
            red_card_fallout: -5.0,
            derby_hero: 5.0,
            // Squad-wide derby win — a moderate lift for everyone on the
            // winning side. Standout performers get the bigger DerbyHero
            // on top, but only one of the two events fires per player.
            derby_win: 2.5,
            // Base defeat is squad-wide; emit site adds up to -3.0 extra
            // for poor performers / red cards, so worst case lands around
            // -6 — a meaningful but not crushing rivalry blow.
            derby_defeat: -3.0,
            // Team season events — major positives are once-a-season
            // career memories; relegation is a year-defining wound.
            trophy_won: 10.0,
            cup_final_defeat: -5.0,
            promotion_celebration: 8.0,
            relegation_fear: -2.5,
            relegated: -10.0,
            qualified_for_europe: 6.0,
            // Role / status — captaincy is the strongest pure status
            // event short of a trophy; squad omission silently chronic.
            won_starting_place: 4.0,
            lost_starting_place: -4.0,
            captaincy_awarded: 7.0,
            captaincy_removed: -7.0,
            youth_breakthrough: 8.0,
            squad_registration_omitted: -5.0,
            // Transfer / media — fan/media events are softer than the
            // dressing-room layer; a collapsed dream move stings.
            wanted_by_bigger_club: 3.0,
            transfer_bid_rejected: -3.0,
            dream_move_collapsed: -7.0,
            fan_praise: 2.0,
            fan_criticism: -2.5,
            media_praise: 1.5,
            media_criticism: -2.0,
            // Social / culture — quiet ongoing events. Friend sold and
            // mentor departed are felt; compatriot/language are gentle
            // integration helpers.
            close_friend_sold: -3.0,
            compatriot_joined: 2.5,
            mentor_departed: -3.0,
            language_progress: 1.5,
            // Awards / nominations — career-visible silverware.
            player_of_the_month: 8.0,
            young_player_of_the_month: 7.0,
            team_of_the_week_selection: 3.0,
            team_of_the_season_selection: 9.0,
            player_of_the_season: 12.0,
            young_player_of_the_season: 10.0,
            league_top_scorer: 10.0,
            league_top_assists: 8.0,
            league_golden_glove: 8.0,
            continental_player_of_year_nomination: 7.0,
            continental_player_of_year: 14.0,
            world_player_of_year_nomination: 10.0,
            world_player_of_year: 18.0,
            // Real-life football events — milestones, hot streaks, slumps.
            senior_debut: 6.0,
            national_team_debut: 8.0,
            hat_trick: 7.0,
            assist_hat_trick: 6.0,
            goal_drought_ended: 3.5,
            scoring_drought_concern: -3.0,
            appearance_milestone: 5.0,
            goal_milestone: 5.0,
            clean_sheet_milestone: 5.0,
            training_ground_bust_up: -4.0,
            public_apology: 1.0,
            fans_chant_player_name: 3.0,
            media_pressure_mounting: -3.5,
            leadership_emergence: 4.0,
        }
    }
}

impl MoraleEventCatalog {
    /// Default magnitude for `source`. Lookup is a match arm — switching
    /// from a HashMap means O(1) without hashing and exhaustiveness checks
    /// flag missing variants if the enum grows.
    pub fn magnitude(&self, source: crate::HappinessEventType) -> f32 {
        use crate::HappinessEventType::*;
        match source {
            ManagerPraise => self.manager_praise,
            ManagerDiscipline => self.manager_discipline,
            ManagerPlayingTimePromise => self.manager_playing_time_promise,
            ManagerCriticism => self.manager_criticism,
            ManagerEncouragement => self.manager_encouragement,
            ManagerTacticalInstruction => self.manager_tactical_instruction,
            GoodTraining => self.good_training,
            PoorTraining => self.poor_training,
            MatchDropped => self.match_dropped,
            ContractOffer => self.contract_offer,
            ContractRenewal => self.contract_renewal,
            SquadStatusChange => self.squad_status_change,
            LackOfPlayingTime => self.lack_of_playing_time,
            LoanListingAccepted => self.loan_listing_accepted,
            InjuryReturn => self.injury_return,
            PlayerOfTheMatch => self.player_of_the_match,
            PlayerOfTheWeek => self.player_of_the_week,
            TeammateBonding => self.teammate_bonding,
            ConflictWithTeammate => self.conflict_with_teammate,
            DressingRoomSpeech => self.dressing_room_speech,
            SettledIntoSquad => self.settled_into_squad,
            FeelingIsolated => self.feeling_isolated,
            SalaryGapNoticed => self.salary_gap_noticed,
            PromiseKept => self.promise_kept,
            PromiseBroken => self.promise_broken,
            AmbitionShock => self.ambition_shock,
            SalaryShock => self.salary_shock,
            RoleMismatch => self.role_mismatch,
            DreamMove => self.dream_move,
            SalaryBoost => self.salary_boost,
            JoiningElite => self.joining_elite,
            ContractTerminated => self.contract_terminated,
            ManagerDeparture => self.manager_departure,
            NationalTeamCallup => self.national_team_callup,
            NationalTeamDropped => self.national_team_dropped,
            ShirtNumberPromotion => self.shirt_number_promotion,
            ControversyIncident => self.controversy_incident,
            FirstClubGoal => self.first_club_goal,
            DecisiveGoal => self.decisive_goal,
            SubstituteImpact => self.substitute_impact,
            CleanSheetPride => self.clean_sheet_pride,
            CostlyMistake => self.costly_mistake,
            RedCardFallout => self.red_card_fallout,
            DerbyHero => self.derby_hero,
            DerbyWin => self.derby_win,
            DerbyDefeat => self.derby_defeat,
            TrophyWon => self.trophy_won,
            CupFinalDefeat => self.cup_final_defeat,
            PromotionCelebration => self.promotion_celebration,
            RelegationFear => self.relegation_fear,
            Relegated => self.relegated,
            QualifiedForEurope => self.qualified_for_europe,
            WonStartingPlace => self.won_starting_place,
            LostStartingPlace => self.lost_starting_place,
            CaptaincyAwarded => self.captaincy_awarded,
            CaptaincyRemoved => self.captaincy_removed,
            YouthBreakthrough => self.youth_breakthrough,
            SquadRegistrationOmitted => self.squad_registration_omitted,
            WantedByBiggerClub => self.wanted_by_bigger_club,
            TransferBidRejected => self.transfer_bid_rejected,
            DreamMoveCollapsed => self.dream_move_collapsed,
            FanPraise => self.fan_praise,
            FanCriticism => self.fan_criticism,
            MediaPraise => self.media_praise,
            MediaCriticism => self.media_criticism,
            CloseFriendSold => self.close_friend_sold,
            CompatriotJoined => self.compatriot_joined,
            MentorDeparted => self.mentor_departed,
            LanguageProgress => self.language_progress,
            PlayerOfTheMonth => self.player_of_the_month,
            YoungPlayerOfTheMonth => self.young_player_of_the_month,
            TeamOfTheWeekSelection => self.team_of_the_week_selection,
            TeamOfTheSeasonSelection => self.team_of_the_season_selection,
            PlayerOfTheSeason => self.player_of_the_season,
            YoungPlayerOfTheSeason => self.young_player_of_the_season,
            LeagueTopScorer => self.league_top_scorer,
            LeagueTopAssists => self.league_top_assists,
            LeagueGoldenGlove => self.league_golden_glove,
            ContinentalPlayerOfYearNomination => self.continental_player_of_year_nomination,
            ContinentalPlayerOfYear => self.continental_player_of_year,
            WorldPlayerOfYearNomination => self.world_player_of_year_nomination,
            WorldPlayerOfYear => self.world_player_of_year,
            SeniorDebut => self.senior_debut,
            NationalTeamDebut => self.national_team_debut,
            HatTrick => self.hat_trick,
            AssistHatTrick => self.assist_hat_trick,
            GoalDroughtEnded => self.goal_drought_ended,
            ScoringDroughtConcern => self.scoring_drought_concern,
            AppearanceMilestone => self.appearance_milestone,
            GoalMilestone => self.goal_milestone,
            CleanSheetMilestone => self.clean_sheet_milestone,
            TrainingGroundBustUp => self.training_ground_bust_up,
            PublicApology => self.public_apology,
            FansChantPlayerName => self.fans_chant_player_name,
            MediaPressureMounting => self.media_pressure_mounting,
            LeadershipEmergence => self.leadership_emergence,
        }
    }
}

impl Default for HappinessConfig {
    fn default() -> Self {
        HappinessConfig {
            default_morale: 50.0,
            happy_threshold: 40.0,
            morale_min: 0.0,
            morale_max: 100.0,
            event_decay_halflife_days: 60.0,
            decay_step_days: 7,
            event_retention_days: 365,
            recent_events_cap: 100,
            catalog: MoraleEventCatalog::default(),
        }
    }
}

impl HappinessConfig {
    /// Decay multiplier for an event recorded `days_ago` ago. Linear, not
    /// exponential — mirrors the original implementation. Returns 0 once
    /// `days_ago >= event_decay_halflife_days`.
    pub fn event_decay(&self, days_ago: u16) -> f32 {
        1.0 - (days_ago as f32 / self.event_decay_halflife_days).min(1.0)
    }

    /// Clamp a candidate morale value to the configured range.
    pub fn clamp_morale(&self, value: f32) -> f32 {
        value.clamp(self.morale_min, self.morale_max)
    }
}

// ============================================================
// PassEvaluatorConfig — match-AI pass evaluator tunables
// ============================================================

#[derive(Debug, Clone)]
pub struct PassEvaluatorConfig {
    // ── Range bands ────────────────────────────────────────────
    /// Skill-to-bonus normaliser. Inputs are 0–20 skills; outputs are
    /// dimensionless multipliers used in range/bonus math.
    pub skill_scale: f32,
    /// `(vision / skill_scale) * vision_bonus_multiplier` extends the
    /// pass-range bands. Higher → vision matters more for long passes.
    pub vision_bonus_multiplier: f32,
    pub technique_bonus_multiplier: f32,
    /// Optimal pass range = `passing_skill * (optimal_range_multiplier + vision_bonus)`.
    pub optimal_range_multiplier: f32,
    /// Max effective range = `passing_skill * (max_effective_range_multiplier + vision_bonus * 2.0)`.
    pub max_effective_range_multiplier: f32,
    /// Distance threshold above which only elite passers can connect.
    pub ultra_long_threshold: f32,
    /// Distance threshold above which the pass becomes a hopeful clearance.
    pub extreme_long_threshold: f32,

    // ── Recommendation gates ───────────────────────────────────
    /// Risk-tolerant players (Playmaker / KillerBallOften / TriesThroughBalls)
    /// will attempt a pass at this success / risk pair.
    pub risk_tolerant_min_success: f32,
    pub risk_tolerant_max_risk: f32,
    /// Default gate for players without risk-tolerant traits.
    pub default_min_success: f32,
    pub default_max_risk: f32,

    // ── Pressure / angle ───────────────────────────────────────
    /// Radius in pitch-units within which an opponent counts as pressuring.
    pub pressure_radius: f32,
    /// Velocity below which the passer is treated as standing still.
    pub stationary_velocity_threshold: f32,
    pub stationary_angle_factor: f32,

    /// Dot-product breakpoints for the angle-factor lookup. Forward (>= forward),
    /// diagonal (>= diagonal), sideways (>= sideways), backward (else).
    pub angle_forward_dot: f32,
    pub angle_diagonal_dot: f32,
    pub angle_sideways_dot: f32,
}

impl Default for PassEvaluatorConfig {
    fn default() -> Self {
        PassEvaluatorConfig {
            skill_scale: 20.0,
            vision_bonus_multiplier: 1.5,
            technique_bonus_multiplier: 0.5,
            optimal_range_multiplier: 2.5,
            max_effective_range_multiplier: 5.0,
            ultra_long_threshold: 200.0,
            extreme_long_threshold: 300.0,

            risk_tolerant_min_success: 0.5,
            risk_tolerant_max_risk: 0.82,
            default_min_success: 0.6,
            default_max_risk: 0.7,

            pressure_radius: 15.0,
            stationary_velocity_threshold: 0.1,
            stationary_angle_factor: 0.95,

            angle_forward_dot: 0.7,
            angle_diagonal_dot: 0.0,
            angle_sideways_dot: -0.5,
        }
    }
}

impl PassEvaluatorConfig {
    /// `(vision / skill_scale) * vision_bonus_multiplier` — the dimensionless
    /// bonus applied to optimal/max range thresholds.
    pub fn vision_bonus(&self, vision_skill: f32) -> f32 {
        (vision_skill / self.skill_scale) * self.vision_bonus_multiplier
    }

    /// Optimal pass range — distance below which short/medium passes
    /// connect with very high success.
    pub fn optimal_range(&self, passing_skill: f32, vision_skill: f32) -> f32 {
        let bonus = self.vision_bonus(vision_skill);
        passing_skill * (self.optimal_range_multiplier + bonus)
    }

    /// Max effective range — beyond this, only ultra-long-pass logic applies.
    pub fn max_effective_range(&self, passing_skill: f32, vision_skill: f32) -> f32 {
        let bonus = self.vision_bonus(vision_skill);
        passing_skill * (self.max_effective_range_multiplier + bonus * 2.0)
    }

    /// Whether a pass attempt should be recommended given success / risk
    /// and whether the passer has a risk-tolerant trait.
    pub fn is_recommended(
        &self,
        success_probability: f32,
        risk_level: f32,
        risk_tolerant: bool,
    ) -> bool {
        if risk_tolerant {
            success_probability > self.risk_tolerant_min_success
                && risk_level < self.risk_tolerant_max_risk
        } else {
            success_probability > self.default_min_success && risk_level < self.default_max_risk
        }
    }

    /// Angle factor lookup based on the dot product between the passer's
    /// facing vector and the pass direction. The original code interpolated
    /// inside each band; we preserve that here so the helper is a drop-in
    /// replacement for the inline match.
    pub fn angle_factor_from_dot(&self, dot_product: f32) -> f32 {
        if dot_product > self.angle_forward_dot {
            1.0
        } else if dot_product > self.angle_diagonal_dot {
            0.8 + (dot_product * 0.2)
        } else if dot_product > self.angle_sideways_dot {
            0.6 + ((dot_product + 0.5) * 0.4)
        } else {
            0.5 + ((dot_product + 1.0) * 0.2)
        }
    }
}

// ============================================================
// Top-level container
// ============================================================

#[derive(Debug, Clone, Default)]
pub struct PlayerBehaviourConfig {
    pub adaptation: AdaptationConfig,
    pub happiness: HappinessConfig,
    pub passing: PassEvaluatorConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Adaptation ───────────────────────────────────────────────

    #[test]
    fn settlement_multiplier_recovers_to_one_at_end_of_window() {
        let c = AdaptationConfig::default();
        let day0 = c.settlement_multiplier(Some(0), false, 10.0, false);
        let day_end = c.settlement_multiplier(Some(c.settlement_window_days), false, 10.0, false);
        assert!(day0 < 1.0);
        assert_eq!(day_end, 1.0);
    }

    #[test]
    fn settlement_multiplier_floor_respected() {
        let c = AdaptationConfig::default();
        // Worst case: day 0, no language, low adaptability, no step-up
        let m = c.settlement_multiplier(Some(0), false, 0.0, false);
        assert!(m >= c.settlement_multiplier_floor);
    }

    #[test]
    fn language_softens_settlement() {
        let c = AdaptationConfig::default();
        let foreign = c.settlement_multiplier(Some(10), false, 10.0, false);
        let native = c.settlement_multiplier(Some(10), true, 10.0, false);
        assert!(native > foreign);
    }

    #[test]
    fn adaptability_softens_settlement() {
        let c = AdaptationConfig::default();
        let low = c.settlement_multiplier(Some(10), false, 0.0, false);
        let high = c.settlement_multiplier(Some(10), false, 20.0, false);
        assert!(high > low);
    }

    #[test]
    fn step_up_softens_settlement() {
        let c = AdaptationConfig::default();
        let lateral = c.settlement_multiplier(Some(10), false, 10.0, false);
        let step_up = c.settlement_multiplier(Some(10), false, 10.0, true);
        assert!(step_up > lateral);
    }

    #[test]
    fn outside_window_returns_one() {
        let c = AdaptationConfig::default();
        assert_eq!(c.settlement_multiplier(None, false, 10.0, false), 1.0);
        let beyond =
            c.settlement_multiplier(Some(c.settlement_window_days + 1), false, 10.0, false);
        assert_eq!(beyond, 1.0);
    }

    #[test]
    fn step_up_predicate() {
        let c = AdaptationConfig::default();
        // Low-ambition player joining elite club → step up
        assert!(c.is_step_up_move(8.0, 0.95));
        // High-ambition player joining lower-mid club → not a step up
        assert!(!c.is_step_up_move(18.0, 0.30));
    }

    #[test]
    fn promise_horizon_branches() {
        let c = AdaptationConfig::default();
        assert_eq!(
            c.promise_horizon_days(true, 0.0),
            c.loan_promise_horizon_days
        );
        assert_eq!(
            c.promise_horizon_days(false, 10_000_000.0),
            c.permanent_promise_horizon_days
        );
        assert_eq!(c.promise_horizon_days(false, 100_000.0), 0);
    }

    #[test]
    fn step_up_dev_multiplier_no_op_for_lateral() {
        let c = AdaptationConfig::default();
        // Player rep equals club rep → no bonus
        let m = c.step_up_dev_multiplier(Some(10), 0.5, 5_000.0);
        assert_eq!(m, 1.0);
    }

    #[test]
    fn step_up_dev_multiplier_caps_at_ceiling() {
        let c = AdaptationConfig::default();
        // Massive gap, day 0 → maximum bonus, capped
        let m = c.step_up_dev_multiplier(Some(0), 1.0, 0.0);
        assert!(m <= c.step_up_dev_multiplier_ceiling + f32::EPSILON);
    }

    // ── Happiness ────────────────────────────────────────────────

    #[test]
    fn event_decay_falls_to_zero_at_halflife() {
        let c = HappinessConfig::default();
        assert_eq!(c.event_decay(0), 1.0);
        assert!(c.event_decay(c.event_decay_halflife_days as u16) <= 0.0 + f32::EPSILON);
    }

    #[test]
    fn morale_clamp_respected() {
        let c = HappinessConfig::default();
        assert_eq!(c.clamp_morale(-50.0), c.morale_min);
        assert_eq!(c.clamp_morale(200.0), c.morale_max);
        assert_eq!(c.clamp_morale(50.0), 50.0);
    }

    // ── Morale catalog ───────────────────────────────────────────

    #[test]
    fn catalog_returns_canonical_magnitude() {
        let cat = MoraleEventCatalog::default();
        // Sanity checks against the documented defaults — protects against
        // accidental shifts in the catalog that would change game balance.
        assert_eq!(
            cat.magnitude(crate::HappinessEventType::PlayerOfTheMatch),
            4.0
        );
        assert_eq!(cat.magnitude(crate::HappinessEventType::MatchDropped), -1.5);
        assert_eq!(
            cat.magnitude(crate::HappinessEventType::ContractRenewal),
            5.0
        );
        assert_eq!(
            cat.magnitude(crate::HappinessEventType::ContractTerminated),
            -3.0
        );
        assert_eq!(cat.magnitude(crate::HappinessEventType::PromiseKept), 4.0);
        assert_eq!(
            cat.magnitude(crate::HappinessEventType::PromiseBroken),
            -6.0
        );
    }

    #[test]
    fn catalog_polarity_matches_intent() {
        // Negative-by-design events must land negative; positive ones positive.
        let cat = MoraleEventCatalog::default();
        let negatives = [
            crate::HappinessEventType::ManagerDiscipline,
            crate::HappinessEventType::ManagerCriticism,
            crate::HappinessEventType::PoorTraining,
            crate::HappinessEventType::MatchDropped,
            crate::HappinessEventType::LackOfPlayingTime,
            crate::HappinessEventType::LoanListingAccepted,
            crate::HappinessEventType::ConflictWithTeammate,
            crate::HappinessEventType::FeelingIsolated,
            crate::HappinessEventType::SalaryGapNoticed,
            crate::HappinessEventType::PromiseBroken,
            crate::HappinessEventType::AmbitionShock,
            crate::HappinessEventType::SalaryShock,
            crate::HappinessEventType::RoleMismatch,
            crate::HappinessEventType::ContractTerminated,
            crate::HappinessEventType::NationalTeamDropped,
            crate::HappinessEventType::ControversyIncident,
            crate::HappinessEventType::CostlyMistake,
            crate::HappinessEventType::RedCardFallout,
            crate::HappinessEventType::DerbyDefeat,
            crate::HappinessEventType::CupFinalDefeat,
            crate::HappinessEventType::RelegationFear,
            crate::HappinessEventType::Relegated,
            crate::HappinessEventType::LostStartingPlace,
            crate::HappinessEventType::CaptaincyRemoved,
            crate::HappinessEventType::SquadRegistrationOmitted,
            crate::HappinessEventType::TransferBidRejected,
            crate::HappinessEventType::DreamMoveCollapsed,
            crate::HappinessEventType::FanCriticism,
            crate::HappinessEventType::MediaCriticism,
            crate::HappinessEventType::CloseFriendSold,
            crate::HappinessEventType::MentorDeparted,
        ];
        for n in negatives {
            assert!(cat.magnitude(n.clone()) < 0.0, "expected {:?} negative", n);
        }
        let positives = [
            crate::HappinessEventType::ManagerPraise,
            crate::HappinessEventType::ManagerEncouragement,
            crate::HappinessEventType::GoodTraining,
            crate::HappinessEventType::PlayerOfTheMatch,
            crate::HappinessEventType::DreamMove,
            crate::HappinessEventType::SalaryBoost,
            crate::HappinessEventType::JoiningElite,
            crate::HappinessEventType::PromiseKept,
            crate::HappinessEventType::NationalTeamCallup,
            crate::HappinessEventType::ContractRenewal,
            crate::HappinessEventType::TeammateBonding,
            crate::HappinessEventType::FirstClubGoal,
            crate::HappinessEventType::DecisiveGoal,
            crate::HappinessEventType::SubstituteImpact,
            crate::HappinessEventType::CleanSheetPride,
            crate::HappinessEventType::DerbyHero,
            crate::HappinessEventType::TrophyWon,
            crate::HappinessEventType::PromotionCelebration,
            crate::HappinessEventType::QualifiedForEurope,
            crate::HappinessEventType::WonStartingPlace,
            crate::HappinessEventType::CaptaincyAwarded,
            crate::HappinessEventType::YouthBreakthrough,
            crate::HappinessEventType::WantedByBiggerClub,
            crate::HappinessEventType::FanPraise,
            crate::HappinessEventType::MediaPraise,
            crate::HappinessEventType::CompatriotJoined,
            crate::HappinessEventType::LanguageProgress,
        ];
        for p in positives {
            assert!(cat.magnitude(p.clone()) > 0.0, "expected {:?} positive", p);
        }
    }

    #[test]
    fn catalog_career_milestones_are_meaningful() {
        // Career-defining events should land in the major-event band
        // (>= |5|). Small routine events (FanPraise, LanguageProgress)
        // and ambient pressure (RelegationFear) should stay below |3|.
        let cat = MoraleEventCatalog::default();
        let career = [
            (crate::HappinessEventType::TrophyWon, 5.0),
            (crate::HappinessEventType::Relegated, 5.0),
            (crate::HappinessEventType::CaptaincyAwarded, 5.0),
            (crate::HappinessEventType::CaptaincyRemoved, 5.0),
            (crate::HappinessEventType::PromotionCelebration, 5.0),
            (crate::HappinessEventType::DreamMoveCollapsed, 5.0),
            (crate::HappinessEventType::YouthBreakthrough, 5.0),
        ];
        for (event, min_abs) in career {
            let m = cat.magnitude(event.clone()).abs();
            assert!(
                m >= min_abs,
                "expected {:?} magnitude >= {} (got {})",
                event,
                min_abs,
                m
            );
        }
        let ambient = [
            crate::HappinessEventType::RelegationFear,
            crate::HappinessEventType::FanPraise,
            crate::HappinessEventType::LanguageProgress,
        ];
        for event in ambient {
            let m = cat.magnitude(event.clone()).abs();
            assert!(
                m < 3.0,
                "expected {:?} ambient magnitude < 3.0 (got {})",
                event,
                m
            );
        }
    }

    // ── Passing ──────────────────────────────────────────────────

    #[test]
    fn vision_bonus_zero_at_zero_skill() {
        let c = PassEvaluatorConfig::default();
        assert_eq!(c.vision_bonus(0.0), 0.0);
    }

    #[test]
    fn optimal_range_grows_with_passing_and_vision() {
        let c = PassEvaluatorConfig::default();
        let low = c.optimal_range(5.0, 5.0);
        let high = c.optimal_range(15.0, 15.0);
        assert!(high > low);
    }

    #[test]
    fn max_range_exceeds_optimal() {
        let c = PassEvaluatorConfig::default();
        let opt = c.optimal_range(15.0, 12.0);
        let max = c.max_effective_range(15.0, 12.0);
        assert!(max > opt);
    }

    #[test]
    fn risk_tolerant_accepts_lower_success() {
        let c = PassEvaluatorConfig::default();
        // Borderline pass: 0.55 success, 0.75 risk
        assert!(c.is_recommended(0.55, 0.75, true));
        assert!(!c.is_recommended(0.55, 0.75, false));
    }

    #[test]
    fn neither_recommends_a_terrible_pass() {
        let c = PassEvaluatorConfig::default();
        assert!(!c.is_recommended(0.3, 0.9, true));
        assert!(!c.is_recommended(0.3, 0.9, false));
    }

    #[test]
    fn angle_factor_is_one_when_facing_target() {
        let c = PassEvaluatorConfig::default();
        // Dot product 1.0 → directly facing → factor 1.0
        assert_eq!(c.angle_factor_from_dot(1.0), 1.0);
    }

    #[test]
    fn angle_factor_decreases_for_backward_pass() {
        let c = PassEvaluatorConfig::default();
        let forward = c.angle_factor_from_dot(0.9);
        let backward = c.angle_factor_from_dot(-0.8);
        assert!(forward > backward);
    }
}
