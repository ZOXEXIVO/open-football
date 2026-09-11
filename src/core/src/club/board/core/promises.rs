//! What the board pledged the manager, and the reckoning when a long-term
//! horizon runs out.

use crate::club::board::ClubBoard;
use crate::club::board::context::BoardContext;
use crate::club::board::core::SaleMandate;
use crate::club::board::decision::{BoardDecision, DecisionReason};
use crate::club::board::ownership::OwnershipType;
use crate::club::board::promise::{BoardPromise, PromiseType};
use crate::club::board::sale::{ForcedSale, ForcedSaleTarget};
use crate::club::board::scoring::SeasonPhase;
use crate::club::board::vision::{LongTermGoal, VisionYouthFocus};
use crate::club::{BoardManagerMeeting, BoardResult};
use chrono::{Duration, NaiveDate};
use log::debug;

impl ClubBoard {
    /// Check whether the long-term horizon has elapsed and reckon with the
    /// manager against the original vision goal. Fires at the START of a
    /// season — the previous season's trophies are already banked in
    /// `vision_goal_achieved`. Horizonless visions (no `long_term_goal`)
    /// don't trigger any judgment.
    pub(crate) fn evaluate_long_term_vision(
        &mut self,
        current_year: i32,
        result: &mut BoardResult,
    ) {
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
            // A missed horizon is a grievance, not an execution.
            //
            // It used to sack outright, bypassing the whole ladder — no
            // warning, no ultimatum, no chance for a squad or a supporter to
            // see it coming, and no regard for whether the man had otherwise
            // been doing well. A board that set out to win the league and
            // came third twice is disappointed; it is not necessarily
            // finished with him. So the miss costs confidence, trust and the
            // chairman's patience, and only tips into dismissal if what is
            // left is already untenable.
            debug!(
                "Long-term vision missed: goal {:?} not met in {} seasons",
                self.vision.long_term_goal, self.vision.long_term_horizon_seasons
            );
            self.confidence.level =
                (self.confidence.level - Self::VISION_MISS_CONFIDENCE_PENALTY).clamp(0, 100);
            self.relationship
                .adjust_results(-Self::VISION_MISS_TRUST_PENALTY);
            self.chairman.manager_loyalty = self
                .chairman
                .manager_loyalty
                .saturating_sub(Self::VISION_MISS_LOYALTY_PENALTY);
            result.decisions.push(BoardDecision::IssueFormalWarning);

            if self.confidence.level < Self::VISION_MISS_SACK_BELOW {
                result.manager_sacked = true;
                // …and the decision the old path forgot to emit, so the news
                // desk and the audit see the same sacking the result does.
                result.decisions.push(BoardDecision::SackManager);
                self.poor_mood_months = 0;
                self.manager_on_final_warning = false;
                self.relationship.reset();
            }
        } else {
            // Horizon met. A confidence bump, a loyalty bump, and a bigger
            // ask next time — a board that got what it wanted does not want
            // the same thing again.
            self.confidence.level = (self.confidence.level + 10).clamp(0, 100);
            self.chairman.manager_loyalty = self
                .chairman
                .manager_loyalty
                .saturating_add(Self::VISION_HIT_LOYALTY_BONUS)
                .min(100);
            if let Some(goal) = self.vision.long_term_goal {
                self.vision.long_term_goal = Some(goal.next_rung());
            }
        }

