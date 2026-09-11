//! The board's estate: the ground and the training pitches it funds, the
//! ownership change it lives through, and the officers it appoints to its
//! own table.

use crate::club::board::ClubBoard;
use crate::club::board::chairman::{ChairmanAmbition, ChairmanPatience};
use crate::club::board::context::BoardContext;
use crate::club::board::decision::{BoardDecision, DecisionReason};
use crate::club::board::infrastructure::FacilityReview;
use crate::club::board::ownership::OwnershipType;
use crate::club::board::roll::DeterministicRoll;
use crate::club::board::strategy::{InfrastructurePriority, ManagerAutonomy, SquadProfile};
use crate::club::board::takeover::TakeoverEngine;
use crate::club::board::vision::{
    ClubVision, FinancialStance, LongTermGoal, SigningPreference, VisionPlayingStyle,
    VisionYouthFocus,
};
use crate::club::{BoardResult, StaffClubContract};
use crate::context::SimulationContext;
use chrono::{Datelike, NaiveDate};

impl ClubBoard {
    /// Season-start facility review with a per-board cooldown. A funded
    /// upgrade can only be approved once every `FacilityReview::COOLDOWN_SEASONS`
    /// seasons — without this a deep-pocketed owner would rubber-stamp an
    /// upgrade every single year, which no real board does. Records the
    /// approval year so the next season's review is suppressed; rejections /
    /// news-only outcomes don't start the cooldown.
    pub(crate) fn run_facility_review(
        &mut self,
        board_ctx: &BoardContext,
        current_year: i32,
    ) -> Vec<BoardDecision> {
        let cooldown_active = self
            .last_facility_upgrade_year
            .map(|y| current_year - y < FacilityReview::COOLDOWN_SEASONS)
            .unwrap_or(false);
        if cooldown_active {
            return Vec::new();
        }

        let decisions = FacilityReview::run(board_ctx, &self.vision, &self.ownership);
        if decisions
            .iter()
            .any(|d| matches!(d, BoardDecision::ApproveFacilityUpgrade { .. }))
        {
            self.last_facility_upgrade_year = Some(current_year);
        }
        decisions
    }

    /// Monthly takeover watch. Opens / resolves rumours and, on completion,
    /// installs a new owner and resets strategy + relationship.
    ///
    /// The takeover roll is *deterministic*: `GlobalContext` carries no
    /// seeded simulation RNG, so rather than draw from the global
    /// (unreplayable) `IntegerUtils::random`, we derive a stable 0..99 roll
    /// from the club id, the current date, and the months spent in the
    /// current takeover status. Identical club/date/state therefore always
    /// produces the identical decision, so saves and tests replay exactly.
    pub(crate) fn tick_takeover(
        &mut self,
        ctx: &BoardContext,
        today: NaiveDate,
        result: &mut BoardResult,
    ) {
        let roll = DeterministicRoll::percent(
            result.club_id,
            today,
            DeterministicRoll::TAKEOVER + self.takeover.months_in_status as u64,
        );
        if let Some(decision) = self.takeover.tick(&self.ownership, ctx, roll) {
            match decision {
                BoardDecision::StartTakeoverRumour => {
                    result.decisions.push(BoardDecision::StartTakeoverRumour);
                }
                BoardDecision::CompleteTakeover => {
                    self.apply_takeover_completion(ctx, today, result.club_id ^ 0x9E37_79B9);
                    result.decisions.push(BoardDecision::CompleteTakeover);
                }
                _ => {}
            }
        }

        // A collapsed takeover leaves instability: morale dip + a short
        // budget freeze. Emit the freeze as an explicit decision (the
        // legacy mood-percentage path in `process` no longer fires).
        if self.takeover.just_failed {
            result.takeover_collapsed = true;
            self.confidence.level = (self.confidence.level - 8).clamp(0, 100);
            let freeze = self
                .season_targets
                .as_ref()
                .map(|t| (t.transfer_budget.max(0) as i64 / 5).max(0))
                .unwrap_or(0);
            if freeze > 0 {
                result.decisions.push(BoardDecision::CutTransferBudget {
                    amount: freeze,
                    reason: DecisionReason::FinancialDiscipline,
                });
            }
            result.cut_transfer_budget = true; // UI flag only
        }
    }

