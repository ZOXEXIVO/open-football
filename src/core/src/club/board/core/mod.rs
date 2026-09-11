//! The board itself: what it knows, and the passes it runs.
//!
//! [`ClubBoard`] is the per-club actor. It reads a [`BoardContext`]
//! snapshot the club assembles for it and returns a [`BoardResult`] the
//! club applies later — it never reaches into the club directly, which is
//! what keeps the daily pass parallel-safe and every boardroom effect
//! auditable as a [`BoardDecision`].
//!
//! The passes live one per file:
//!
//! * [`review`] — the monthly performance review, the pressure gauges, the
//!   budget decisions and the sacking ladder
//! * [`promises`] — what the board pledged, and the long-term reckoning
//! * [`personality`] — where a club's owner, chairman and vision come from
//! * [`estate`] — facilities, takeovers and the boardroom's own officers

mod estate;
mod personality;
mod promises;
mod review;

#[cfg(test)]
mod tests;

use crate::club::board::chairman::ChairmanProfile;
use crate::club::board::context::BoardContext;
use crate::club::board::decision::{BoardDecision, DecisionReason};
use crate::club::board::manager::ManagerCandidate;
use crate::club::board::ownership::{ClubBenefactor, OwnershipModel};
use crate::club::board::pressure::BoardPressure;
use crate::club::board::promise::PromiseLedger;
use crate::club::board::relationship::ManagerRelationship;
use crate::club::board::scoring::BoardComponentScores;
use crate::club::board::takeover::TakeoverWatch;
use crate::club::board::targets::{BoardConfidence, SeasonBudget, SeasonTargets};
use crate::club::board::vision::{ClubVision, LongTermGoal};
use crate::club::finance::DebtStanding;
use crate::club::team::reputation::AchievementType;
use crate::club::{BoardMood, BoardResult, StaffClubContract};
use crate::context::GlobalContext;
use chrono::{Datelike, NaiveDate};

#[derive(Debug, Clone)]
pub struct ClubBoard {
    pub mood: BoardMood,
    pub confidence: BoardConfidence,
    pub director: Option<StaffClubContract>,
    pub sport_director: Option<StaffClubContract>,
    pub season_targets: Option<SeasonTargets>,
    /// Consecutive months the board has been in Poor mood
    pub poor_mood_months: u8,
    /// The board has publicly put the manager on final warning (the
    /// crisis-meeting ultimatum). A sack — barring a total confidence
    /// collapse — requires this to have been set in an EARLIER month,
    /// so the ultimatum is a real stage the squad gets to react to,
    /// not a same-tick formality. Cleared on recovery and on sacking.
    pub manager_on_final_warning: bool,
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
    /// stale enough to rebuild — see `ManagerShortlist::REFRESH_DAYS`.
    pub shortlist_built_at: Option<NaiveDate>,
    /// How long the search may run before the board commits to a
    /// hire. Locked in when `manager_search_since` is set so it stays
    /// stable across the search window. Top clubs hold out longer.
    pub search_window_days: u16,
    /// Ownership archetype. Modulates budget size, sacking threshold,
    /// and long-term tolerance. Populated at club creation; stable for
    /// the lifetime of the chairman.
    pub chairman: ChairmanProfile,
    /// Richer ownership submodel layered on the chairman — wealth,
    /// interference, risk appetite, exit pressure. Derived once from the
    /// club's durable signals (reputation, finances, league) on the first
    /// simulate tick, then stable for the chairman's tenure.
    pub ownership: OwnershipModel,
    /// Slow-moving pressure gauges (supporters, media, dressing room,
    /// finances, regulatory) read as inputs to confidence and meetings.
    pub pressure: BoardPressure,
    /// Five-facet board↔manager trust relationship. Drives renewals,
    /// relationship-driven dismissal, and transfer autonomy.
    pub relationship: ManagerRelationship,
    /// Live board promises to the manager and their kept/broken record.
    pub promises: PromiseLedger,
    /// Latest component scores from the monthly review — stored so the UI
    /// and tests can inspect *why* the board feels how it does.
    pub latest_scores: BoardComponentScores,
    /// Rare ownership-change watch (takeover rumours / completion).
    pub takeover: TakeoverWatch,
    /// 0-based month index since the current season started — drives the
    /// quarterly / season-end review cadence.
    pub season_month_index: u32,
    /// One-shot guard: ownership/personality is derived from club data on
    /// the first simulate tick (which, unlike `new()`, has club context).
    pub personality_initialized: bool,
    /// Calendar year the board last approved a *funded* facility upgrade.
    /// Drives a cooldown so even a wealthy owner can't upgrade every single
    /// season — see `FacilityReview::COOLDOWN_SEASONS`.
    pub last_facility_upgrade_year: Option<i32>,
    /// The sale the board is currently insisting on, if any.
    pub sale_mandate: Option<SaleMandate>,
    /// One-shot guards on the money the board moves.
    ///
    /// The budget used to be cut every single month a board stayed unhappy,
    /// each cut a fresh news story and a fresh dent in a mandate the club
    /// then rebuilt from scratch anyway. A board makes a decision once and
    /// lives with it; these remember that it did.
    pub budget_moves: BudgetMoves,
    /// The finance figures the board last judged this club on. Written
    /// every tick that carries a [`BoardContext`]; read by nothing in the
    /// model. See [`BoardIncomeRead`].
    pub last_income_read: BoardIncomeRead,
}