        self.vision_start_year = Some(current_year);
        self.vision_goal_achieved = false;
    }

    /// Record a sale the board has just demanded, so somebody is held to
    /// it. At most one runs at a time — a board that wants two players
    /// gone says so once the first has moved.
    pub fn open_sale_mandate(
        &mut self,
        target: ForcedSaleTarget,
        fees_received_now: f64,
        today: NaiveDate,
    ) {
        if self.promises.has_active(PromiseType::SaleMandate) {
            return;
        }
        self.sale_mandate = Some(SaleMandate {
            player_id: target.player_id,
            asking_price: target.asking_price,
            fees_received_at_issue: fees_received_now,
        });
        self.promises.add(BoardPromise::new(
            PromiseType::SaleMandate,
            today,
            today + Duration::days(ForcedSale::MANDATE_DAYS),
        ));
    }

    /// Open this season's board promises from the long-term goal / season
    /// targets, the youth brief, and any capex the board just deferred on
    /// affordability. Idempotent per type via `has_active`, so a promise is
    /// never duplicated inside its window (keeps the ledger bounded).
    pub(crate) fn open_season_promises(
        &mut self,
        ctx: &BoardContext,
        today: NaiveDate,
        facility_decisions: &[BoardDecision],
    ) {
        let season_due = today + Duration::days(330);

        // Headline season-outcome promise (survival / continental / title).
        if let Some(kind) = self.season_outcome_promise(ctx) {
            if !self.promises.has_active(kind) {
                self.promises
                    .add(BoardPromise::new(kind, today, season_due));
            }
        }

        // Youth-minutes commitment for development-minded or member-owned
        // boards — they pledge a genuine pathway for academy players.
        let youth_minded = matches!(self.vision.youth_focus, VisionYouthFocus::DevelopYouth)
            || matches!(self.ownership.ownership_type, OwnershipType::MemberOwned);
        if youth_minded && !self.promises.has_active(PromiseType::YouthMinutes) {
            self.promises.add(BoardPromise::new(
                PromiseType::YouthMinutes,
                today,
                season_due,
            ));
        }

        // A requested upgrade the board declined *purely* on affordability
        // becomes a "we'll revisit next season" facility promise. Due a
        // little past the next season start so next year's review has a
        // chance to deliver it before it's judged overdue.
        let deferred_capex = facility_decisions.iter().any(|d| {
            matches!(
                d,
                BoardDecision::RejectFacilityUpgrade {
                    reason: DecisionReason::DebtTooHigh,
                    ..
                }
            )
        });
        if deferred_capex && !self.promises.has_active(PromiseType::FacilityImprovement) {
            let revisit_due = today + Duration::days(400);
            self.promises.add(BoardPromise::new(
                PromiseType::FacilityImprovement,
                today,
                revisit_due,
            ));
        }
    }

    /// The headline season-outcome promise type, derived from the explicit
    /// long-term goal where set, otherwise inferred from where the board
    /// expects to finish. A comfortable mid-table brief makes no headline
    /// promise (returns `None`).
    pub(crate) fn season_outcome_promise(&self, ctx: &BoardContext) -> Option<PromiseType> {
        if let Some(goal) = self.vision.long_term_goal {
            match goal {
                LongTermGoal::WinLeague | LongTermGoal::PromotionToTopFlight => {
                    return Some(PromiseType::TitleChallenge);
                }
                LongTermGoal::WinContinental | LongTermGoal::EstablishTopHalf => {
                    return Some(PromiseType::ContinentalQualification);
                }
                LongTermGoal::Survive => return Some(PromiseType::Survival),
                // A domestic-cup goal isn't a league-table promise.
                LongTermGoal::WinDomesticCup => {}
            }
        }

        let targets = self.season_targets.as_ref()?;
        if ctx.league_size == 0 {
            return None;
        }
        let frac = targets.expected_position as f32 / ctx.league_size as f32; // 0 = top
        if frac <= 0.15 {
            Some(PromiseType::TitleChallenge)
        } else if frac <= 0.35 {
            Some(PromiseType::ContinentalQualification)
        } else if frac >= 0.80 {
            Some(PromiseType::Survival)
        } else {
            None
        }
    }

    /// Resolve outstanding promises against this tick's decisions and league
    /// standing, then reward the manager relationship for any kept. Overdue
    /// breakage is handled separately at season start (`break_overdue`).
    pub(crate) fn resolve_promises(
        &mut self,
        ctx: &BoardContext,
        today: NaiveDate,
        result: &mut BoardResult,
    ) {
        let phase = SeasonPhase::classify(ctx.matches_played, ctx.total_matches);
        let mut reward = 0i32;
        let mut kept = 0u8;

        // Decision-driven fulfilment: the board delivered what it pledged.
        let delivered_funds = result
            .decisions
            .iter()
            .any(|d| matches!(d, BoardDecision::IncreaseTransferBudget { .. }));
        if delivered_funds {
            if let Some(r) = self.promises.fulfil(PromiseType::TransferBudget) {
                reward += r as i32;
                kept += 1;
            }
        }
        let upgraded_facility = result
            .decisions
            .iter()
            .any(|d| matches!(d, BoardDecision::ApproveFacilityUpgrade { .. }));
        if upgraded_facility {
            if let Some(r) = self.promises.fulfil(PromiseType::FacilityImprovement) {
                reward += r as i32;
                kept += 1;
            }
        }

        // The sale the board insisted on. Judged on the money, not on a
        // headcount: a club that sold somebody cheap has not done what it
        // was told to do.
        if let Some(mandate) = self.sale_mandate {
            if mandate.is_satisfied_by(ctx.fees_received_this_season) {
                if let Some(r) = self.promises.fulfil(PromiseType::SaleMandate) {
                    reward += r as i32;
                    kept += 1;
                }
                self.sale_mandate = None;
            }
        }
        // A mandate whose promise has already lapsed is over either way —
        // `break_overdue` took the trust off at the season turn.
        if !self.promises.has_active(PromiseType::SaleMandate) {
            self.sale_mandate = None;
        }

        // Youth pathway visibly delivering.
        if ctx.academy_graduates_this_season > 0 || ctx.u21_minutes_share >= 0.25 {
            if let Some(r) = self.promises.fulfil(PromiseType::YouthMinutes) {
                reward += r as i32;
                kept += 1;
            }
        }

        // Season-outcome promises are judged once the table means something.
        if phase.can_judge_table() && ctx.league_position > 0 {
            if ctx.distance_to_relegation > 0 {
                if let Some(r) = self.promises.fulfil(PromiseType::Survival) {
                    reward += r as i32;
                    kept += 1;
                }
            }
            if ctx.distance_to_europe_or_playoff <= 0 {
                if let Some(r) = self.promises.fulfil(PromiseType::ContinentalQualification) {
                    reward += r as i32;
                    kept += 1;
                }
            }
            if ctx.league_position <= 2 {
                if let Some(r) = self.promises.fulfil(PromiseType::TitleChallenge) {
                    reward += r as i32;
                    kept += 1;
                }
            }
        }

        // A public backing carries a budget commitment for the next window.
        // Created after fulfilment so it persists to be delivered later (and
        // breaks, denting trust, if the board never follows through).
        if matches!(result.manager_meeting, Some(BoardManagerMeeting::Backing))
            && !self.promises.has_active(PromiseType::TransferBudget)
        {
            let due = today + Duration::days(210);
            self.promises
                .add(BoardPromise::new(PromiseType::TransferBudget, today, due));
        }

        if reward != 0 {
            self.relationship.adjust_communication(reward);
        }
        result.promises_kept = result.promises_kept.saturating_add(kept);
    }
}
