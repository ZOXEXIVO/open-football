//! The monthly review: four component scores, the pressure gauges, the
//! money the board moves, and the ladder that ends in a dismissal.

use crate::club::board::ClubBoard;
use crate::club::board::context::{BoardContext, FfpStatus};
use crate::club::board::decision::{BoardDecision, DecisionReason};
use crate::club::board::pressure::SupporterEvent;
use crate::club::board::promise::PromiseType;
use crate::club::board::scoring::{BoardComponentScores, SeasonPhase};
use crate::club::board::severance::Severance;
use crate::club::board::targets::SeasonTargets;
use crate::club::board::vision::FinancialStance;
use crate::club::{BoardManagerMeeting, BoardMoodState, BoardResult};
use log::debug;

impl ClubBoard {
    /// Monthly performance evaluation — the core of board behaviour. Scores
    /// four independent dimensions (sporting / financial / squad-building /
    /// strategy), folds in supporter & financial pressure, drifts the
    /// manager relationship, and gates meetings / sackings / budget moves.
    pub(crate) fn evaluate_performance(
        &mut self,
        board_ctx: &BoardContext,
        result: &mut BoardResult,
    ) {
        // Own a copy of the targets so we can freely mutate other board
        // fields below without fighting the borrow checker.
        let targets = match self.season_targets.clone() {
            Some(t) => t,
            None => return,
        };

        let phase = SeasonPhase::classify(board_ctx.matches_played, board_ctx.total_matches);

        // Respect the board's review cadence — quarterly / season-end
        // boards don't re-judge every month. Still surface current state.
        if !self
            .vision
            .review_frequency
            .evaluates_on_month(self.season_month_index)
        {
            result.mood = self.mood.state.clone();
            result.confidence = self.confidence.level;
            return;
        }

        // Playing-style mismatch (legacy helper retained).
        let style_drag = match board_ctx.main_tactic {
            Some(t) => self.vision.playing_style.drag_against(t),
            None => 0,
        };

        // ── Component scores ──
        let scores = BoardComponentScores::evaluate(
            board_ctx,
            &targets,
            &self.vision,
            &self.promises,
            phase,
            style_drag,
        );
        self.latest_scores = scores;

        // ── Pressure inputs (supporters / media / finances / regulatory) ──
        self.refresh_pressure(board_ctx);
        let pressure_drag = self.pressure.confidence_drag(self.ownership.ownership_type);

        // ── Confidence: component delta minus pressure drag ──
        let confidence_change = scores.confidence_delta(phase) - pressure_drag;
        self.confidence.level = (self.confidence.level + confidence_change).clamp(0, 100);

        // ── Manager relationship drift ──
        self.relationship.update_from_scores(&scores, style_drag);
        // Keep the legacy loyalty scalar broadly in step (blend, so the
        // fast-moving transfer/achievement nudges from other systems
        // aren't wholly overwritten).
        let blended =
            ((self.chairman.manager_loyalty as i16 + self.relationship.overall_trust() as i16) / 2)
                .clamp(0, 100) as u8;
        self.chairman.manager_loyalty = blended;

        // Position-vs-expectation delta retained for backing / meetings.
        let performance_delta = if board_ctx.league_position > 0 && board_ctx.matches_played >= 5 {
            targets.expected_position as i32 - board_ctx.league_position as i32
        } else {
            0
        };

        // ── Mood from confidence ──
        let new_mood = if self.confidence.level >= 80 {
            BoardMoodState::Excellent
        } else if self.confidence.level >= 55 {
            BoardMoodState::Good
        } else if self.confidence.level >= 30 {
            BoardMoodState::Normal
        } else {
            BoardMoodState::Poor
        };
        if matches!(new_mood, BoardMoodState::Poor) {
            self.poor_mood_months += 1;
        } else {
            self.poor_mood_months = 0;
        }
        self.mood.state = new_mood;

        // ── Manager satisfaction (mood + style friction) ──
        let mood_delta = match self.mood.state {
            BoardMoodState::Excellent => 1.5,
            BoardMoodState::Good => 0.5,
            BoardMoodState::Normal => 0.0,
            BoardMoodState::Poor => -1.0 - (self.poor_mood_months as f32 * 0.5).min(3.0),
        };
        let style_friction = (style_drag as f32 * 0.35).min(1.5);
        result.manager_satisfaction_delta = mood_delta - style_friction;

        // ── Squad limits ──
        let total_squad = board_ctx.main_squad_size + board_ctx.reserve_squad_size;
        if total_squad > targets.max_squad_size as usize + 5 {
            result.squad_over_limit = true;
            result.squad_excess = total_squad.saturating_sub(targets.max_squad_size as usize);
        }
        if board_ctx.main_squad_size < targets.min_squad_size as usize {
            result.squad_under_limit = true;
        }

        // ── Underperformance alarm ──
        if board_ctx.league_position > 0
            && board_ctx.league_position > targets.min_acceptable_position
            && phase.can_judge_table()
        {
            result.underperforming = true;
        }

        result.mood = self.mood.state.clone();
        result.confidence = self.confidence.level;

        // ── Budget / FFP / owner-injection decisions. Single source of
        // truth: emits at most one cut OR one increase per tick, and sets
        // the legacy `cut_transfer_budget` / `bonus_transfer_funds` flags
        // purely for the UI — `process` no longer applies them itself. ──
        self.emit_budget_decisions(board_ctx, &targets, performance_delta, result);

        if result.underperforming || matches!(self.mood.state, BoardMoodState::Poor) {
            debug!(
                "Board unhappy at confidence {} (weakest: {}): pos {}/{} expected {}",
                self.confidence.level,
                scores.headline(),
                board_ctx.league_position,
                board_ctx.league_size,
                targets.expected_position
            );
        }

        // ── Sacking gate ──
        // Triggers: zero confidence; sustained poor mood (+ underperformance);
        // sustained poor mood regardless; or a full relationship breakdown.
        // Patience is the chairman's threshold adjusted by manager autonomy.
        // Early-season grace via `phase.can_sack_manager()`.
        let enough_data = phase.can_sack_manager();
        let zero_confidence = self.confidence.level <= 0;
        let patience_threshold = self.patience_threshold(board_ctx);
        let sustained_poor_with_underperformance =
            self.poor_mood_months >= patience_threshold && result.underperforming;
        let sustained_poor_absolute = self.poor_mood_months >= patience_threshold + 2;
        let relationship_breakdown =
            self.relationship.relationship_breakdown() && phase.can_judge_table();

        // The ladder needs LAST month's warning state: a sack only
        // follows an ultimatum the squad has already lived with — never
        // the same meeting that issued it.
        let already_on_final_warning = self.manager_on_final_warning;
        let crisis = sustained_poor_with_underperformance
            || sustained_poor_absolute
            || relationship_breakdown;
        // Confidence collapses faster than the mood counters — a board
        // sliding toward zero goes public BEFORE it reaches the axe, so
        // the critical band is what triggers the ultimatum.
        let confidence_critical = self.confidence.level <= 20;
        let ultimatum_danger = crisis || confidence_critical;

        // Meetings + matching decisions.
        if crisis {
            result.manager_meeting = Some(BoardManagerMeeting::Crisis);
            result.decisions.push(BoardDecision::HoldCrisisMeeting);
        } else if result.underperforming
            || matches!(self.mood.state, BoardMoodState::Poor)
            || self.pressure.demands_meeting(self.ownership.ownership_type)
        {
            result.manager_meeting = Some(BoardManagerMeeting::Warning);
            result.decisions.push(BoardDecision::IssueFormalWarning);
        } else if matches!(self.mood.state, BoardMoodState::Excellent) && performance_delta >= 3 {
            result.manager_meeting = Some(BoardManagerMeeting::Backing);
            result.decisions.push(BoardDecision::IssueManagerBacking);
        }

        // First month in the danger zone = the public ultimatum. The
        // result carries the announcement so the squad reacts once.
        // Respects the early-season grace like the sack itself.
        if ultimatum_danger && enough_data && !self.manager_on_final_warning {
            self.manager_on_final_warning = true;
            result.manager_ultimatum_announced = true;
        }

        // Results picked up while on the final warning save the job —
        // the ultimatum lapses quietly (the backing IS the survival).
        // Keyed to the TABLE, not the board's mood: the mood trails
        // results by months, and an ultimatum answered with wins must
        // not become a sack while the boardroom is still sulking. If
        // form slides again, a NEW ultimatum is announced and the
        // squad reacts afresh.
        let form_recovering = !result.underperforming;
        if form_recovering {
            self.manager_on_final_warning = false;
        }

        // The axe: only for a manager already living on the final
        // warning whose situation stayed terminal — total confidence
        // collapse or the full crisis picture, with no visible upturn.
        if enough_data
            && already_on_final_warning
            && (zero_confidence || crisis)
            && !form_recovering
        {
            result.manager_sacked = true;
            result.decisions.push(BoardDecision::SackManager);
            // Reset confidence / relationship so the successor starts neutral.
            self.confidence.level = 50;
            self.poor_mood_months = 0;
            self.relationship.reset();
            self.manager_on_final_warning = false;
        }
    }