/// A sale the board has demanded and is waiting on.
///
/// Held so the promise can be judged against something concrete: not "did
/// the club sell anybody" but "did the money the board asked for arrive
/// before the deadline".
#[derive(Debug, Clone, Copy)]
pub struct SaleMandate {
    /// The player the board named.
    pub player_id: u32,
    /// What it expects for him.
    pub asking_price: f64,
    /// Season sale income at the moment the demand was made, so the
    /// judgement reads the money that came in AFTER it.
    pub fees_received_at_issue: f64,
}

impl SaleMandate {
    /// Whether enough has come in since the demand to call it delivered.
    pub fn is_satisfied_by(&self, fees_received_now: f64) -> bool {
        fees_received_now - self.fees_received_at_issue >= self.asking_price
    }
}

/// What the board has already done to this season's budgets.
///
/// One decision per cause per season (or per spell, for a mood that comes
/// and goes), plus a ceiling on the lot. Reset at the season turn with the
/// mandate the guards protect.
#[derive(Debug, Clone, Copy, Default)]
pub struct BudgetMoves {
    /// A cut has been issued for the current spell of Poor mood. Cleared
    /// when the mood lifts, so a second slump earns a second cut.
    pub poor_cut_issued: bool,
    /// The overperformance bonus has been paid this season.
    pub overperformance_bonus_issued: bool,
    /// Owner injections made this season.
    pub owner_injections: u8,
    /// Season month index of the last injection, for the gap rule.
    pub last_injection_month: Option<u32>,
    /// Absolute sum of every transfer-mandate move this season, for the cap.
    pub transfer_moved: i64,
    /// The same for the wage mandate.
    pub wage_moved: i64,
    /// Consecutive reviews the wage bill has run past its mandate.
    pub wage_overrun_reviews: u8,
    /// A wage raise has been granted this season.
    pub wage_raise_issued: bool,
}

impl BudgetMoves {
    /// Everything except the mood guard resets when a new season opens; the
    /// mood guard follows the mood, not the calendar.
    pub fn on_new_season(&mut self) {
        let poor_cut_issued = self.poor_cut_issued;
        *self = Self::default();
        self.poor_cut_issued = poor_cut_issued;
    }
}

