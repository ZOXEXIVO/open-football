use chrono::NaiveDate;
use std::collections::HashMap;

use crate::transfers::TransferWindowManager;
use crate::transfers::pipeline::ScoutMonitoringSource;
use crate::transfers::pipeline::ScoutPlayerMonitoring;
use crate::transfers::pipeline::breakout::{BreakoutPerformanceSignal, LeaguePerformanceLookup};
use crate::transfers::pipeline::exposure::{
    AvailabilityExposure, AvailabilitySignals, ExposureStage, FreeAgentBuyerContext,
    FreeAgentRecommendationSignals, OpportunisticFreeAgentScout,
};
use crate::transfers::pipeline::plausibility::{
    BuyerPlausibilityContext, EffectivePlayerReputation, TransferPlausibilityBuilder,
    TransferPlausibilityVerdict,
};
use crate::transfers::pipeline::processor::PipelineProcessor;
use crate::transfers::pipeline::{
    RecommendationSource, RecommendationType, ShortlistCandidate, ShortlistCandidateStatus,
    StaffRecommendation, TransferNeedPriority, TransferNeedReason, TransferRequest,
    TransferRequestStatus, TransferShortlist,
};
use crate::transfers::window::PlayerValuationCalculator;
use crate::utils::IntegerUtils;
use crate::{
    Country, Person, PlayerFieldPositionGroup, PlayerPositionType, PlayerStatusType,
    ReputationLevel, WageCalculator,
};
use chrono::Duration;
use std::cmp::Ordering;

/// Compact view of a listed-target candidate. Decouples the filter
/// from the live `PlayerSnapshot` so tests can construct synthetic
/// candidates without booting the full snapshot pipeline. The
/// player's id isn't needed by the evaluator (it's pure scoring) —
/// callers carry the snapshot reference alongside the view.
#[derive(Debug, Clone, Copy)]
pub(in crate::transfers::pipeline) struct ListedTargetView {
    pub ability: u8,
    pub estimated_potential: u8,
    pub age: u8,
    pub estimated_value: f64,
    pub position_group: PlayerFieldPositionGroup,
    pub is_listed: bool,
    pub is_transfer_requested: bool,
    pub is_unhappy: bool,
    /// Parent club has loan-listed the player. On its own this routes to
    /// the loan market, but a loan-listed player with a strong breakout
    /// score is treated as "available enough" for a permanent approach too
    /// — clubs buy the players smaller clubs only meant to loan out.
    pub is_loan_listed: bool,
    /// Performance-breakout discovery score (0..100) from
    /// [`crate::transfers::pipeline::breakout::BreakoutPerformanceSignal`].
    /// A high score lets the player be discovered on *form* — admitting a
    /// loan-listed (or, in form-discovery mode, an unlisted) breakout
    /// player into this path and lifting his ranking — but it never relaxes
    /// the affordability / tier / reputation gates below.
    pub breakout_score: f32,
    pub world_reputation: i16,
    pub current_reputation: i16,
    pub ambition: f32,
    pub parent_club_score: f32,
    pub parent_club_in_debt: bool,
    /// Days since the player first became available. Drives the
    /// market-exposure staleness curve — softening and circulation lift.
    pub days_available: i64,
    /// Months left on contract; <= 6 reads as a bargain pickup.
    pub contract_months_remaining: i16,
    /// Capable player who is barely featuring — the market should want him
    /// even when his own club is ambivalent.
    pub low_usage: bool,
    /// Concrete approaches in the last 30 days (from the player's durable
    /// availability state). High interest damps the circulation lift.
    pub recent_interest_count: u8,
    /// Consecutive weekly circulation scans that found no taker.
    pub failed_scans: u16,
}

/// Buyer-side context the filter consults. One struct, one place to
/// describe "who is looking, with what means, against what squad" —
/// keeps the per-target filter pure and trivially testable.
#[derive(Debug, Clone, Copy)]
pub(in crate::transfers::pipeline) struct BuyerContext {
    /// Continuous reputation score (0..1) of the buyer.
    pub buyer_rep_score: f32,
    pub buyer_world_rep: i16,
    pub buyer_league_reputation: u16,
    pub buyer_total_wages: u32,
    pub buyer_wage_budget: u32,
    /// `plan.total_budget` — the transfer-budget cap.
    pub plan_total_budget: f64,
    /// Soft cap from `plan.total_budget * 2.0` — scouts shouldn't tag
    /// players the club cannot afford even with stretch. Pass 0.0 to
    /// disable.
    pub max_recommend_value: f64,
    /// Best CA at the target's position group on the buying squad.
    pub buyer_best_in_group: u8,
    /// Buyer has an open `TransferRequest` matching the target's group.
    pub has_open_request: bool,
    /// Buyer has a 30+ at-tier starter at the target's group — a
    /// succession opportunity that opens up a slot.
    pub has_aging_starter: bool,
    /// Year-round "breakout watch" mode. When set, a strong-breakout player
    /// who is NOT publicly available is still admitted so the club can open
    /// scouting monitoring on him purely on form. Off (the default) for the
    /// in-window listed-star sweep, which only surfaces players who have
    /// advertised availability. Either way the affordability / tier /
    /// reputation / squad-need gates are unchanged.
    pub form_discovery_mode: bool,
}

/// Outcome of evaluating a listed target. Either rejected with a
/// specific reason (debug surface, also a clean test API) or accepted
/// with its weighted recruitment score.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::transfers::pipeline) enum ListedTargetVerdict {
    Reject(ListedRejectReason),
    Accept(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::transfers::pipeline) enum ListedRejectReason {
    NotListed,
    OutOfTierWindow,
    UnaffordableFee,
    UnaffordableWage,
    ReputationGapTooLarge,
    NoSquadNeed,
    NotAnUpgrade,
}