    /// Refresh the pressure gauges from this month's context: decay, then
    /// re-derive the hard-number gauges and fold in inferable narrative
    /// events (relegation scrap, winless run, promotion push, youth break).
    pub(crate) fn refresh_pressure(&mut self, ctx: &BoardContext) {
        self.pressure.decay();
        self.pressure
            .set_financial(ctx.wage_budget_usage, ctx.debt_ratio, ctx.profit_loss_12m);
        self.pressure.set_regulatory(
            matches!(ctx.ffp_status, FfpStatus::Breach),
            matches!(ctx.ffp_status, FfpStatus::Watchlist),
        );
        self.pressure.set_dressing_room(ctx.key_player_unrest_count);

        if ctx.matches_played >= 5 && ctx.distance_to_relegation <= 0 {
            self.pressure.apply_event(SupporterEvent::InRelegationZone);
        }
        if ctx.matches_played >= 5 && ctx.recent_wins == 0 && ctx.recent_losses >= 3 {
            self.pressure.apply_event(SupporterEvent::LongWinlessRun);
        }
        if ctx.league_position > 0 && ctx.distance_to_europe_or_playoff <= 0 {
            self.pressure.apply_event(SupporterEvent::InPromotionRace);
        }
        if ctx.academy_graduates_this_season > 0 {
            self.pressure
                .apply_event(SupporterEvent::YouthProspectBreakthrough);
        }
        if ctx.supporter_mood < 0.35 {
            self.pressure.supporter_pressure = self.pressure.supporter_pressure.max(40);
        }
    }