    /// Install a new owner after a successful takeover and reset the club's
    /// strategy + manager relationship to match the fresh mandate.
    ///
    /// The new owner's archetype dictates the strategy rather than a blanket
    /// "buy stars, win the league": a sovereign buyer chases trophies, a
    /// private-equity buyer chases resale and wage discipline, and a
    /// consortium builds a balanced prime-age side aiming for the top half /
    /// continental places.
    pub(crate) fn apply_takeover_completion(
        &mut self,
        ctx: &BoardContext,
        today: NaiveDate,
        seed: u32,
    ) {
        let owner = TakeoverEngine::post_takeover_owner(seed);
        match owner.ownership_type {
            OwnershipType::StateBacked => {
                // Sovereign wealth: trophies now, money no object.
                self.chairman.ambition = ChairmanAmbition::Reckless;
                self.chairman.patience = ChairmanPatience::Low;
                self.vision.preferred_squad_profile = SquadProfile::Stars;
                self.vision.financial_stance = FinancialStance::Ambitious;
                self.vision.long_term_goal = Some(LongTermGoal::WinLeague);
                self.vision.infrastructure_priority = InfrastructurePriority::Stadium;
                self.vision.manager_autonomy = ManagerAutonomy::Low;
            }
            OwnershipType::PrivateEquity => {
                // Leveraged buyer: trade players for profit, control wages,
                // monetise the brand. Ambitious but financially disciplined.
                self.chairman.ambition = ChairmanAmbition::Ambitious;
                self.chairman.patience = ChairmanPatience::Low;
                self.vision.preferred_squad_profile = SquadProfile::ResaleValue;
                self.vision.financial_stance = FinancialStance::Conservative;
                self.vision.long_term_goal = Some(LongTermGoal::EstablishTopHalf);
                self.vision.infrastructure_priority = InfrastructurePriority::Commercial;
                self.vision.manager_autonomy = ManagerAutonomy::Medium;
            }
            // Consortium (and any future archetype): patient, balanced build
            // around prime-age players, aiming high but living within means.
            _ => {
                self.chairman.ambition = ChairmanAmbition::Ambitious;
                self.chairman.patience = ChairmanPatience::Medium;
                self.vision.preferred_squad_profile = SquadProfile::PrimeAge;
                self.vision.financial_stance = FinancialStance::Balanced;
                self.vision.long_term_goal = Some(LongTermGoal::WinContinental);
                self.vision.infrastructure_priority = InfrastructurePriority::Training;
                self.vision.manager_autonomy = ManagerAutonomy::Medium;
            }
        }
        // The rest of the brief the new owner writes. The goal above is his
        // own statement of intent and stands; the horizon, the football and
        // the recruitment identity are filled in so the man he appoints has
        // something to be judged against — a takeover that set a goal and
        // left the horizon at zero could never reckon with anybody.
        let style = VisionPlayingStyle::for_owner(
            owner.ownership_type,
            DeterministicRoll::percent(seed, today, DeterministicRoll::VISION_STYLE),
        );
        self.vision.playing_style = style;
        self.vision.youth_focus = VisionYouthFocus::derive(
            owner.ownership_type,
            self.vision.preferred_squad_profile,
            ctx.reputation_score,
        );
        self.vision.signing_preference =
            SigningPreference::derive(owner.ownership_type, ctx.reputation_score);
        let (_, base_horizon) =
            LongTermGoal::derive(ctx.league_tier, ctx.reputation_score, owner.ownership_type);
        self.vision.long_term_horizon_seasons =
            ClubVision::horizon_for(base_horizon, self.chairman.patience);
        self.vision_start_year = None;
        self.vision_goal_achieved = false;

        self.ownership = owner;
        self.relationship.reset();
        self.confidence.level = 60;
    }

    pub(crate) fn is_director_contract_expiring(&self, simulation_ctx: &SimulationContext) -> bool {
        match &self.director {
            Some(d) => d.is_expired(simulation_ctx),
            None => false,
        }
    }

    /// Stand up a fresh director contract — four-year term, salary
    /// indexed to board ambition. This is the board's own administrative
    /// slot, separate from the team's DoF staff member.
    pub(crate) fn run_director_election(&mut self, ctx: &SimulationContext) {
        use crate::{StaffPosition, StaffStatus};
        let base_salary: u32 = match self.chairman.ambition {
            ChairmanAmbition::Reckless | ChairmanAmbition::Ambitious => 200_000,
            ChairmanAmbition::Balanced => 120_000,
            ChairmanAmbition::Conservative => 80_000,
        };
        let expires = ctx
            .date
            .date()
            .with_year(ctx.date.date().year() + 4)
            .unwrap_or(ctx.date.date());
        self.director = Some(StaffClubContract::new(
            base_salary,
            expires,
            StaffPosition::Director,
            StaffStatus::Active,
        ));
    }

    pub(crate) fn is_sport_director_contract_expiring(
        &self,
        simulation_ctx: &SimulationContext,
    ) -> bool {
        match &self.sport_director {
            Some(d) => d.is_expired(simulation_ctx),
            None => false,
        }
    }

    /// Stand up a sport director contract — three-year term; this is a
    /// more "football-side" role so salary floor is slightly higher.
    pub(crate) fn run_sport_director_election(&mut self, ctx: &SimulationContext) {
        use crate::{StaffPosition, StaffStatus};
        let base_salary: u32 = match self.chairman.ambition {
            ChairmanAmbition::Reckless | ChairmanAmbition::Ambitious => 250_000,
            ChairmanAmbition::Balanced => 150_000,
            ChairmanAmbition::Conservative => 100_000,
        };
        let expires = ctx
            .date
            .date()
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