/// What the board last saw of the club's money.
///
/// A diagnostic surface, not an input: every number here is recomputed
/// from `BoardContext` each tick, and nothing in the model reads it back.
/// It exists because [`ClubBenefactor::signal`]'s inputs are otherwise
/// unreachable from outside a tick — `projected_annual_income` is derived
/// inside `Club::simulate` from a `GlobalContext` the census does not have
/// — and a signal whose inputs cannot be printed cannot be argued with.
/// The first benefactor read found four second-division clubs and no
/// giant, and the census could not say why (memory
/// `feedback_keep_match_debug_data`).
#[derive(Debug, Clone, Copy, Default)]
pub struct BoardIncomeRead {
    /// Twelve-month income from the finance history; 0 before a month has
    /// closed.
    pub trailing_annual_income: i64,
    /// A year's income the club can be judged on today — the trailing sum
    /// once it exists, its own revenue model's projection before that.
    pub projected_annual_income: i64,
    /// The whole club's wage bill for a year.
    pub annual_wages: i64,
    /// Cash in the bank.
    pub balance: i64,
}

impl ClubBoard {
    /// Confidence a board loses when a long-term horizon runs out with the
    /// goal unmet.
    pub const VISION_MISS_CONFIDENCE_PENALTY: i32 = 25;

    /// Confidence below which that miss tips into a dismissal. Above it the
    /// manager keeps his job on a formal warning.
    pub const VISION_MISS_SACK_BELOW: i32 = 35;

    /// Results-trust the miss costs.
    pub const VISION_MISS_TRUST_PENALTY: i32 = 15;

    /// Chairman loyalty the miss costs.
    pub const VISION_MISS_LOYALTY_PENALTY: u8 = 20;

    /// Chairman loyalty a delivered horizon earns.
    pub const VISION_HIT_LOYALTY_BONUS: u8 = 10;

    /// Share of the transfer mandate a sustained poor mood takes away, once
    /// per spell of it.
    pub const POOR_MOOD_CUT_SHARE: f64 = 0.25;

    /// Share an FFP breach takes away. The breach is the dominant
    /// grievance and pre-empts a mood cut.
    pub const FFP_BREACH_CUT_SHARE: f64 = 0.33;

    /// Share clearly beating expectations earns back, once a season.
    pub const OVERPERFORMANCE_BONUS_SHARE: f64 = 0.20;

    /// Share a wealthy owner puts in after a strong run.
    pub const OWNER_INJECTION_SHARE: f64 = 0.25;

    /// Floor on that injection, so a small club's owner still writes a
    /// cheque worth having.
    pub const OWNER_INJECTION_MIN: i64 = 2_000_000;

    /// Share a collapsed takeover freezes while the dust settles.
    pub const TAKEOVER_COLLAPSE_FREEZE_SHARE: f64 = 0.20;

    /// Owner injections a single season may carry.
    pub const MAX_INJECTIONS_PER_SEASON: u8 = 2;

    /// Months between them, so a good run is not milked monthly.
    pub const INJECTION_GAP_MONTHS: u32 = 3;

    /// Ceiling on the absolute sum of everything the board moves on or off
    /// the transfer mandate in one season, as a share of that mandate.
    /// Past it the boardroom stops re-litigating its own budget.
    pub const SEASON_ADJUSTMENT_CAP: f64 = 0.50;

    /// Wage spend against mandate past which the board considers the bill
    /// an overrun rather than a rounding error.
    pub const WAGE_OVERRUN_RATIO: f32 = 1.10;

    /// Consecutive reviews an overrun must persist before the board acts.
    /// One month is a signing landing mid-window; two is a habit.
    pub const WAGE_OVERRUN_REVIEWS: u8 = 2;

    /// Share of the wage mandate each grievance trims.
    pub const WAGE_CUT_OVERRUN: f64 = 0.05;
    pub const WAGE_CUT_WATCHLIST: f64 = 0.08;
    pub const WAGE_CUT_BREACH: f64 = 0.12;

    /// Share a strong season earns back, once.
    pub const WAGE_RAISE_STRONG: f64 = 0.05;

    /// Floor on the wage mandate as a share of the bill already contracted.
    /// A board cannot mandate a wage bill the signed contracts exceed by
    /// more than a haircut — squads unwind through expiries and sales.
    pub const WAGE_FLOOR_OF_BILL: f64 = 0.85;