    /// Emit this tick's budget / FFP / forced-sale decisions as the single
    /// source of truth for transfer-budget movement.
    ///
    /// The historical bug: `process` cut the budget 25% on Poor mood *and*
    /// `emit_financial_decisions` could emit an FFP `CutTransferBudget`
    /// amount in the very same month, double-punishing the club. Here the
    /// budget moves by **at most one** decision per tick — a cut OR an
    /// increase, never both, and never stacked with a percentage tweak in
    /// `process` (that path has been removed). Player-sale demands are
    /// informational and don't touch the budget, so they're emitted
    /// independently.
    pub(crate) fn emit_budget_decisions(
        &mut self,
        ctx: &BoardContext,
        targets: &SeasonTargets,
        performance_delta: i32,
        result: &mut BoardResult,
    ) {
        let budget = targets.adjusted_transfer_budget();
        let austere = matches!(
            self.vision.financial_stance,
            FinancialStance::Conservative | FinancialStance::Austerity
        );
        let breach = matches!(ctx.ffp_status, FfpStatus::Breach);

        // ── Player-sale demands. These now list somebody (see
        // `BoardResult::apply_decisions`), so they are emitted at most once
        // while a mandate is outstanding. ──
        if !self.promises.has_active(PromiseType::SaleMandate) {
            if breach && (austere || self.ownership.ownership_type.resale_driven()) {
                result.decisions.push(BoardDecision::DemandPlayerSale {
                    reason: DecisionReason::FfpPressure,
                });
            } else if !breach && ctx.wage_budget_usage > 1.1 && austere {
                result.decisions.push(BoardDecision::DemandPlayerSale {
                    reason: DecisionReason::WageControl,
                });
            }
        }

        self.emit_transfer_budget_move(ctx, budget, performance_delta, result);
        self.emit_wage_budget_move(ctx, targets, result);
    }

