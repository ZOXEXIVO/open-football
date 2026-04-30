use chrono::NaiveDate;
use std::collections::HashMap;

use crate::transfers::pipeline::processor::PipelineProcessor;
use crate::transfers::pipeline::{
    RecommendationSource, RecommendationType, ShortlistCandidate, ShortlistCandidateStatus,
    StaffRecommendation, TransferNeedPriority, TransferNeedReason, TransferRequest,
    TransferRequestStatus, TransferShortlist,
};
use crate::transfers::window::PlayerValuationCalculator;
use crate::transfers::TransferWindowManager;
use crate::utils::IntegerUtils;
use crate::{
    Country, Person, PlayerFieldPositionGroup, PlayerPositionType, PlayerStatusType,
    ReputationLevel, WageCalculator,
};

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
    pub world_reputation: i16,
    pub current_reputation: i16,
    pub ambition: f32,
    pub parent_club_score: f32,
    pub parent_club_in_debt: bool,
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

/// Pure evaluator for the listed-star sweep.
///
/// Hard gates (reject if any fail):
///   • status flag present (Lst|Req|Unh)
///   • CA inside the buyer's tier window
///   • estimated fee within `plan_total_budget × 1.4`
///   • estimated wage within `wage_headroom × 1.3`
///   • world-reputation gap ≤ tier-scaled allowance
///   • at least one squad-need signal: weak group, open request,
///     or aging starter at the target's group
///   • improvement: ≥ 3 CA above current best, or open request
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
pub(in crate::transfers::pipeline) fn evaluate_listed_target(
    target: &ListedTargetView,
    ctx: &BuyerContext,
) -> ListedTargetVerdict {
    use ListedRejectReason::*;
    use ListedTargetVerdict::*;

    if !(target.is_listed || target.is_transfer_requested || target.is_unhappy) {
        return Reject(NotListed);
    }

    let baseline = PipelineProcessor::tier_starter_ca_score(
        ctx.buyer_rep_score,
        target.position_group,
    );
    let ceiling = PipelineProcessor::tier_target_ceiling_score(
        ctx.buyer_rep_score,
        target.position_group,
    );
    let floor = baseline.saturating_sub(20);
    if target.ability < floor || target.ability > ceiling {
        return Reject(OutOfTierWindow);
    }

    // Affordability — fee
    let affordability_cap = (ctx.plan_total_budget * 1.4).max(0.0);
    if affordability_cap <= 0.0 || target.estimated_value > affordability_cap {
        return Reject(UnaffordableFee);
    }
    if ctx.max_recommend_value > 0.0 && target.estimated_value > ctx.max_recommend_value {
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
    let wage_headroom =
        (ctx.buyer_wage_budget as i64 - ctx.buyer_total_wages as i64).max(0);
    let wage_cap = (wage_headroom as f64 * 1.3) as u64;
    if wage_cap > 0 && (estimated_wage as u64) > wage_cap {
        return Reject(UnaffordableWage);
    }

    // Reputation plausibility — tier-scaled gap
    let gap_allowed = (1200.0 + 2400.0 * ctx.buyer_rep_score) as i32;
    if (target.world_reputation as i32 - ctx.buyer_world_rep as i32) > gap_allowed {
        return Reject(ReputationGapTooLarge);
    }

    // Squad need
    let weak_group = (ctx.buyer_best_in_group as i16) < baseline as i16;
    if !(weak_group || ctx.has_open_request || ctx.has_aging_starter) {
        return Reject(NoSquadNeed);
    }

    // Improvement: must be a meaningful upgrade or coach-requested
    let upgrade = (target.ability as i16) - (ctx.buyer_best_in_group as i16);
    if !ctx.has_open_request && upgrade < 3 {
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
        ((affordability_cap - target.estimated_value) / affordability_cap).clamp(0.0, 1.0)
            as f32
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
        let window_mgr = TransferWindowManager::new();
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
        }

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
                        average_rating: player.statistics.average_rating,
                        appearances: player.statistics.total_games(),
                        is_transfer_protected: player.is_transfer_protected(date, current_window),
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
                if avg == 0 {
                    50
                } else {
                    avg
                }
            };

            let club_rep = team.reputation.level();
            let club_rep_score = team.reputation.overall_score();
            let club_world_rep = team.reputation.world as i16;
            let club_league_reputation = team
                .league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);
            let club_total_wages: u32 = club
                .teams
                .iter()
                .map(|t| t.get_annual_salary())
                .sum();
            let club_wage_budget: u32 = club
                .finance
                .wage_budget
                .as_ref()
                .map(|b| b.amount.max(0.0) as u32)
                .unwrap_or(club_total_wages.saturating_mul(11) / 10);

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
                        || memory.last_seen < date - chrono::Duration::days(540)
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
                    })
                    .collect();

                if candidates.is_empty() {
                    continue;
                }

                // Score candidates
                let mut best_score = 0.0f32;
                let mut best_candidate: Option<&PlayerSnapshot> = None;

                for cand in &candidates {
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

                    // High potential gap
                    if cand.estimated_potential > cand.ability + 15 {
                        score += 2.5;
                    } else if cand.estimated_potential > cand.ability + 8 {
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

                    // Ability fit
                    if cand.ability >= avg_ability.saturating_sub(5) {
                        score += 1.0;
                    }

                    if score > best_score {
                        best_score = score;
                        best_candidate = Some(cand);
                    }
                }

                if let Some(cand) = best_candidate {
                    // Assess with error based on judging skill
                    let ability_error = (20i16 - judging as i16).max(1) as i32;
                    let potential_error = (20i16 - judging_pot as i16).max(1) as i32;

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
                    m.entry(g).and_modify(|v| { if ca > *v { *v = ca } }).or_insert(ca);
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

                    let view = ListedTargetView {
                        ability: p.ability,
                        estimated_potential: p.estimated_potential,
                        age: p.age,
                        estimated_value: p.estimated_value,
                        position_group: p.position_group,
                        is_listed: p.is_listed,
                        is_transfer_requested: p.is_transfer_requested,
                        is_unhappy: p.is_unhappy,
                        world_reputation: p.world_reputation,
                        current_reputation: p.current_reputation,
                        ambition: p.ambition,
                        parent_club_score: p.parent_club_score,
                        parent_club_in_debt: p.club_in_debt,
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
                    };
                    match evaluate_listed_target(&view, &ctx) {
                        ListedTargetVerdict::Accept(score) => Some((p, score)),
                        ListedTargetVerdict::Reject(_) => None,
                    }
                })
                .collect();

            let mut ranked = scored_targets;
            ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

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
                        })
                        .collect();

                    if let Some(best) = dof_candidates.iter().max_by_key(|p| p.ability) {
                        let ability_error = (20i16 - judging as i16).max(1) as i32;
                        let potential_error = (20i16 - judging_pot as i16).max(1) as i32;

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
                    club.transfer_plan
                        .staff_recommendations
                        .push(action.recommendation);
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
        let seven_days_ago = date - chrono::Duration::days(7);

        for club in &country.clubs {
            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

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

                // Check if an existing unfulfilled request covers the same position group
                let matching_request = plan.transfer_requests.iter().find(|r| {
                    r.position.position_group() == player_pos_group
                        && r.status != TransferRequestStatus::Fulfilled
                        && r.status != TransferRequestStatus::Abandoned
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