/// Pure evaluator for the listed-star / breakout sweep.
///
/// Hard gates (reject if any fail):
///   • availability: a public market flag (Lst|Req|Unh), OR loan-listed
///     with a strong breakout score, OR — in `form_discovery_mode` — a
///     strong breakout alone (the year-round watch discovers on form)
///   • CA inside the buyer's tier window
///   • estimated fee within `plan_total_budget × 1.4`
///   • estimated wage within `wage_headroom × 1.3`
///   • world-reputation gap ≤ tier-scaled allowance
///   • a reason to act: a squad-need signal (weak group, open request,
///     aging starter) OR a market opportunity — a stale available player
///     or a genuine breakout who is at least depth-relevant or a resale
///     prospect. A breakout never relaxes the fee / wage / tier / rep gates.
///   • improvement: ≥ 3 CA above current best, an open request, or a
///     qualifying opportunity
///
/// Soft scoring (sum, higher is better — used for top-N selection):
///   • improvement margin (capped to +30)
///   • prime-age bonus
///   • youth potential bonus
///   • status urgency (Req > Lst > Unh)
///   • seller-debt distress
///   • affordability headroom
///   • squad-need fit
///   • ambition-driven step-up
///   • stale-availability circulation lift
///   • performance-breakout lift
pub(in crate::transfers::pipeline) fn evaluate_listed_target(
    target: &ListedTargetView,
    ctx: &BuyerContext,
) -> ListedTargetVerdict {
    use ListedRejectReason::*;
    use ListedTargetVerdict::*;

    // Availability gate. A player is "available enough" to pursue when he
    // is publicly on the market (Lst/Req/Unh); OR he is loan-listed AND his
    // form is a genuine breakout (clubs buy the players smaller clubs only
    // meant to loan out); OR, in year-round form-discovery mode, his form
    // alone is a strong breakout (scouting monitoring opens on talent, not
    // just on a for-sale sign). None of these relax the realism gates below
    // — they only decide whether the player enters this path at all.
    let strong_breakout = target.breakout_score >= BreakoutPerformanceSignal::BREAKOUT_THRESHOLD;
    let publicly_available = target.is_listed || target.is_transfer_requested || target.is_unhappy;
    // A clearly bigger club may pursue a smaller club's breakout star even
    // when he isn't listed — the realistic "giant comes for the second-
    // division top scorer". Bounded by a genuine breakout AND a real
    // reputation gap to the parent club; the tier-window, affordability and
    // plausibility gates below still apply, so this never becomes a
    // free-for-all. It converts the year-round breakout MONITORING that
    // `scan_breakout_form` builds into an actual in-window approach instead
    // of a row that just sits on the books waiting for the player to be
    // listed (which his selling club, holding an asset, rarely does).
    let buyer_outranks_parent = ctx.buyer_rep_score >= target.parent_club_score + 0.10;
    let available_enough = publicly_available
        || (target.is_loan_listed && strong_breakout)
        || (ctx.form_discovery_mode && strong_breakout)
        || (strong_breakout && buyer_outranks_parent);
    if !available_enough {
        return Reject(NotListed);
    }

    let baseline =
        PipelineProcessor::tier_starter_ca_score(ctx.buyer_rep_score, target.position_group);
    let ceiling =
        PipelineProcessor::tier_target_ceiling_score(ctx.buyer_rep_score, target.position_group);
    let floor = baseline.saturating_sub(20);
    if target.ability < floor || target.ability > ceiling {
        return Reject(OutOfTierWindow);
    }

    // Market-exposure verdict — staleness, the seller/player softening
    // curves, and the circulation lift for the soft score. Pure function
    // of the observable signals; never relaxes the tier window above.
    let exposure = AvailabilityExposure::compute(&AvailabilitySignals {
        days_available: target.days_available,
        is_listed: target.is_listed,
        is_transfer_requested: target.is_transfer_requested,
        is_unhappy: target.is_unhappy,
        is_loan_listed: target.is_loan_listed,
        current_ability: target.ability,
        estimated_potential: target.estimated_potential,
        age: target.age,
        estimated_value: target.estimated_value,
        asking_to_value_ratio: if target.parent_club_in_debt { 0.9 } else { 1.1 },
        current_salary: 0,
        world_reputation: target.world_reputation,
        ambition: target.ambition,
        contract_months_remaining: target.contract_months_remaining,
        seller_in_debt: target.parent_club_in_debt,
        squad_surplus: false,
        low_usage_despite_ability: target.low_usage,
        recent_interest_count: target.recent_interest_count,
        failed_scans: target.failed_scans,
    });

    // Affordability — fee. The asking price softens the longer the player
    // sits unsold, so a stale listing becomes reachable for a club that
    // couldn't fund the headline value — bounded, never a giveaway.
    let asking_value = target.estimated_value * (1.0 - exposure.price_softening as f64);
    let affordability_cap = (ctx.plan_total_budget * 1.4).max(0.0);
    if affordability_cap <= 0.0 || asking_value > affordability_cap {
        return Reject(UnaffordableFee);
    }
    if ctx.max_recommend_value > 0.0 && asking_value > ctx.max_recommend_value {
        return Reject(UnaffordableFee);
    }

    // Affordability — wage proxy
    let estimated_wage = WageCalculator::expected_annual_wage_raw(
        target.ability,
        target.current_reputation,
        matches!(target.position_group, PlayerFieldPositionGroup::Forward),
        matches!(target.position_group, PlayerFieldPositionGroup::Goalkeeper),
        target.age,
        ctx.buyer_rep_score,
        ctx.buyer_league_reputation,
    );
    // The player relaxes his wage expectation over a dry spell, again
    // bounded by the softening curve.
    let softened_wage = (estimated_wage as f64 * (1.0 - exposure.wage_softening as f64)) as u64;
    let wage_headroom = (ctx.buyer_wage_budget as i64 - ctx.buyer_total_wages as i64).max(0);
    let wage_cap = (wage_headroom as f64 * 1.3) as u64;
    if wage_cap > 0 && softened_wage > wage_cap {
        return Reject(UnaffordableWage);
    }

    // Reputation plausibility — tier-scaled gap (never softened: an
    // impossible-prestige move stays impossible regardless of staleness).
    // Uses EFFECTIVE reputation, not bare world rep: a player who is a
    // recognised name in his own market (high current/home standing) is
    // gauged on that domestic renown, so a low-world-rep domestic star is
    // not mistaken for a reachable bargain. `max(world, blend)` means a
    // player whose current rep is at/below his world rep is judged exactly
    // as before — the blend only ever raises the bar, never lowers it.
    let effective_rep = EffectivePlayerReputation::compute(
        target.world_reputation,
        target.current_reputation,
        target.current_reputation,
        true,
    )
    .max(target.world_reputation);
    let gap_allowed = (1200.0 + 2400.0 * ctx.buyer_rep_score) as i32;
    if (effective_rep as i32 - ctx.buyer_world_rep as i32) > gap_allowed {
        return Reject(ReputationGapTooLarge);
    }

    // Squad need — OR a strong, stale market opportunity. A high-exposure
    // available player who is affordable and would add depth or future
    // resale value can be recommended even to a club without an open
    // positional need. Gated on non-Fresh staleness: a brand-new listing
    // never bypasses the need check, so the opportunity route is reserved
    // for players the market has had time to leave sitting — and it never
    // relaxes the tier / fee / wage / reputation gates above.
    let weak_group = (ctx.buyer_best_in_group as i16) < baseline as i16;
    let has_need = weak_group || ctx.has_open_request || ctx.has_aging_starter;
    let resale_value = target.age <= 23 && target.estimated_potential > target.ability + 5;
    let depth_value = (target.ability as i16) >= baseline as i16 - 10;
    // A stale, untouched available player OR a genuine performance breakout
    // is a market opportunity even without a conventional positional need —
    // but only when he is at least squad-relevant (depth) or a resale
    // prospect, so a club never chases a hot scorer plainly below its level.
    let stale_opportunity = !matches!(exposure.stage, ExposureStage::Fresh)
        && exposure.score >= 45.0
        && (resale_value || depth_value);
    let breakout_opportunity = strong_breakout && (resale_value || depth_value);
    let strong_opportunity = stale_opportunity || breakout_opportunity;
    if !(has_need || strong_opportunity) {
        return Reject(NoSquadNeed);
    }

    // Improvement: a meaningful upgrade, coach-requested, or a strong
    // market opportunity (depth / resale add).
    let upgrade = (target.ability as i16) - (ctx.buyer_best_in_group as i16);
    if !ctx.has_open_request && !strong_opportunity && upgrade < 3 {
        return Reject(NotAnUpgrade);
    }

    // ── Soft scoring ──
    let mut score = 0.0_f32;
    score += (upgrade as f32).clamp(0.0, 30.0);

    score += match target.age {
        25..=29 => 5.0,
        22..=24 => 3.0,
        30 => 1.0,
        _ => 0.0,
    };

    if target.age <= 23 && target.estimated_potential > target.ability {
        let gap = (target.estimated_potential - target.ability) as f32;
        score += gap.clamp(0.0, 10.0);
    }

    if target.is_transfer_requested {
        score += 4.0;
    } else if target.is_listed {
        score += 2.0;
    } else if target.is_unhappy {
        score += 1.5;
    }

    if target.parent_club_in_debt {
        score += 2.0;
    }

    let headroom_ratio = if affordability_cap > 0.0 {
        ((affordability_cap - target.estimated_value) / affordability_cap).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    score += headroom_ratio * 5.0;

    if ctx.has_open_request {
        score += 8.0;
    } else if weak_group {
        score += 4.0;
    } else if ctx.has_aging_starter {
        score += 2.0;
    }

    let tier_delta = ctx.buyer_rep_score - target.parent_club_score;
    score += tier_delta * 4.0 * target.ambition.clamp(0.0, 1.0);

    // Stale, untouched availability lifts the player up the ranking so a
    // genuinely-stuck Req/Unh/listed player doesn't disappear behind a
    // churn of fresher candidates.
    score += exposure.circulation_boost;

    // Breakout form lifts the player up the ranking so a genuinely hot
    // talent is pursued ahead of a merely-available one. Ranking only —
    // the hard gates above already passed.
    score += (target.breakout_score / 100.0) * 12.0;

    Accept(score)
}

impl PipelineProcessor {
    pub fn generate_staff_recommendations(country: &mut Country, date: NaiveDate) {
        // Only runs weekly (same schedule as should_evaluate)
        if !Self::should_evaluate(date) {
            return;
        }

        let is_january = Self::is_january_window(date);
        let price_level = country.settings.pricing.price_level;
        let window_mgr = TransferWindowManager::for_country(country, date);
        let current_window = window_mgr.current_window_dates(country.id, date);

        // Pass 1: Build player snapshots across all clubs
        #[allow(dead_code)]
        struct PlayerSnapshot {
            id: u32,
            club_id: u32,
            position: PlayerPositionType,
            position_group: PlayerFieldPositionGroup,
            ability: u8,             // skill-based, not CA
            estimated_potential: u8, // estimated from age + mentals, not PA
            age: u8,
            estimated_value: f64,
            contract_months_remaining: u32,
            club_in_debt: bool,
            parent_club_reputation: ReputationLevel,
            /// Continuous reputation score of the parent club (0..1).
            /// Drives wage proxy, plausibility, and tier-delta scoring
            /// without snapping into the enum bucket.
            parent_club_score: f32,
            /// League reputation for the parent club, 0..10000. Feeds
            /// wage estimation when the player moves to another country.
            parent_league_reputation: u16,
            is_loan_listed: bool,
            /// Listed for permanent transfer by the parent club.
            is_listed: bool,
            /// Player has formally requested a move.
            is_transfer_requested: bool,
            /// Player carries the Unh status — extended unhappiness.
            is_unhappy: bool,
            /// Player ambition (0..1). Drives willingness to step up
            /// or accept a lateral/down move.
            ambition: f32,
            /// World reputation 0..10000 — how plausible it is for any
            /// given club to land this player at all (Mbappé to Levante
            /// is reputation-implausible regardless of fee).
            world_reputation: i16,
            /// Current reputation 0..10000 — drives wage proxy.
            current_reputation: i16,
            // Observable performance
            average_rating: f32,
            appearances: u16,
            is_transfer_protected: bool,
            /// Days the player has been advertised as available (earliest
            /// Lst/Req/Unh/Loa status), 0 when not available. Drives the
            /// market-exposure staleness curve.
            days_available: i64,
            /// Concrete approaches in the last 30 days, read from the
            /// player's durable availability state (updated by the weekly
            /// circulation pass). 0 before the state is seeded.
            recent_interest_count: u8,
            /// Consecutive weekly circulation scans that found no taker.
            failed_scans: u16,
            /// Performance-breakout discovery score (0..100), computed once
            /// here from the league performance lookup so the listed-star
            /// sweep and the scout-network scorer share one number.
            breakout_score: f32,
        }

        // Per-country scoring-chart + recent-award lookup, built once for
        // the whole pass so the breakout score on each snapshot is cheap.
        let performance_lookup = LeaguePerformanceLookup::build(country);
        let mut all_snapshots: Vec<PlayerSnapshot> = Vec::new();

        for club in &country.clubs {
            let club_in_debt = club.finance.balance.balance < 0;
            let main_team_ref = club.teams.main();
            let rep_level = main_team_ref
                .map(|t| t.reputation.level())
                .unwrap_or(ReputationLevel::Amateur);
            let parent_club_score = main_team_ref
                .map(|t| t.reputation.overall_score())
                .unwrap_or(0.0);
            let parent_league_reputation = main_team_ref
                .and_then(|t| t.league_id)
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);

            // Pull the seller's full market context once per club —
            // every player's snapshot value should reflect the league
            // and club they're actually playing for, not a flat 0/0.
            let (seller_league_rep, seller_club_rep) =
                PlayerValuationCalculator::seller_context(country, club);

            for team in &club.teams.teams {
                for player in &team.players.players {
                    if player.is_on_loan() {
                        continue;
                    }
                    let value = PlayerValuationCalculator::calculate_value_with_price_level(
                        player,
                        date,
                        price_level,
                        seller_league_rep,
                        seller_club_rep,
                    );
                    let contract_months = player
                        .contract
                        .as_ref()
                        .map(|c| {
                            let days = (c.expiration - date).num_days().max(0) as u32;
                            days / 30
                        })
                        .unwrap_or(0);

                    let statuses = player.statuses.get();

                    let skill_ability = Self::position_evaluation_ability(player);
                    let player_age = player.age(date);
                    let estimated_potential = skill_ability
                        + Self::estimate_growth_potential(
                            player_age,
                            player.skills.mental.determination,
                            player.skills.mental.work_rate,
                            player.skills.mental.composure,
                            player.skills.mental.anticipation,
                            skill_ability,
                        );

                    let appearances = player.statistics.total_games();
                    let average_rating = player
                        .statistics
                        .average_rating_realistic(player.position().position_group());
                    // Form-discovery signal — built once per player from the
                    // observable output / rating / scoring-chart / award data.
                    let breakout_score = performance_lookup
                        .breakout_for_player(
                            player,
                            appearances,
                            average_rating,
                            player_age,
                            parent_league_reputation,
                        )
                        .score;

                    all_snapshots.push(PlayerSnapshot {
                        id: player.id,
                        club_id: club.id,
                        position: player.position(),
                        position_group: player.position().position_group(),
                        ability: skill_ability,
                        estimated_potential,
                        age: player_age,
                        estimated_value: value.amount,
                        contract_months_remaining: contract_months,
                        club_in_debt,
                        parent_club_reputation: rep_level.clone(),
                        parent_club_score,
                        parent_league_reputation,
                        is_loan_listed: statuses.contains(&PlayerStatusType::Loa),
                        is_listed: statuses.contains(&PlayerStatusType::Lst),
                        is_transfer_requested: statuses.contains(&PlayerStatusType::Req),
                        is_unhappy: statuses.contains(&PlayerStatusType::Unh),
                        ambition: player.attributes.ambition,
                        world_reputation: player.player_attributes.world_reputation,
                        current_reputation: player.player_attributes.current_reputation,
                        // Recommendation feature row: regressed value.
                        average_rating,
                        appearances,
                        is_transfer_protected: player.is_transfer_protected(date, current_window),
                        days_available: player.days_available(date),
                        recent_interest_count: player
                            .availability_market_state()
                            .map(|s| s.recent_interest(date))
                            .unwrap_or(0),
                        failed_scans: player
                            .availability_market_state()
                            .map(|s| s.failed_scans)
                            .unwrap_or(0),
                        breakout_score,
                    });
                }
            }
        }

        // Collect recommendations per club
        struct RecommendationAction {
            club_id: u32,
            recommendation: StaffRecommendation,
        }

        let mut actions: Vec<RecommendationAction> = Vec::new();

        for club in &country.clubs {
            if club.teams.teams.is_empty() {
                continue;
            }
            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            // Cap: 6 recommendations per club per window
            if plan.staff_recommendations.len() >= 6 {
                continue;
            }

            let team = &club.teams.teams[0];
            let resolved = team.staffs.resolve_for_transfers();

            let avg_ability = {
                let avg = team.players.current_ability_avg();
                if avg == 0 { 50 } else { avg }
            };

            let club_rep = team.reputation.level();
            let club_rep_score = team.reputation.overall_score();
            let club_world_rep = team.reputation.world as i16;
            let club_league_reputation = team
                .league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);
            let club_total_wages: u32 = club.teams.iter().map(|t| t.get_annual_salary()).sum();
            let club_wage_budget: u32 = club
                .finance
                .wage_budget
                .as_ref()
                .map(|b| b.amount.max(0.0) as u32)
                .unwrap_or(club_total_wages.saturating_mul(11) / 10);

            // Buyer context for the plausibility gate. Built once per
            // club so each recommendation sub-path can veto unrealistic
            // targets without re-walking reputation / wage data.
            let buyer_plaus_ctx = BuyerPlausibilityContext::build(country, club);
            // Closure shorthand: true when adding `player_id` to the
            // recommendation list would push an impossible move (Maximenko-
            // class step-down) into the pipeline.
            let plausibility_rejects = |player_id: u32, is_loan: bool| -> bool {
                let summary = match Self::find_player_summary_in_country(country, player_id, date) {
                    Some(s) => s,
                    None => return false,
                };
                matches!(
                    TransferPlausibilityBuilder::evaluate_summary(
                        &buyer_plaus_ctx,
                        &summary,
                        is_loan,
                        true,
                        date,
                    ),
                    Some(TransferPlausibilityVerdict::HardReject(_))
                )
            };

            let already_recommended: Vec<u32> = plan
                .staff_recommendations
                .iter()
                .map(|r| r.player_id)
                .collect();

            // Budget cap: scouts should not recommend players the club cannot afford
            let max_recommend_value = plan.total_budget * 2.0;

            let memory_recommender_id = resolved
                .scouts
                .first()
                .copied()
                .or(resolved.director_of_football)
                .or_else(|| Some(team.staffs.head_coach()))
                .map(|s| s.id);

            if let Some(recommender_staff_id) = memory_recommender_id {
                for memory in &plan.known_players {
                    if plan.staff_recommendations.len()
                        + actions.iter().filter(|a| a.club_id == club.id).count()
                        >= 6
                    {
                        break;
                    }
                    if memory.last_known_club_id == club.id
                        || memory.last_seen < date - Duration::days(540)
                        || memory.confidence < 0.25
                        || already_recommended.contains(&memory.player_id)
                        || actions.iter().any(|a| {
                            a.club_id == club.id && a.recommendation.player_id == memory.player_id
                        })
                    {
                        continue;
                    }
                    let seen_score = memory.official_appearances_seen as f32
                        + memory.friendly_appearances_seen as f32 * 0.35;
                    if seen_score < 0.35 {
                        continue;
                    }
                    if max_recommend_value > 0.0 && memory.estimated_fee > max_recommend_value {
                        continue;
                    }
                    if memory.assessed_ability < avg_ability.saturating_sub(12) {
                        continue;
                    }
                    if plausibility_rejects(memory.player_id, false) {
                        continue;
                    }

                    let rec_type = if memory.assessed_potential > memory.assessed_ability + 12 {
                        RecommendationType::HiddenGem
                    } else if memory.official_appearances_seen == 0 {
                        RecommendationType::YouthMatchStandout
                    } else {
                        RecommendationType::ReadyForStepUp
                    };

                    actions.push(RecommendationAction {
                        club_id: club.id,
                        recommendation: StaffRecommendation {
                            player_id: memory.player_id,
                            recommender_staff_id,
                            source: RecommendationSource::ScoutNetwork,
                            recommendation_type: rec_type,
                            assessed_ability: memory.assessed_ability,
                            assessed_potential: memory.assessed_potential,
                            confidence: memory.confidence,
                            estimated_fee: memory.estimated_fee,
                            date_recommended: date,
                        },
                    });
                }
            }

            // ── Scout network recommendations ──
            for scout in &resolved.scouts {
                let judging = scout.staff_attributes.knowledge.judging_player_ability;
                let judging_pot = scout.staff_attributes.knowledge.judging_player_potential;

                // Discovery chance: 10 + (judging_ability * 3) percent
                let discovery_chance = 10 + (judging as i32 * 3);
                if IntegerUtils::random(0, 100) > discovery_chance {
                    continue;
                }

                // Elite/Continental clubs require candidates from at least National-level clubs.
                // This prevents top clubs from scouting players in semi-professional leagues
                // whose inflated ability numbers don't reflect proven quality at a high level.
                let min_source_rep = match club_rep {
                    ReputationLevel::Elite => ReputationLevel::National,
                    ReputationLevel::Continental => ReputationLevel::Regional,
                    _ => ReputationLevel::Amateur,
                };

                // Filter candidates from other clubs.
                // Upper bound is tier-aware: an Elite-club scout can tag
                // genuine world-class targets; a small-club scout stays
                // disciplined. The previous `avg + judging/2` cap silently
                // hid elite players from elite clubs whenever the squad
                // average lagged the tier baseline (youth/reserves dragging
                // the mean down).
                let candidates: Vec<&PlayerSnapshot> = all_snapshots
                    .iter()
                    .filter(|p| {
                        let ceiling =
                            Self::tier_target_ceiling_score(club_rep_score, p.position_group);
                        p.club_id != club.id
                            && !club.is_rival(p.club_id)
                            && !p.is_transfer_protected
                            && p.ability >= avg_ability.saturating_sub(10)
                            && p.ability <= ceiling
                            && (max_recommend_value <= 0.0
                                || p.estimated_value <= max_recommend_value)
                            && Self::rep_level_value(&p.parent_club_reputation)
                                >= Self::rep_level_value(&min_source_rep)
                            && !already_recommended.contains(&p.id)
                            && !actions
                                .iter()
                                .any(|a| a.club_id == club.id && a.recommendation.player_id == p.id)
                            && !plausibility_rejects(p.id, false)
                    })
                    .collect();

                if candidates.is_empty() {
                    continue;
                }

                // Assessment error from the scout's judging skill: ≈±1 for a
                // top scout (judging 20), wide for a poor one. Drives BOTH the
                // ranking below and the report further down, so a sharp scout
                // converges on the genuine best target while a weaker one
                // legitimately misjudges and disagrees — different clubs chase
                // different players instead of the whole division funnelling
                // onto one deterministic "best" name.
                let ability_error = (20i16 - judging as i16).max(1) as i32;
                let potential_error = (20i16 - judging_pot as i16).max(1) as i32;

                // Score candidates by the scout's PERCEIVED quality, not their
                // true numbers.
                let mut best_score = 0.0f32;
                let mut best_candidate: Option<&PlayerSnapshot> = None;

                for cand in &candidates {
                    let perceived_ability = (cand.ability as i32
                        + IntegerUtils::random(-ability_error, ability_error))
                    .clamp(1, 200) as u8;
                    let perceived_potential = (cand.estimated_potential as i32
                        + IntegerUtils::random(-potential_error, potential_error))
                    .clamp(1, 200) as u8;

                    let mut score: f32 = 0.0;

                    // Expiring contract
                    if cand.contract_months_remaining <= 6 {
                        score += 3.0;
                    } else if cand.contract_months_remaining <= 12 {
                        score += 1.5;
                    }

                    // Club in debt
                    if cand.club_in_debt {
                        score += 2.0;
                    }

                    // High potential gap (as the scout reads it)
                    if perceived_potential > perceived_ability + 15 {
                        score += 2.5;
                    } else if perceived_potential > perceived_ability + 8 {
                        score += 1.5;
                    }

                    // Lower-rep club
                    if Self::rep_level_value(&cand.parent_club_reputation)
                        < Self::rep_level_value(&club_rep)
                    {
                        score += 1.0;
                    }

                    // Loan-listed
                    if cand.is_loan_listed {
                        score += if is_january { 2.0 } else { 1.0 };
                    }

                    // Performance breakout — a player whose *results* are
                    // outrunning his current level (league-rep discounted)
                    // is exactly who a scout network should flag.
                    score += (cand.breakout_score / 100.0) * 4.0;

                    // Ability fit (as the scout reads it)
                    if perceived_ability >= avg_ability.saturating_sub(5) {
                        score += 1.0;
                    }

                    // Split genuine near-ties so they don't always resolve to
                    // the same iteration-order winner. Bounded well under the
                    // scoring steps above (never leapfrogs a clearly-preferred
                    // target) and shrinks toward zero as judging improves.
                    score += IntegerUtils::random(0, ability_error.min(10)) as f32 * 0.05;

                    if score > best_score {
                        best_score = score;
                        best_candidate = Some(cand);
                    }
                }

                if let Some(cand) = best_candidate {
                    // Report figure: an independent roll on the same judging
                    // error hoisted above, so what the board sees isn't pinned
                    // to the draw that won selection.
                    let assessed_ability = (cand.ability as i32
                        + IntegerUtils::random(-ability_error, ability_error))
                    .clamp(1, 200) as u8;
                    let assessed_potential = (cand.estimated_potential as i32
                        + IntegerUtils::random(-potential_error, potential_error))
                    .clamp(1, 200) as u8;

                    let confidence = (0.3 + (judging as f32 * 0.035)).min(0.95);

                    let rec_type = if cand.contract_months_remaining <= 6 {
                        RecommendationType::ExpiringContract
                    } else if cand.club_in_debt {
                        RecommendationType::FinancialDistress
                    } else if cand.estimated_potential > cand.ability + 15 && cand.age <= 22 {
                        RecommendationType::HiddenGem
                    } else if cand.is_loan_listed {
                        RecommendationType::LoanOpportunity
                    } else {
                        RecommendationType::ReadyForStepUp
                    };

                    actions.push(RecommendationAction {
                        club_id: club.id,
                        recommendation: StaffRecommendation {
                            player_id: cand.id,
                            recommender_staff_id: scout.id,
                            source: RecommendationSource::ScoutNetwork,
                            recommendation_type: rec_type,
                            assessed_ability,
                            assessed_potential,
                            confidence,
                            estimated_fee: cand.estimated_value,
                            date_recommended: date,
                        },
                    });
                }
            }

            // ── Listed-star sweep ──
            // Players who have advertised themselves as available — Lst
            // (transfer-listed by the club), Req (player handed in a
            // transfer request), or Unh (extended unhappiness) — surface
            // to clubs whose tier window matches their quality. This
            // closes a structural hole: the demand-driven scout pipeline
            // only identifies targets when a club has an open positional
            // need, so a 14M unhappy player at a smaller club generates
            // no signal at any top club whose own positions are filled.
            //
            // Three-stage gate, then weighted scoring:
            //
            //   Hard filters (impossible signings)
            //     • status flag present (Lst|Req|Unh)
            //     • not at this club / not a rival / not transfer-protected
            //     • CA inside the club's tier window
            //         floor = baseline - 20
            //         ceiling = tier_target_ceiling_score
            //     • affordability: estimated fee within plan.total_budget × 1.4
            //         (40% margin lets the board approve a reach signing)
            //     • wage realism: estimated wage fits headroom × 1.3
            //         (some slack for board renegotiation)
            //     • reputation plausibility: world-rep gap < 2200
            //         (Mbappé to Levante stays unrealistic regardless of fee)
            //     • squad need: matching open request OR best-in-group
            //         below tier baseline OR aging starter (30+)
            //     • improvement: at least 3 CA above club's best-in-group,
            //         OR open request explicitly asks for the position
            //
            //   Soft scoring (rank survivors)
            //     • upgrade margin over current best
            //     • prime-age bonus
            //     • youth potential bonus
            //     • status urgency (Req > Lst > Unh)
            //     • affordability headroom
            //     • debt-distressed seller
            //     • squad-need fit (open request match boosts heavily)
            //
            // Same mechanism for every status, group, and tier — no
            // hardcoded club or player exceptions. Confidence and
            // recommender are filled in once a target is selected.
            let listed_recommender_id = resolved
                .director_of_football
                .map(|s| s.id)
                .or_else(|| resolved.scouts.first().map(|s| s.id))
                .unwrap_or(team.staffs.head_coach().id);

            // Per-group best CA at the buying club — cached so the
            // filter doesn't re-scan the squad N times.
            let buyer_best_in_group: HashMap<PlayerFieldPositionGroup, u8> = {
                let mut m: HashMap<PlayerFieldPositionGroup, u8> = HashMap::new();
                for p in team.players.players.iter() {
                    let g = p.position().position_group();
                    let ca = p.player_attributes.current_ability;
                    m.entry(g)
                        .and_modify(|v| {
                            if ca > *v {
                                *v = ca
                            }
                        })
                        .or_insert(ca);
                }
                m
            };
            // Aging starter per group: any player at-tier in this group
            // who's 30+ — succession candidate that opens up a slot.
            let buyer_has_aging_starter = |group: PlayerFieldPositionGroup| -> bool {
                let baseline = Self::tier_starter_ca_score(club_rep_score, group);
                team.players.players.iter().any(|p| {
                    p.position().position_group() == group
                        && p.age(date) >= 30
                        && p.player_attributes.current_ability + 5 >= baseline
                })
            };
            // Open request matching this group — explicit demand-side
            // signal that raises the priority of any matching listed
            // candidate.
            let buyer_open_request_for = |group: PlayerFieldPositionGroup| -> bool {
                plan.transfer_requests.iter().any(|r| {
                    r.position.position_group() == group
                        && r.status != TransferRequestStatus::Fulfilled
                        && r.status != TransferRequestStatus::Abandoned
                })
            };

            let scored_targets: Vec<(&PlayerSnapshot, f32)> = all_snapshots
                .iter()
                .filter_map(|p| {
                    // Identity gates handled here — they refer to the
                    // live `PlayerSnapshot` / `actions` Vec and don't
                    // belong in the pure recruitment evaluator.
                    if p.club_id == club.id || club.is_rival(p.club_id) || p.is_transfer_protected {
                        return None;
                    }
                    if already_recommended.contains(&p.id)
                        || actions
                            .iter()
                            .any(|a| a.club_id == club.id && a.recommendation.player_id == p.id)
                    {
                        return None;
                    }
                    if plausibility_rejects(p.id, false) {
                        return None;
                    }

                    let view = ListedTargetView {
                        ability: p.ability,
                        estimated_potential: p.estimated_potential,
                        age: p.age,
                        estimated_value: p.estimated_value,
                        position_group: p.position_group,
                        is_listed: p.is_listed,
                        is_transfer_requested: p.is_transfer_requested,
                        is_unhappy: p.is_unhappy,
                        is_loan_listed: p.is_loan_listed,
                        breakout_score: p.breakout_score,
                        world_reputation: p.world_reputation,
                        current_reputation: p.current_reputation,
                        ambition: p.ambition,
                        parent_club_score: p.parent_club_score,
                        parent_club_in_debt: p.club_in_debt,
                        days_available: p.days_available,
                        contract_months_remaining: p
                            .contract_months_remaining
                            .min(i16::MAX as u32) as i16,
                        low_usage: p.appearances < 8,
                        recent_interest_count: p.recent_interest_count,
                        failed_scans: p.failed_scans,
                    };
                    let ctx = BuyerContext {
                        buyer_rep_score: club_rep_score,
                        buyer_world_rep: club_world_rep,
                        buyer_league_reputation: club_league_reputation,
                        buyer_total_wages: club_total_wages,
                        buyer_wage_budget: club_wage_budget,
                        plan_total_budget: plan.total_budget,
                        max_recommend_value,
                        buyer_best_in_group: buyer_best_in_group
                            .get(&p.position_group)
                            .copied()
                            .unwrap_or(0),
                        has_open_request: buyer_open_request_for(p.position_group),
                        has_aging_starter: buyer_has_aging_starter(p.position_group),
                        // In-window listed-star sweep: only publicly
                        // available players (Lst/Req/Unh, or Loa+breakout).
                        form_discovery_mode: false,
                    };
                    match evaluate_listed_target(&view, &ctx) {
                        ListedTargetVerdict::Accept(score) => Some((p, score)),
                        ListedTargetVerdict::Reject(_) => None,
                    }
                })
                .collect();

            let mut ranked = scored_targets;
            ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));

            for (target, _score) in ranked.iter().take(3) {
                let current_recs = plan.staff_recommendations.len()
                    + actions.iter().filter(|a| a.club_id == club.id).count();
                if current_recs >= 6 {
                    break;
                }

                let rec_type = if Self::rep_level_value(&target.parent_club_reputation)
                    > Self::rep_level_value(&club_rep)
                {
                    // Player is at a bigger club but on the market — the
                    // smaller buyer benefits from quality leftovers.
                    RecommendationType::BigClubSurplus
                } else if target.is_transfer_requested || target.is_unhappy {
                    // The player is pushing for the move — frame it as
                    // ambition, not a step-up label that doesn't apply.
                    RecommendationType::WeakSpotFix
                } else {
                    RecommendationType::ReadyForStepUp
                };

                actions.push(RecommendationAction {
                    club_id: club.id,
                    recommendation: StaffRecommendation {
                        player_id: target.id,
                        recommender_staff_id: listed_recommender_id,
                        source: RecommendationSource::DirectorOfFootball,
                        recommendation_type: rec_type,
                        assessed_ability: target.ability,
                        assessed_potential: target.estimated_potential,
                        // Public listing → high baseline confidence; no
                        // observation noise to wash out.
                        confidence: 0.7,
                        estimated_fee: target.estimated_value,
                        date_recommended: date,
                    },
                });
            }

            // ── Opportunistic free-agent / soon-free recommendations ──
            // Quality players whose contracts are running down get
            // circulated to clubs where they are a plausible, affordable
            // depth or future-resale add — even without an open positional
            // request. The pure `OpportunisticFreeAgentScout` owns the
            // useful / affordable / level judgement, so a club is never
            // recommended a free agent it can't use or fund, and a player
            // well above the club's level is only floated once long
            // unemployment (career pressure) would make him flexible. Pool
            // free agents already without a club are discovered by the
            // dedicated country-level matcher; this path covers the
            // soon-free domestic players that matcher can't see until they
            // are actually released. Deduped + capped through the same
            // pipeline as every other recommendation.
            {
                let opportunistic_cap = match club_rep {
                    ReputationLevel::Regional
                    | ReputationLevel::Local
                    | ReputationLevel::Amateur => 10,
                    ReputationLevel::National => 8,
                    _ => 6,
                };
                let current_recs = plan.staff_recommendations.len()
                    + actions.iter().filter(|a| a.club_id == club.id).count();
                if current_recs < opportunistic_cap {
                    let max_squad = club
                        .board
                        .season_targets
                        .as_ref()
                        .map(|t| t.max_squad_size as usize)
                        .unwrap_or(50);
                    let squad_room = team.players.players.len() < max_squad;
                    let wage_headroom = club_wage_budget as i64 - club_total_wages as i64;

                    // Per-group body count on the main team — drives the
                    // "thin at this position" depth signal.
                    let mut group_counts: HashMap<PlayerFieldPositionGroup, usize> = HashMap::new();
                    for p in team.players.players.iter() {
                        *group_counts
                            .entry(p.position().position_group())
                            .or_insert(0) += 1;
                    }
                    let group_thin = |group: PlayerFieldPositionGroup| -> bool {
                        let count = group_counts.get(&group).copied().unwrap_or(0);
                        let target = group.ideal_squad_depth();
                        count < target
                    };

                    let mut fa_targets: Vec<&PlayerSnapshot> = all_snapshots
                        .iter()
                        .filter(|p| {
                            if p.club_id == club.id
                                || club.is_rival(p.club_id)
                                || p.is_transfer_protected
                                || p.contract_months_remaining > 6
                                || already_recommended.contains(&p.id)
                                || actions.iter().any(|a| {
                                    a.club_id == club.id && a.recommendation.player_id == p.id
                                })
                            {
                                return false;
                            }
                            let signals = FreeAgentRecommendationSignals {
                                current_ability: p.ability,
                                estimated_potential: p.estimated_potential,
                                age: p.age,
                                contract_months_remaining: p.contract_months_remaining,
                                // Still under contract — no free-agent
                                // career pressure has accrued yet.
                                career_pressure: 0.0,
                            };
                            let buyer = FreeAgentBuyerContext {
                                buyer_avg_ability: avg_ability,
                                buyer_squad_room: squad_room,
                                buyer_wage_headroom: wage_headroom,
                                group_below_depth: group_thin(p.position_group),
                            };
                            OpportunisticFreeAgentScout::should_recommend(&signals, &buyer)
                                && !plausibility_rejects(p.id, false)
                        })
                        .collect();
                    fa_targets.sort_by(|a, b| b.ability.cmp(&a.ability));

                    let remaining = opportunistic_cap - current_recs;
                    for target in fa_targets.iter().take(remaining.min(2)) {
                        actions.push(RecommendationAction {
                            club_id: club.id,
                            recommendation: StaffRecommendation {
                                player_id: target.id,
                                recommender_staff_id: listed_recommender_id,
                                source: RecommendationSource::DirectorOfFootball,
                                recommendation_type: RecommendationType::FreeAgentBargain,
                                assessed_ability: target.ability,
                                assessed_potential: target.estimated_potential,
                                // Public soon-free status → high baseline
                                // confidence; no observation noise.
                                confidence: 0.6,
                                estimated_fee: 0.0,
                                date_recommended: date,
                            },
                        });
                    }
                }
            }

            // ── DoF bargain identification ──
            if let Some(dof) = resolved.director_of_football {
                let judging = dof.staff_attributes.knowledge.judging_player_ability;
                let judging_pot = dof.staff_attributes.knowledge.judging_player_potential;
                let dof_chance = 40 + (judging as i32 * 3);

                if IntegerUtils::random(0, 100) <= dof_chance {
                    // Look for expiring contracts with ability >= avg-5
                    let dof_candidates: Vec<&PlayerSnapshot> = all_snapshots
                        .iter()
                        .filter(|p| {
                            p.club_id != club.id
                                && !club.is_rival(p.club_id)
                                && !p.is_transfer_protected
                                && p.contract_months_remaining <= 6
                                && p.ability >= avg_ability.saturating_sub(5)
                                && !already_recommended.contains(&p.id)
                                && !actions.iter().any(|a| {
                                    a.club_id == club.id && a.recommendation.player_id == p.id
                                })
                                && !plausibility_rejects(p.id, false)
                        })
                        .collect();

                    // Rank by the DoF's PERCEIVED ability (judging-driven
                    // error), not true ability — two equally-equipped
                    // directors no longer both converge on the same single
                    // name, so the bargain hunt spreads across comparable
                    // expiring-contract targets.
                    let ability_error = (20i16 - judging as i16).max(1) as i32;
                    let potential_error = (20i16 - judging_pot as i16).max(1) as i32;
                    if let Some(best) = dof_candidates.iter().max_by_key(|p| {
                        (p.ability as i32 + IntegerUtils::random(-ability_error, ability_error))
                            .clamp(1, 200)
                    }) {
                        let assessed_ability = (best.ability as i32
                            + IntegerUtils::random(-ability_error, ability_error))
                        .clamp(1, 200) as u8;
                        let assessed_potential = (best.estimated_potential as i32
                            + IntegerUtils::random(-potential_error, potential_error))
                        .clamp(1, 200) as u8;

                        let confidence = (0.4 + (judging as f32 * 0.035)).min(0.95);

                        actions.push(RecommendationAction {
                            club_id: club.id,
                            recommendation: StaffRecommendation {
                                player_id: best.id,
                                recommender_staff_id: dof.id,
                                source: RecommendationSource::DirectorOfFootball,
                                recommendation_type: RecommendationType::ExpiringContract,
                                assessed_ability,
                                assessed_potential,
                                confidence,
                                estimated_fee: best.estimated_value,
                                date_recommended: date,
                            },
                        });
                    }
                }
            }

            // ── Small club staff: aggressive loan/bargain hunting ──
            // Small clubs rely on their staff to find cheap deals, loans,
            // free agents, and surplus players from bigger clubs.
            // Even a head coach at a small club knows what the squad needs.
            let is_small_club = matches!(
                club_rep,
                ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur
            );
            let is_mid_club = club_rep == ReputationLevel::National;

            if is_small_club || is_mid_club {
                let rec_cap = if is_small_club { 10 } else { 8 };
                let current_recs = plan.staff_recommendations.len()
                    + actions.iter().filter(|a| a.club_id == club.id).count();

                if current_recs < rec_cap {
                    let remaining = rec_cap - current_recs;

                    // Coach recommends players available on loan
                    let head_coach = team.staffs.head_coach();
                    let coach_id = head_coach.id;
                    let coach_judging =
                        head_coach.staff_attributes.knowledge.judging_player_ability;
                    let coach_judging_pot = head_coach
                        .staff_attributes
                        .knowledge
                        .judging_player_potential;

                    // ── Cheap loan targets (loan-listed players the club could afford) ──
                    let mut loan_targets: Vec<&PlayerSnapshot> = all_snapshots
                        .iter()
                        .filter(|p| {
                            p.club_id != club.id
                                && !club.is_rival(p.club_id)
                                && !p.is_transfer_protected
                                && p.is_loan_listed
                                && p.ability >= avg_ability.saturating_sub(8)
                                && !already_recommended.contains(&p.id)
                                && !actions.iter().any(|a| {
                                    a.club_id == club.id && a.recommendation.player_id == p.id
                                })
                                && !plausibility_rejects(p.id, true)
                        })
                        .collect();
                    loan_targets.sort_by(|a, b| b.ability.cmp(&a.ability));

                    for target in loan_targets.iter().take(remaining.min(3)) {
                        let ability_error = (20i16 - coach_judging as i16).max(1) as i32;
                        let potential_error = (20i16 - coach_judging_pot as i16).max(1) as i32;

                        let assessed_ability = (target.ability as i32
                            + IntegerUtils::random(-ability_error, ability_error))
                        .clamp(1, 200) as u8;
                        let assessed_potential = (target.estimated_potential as i32
                            + IntegerUtils::random(-potential_error, potential_error))
                        .clamp(1, 200) as u8;

                        let rec_type = if target.ability > avg_ability + 5 {
                            RecommendationType::BigClubSurplus
                        } else if target.age >= 28 {
                            RecommendationType::ExperiencedLoanMentor
                        } else {
                            RecommendationType::CheapLoanAvailable
                        };

                        let confidence = (0.4 + (coach_judging as f32 * 0.03)).min(0.9);

                        actions.push(RecommendationAction {
                            club_id: club.id,
                            recommendation: StaffRecommendation {
                                player_id: target.id,
                                recommender_staff_id: coach_id,
                                source: RecommendationSource::HeadCoach,
                                recommendation_type: rec_type,
                                assessed_ability,
                                assessed_potential,
                                confidence,
                                estimated_fee: target.estimated_value * 0.1, // loan fee
                                date_recommended: date,
                            },
                        });
                    }

                    let current_recs_after_loans = plan.staff_recommendations.len()
                        + actions.iter().filter(|a| a.club_id == club.id).count();
                    let remaining_after_loans = rec_cap.saturating_sub(current_recs_after_loans);

                    // ── Free agent bargains (expiring contracts) ──
                    if remaining_after_loans > 0 {
                        let mut free_targets: Vec<&PlayerSnapshot> = all_snapshots
                            .iter()
                            .filter(|p| {
                                p.club_id != club.id
                                    && !club.is_rival(p.club_id)
                                    && !p.is_transfer_protected
                                    && p.contract_months_remaining <= 6
                                    && p.ability >= avg_ability.saturating_sub(10)
                                    && !already_recommended.contains(&p.id)
                                    && !actions.iter().any(|a| {
                                        a.club_id == club.id && a.recommendation.player_id == p.id
                                    })
                                    && !plausibility_rejects(p.id, false)
                            })
                            .collect();
                        free_targets.sort_by(|a, b| b.ability.cmp(&a.ability));

                        for target in free_targets.iter().take(remaining_after_loans.min(2)) {
                            let ability_error = (20i16 - coach_judging as i16).max(1) as i32;
                            let potential_error = (20i16 - coach_judging_pot as i16).max(1) as i32;

                            let assessed_ability = (target.ability as i32
                                + IntegerUtils::random(-ability_error, ability_error))
                            .clamp(1, 200) as u8;
                            let assessed_potential = (target.estimated_potential as i32
                                + IntegerUtils::random(-potential_error, potential_error))
                            .clamp(1, 200)
                                as u8;

                            let confidence = (0.5 + (coach_judging as f32 * 0.03)).min(0.9);

                            actions.push(RecommendationAction {
                                club_id: club.id,
                                recommendation: StaffRecommendation {
                                    player_id: target.id,
                                    recommender_staff_id: coach_id,
                                    source: RecommendationSource::HeadCoach,
                                    recommendation_type: RecommendationType::FreeAgentBargain,
                                    assessed_ability,
                                    assessed_potential,
                                    confidence,
                                    estimated_fee: 0.0, // free agent
                                    date_recommended: date,
                                },
                            });
                        }
                    }

                    let current_recs_after_free = plan.staff_recommendations.len()
                        + actions.iter().filter(|a| a.club_id == club.id).count();
                    let remaining_after_free = rec_cap.saturating_sub(current_recs_after_free);

                    // ── Players wanting game time from bigger clubs ──
                    // Young players at bigger clubs who aren't loan-listed yet but
                    // are below their club's average — they'd benefit from a loan
                    if remaining_after_free > 0 && is_small_club {
                        let mut game_time_seekers: Vec<&PlayerSnapshot> = all_snapshots
                            .iter()
                            .filter(|p| {
                                p.club_id != club.id
                                    && !club.is_rival(p.club_id)
                                    && !p.is_transfer_protected
                                    && p.age <= 23
                                    && p.estimated_potential > p.ability + 5
                                    && p.ability >= avg_ability.saturating_sub(5)
                                    && Self::rep_level_value(&p.parent_club_reputation)
                                        > Self::rep_level_value(&club_rep)
                                    && !p.is_loan_listed
                                    && !already_recommended.contains(&p.id)
                                    && !actions.iter().any(|a| {
                                        a.club_id == club.id && a.recommendation.player_id == p.id
                                    })
                                    && !plausibility_rejects(p.id, true)
                            })
                            .collect();
                        game_time_seekers
                            .sort_by(|a, b| b.estimated_potential.cmp(&a.estimated_potential));

                        for target in game_time_seekers.iter().take(remaining_after_free.min(2)) {
                            let ability_error = (20i16 - coach_judging as i16).max(1) as i32;
                            let potential_error = (20i16 - coach_judging_pot as i16).max(1) as i32;

                            let assessed_ability = (target.ability as i32
                                + IntegerUtils::random(-ability_error, ability_error))
                            .clamp(1, 200) as u8;
                            let assessed_potential = (target.estimated_potential as i32
                                + IntegerUtils::random(-potential_error, potential_error))
                            .clamp(1, 200)
                                as u8;

                            let confidence = (0.3 + (coach_judging as f32 * 0.025)).min(0.8);

                            actions.push(RecommendationAction {
                                club_id: club.id,
                                recommendation: StaffRecommendation {
                                    player_id: target.id,
                                    recommender_staff_id: coach_id,
                                    source: RecommendationSource::HeadCoach,
                                    recommendation_type: RecommendationType::GameTimeSeeker,
                                    assessed_ability,
                                    assessed_potential,
                                    confidence,
                                    estimated_fee: target.estimated_value * 0.05, // loan fee
                                    date_recommended: date,
                                },
                            });
                        }
                    }
                }
            }
        }

        // Pass 2: Push recommendations into club transfer plans
        // Small clubs get higher cap
        for action in actions {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                let rep = club
                    .teams
                    .teams
                    .first()
                    .map(|t| t.reputation.level())
                    .unwrap_or(ReputationLevel::Amateur);
                let cap = match rep {
                    ReputationLevel::Regional
                    | ReputationLevel::Local
                    | ReputationLevel::Amateur => 10,
                    ReputationLevel::National => 8,
                    _ => 6,
                };
                if club.transfer_plan.staff_recommendations.len() < cap {
                    let rec = action.recommendation;
                    let recommender_id = rec.recommender_staff_id;
                    let player_id = rec.player_id;
                    let assessed_ability = rec.assessed_ability;
                    let assessed_potential = rec.assessed_potential;
                    let confidence = rec.confidence;
                    let estimated_fee = rec.estimated_fee;
                    let source = match rec.source {
                        RecommendationSource::ScoutNetwork => {
                            ScoutMonitoringSource::StaffRecommendation
                        }
                        RecommendationSource::ChiefScoutReport => {
                            ScoutMonitoringSource::StaffRecommendation
                        }
                        RecommendationSource::DirectorOfFootball => {
                            ScoutMonitoringSource::StaffRecommendation
                        }
                        RecommendationSource::HeadCoach => {
                            ScoutMonitoringSource::StaffRecommendation
                        }
                    };
                    club.transfer_plan.staff_recommendations.push(rec);

                    // Mirror the recommendation into a monitoring row
                    // so the recruitment meeting and UI surfaces see
                    // the player on this scout's books too.
                    let plan = &mut club.transfer_plan;
                    if plan
                        .find_monitoring_mut(recommender_id, player_id)
                        .is_none()
                    {
                        let id = plan.next_monitoring_id();
                        let mut row =
                            ScoutPlayerMonitoring::new(id, recommender_id, player_id, source, date);
                        row.record_observation(
                            assessed_ability,
                            assessed_potential,
                            confidence,
                            1.0,
                            estimated_fee,
                            Vec::new(),
                            date,
                            false,
                        );
                        plan.scout_monitoring.push(row);
                    }
                }
            }
        }
    }

    pub fn process_staff_recommendations(country: &mut Country, date: NaiveDate) {
        // Only runs weekly (same schedule as should_evaluate)
        if !Self::should_evaluate(date) {
            return;
        }

        struct RecommendationProcessAction {
            club_id: u32,
            kind: RecommendationProcessKind,
        }

        enum RecommendationProcessKind {
            AddToShortlist {
                shortlist_request_id: u32,
                candidate: ShortlistCandidate,
            },
            CreateRequest {
                request: TransferRequest,
                candidate: ShortlistCandidate,
            },
        }

        let mut actions: Vec<RecommendationProcessAction> = Vec::new();
        let seven_days_ago = date - Duration::days(7);

        for club in &country.clubs {
            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }
            let buyer_ctx = BuyerPlausibilityContext::build(country, club);

            let recent_recs: Vec<&StaffRecommendation> = plan
                .staff_recommendations
                .iter()
                .filter(|r| r.date_recommended >= seven_days_ago)
                .collect();

            for rec in &recent_recs {
                // Determine player's position group
                let memory = plan.known_player(rec.player_id);
                let player_pos_group =
                    if let Some(player) = Self::find_player_in_country(country, rec.player_id) {
                        player.position().position_group()
                    } else if let Some(memory) = memory {
                        memory.position_group
                    } else {
                        continue;
                    };

                // Re-check plausibility before promoting a stale
                // recommendation. Player status and seller balance may
                // have shifted since the recommendation was filed.
                if let Some(summary) =
                    Self::find_player_summary_in_country(country, rec.player_id, date)
                {
                    let plausibility = TransferPlausibilityBuilder::evaluate_summary(
                        &buyer_ctx, &summary, false, true, date,
                    );
                    if let Some(TransferPlausibilityVerdict::HardReject(_)) = plausibility {
                        continue;
                    }
                }

                // Check if an existing unfulfilled request covers the same position group.
                // Emergency free-agent depth requests don't count — attaching a paid
                // recommendation to their shortlist would route a zero-budget request
                // into the paid negotiation path.
                let matching_request = plan.transfer_requests.iter().find(|r| {
                    r.position.position_group() == player_pos_group
                        && r.status != TransferRequestStatus::Fulfilled
                        && r.status != TransferRequestStatus::Abandoned
                        && !r.is_emergency_free_agent_depth()
                });

                if let Some(req) = matching_request {
                    // Find the shortlist for this request
                    let has_shortlist = plan
                        .shortlists
                        .iter()
                        .any(|s| s.transfer_request_id == req.id);

                    if has_shortlist {
                        // Add as candidate to existing shortlist
                        let already_in = plan.shortlists.iter().any(|s| {
                            s.transfer_request_id == req.id
                                && s.candidates.iter().any(|c| c.player_id == rec.player_id)
                        });

                        if !already_in {
                            actions.push(RecommendationProcessAction {
                                club_id: club.id,
                                kind: RecommendationProcessKind::AddToShortlist {
                                    shortlist_request_id: req.id,
                                    candidate: ShortlistCandidate {
                                        player_id: rec.player_id,
                                        score: rec.assessed_ability as f32 / 100.0
                                            + rec.confidence * 0.1,
                                        estimated_fee: rec.estimated_fee,
                                        status: ShortlistCandidateStatus::Available,
                                    },
                                },
                            });
                        }
                    }
                } else if rec.confidence >= 0.6 && rec.assessed_ability >= 50 {
                    // No existing request — create a new one
                    let player_position = if let Some(player) =
                        Self::find_player_in_country(country, rec.player_id)
                    {
                        player.position()
                    } else if let Some(memory) = memory {
                        memory.position
                    } else {
                        continue;
                    };

                    // Check we don't already have too many requests
                    let active_requests = plan
                        .transfer_requests
                        .iter()
                        .filter(|r| {
                            r.status != TransferRequestStatus::Fulfilled
                                && r.status != TransferRequestStatus::Abandoned
                        })
                        .count();

                    if active_requests >= 8 {
                        continue;
                    }

                    // Allocate 15% of available budget
                    let available_budget = plan.available_budget();
                    let alloc = available_budget * 0.15;

                    if alloc <= 0.0 {
                        continue;
                    }

                    let next_id = plan.next_request_id
                        + actions
                            .iter()
                            .filter(|a| {
                                a.club_id == club.id
                                    && matches!(
                                        a.kind,
                                        RecommendationProcessKind::CreateRequest { .. }
                                    )
                            })
                            .count() as u32;

                    actions.push(RecommendationProcessAction {
                        club_id: club.id,
                        kind: RecommendationProcessKind::CreateRequest {
                            candidate: ShortlistCandidate {
                                player_id: rec.player_id,
                                score: rec.assessed_ability as f32 / 100.0 + rec.confidence * 0.1,
                                estimated_fee: rec.estimated_fee,
                                status: ShortlistCandidateStatus::Available,
                            },
                            request: TransferRequest::new(
                                next_id,
                                player_position,
                                TransferNeedPriority::Optional,
                                TransferNeedReason::StaffRecommendation,
                                rec.assessed_ability.saturating_sub(5),
                                rec.assessed_ability,
                                alloc,
                            ),
                        },
                    });
                }
            }
        }

        // Pass 2: Apply actions
        for action in actions {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                let plan = &mut club.transfer_plan;

                match action.kind {
                    RecommendationProcessKind::AddToShortlist {
                        shortlist_request_id,
                        candidate,
                    } => {
                        if let Some(shortlist) = plan
                            .shortlists
                            .iter_mut()
                            .find(|s| s.transfer_request_id == shortlist_request_id)
                        {
                            shortlist.candidates.push(candidate);
                        }
                    }
                    RecommendationProcessKind::CreateRequest { request, candidate } => {
                        let req_id = request.id;
                        if req_id >= plan.next_request_id {
                            plan.next_request_id = req_id + 1;
                        }
                        plan.transfer_requests.push(request);
                        let mut shortlist = TransferShortlist::new(req_id, candidate.estimated_fee);
                        shortlist.candidates.push(candidate);
                        plan.shortlists.push(shortlist);
                    }
                }
            }
        }
    }
}