    /// At most one move on the transfer mandate per review, at most once per
    /// cause, and never past the season's ceiling.
    fn emit_transfer_budget_move(
        &mut self,
        ctx: &BoardContext,
        budget: i64,
        performance_delta: i32,
        result: &mut BoardResult,
    ) {
        // An FFP breach is the dominant grievance and pre-empts a mood cut;
        // an unhappy board never simultaneously hands out money.
        if matches!(ctx.ffp_status, FfpStatus::Breach) {
            let cut = (budget as f64 * Self::FFP_BREACH_CUT_SHARE) as i64;
            self.push_transfer_move(-cut, DecisionReason::FfpPressure, budget, result);
            return;
        }

        if matches!(self.mood.state, BoardMoodState::Poor) {
            // Once per spell of it. The guard lifts when the mood does, so a
            // board that recovers and slumps again cuts again.
            if !self.budget_moves.poor_cut_issued {
                let cut = (budget as f64 * Self::POOR_MOOD_CUT_SHARE) as i64;
                if self.push_transfer_move(-cut, DecisionReason::Underperformance, budget, result) {
                    self.budget_moves.poor_cut_issued = true;
                }
            }
            return;
        }
        // Mood has lifted: the next slump earns its own cut.
        self.budget_moves.poor_cut_issued = false;

        // Positive side: a wealthy, risk-tolerant owner's injection after a
        // strong run takes precedence over the smaller excellent-mood bonus.
        let strong = self.latest_scores.sporting > 18.0 && self.latest_scores.financial > 0.0;
        if strong && self.ownership.injection_appetite() > 0.6 && self.injection_is_due() {
            let inject = ((budget as f64 * Self::OWNER_INJECTION_SHARE) as i64)
                .max(Self::OWNER_INJECTION_MIN);
            if self.push_transfer_move(inject, DecisionReason::OwnerInjection, budget, result) {
                self.budget_moves.owner_injections += 1;
                self.budget_moves.last_injection_month = Some(self.season_month_index);
            }
            return;
        }

        if matches!(self.mood.state, BoardMoodState::Excellent)
            && performance_delta > 3
            && !self.budget_moves.overperformance_bonus_issued
        {
            let bonus = (budget as f64 * Self::OVERPERFORMANCE_BONUS_SHARE) as i64;
            if self.push_transfer_move(bonus, DecisionReason::Overperformance, budget, result) {
                self.budget_moves.overperformance_bonus_issued = true;
            }
        }
    }

    /// Months of Poor mood the board will sit through before the picture
    /// becomes a crisis.
    ///
    /// The chairman's own temperament, how much rope the manager was given,
    /// and — new — what ending the deal would actually cost. A total
    /// confidence collapse has its own trigger and does not consult this,
    /// so a board at zero still acts however expensive the pay-off.
    pub(crate) fn patience_threshold(&self, ctx: &BoardContext) -> u8 {
        let base = self.chairman.poor_mood_threshold() as i16;
        let autonomy = self.vision.manager_autonomy.patience_bonus() as i16;
        let affordability = self.severance_patience_bonus(ctx) as i16;
        (base + autonomy + affordability).clamp(1, 12) as u8
    }

    /// Extra months of patience the price of a dismissal buys the
    /// manager.
    ///
    /// Priced off the deal the club would have to settle and the cash it
    /// holds: a bill worth a serious slice of the bank, or any bill at all
    /// at a club already overdrawn, is a reason to wait that has nothing to
    /// do with the football.
    pub(crate) fn severance_patience_bonus(&self, ctx: &BoardContext) -> i8 {
        if ctx.manager_annual_salary == 0 {
            return 0;
        }
        let months_left = ctx.manager_contract_months_left.max(0) as f64;
        let share = self.ownership.ownership_type.severance_share();
        let bill = ctx.manager_annual_salary as f64
            * (months_left / 12.0).max(Severance::MIN_MONTHS / 12.0)
            * share;

        if ctx.balance <= 0 {
            return ClubBoard::SEVERANCE_PATIENCE_BONUS;
        }
        if bill > ctx.balance as f64 * ClubBoard::SEVERANCE_PATIENCE_BAR {
            return ClubBoard::SEVERANCE_PATIENCE_BONUS;
        }
        0
    }