    /// Ceiling on the absolute sum of wage-mandate moves in one season.
    pub const WAGE_SEASON_CAP: f64 = 0.20;

    /// Share of the bank a dismissal bill has to reach before the price of
    /// it buys the manager time.
    pub const SEVERANCE_PATIENCE_BAR: f64 = 0.25;

    /// Months of extra patience an unaffordable pay-off is worth.
    pub const SEVERANCE_PATIENCE_BONUS: i8 = 1;

    pub fn new() -> Self {
        ClubBoard {
            mood: BoardMood::default(),
            confidence: BoardConfidence::default(),
            director: None,
            sport_director: None,
            season_targets: None,
            poor_mood_months: 0,
            manager_on_final_warning: false,
            vision: ClubVision::default(),
            vision_start_year: None,
            vision_goal_achieved: false,
            manager_search_since: None,
            manager_shortlist: Vec::new(),
            shortlist_built_at: None,
            search_window_days: 0,
            chairman: ChairmanProfile::new(),
            ownership: OwnershipModel::new(),
            pressure: BoardPressure::new(),
            relationship: ManagerRelationship::new(),
            promises: PromiseLedger::new(),
            latest_scores: BoardComponentScores::default(),
            takeover: TakeoverWatch::new(),
            season_month_index: 0,
            personality_initialized: false,
            last_facility_upgrade_year: None,
            sale_mandate: None,
            budget_moves: BudgetMoves::default(),
            last_income_read: BoardIncomeRead::default(),
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
        let today = ctx.simulation.date.date();

        // Derive the ownership archetype + opening vision once, the first
        // time we have club context to read. Different clubs get different
        // boards purely from durable signals — no hard-coded names.
        // What the board is looking at, banked for the census before
        // anything spends it (see [`BoardIncomeRead`]).
        if let Some(board_ctx) = &ctx.board {
            self.last_income_read = BoardIncomeRead {
                trailing_annual_income: board_ctx.trailing_annual_income,
                projected_annual_income: board_ctx.projected_annual_income,
                annual_wages: board_ctx.total_annual_wages as i64,
                balance: board_ctx.balance,
            };
        }

        if !self.personality_initialized {
            if let Some(board_ctx) = &ctx.board {
                self.bootstrap_personality(board_ctx, result.club_id);
            }
        }

        // …and a budget from the first tick, not from the first season
        // start. `calculate_season_targets` had exactly one production
        // caller — the season-start branch below — so from world creation
        // until that date EVERY club in the world carried no mandate, no
        // owner subsidy and empty envelopes, and `WagePower::for_player`
        // fell back to `level x 1.30` for all of them. A whole first
        // season with no owner cheque anywhere is the `—` column the
        // census printed.
        if self.season_targets.is_none() {
            if let Some(board_ctx) = &ctx.board {
                self.calculate_season_targets(board_ctx);
            }
        }

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

        let season = ctx
            .country
            .as_ref()
            .map(|c| c.season_dates)
            .unwrap_or_default();
        let is_season_start = ctx.simulation.is_season_start(&season);
        let is_month_beginning = ctx.simulation.is_month_beginning();

        // ── Season start: targets, vision reckoning, facility review ──
        if is_season_start {
            if let Some(board_ctx) = &ctx.board {
                let current_year = today.year();
                self.evaluate_long_term_vision(current_year, &mut result);
                // What the owner is funding, re-read once a season on a
                // three-year half-life — BEFORE the budgets, so this
                // year's wage mandate and transfer ceiling both spend it.
                let flipped = self.ownership.refresh_benefactor(
                    board_ctx.balance,
                    board_ctx.total_annual_wages as i64,
                    board_ctx.projected_annual_income,
                );
                if flipped {
                    // A club whose owner has started funding it gets the
                    // new archetype's boardroom too, not just its label.
                    Self::map_chairman_knobs(&mut self.chairman, &self.ownership);
                }
                // What the owner puts BACK this year. Booked as funding
                // rather than revenue (`push_owner_investment`), so the
                // market's inflation read still sees spend without income.
                let idle_now = ClubBenefactor::idle_cash(
                    board_ctx.balance,
                    board_ctx.total_annual_wages as i64,
                );
                let top_up = self.ownership.annual_top_up(
                    idle_now,
                    matches!(board_ctx.debt_standing, DebtStanding::Emergency),
                );
                // …and the cheque is in the pot BEFORE the envelope is
                // sized against it. `BoardDecision::OwnerTopUp` is applied
                // later, in `BoardResult::process`, so the idle cash the
                // envelope split reads was the PRE-top-up figure: a
                // benefactor that had drained to zero sized its whole
                // season off nothing and then banked the money the same
                // tick. One local copy of the context, and the order the
                // owner actually writes it in.
                let funded_ctx;
                let budget_ctx = if top_up >= 1.0 {
                    result.decisions.push(BoardDecision::OwnerTopUp {
                        amount: top_up as i64,
                        reason: DecisionReason::OwnerInjection,
                    });
                    let mut ctx = (*board_ctx).clone();
                    ctx.balance = ctx.balance.saturating_add(top_up as i64);
                    funded_ctx = ctx;
                    &funded_ctx
                } else {
                    board_ctx
                };
                self.calculate_season_targets(budget_ctx);

                // Promises whose deadline lapsed unfulfilled break now and
                // cost the manager board trust. Counted before they are
                // resolved — afterwards they are indistinguishable from
                // promises broken in earlier seasons, and the club needs
                // to date this one for the press.
                let overdue = self
                    .promises
                    .active()
                    .filter(|p| p.is_overdue(today))
                    .count();
                let penalty = self.promises.break_overdue(today);
                if penalty != 0 {
                    self.relationship.adjust_communication(penalty);
                }
                result.promises_broken = overdue.min(u8::MAX as usize) as u8;
                self.promises.prune(today, 800);

                // Yearly infrastructure review → facility decisions applied
                // in `BoardResult::process`. Gated by a per-board cooldown
                // so wealthy owners can't upgrade every season.
                let facility_decisions = self.run_facility_review(board_ctx, current_year);

                // Open this season's board promises (season goal, youth
                // pathway, deferred capex). Done after the review so a
                // declined-on-affordability upgrade becomes a "we'll revisit"
                // facility promise.
                self.open_season_promises(board_ctx, today, &facility_decisions);
                result.decisions.extend(facility_decisions);

                // Renewal: a happy board moves to tie the manager down, but
                // only when the deal is genuinely running down (or its
                // length is unknown). Driven by legacy confidence/loyalty OR
                // sustained multi-facet trust.
                let contract_at_risk = board_ctx.manager_contract_months_left == 0
                    || board_ctx.manager_contract_months_left <= 18;
                if !result.manager_sacked
                    && contract_at_risk
                    && ((self.confidence.level >= 70 && self.chairman.manager_loyalty >= 55)
                        || self.relationship.merits_renewal())
                {
                    result.offer_manager_renewal = true;
                }
                self.confidence.level = 65; // Reset confidence at season start
                self.poor_mood_months = 0;
                self.season_month_index = 0;
            }
        }

        // ── Monthly review + takeover watch ──
        if is_month_beginning {
            if let Some(board_ctx) = &ctx.board {
                self.evaluate_performance(board_ctx, &mut result);
                self.tick_takeover(board_ctx, today, &mut result);
                // Resolve outstanding promises against this tick's decisions
                // and league standing (kept promises build manager trust).
                self.resolve_promises(board_ctx, today, &mut result);
            }
            if !is_season_start {
                self.season_month_index = self.season_month_index.saturating_add(1);
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

    /// Re-derive this season's mandate from the club's standing.
    pub(crate) fn calculate_season_targets(&mut self, board_ctx: &BoardContext) {
        self.season_targets = Some(SeasonBudget::derive(
            board_ctx,
            &self.vision,
            &self.chairman,
            &self.ownership,
        ));
    }
}