    /// Whether the owner may write another cheque this season.
    fn injection_is_due(&self) -> bool {
        if self.budget_moves.owner_injections >= Self::MAX_INJECTIONS_PER_SEASON {
            return false;
        }
        match self.budget_moves.last_injection_month {
            Some(last) => {
                self.season_month_index.saturating_sub(last) >= Self::INJECTION_GAP_MONTHS
            }
            None => true,
        }
    }

    /// Book a signed move on the transfer mandate, unless the season's
    /// ceiling on boardroom second-guessing has been reached. Returns
    /// whether a decision was emitted.
    fn push_transfer_move(
        &mut self,
        amount: i64,
        reason: DecisionReason,
        budget: i64,
        result: &mut BoardResult,
    ) -> bool {
        if amount == 0 {
            return false;
        }
        let cap = (budget as f64 * Self::SEASON_ADJUSTMENT_CAP) as i64;
        let room = (cap - self.budget_moves.transfer_moved).max(0);
        let amount = amount.signum() * amount.abs().min(room);
        if amount == 0 {
            return false;
        }
        self.budget_moves.transfer_moved += amount.abs();
        if amount < 0 {
            result.decisions.push(BoardDecision::CutTransferBudget {
                amount: -amount,
                reason,
            });
        } else {
            result
                .decisions
                .push(BoardDecision::IncreaseTransferBudget { amount, reason });
        }
        true
    }

    /// The wage mandate is a lever the board pulls during the season, not a
    /// number it sets once in July and then watches a manager run past.
    ///
    /// At most one move per review. A bill over its mandate for two reviews
    /// running gets trimmed; regulatory trouble trims harder; a club whose
    /// football and finances are both strong gets a little more room, once.
    fn emit_wage_budget_move(
        &mut self,
        ctx: &BoardContext,
        targets: &SeasonTargets,
        result: &mut BoardResult,
    ) {
        let mandate = targets.adjusted_wage_budget();
        if mandate <= 0 {
            return;
        }

        if ctx.wage_budget_usage > Self::WAGE_OVERRUN_RATIO {
            self.budget_moves.wage_overrun_reviews =
                self.budget_moves.wage_overrun_reviews.saturating_add(1);
        } else {
            self.budget_moves.wage_overrun_reviews = 0;
        }

        let (share, reason) = match ctx.ffp_status {
            FfpStatus::Breach => (Self::WAGE_CUT_BREACH, DecisionReason::FfpPressure),
            FfpStatus::Watchlist => (Self::WAGE_CUT_WATCHLIST, DecisionReason::FfpPressure),
            FfpStatus::Clean => {
                if self.budget_moves.wage_overrun_reviews >= Self::WAGE_OVERRUN_REVIEWS {
                    (Self::WAGE_CUT_OVERRUN, DecisionReason::WageControl)
                } else if matches!(self.mood.state, BoardMoodState::Excellent)
                    && ctx.wage_budget_usage > 0.0
                    && ctx.wage_budget_usage < 0.85
                    && ctx.profit_loss_12m > 0
                    && !self.budget_moves.wage_raise_issued
                {
                    self.budget_moves.wage_raise_issued = true;
                    (-Self::WAGE_RAISE_STRONG, DecisionReason::StrongFinances)
                } else {
                    return;
                }
            }
        };

        // A board cannot mandate a bill the contracts already exceed by more
        // than a haircut: squads unwind through expiries and sales, not
        // overnight.
        let floor = (ctx.total_annual_wages as f64 * Self::WAGE_FLOOR_OF_BILL) as i64;
        let wanted = -((mandate as f64 * share) as i64);
        let headroom = (mandate - floor).max(0);
        let amount = if wanted < 0 {
            -wanted.abs().min(headroom)
        } else {
            wanted
        };
        if amount == 0 {
            return;
        }

        let cap = (targets.wage_budget.max(0) as f64 * Self::WAGE_SEASON_CAP) as i64;
        let room = (cap - self.budget_moves.wage_moved).max(0);
        let amount = amount.signum() * amount.abs().min(room);
        if amount == 0 {
            return;
        }
        self.budget_moves.wage_moved += amount.abs();
        result
            .decisions
            .push(BoardDecision::AdjustWageBudget { amount, reason });
    }
}
