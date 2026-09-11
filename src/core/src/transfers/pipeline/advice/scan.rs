//! The weekly staff-recommendation pass.
//!
//! Three stages, which the comments inside the old 1 366-line
//! `generate_staff_recommendations` already named but could not express: one
//! snapshot of every sellable player in the country, one scan per club against
//! that snapshot, and one commit that writes the surviving recommendations into
//! the clubs' plans.
//!
//! The scan is where the length was. Six sources feed it — the scouts' standing
//! memory, the scout network's weekly discovery roll, the listed-star sweep,
//! expiring contracts, the director of football's bargain eye, and the small-club
//! hunt for loans and free agents — and each was a `── section ──` comment inside
//! one function body. They are methods on [`ClubAdviceScan`] now. Each body moved
//! unchanged: the scan reads its context back into the names the body already
//! used, because the hazard here is not a compile error, it is silently swapping
//! one reputation score for another.

use crate::transfers::market::window::MarketCadence;
use crate::transfers::pipeline::StaffRecommendations;
use crate::transfers::scouting::judgement::ScoutJudgement;
use crate::transfers::squad::bands::TierBands;
use crate::transfers::view::player::PlayerView;
use chrono::Duration;
use chrono::NaiveDate;
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::HashMap;

use super::{
    BuyerContext, BuyerNeedPicture, ListedTargetScreen, ListedTargetVerdict, ListedTargetView,
};
use crate::club::player::transfer::AvailabilityBlockReason;
use crate::club::staff::recruitment::ResolvedStaff;
use crate::transfers::TransferWindowManager;
use crate::transfers::gate::TransferPlausibilityVerdict;
use crate::transfers::gate::build::{BuyerPlausibilityContext, TransferPlausibilityBuilder};
use crate::transfers::gate::fit::{SquadFitSnapshot, SquadRegistrationLimits};
use crate::transfers::loan::interest::{ClubOpinion, InterestDraw};
use crate::transfers::pipeline::{
    ClubTransferPlan, RecommendationSource, RecommendationType, StaffRecommendation,
    TransferRequestStatus,
};
use crate::transfers::scouting::breakout::LeaguePerformanceLookup;
use crate::transfers::scouting::exposure::{
    FreeAgentBuyerContext, FreeAgentRecommendationSignals, OpportunisticFreeAgentScout,
};
use crate::transfers::scouting::recruitment::ScoutMonitoringSource;
use crate::transfers::scouting::recruitment::ScoutPlayerMonitoring;
use crate::transfers::value::PlayerValuationCalculator;
use crate::transfers::view::player::CountryPlayerLookup;
use crate::utils::IntegerUtils;
use crate::utils::PerformanceProfiler;
use crate::{
    Club, Country, Person, PlayerFieldPositionGroup, PlayerPositionType, PlayerStatusType,
    PositionCoverage, ReputationLevel, Team,
};

#[cfg(test)]
mod tests;

/// The tick every stage of the pass runs against. Bundled because the scan
/// wants the date and the January flag, the snapshot wants the price level and
/// the window, and threading four scalars through three stages is how a
/// parameter list gets to twelve.
#[derive(Clone, Copy)]
struct AdviceTick {
    date: NaiveDate,
    /// True inside a country's mid-season window — loan-listed targets score
    /// higher in January than they do in summer.
    is_january: bool,
    price_level: f32,
    current_window: Option<(NaiveDate, NaiveDate)>,
}

#[allow(dead_code)]
struct PlayerSnapshot {
    id: u32,
    club_id: u32,
    /// His passport. Read by the registration gate — a club at its
    /// league's foreigner quota cannot sign one more.
    country_id: u32,
    position: PlayerPositionType,
    position_group: PlayerFieldPositionGroup,
    /// Every group he can play in, not just the one his primary label
    /// falls in — see [`BuyerNeedPicture::role_for`].
    coverage: PositionCoverage,
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
    /// Most recent circulation diagnosis of why the market stalled.
    last_block: Option<AvailabilityBlockReason>,
    /// Performance-breakout discovery score (0..100), computed once
    /// here from the league performance lookup so the listed-star
    /// sweep and the scout-network scorer share one number.
    breakout_score: f32,
}

/// A recommendation staged against the club that should receive it. The
/// scan runs in parallel and cannot touch a plan, so every source stages
/// here and [`AdviceCommit`] does the writing.
struct RecommendationAction {
    club_id: u32,
    recommendation: StaffRecommendation,
}

/// Every sellable player in the country, as the recommendation pass reads him.
/// Built once and shared by all six sources below, so a target's valuation and
/// breakout score are computed once per week rather than once per interested club.
struct SnapshotPass;

impl SnapshotPass {
    fn build(
        country: &Country,
        tick: AdviceTick,
        performance_lookup: &LeaguePerformanceLookup,
    ) -> Vec<PlayerSnapshot> {
        let date = tick.date;
        let price_level = tick.price_level;
        let current_window = tick.current_window;
        // Snapshot pass (PARALLEL): pure per-player reads (valuation,
        // breakout score) — clubs fan out, ordered flatten keeps the
        // snapshot sequence identical to the serial walk.
        let snapshot_stage = PerformanceProfiler::stage_scope("recs_snapshots", 4);
        let all_snapshots: Vec<PlayerSnapshot> = {
            let country: &Country = country;
            country
                .clubs
                .par_iter()
                .map(|club| {
                    let mut all_snapshots: Vec<PlayerSnapshot> = Vec::new();
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

                            let skill_ability = PlayerView::position_evaluation_ability(player);
                            let player_age = player.age(date);
                            let estimated_potential = skill_ability
                                + ScoutJudgement::estimate_growth_potential(
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
                                country_id: player.country_id,
                                position: player.position(),
                                position_group: player.position().position_group(),
                                coverage: PositionCoverage::of(&player.positions),
                                ability: skill_ability,
                                estimated_potential,
                                age: player_age,
                                estimated_value: value.amount,
                                contract_months_remaining: contract_months,
                                club_in_debt,
                                parent_club_reputation: rep_level.clone(),
                                parent_club_score,
                                parent_league_reputation,
                                is_loan_listed: player.statuses.has(PlayerStatusType::Loa),
                                is_listed: player.statuses.has(PlayerStatusType::Lst),
                                is_transfer_requested: player.statuses.has(PlayerStatusType::Req),
                                is_unhappy: player.statuses.has(PlayerStatusType::Unh),
                                ambition: player.attributes.ambition,
                                world_reputation: player.player_attributes.world_reputation,
                                current_reputation: player.player_attributes.current_reputation,
                                // Recommendation feature row: regressed value.
                                average_rating,
                                appearances,
                                is_transfer_protected: player
                                    .is_transfer_protected(date, current_window),
                                days_available: player.days_available(date),
                                recent_interest_count: player
                                    .availability_market_state()
                                    .map(|s| s.recent_interest(date))
                                    .unwrap_or(0),
                                failed_scans: player
                                    .availability_market_state()
                                    .map(|s| s.failed_scans)
                                    .unwrap_or(0),
                                last_block: player
                                    .availability_market_state()
                                    .and_then(|s| s.last_block.map(|(_, reason)| reason)),
                                breakout_score,
                            });
                        }
                    }
                    all_snapshots
                })
                .collect::<Vec<Vec<PlayerSnapshot>>>()
                .into_iter()
                .flatten()
                .collect()
        };
        drop(snapshot_stage);
        all_snapshots
    }
}

/// One club's recommendation scan. Everything the six sources read is
/// resolved once here — the staff, the reputation and wage picture, the budget
/// cap and the plausibility context — so a source can veto an unrealistic
/// target without re-walking the club.
///
/// Read-only by construction: the scan never touches a plan. Each source is
/// handed the staged `actions` and appends to it, because the sources genuinely
/// interleave — every one of them checks what the earlier ones already staged,
/// both for the per-club cap and to avoid recommending the same name twice.
struct ClubAdviceScan<'a> {
    country: &'a Country,
    club: &'a Club,
    plan: &'a ClubTransferPlan,
    team: &'a Team,
    resolved: ResolvedStaff<'a>,
    snapshots: &'a [PlayerSnapshot],
    lookup: &'a CountryPlayerLookup,
    date: NaiveDate,
    is_january: bool,
    avg_ability: u8,
    club_rep: ReputationLevel,
    club_rep_score: f32,
    club_world_rep: i16,
    club_league_reputation: u16,
    club_total_wages: u32,
    club_wage_budget: u32,
    buyer_plaus_ctx: BuyerPlausibilityContext,
    already_recommended: Vec<u32>,
    max_recommend_value: f64,
    memory_recommender_id: Option<u32>,
}

/// What the small-club sweeps read beyond the scan itself: whose eyes are
/// doing the reading, and the ceiling they read up to. Each sweep recomputes
/// how much of that ceiling is left from what the sweeps before it staged.
#[derive(Clone, Copy)]
struct CoachHunt {
    coach_id: u32,
    coach_judging: u8,
    coach_judging_pot: u8,
    rec_cap: usize,
    is_small_club: bool,
}

impl<'a> ClubAdviceScan<'a> {
    /// `None` when this club has nothing to scan for — no squad, no initialised
    /// plan, or a recommendation list already at the cap the commit applies.
    fn open(
        country: &'a Country,
        club: &'a Club,
        snapshots: &'a [PlayerSnapshot],
        lookup: &'a CountryPlayerLookup,
        tick: AdviceTick,
    ) -> Option<Self> {
        let date = tick.date;
        if club.teams.teams.is_empty() {
            return None;
        }
        let plan = &club.transfer_plan;
        if !plan.initialized {
            return None;
        }

        // Cap: 10 recommendations per club per window — matches
        // the pass-2 apply caps. The old early-return at 6
        // silently disabled the small-club loan-bargain /
        // free-agent / game-time sections for the rest of the
        // window once six recs accumulated (they only clear at
        // window reset).
        if plan.staff_recommendations.len() >= 10 {
            return None;
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
        let buyer_plaus_ctx = BuyerPlausibilityContext::build(country, club, date);

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

        Some(Self {
            country,
            club,
            plan,
            team,
            resolved,
            snapshots,
            lookup,
            date,
            is_january: tick.is_january,
            avg_ability,
            club_rep,
            club_rep_score,
            club_world_rep,
            club_league_reputation,
            club_total_wages,
            club_wage_budget,
            buyer_plaus_ctx,
            already_recommended,
            max_recommend_value,
            memory_recommender_id,
        })
    }

    /// True when adding `player_id` to the recommendation list would push an
    /// impossible move (Maximenko-class step-down) into the pipeline.
    fn rejects(&self, player_id: u32, is_loan: bool) -> bool {
        let date = self.date;
        let summary = match self.lookup.find_summary(self.country, player_id, date) {
            Some(s) => s,
            None => return false,
        };
        matches!(
            TransferPlausibilityBuilder::evaluate_summary(
                &self.buyer_plaus_ctx,
                &summary,
                is_loan,
                true,
                date,
                None,
            ),
            Some(TransferPlausibilityVerdict::HardReject(_))
        )
    }

    /// Who signs a recommendation that came from the listed-star or
    /// free-agent sweeps rather than from a named scout: the director of
    /// football if the club has one, else its first scout, else the head coach.
    fn listed_recommender_id(&self) -> u32 {
        self.resolved
            .director_of_football
            .map(|s| s.id)
            .or_else(|| self.resolved.scouts.first().map(|s| s.id))
            .unwrap_or(self.team.staffs.head_coach().id)
    }

    /// How good the club's eyes are — the spread on every "which of these do
    /// we like best" read in those two sweeps.
    fn listed_recommender_judging(&self) -> u8 {
        self.resolved
            .director_of_football
            .map(|s| s.staff_attributes.knowledge.judging_player_ability)
            .unwrap_or_else(|| self.resolved.best_scout_judging_ability())
    }

    /// The six sources, in the order the old body ran them. The order is
    /// load-bearing: each source reads what the earlier ones staged.
    fn run(&self) -> Vec<RecommendationAction> {
        let mut actions: Vec<RecommendationAction> = Vec::new();
        self.scout_memories(&mut actions);
        self.scout_network(&mut actions);
        self.listed_stars(&mut actions);
        self.expiring_contracts(&mut actions);
        self.director_bargains(&mut actions);
        self.small_club_hunt(&mut actions);
        actions
    }

    /// The scouts' standing memory: names already on the club's books from
    /// past viewings, re-surfaced when the sighting is recent enough and the
    /// scout saw enough of him to have an opinion.
    fn scout_memories(&self, actions: &mut Vec<RecommendationAction>) {
        let club = self.club;
        let plan = self.plan;
        let date = self.date;
        let avg_ability = self.avg_ability;
        let already_recommended = &self.already_recommended;
        let max_recommend_value = self.max_recommend_value;
        let memory_recommender_id = self.memory_recommender_id;
        let plausibility_rejects = |player_id: u32, is_loan: bool| self.rejects(player_id, is_loan);

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
    }

    /// The weekly discovery roll. Each scout rolls against his own judging
    /// attribute, then ranks the country's snapshots by what he *perceives*
    /// them to be — so a sharp scout converges on the genuine best target and
    /// a weaker one legitimately disagrees.
    fn scout_network(&self, actions: &mut Vec<RecommendationAction>) {
        let club = self.club;
        let resolved = &self.resolved;
        let date = self.date;
        let is_january = self.is_january;
        let avg_ability = self.avg_ability;
        let club_rep = self.club_rep.clone();
        let club_rep_score = self.club_rep_score;
        let already_recommended = &self.already_recommended;
        let max_recommend_value = self.max_recommend_value;
        let all_snapshots = self.snapshots;
        let plausibility_rejects = |player_id: u32, is_loan: bool| self.rejects(player_id, is_loan);

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
                        TierBands::tier_target_ceiling_score(club_rep_score, p.position_group);
                    p.club_id != club.id
                        && !club.is_rival(p.club_id)
                        && !p.is_transfer_protected
                        && p.ability >= avg_ability.saturating_sub(10)
                        && p.ability <= ceiling
                        && (max_recommend_value <= 0.0 || p.estimated_value <= max_recommend_value)
                        && TierBands::rep_level_value(&p.parent_club_reputation)
                            >= TierBands::rep_level_value(&min_source_rep)
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
                if TierBands::rep_level_value(&cand.parent_club_reputation)
                    < TierBands::rep_level_value(&club_rep)
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
    }

    /// Players who have advertised themselves as available — Lst, Req or Unh —
    /// surfacing to the clubs whose tier window matches their quality.
    fn listed_stars(&self, actions: &mut Vec<RecommendationAction>) {
        let club = self.club;
        let plan = self.plan;
        let team = self.team;
        let date = self.date;
        let club_rep_score = self.club_rep_score;
        let club_world_rep = self.club_world_rep;
        let club_league_reputation = self.club_league_reputation;
        let club_total_wages = self.club_total_wages;
        let club_wage_budget = self.club_wage_budget;
        let already_recommended = &self.already_recommended;
        let max_recommend_value = self.max_recommend_value;
        let all_snapshots = self.snapshots;
        let listed_recommender_id = self.listed_recommender_id();
        let plausibility_rejects = |player_id: u32, is_loan: bool| self.rejects(player_id, is_loan);

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

        // Per-group squad-fit projection and best-in-group bar, walked once
        // per club rather than once per candidate.
        let buyer_fit_by_group = self.buyer_fit_by_group();
        let buyer_best_in_group = self.buyer_best_in_group();
        // Aging starter per group: any player at-tier in this group
        // who's 30+ — succession candidate that opens up a slot.
        let buyer_has_aging_starter = |group: PlayerFieldPositionGroup| -> bool {
            let baseline = TierBands::tier_starter_ca_score(club_rep_score, group);
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
        // The lowest bar among those requests. A request is an
        // ask for a LEVEL, not for a body in the group: the
        // improvement gate below waives "must be an upgrade"
        // for a candidate the club has an open request for,
        // and that waiver must only ever reach the players the
        // request actually describes.
        let open_request_bar = |group: PlayerFieldPositionGroup| -> Option<u8> {
            plan.transfer_requests
                .iter()
                .filter(|r| {
                    r.position.position_group() == group
                        && r.status != TransferRequestStatus::Fulfilled
                        && r.status != TransferRequestStatus::Abandoned
                })
                .map(|r| r.min_ability)
                .min()
        };

        // Built ONCE per club: `buyer_has_aging_starter` walks the
        // whole squad, so resolving a candidate's judged role must
        // not re-derive the picture per candidate.
        let need_picture = BuyerNeedPicture {
            open_request: PlayerFieldPositionGroup::ALL.map(buyer_open_request_for),
            aging_starter: PlayerFieldPositionGroup::ALL.map(buyer_has_aging_starter),
            best_in_group: PlayerFieldPositionGroup::ALL
                .map(|g| buyer_best_in_group.get(&g).copied().unwrap_or(0)),
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

                // Judge him in the shirt this club actually wants
                // filled, which need not be the one his primary
                // label falls in.
                let judged_group = need_picture.role_for(p.coverage, p.position_group);

                let view = ListedTargetView {
                    ability: p.ability,
                    nationality_country_id: p.country_id,
                    estimated_potential: p.estimated_potential,
                    age: p.age,
                    estimated_value: p.estimated_value,
                    position_group: judged_group,
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
                    contract_months_remaining: p.contract_months_remaining.min(i16::MAX as u32)
                        as i16,
                    low_usage: p.appearances < 8,
                    recent_interest_count: p.recent_interest_count,
                    failed_scans: p.failed_scans,
                    last_block: p.last_block,
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
                        .get(&judged_group)
                        .copied()
                        .unwrap_or(0),
                    has_open_request: open_request_bar(judged_group).is_some_and(|bar| {
                        p.ability
                            .saturating_add(BuyerNeedPicture::STAFF_TIP_ABILITY_TOLERANCE)
                            >= bar
                    }),
                    has_aging_starter: buyer_has_aging_starter(judged_group),
                    // In-window listed-star sweep: only publicly
                    // available players (Lst/Req/Unh, or Loa+breakout).
                    form_discovery_mode: false,
                    fit: buyer_fit_by_group
                        .get(&judged_group)
                        .copied()
                        .unwrap_or_else(SquadFitSnapshot::disabled),
                };
                match ListedTargetScreen::evaluate(&view, &ctx) {
                    ListedTargetVerdict::Accept(score) => Some((p, score)),
                    ListedTargetVerdict::Reject(_) => None,
                }
            })
            .collect();

        let mut ranked = scored_targets;
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        self.stage_listed_stars(ranked, listed_recommender_id, actions);
    }

    /// Per-group squad-fit projection at the buying club. Cached because the
    /// listed-star filter would otherwise re-scan the squad once per candidate.
    fn buyer_fit_by_group(&self) -> HashMap<PlayerFieldPositionGroup, SquadFitSnapshot> {
        let country = self.country;
        let club = self.club;
        let date = self.date;
        let registration = SquadRegistrationLimits::new(country.id, &country.regulations);
        // Per-group squad-fit projection at the buying club —
        // cached so the filter doesn't re-scan the squad N times.
        let buyer_fit_by_group: HashMap<PlayerFieldPositionGroup, SquadFitSnapshot> = [
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ]
        .into_iter()
        .map(|g| (g, SquadFitSnapshot::build(club, g, date, registration)))
        .collect();
        buyer_fit_by_group
    }

    /// Per-group best current ability at the buying club — the bar a listed
    /// target has to clear to be an upgrade rather than another squad player.
    fn buyer_best_in_group(&self) -> HashMap<PlayerFieldPositionGroup, u8> {
        let team = self.team;
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
        buyer_best_in_group
    }

    /// Stages the top of the ranked listed-star slate, stopping at six — the
    /// listed sweep does not get to fill a club's whole recommendation list.
    fn stage_listed_stars(
        &self,
        ranked: Vec<(&PlayerSnapshot, f32)>,
        listed_recommender_id: u32,
        actions: &mut Vec<RecommendationAction>,
    ) {
        let club = self.club;
        let plan = self.plan;
        let date = self.date;
        let club_rep = self.club_rep.clone();
        for (target, _score) in ranked.iter().take(3) {
            let current_recs = plan.staff_recommendations.len()
                + actions.iter().filter(|a| a.club_id == club.id).count();
            if current_recs >= 6 {
                break;
            }

            let rec_type = if TierBands::rep_level_value(&target.parent_club_reputation)
                > TierBands::rep_level_value(&club_rep)
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
    }

    /// Quality players whose contracts are running down, circulated to clubs
    /// where they are a plausible, affordable depth or future-resale add even
    /// without an open positional request.
    fn expiring_contracts(&self, actions: &mut Vec<RecommendationAction>) {
        let club = self.club;
        let plan = self.plan;
        let team = self.team;
        let date = self.date;
        let avg_ability = self.avg_ability;
        let club_rep = self.club_rep.clone();
        let club_total_wages = self.club_total_wages;
        let club_wage_budget = self.club_wage_budget;
        let already_recommended = &self.already_recommended;
        let all_snapshots = self.snapshots;
        let listed_recommender_id = self.listed_recommender_id();
        let listed_recommender_judging = self.listed_recommender_judging();
        let plausibility_rejects = |player_id: u32, is_loan: bool| self.rejects(player_id, is_loan);

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
                ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur => 10,
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
                            || actions
                                .iter()
                                .any(|a| a.club_id == club.id && a.recommendation.player_id == p.id)
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
                // Rank on what this club's recruitment department
                // BELIEVES, not on true ability — the same
                // correction the DoF bargain hunt above already
                // carries. A true-ability sort made every club with
                // an opportunistic slot free recommend the same
                // name, then dressed the report up with a noise
                // term that changed nothing about the choice.
                fa_targets.sort_by(|a, b| {
                    ClubOpinion::of(club.id, b.id, listed_recommender_judging)
                        .believed_ability(b.ability)
                        .partial_cmp(
                            &ClubOpinion::of(club.id, a.id, listed_recommender_judging)
                                .believed_ability(a.ability),
                        )
                        .unwrap_or(Ordering::Equal)
                });

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
    }

    /// The director of football's own eye — a second, independent roll that
    /// does not go through the scout network at all.
    fn director_bargains(&self, actions: &mut Vec<RecommendationAction>) {
        let club = self.club;
        let resolved = &self.resolved;
        let date = self.date;
        let avg_ability = self.avg_ability;
        let already_recommended = &self.already_recommended;
        let all_snapshots = self.snapshots;
        let plausibility_rejects = |player_id: u32, is_loan: bool| self.rejects(player_id, is_loan);

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
                            && !actions
                                .iter()
                                .any(|a| a.club_id == club.id && a.recommendation.player_id == p.id)
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
    }

    /// Small clubs live on cheap loans, free agents and the surplus of bigger
    /// clubs, and their staff hunt for all three whether or not a scout
    /// network exists. Three sub-sweeps, in the order the old body ran them.
    fn small_club_hunt(&self, actions: &mut Vec<RecommendationAction>) {
        let club = self.club;
        let plan = self.plan;
        let team = self.team;
        let club_rep = self.club_rep.clone();

        // ── Small club staff: aggressive loan/bargain hunting ──
        // Small clubs rely on their staff to find cheap deals, loans,
        // free agents, and surplus players from bigger clubs.
        // Even a head coach at a small club knows what the squad needs.
        let is_small_club = matches!(
            club_rep,
            ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur
        );
        let is_mid_club = club_rep == ReputationLevel::National;
        if !is_small_club && !is_mid_club {
            return;
        }

        let rec_cap = if is_small_club { 10 } else { 8 };
        let current_recs = plan.staff_recommendations.len()
            + actions.iter().filter(|a| a.club_id == club.id).count();
        if current_recs >= rec_cap {
            return;
        }
        let remaining = rec_cap - current_recs;

        // Coach recommends players available on loan
        let head_coach = team.staffs.head_coach();
        let hunt = CoachHunt {
            coach_id: head_coach.id,
            coach_judging: head_coach.staff_attributes.knowledge.judging_player_ability,
            coach_judging_pot: head_coach
                .staff_attributes
                .knowledge
                .judging_player_potential,
            rec_cap,
            is_small_club,
        };

        // Order is load-bearing: each sweep reads what the ones before it staged.
        self.cheap_loans(hunt, remaining, actions);
        self.free_agent_bargains(hunt, actions);
        self.game_time_seekers(hunt, actions);
    }

    /// Loan-listed players the club could actually afford, drawn by weight over
    /// what this coach *believes* rather than off a true-ability sort — every
    /// club used to read the loan market off the same ordering and float the
    /// same three names, week after week, until the listings moved.
    fn cheap_loans(
        &self,
        hunt: CoachHunt,
        remaining: usize,
        actions: &mut Vec<RecommendationAction>,
    ) {
        let club = self.club;
        let date = self.date;
        let avg_ability = self.avg_ability;
        let already_recommended = &self.already_recommended;
        let all_snapshots = self.snapshots;
        let coach_id = hunt.coach_id;
        let coach_judging = hunt.coach_judging;
        let coach_judging_pot = hunt.coach_judging_pot;
        let plausibility_rejects = |player_id: u32, is_loan: bool| self.rejects(player_id, is_loan);

        // ── Cheap loan targets (loan-listed players the club could afford) ──
        let loan_targets: Vec<&PlayerSnapshot> = all_snapshots
            .iter()
            .filter(|p| {
                p.club_id != club.id
                    && !club.is_rival(p.club_id)
                    && !p.is_transfer_protected
                    && p.is_loan_listed
                    && p.ability >= avg_ability.saturating_sub(8)
                    && !already_recommended.contains(&p.id)
                    && !actions
                        .iter()
                        .any(|a| a.club_id == club.id && a.recommendation.player_id == p.id)
                    && !plausibility_rejects(p.id, true)
            })
            .collect();
        // A weighted draw over what this coach BELIEVES,
        // rather than the top three rows of a true-ability
        // sort: every club read the loan market off the
        // same ordering and floated the same three names,
        // week after week, until the listings moved. No
        // pre-sort — the slate carries the scores.
        let loan_slate: Vec<(u32, f32)> = loan_targets
            .iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    i as u32,
                    ClubOpinion::of(club.id, p.id, coach_judging).believed_ability(p.ability)
                        / 100.0,
                )
            })
            .collect();
        let loan_picks: Vec<&PlayerSnapshot> =
            InterestDraw::pick_several(&loan_slate, remaining.min(3))
                .into_iter()
                .map(|i| loan_targets[i as usize])
                .collect();

        for target in loan_picks.iter() {
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
    }

    /// Contracts running down at other clubs, taken up to whatever the loan
    /// sweep above left under the cap.
    fn free_agent_bargains(&self, hunt: CoachHunt, actions: &mut Vec<RecommendationAction>) {
        let club = self.club;
        let plan = self.plan;
        let date = self.date;
        let avg_ability = self.avg_ability;
        let already_recommended = &self.already_recommended;
        let all_snapshots = self.snapshots;
        let coach_id = hunt.coach_id;
        let coach_judging = hunt.coach_judging;
        let coach_judging_pot = hunt.coach_judging_pot;
        let rec_cap = hunt.rec_cap;
        let plausibility_rejects = |player_id: u32, is_loan: bool| self.rejects(player_id, is_loan);

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
                        && !actions
                            .iter()
                            .any(|a| a.club_id == club.id && a.recommendation.player_id == p.id)
                        && !plausibility_rejects(p.id, false)
                })
                .collect();
            free_targets.sort_by(|a, b| {
                ClubOpinion::of(club.id, b.id, coach_judging)
                    .believed_ability(b.ability)
                    .partial_cmp(
                        &ClubOpinion::of(club.id, a.id, coach_judging).believed_ability(a.ability),
                    )
                    .unwrap_or(Ordering::Equal)
            });

            for target in free_targets.iter().take(remaining_after_loans.min(2)) {
                let ability_error = (20i16 - coach_judging as i16).max(1) as i32;
                let potential_error = (20i16 - coach_judging_pot as i16).max(1) as i32;

                let assessed_ability = (target.ability as i32
                    + IntegerUtils::random(-ability_error, ability_error))
                .clamp(1, 200) as u8;
                let assessed_potential = (target.estimated_potential as i32
                    + IntegerUtils::random(-potential_error, potential_error))
                .clamp(1, 200) as u8;

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
    }

    /// Young players at bigger clubs who are not loan-listed yet but sit below
    /// their own club's average — they would benefit from a loan, and a small
    /// club is exactly who benefits from asking. Small clubs only.
    fn game_time_seekers(&self, hunt: CoachHunt, actions: &mut Vec<RecommendationAction>) {
        let club = self.club;
        let plan = self.plan;
        let date = self.date;
        let avg_ability = self.avg_ability;
        let club_rep = self.club_rep.clone();
        let already_recommended = &self.already_recommended;
        let all_snapshots = self.snapshots;
        let coach_id = hunt.coach_id;
        let coach_judging = hunt.coach_judging;
        let coach_judging_pot = hunt.coach_judging_pot;
        let rec_cap = hunt.rec_cap;
        let is_small_club = hunt.is_small_club;
        let plausibility_rejects = |player_id: u32, is_loan: bool| self.rejects(player_id, is_loan);

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
                        && TierBands::rep_level_value(&p.parent_club_reputation)
                            > TierBands::rep_level_value(&club_rep)
                        && !p.is_loan_listed
                        && !already_recommended.contains(&p.id)
                        && !actions
                            .iter()
                            .any(|a| a.club_id == club.id && a.recommendation.player_id == p.id)
                        && !plausibility_rejects(p.id, true)
                })
                .collect();
            // Believed ceiling, not the true one. Potential
            // is the least visible thing about a player, so
            // a shared true-potential ordering was the
            // strongest convergence of the lot.
            game_time_seekers.sort_by(|a, b| {
                ClubOpinion::of(club.id, b.id, coach_judging_pot)
                    .believed_ability(b.estimated_potential)
                    .partial_cmp(
                        &ClubOpinion::of(club.id, a.id, coach_judging_pot)
                            .believed_ability(a.estimated_potential),
                    )
                    .unwrap_or(Ordering::Equal)
            });

            for target in game_time_seekers.iter().take(remaining_after_free.min(2)) {
                let ability_error = (20i16 - coach_judging as i16).max(1) as i32;
                let potential_error = (20i16 - coach_judging_pot as i16).max(1) as i32;

                let assessed_ability = (target.ability as i32
                    + IntegerUtils::random(-ability_error, ability_error))
                .clamp(1, 200) as u8;
                let assessed_potential = (target.estimated_potential as i32
                    + IntegerUtils::random(-potential_error, potential_error))
                .clamp(1, 200) as u8;

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

/// The single writer. The scan runs in parallel and stages; this walks the
/// clubs once, in club order, and commits each bucket into its own plan — so
/// the cap check only ever reads the plan it is about to write.
struct AdviceCommit;

impl AdviceCommit {
    fn apply(country: &mut Country, actions: Vec<Vec<RecommendationAction>>, date: NaiveDate) {
        // Pass 2: Push recommendations into club transfer plans (small clubs
        // get a higher cap). The scan collects one bucket per club, in club
        // order, so the commit is an indexed zip rather than a club scan per
        // action — and each club's cap check only ever reads its own plan,
        // so the clubs commit in parallel.
        let apply_stage = PerformanceProfiler::stage_scope("recs_apply", 4);
        country
            .clubs
            .par_iter_mut()
            .zip(actions.into_par_iter())
            .for_each(|(club, club_actions)| {
                for action in club_actions {
                    debug_assert_eq!(
                        club.id, action.club_id,
                        "staff recommendations: staged actions must belong to their own club"
                    );
                    let team = club.teams.teams.first();
                    let rep = team
                        .map(|t| t.reputation.level())
                        .unwrap_or(ReputationLevel::Amateur);
                    let rep_score = team.map(|t| t.reputation.overall_score()).unwrap_or(0.0);
                    let cap = StaffRecommendations::staff_recommendation_cap_score(rep, rep_score);
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
                            let mut row = ScoutPlayerMonitoring::new(
                                id,
                                recommender_id,
                                player_id,
                                source,
                                date,
                            );
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
            });
        drop(apply_stage);
    }
}

/// The pass itself: snapshot, scan, commit.
pub(in crate::transfers::pipeline) struct StaffAdvicePass;

impl StaffAdvicePass {
    pub(in crate::transfers::pipeline) fn run(country: &mut Country, date: NaiveDate) {
        // One country walk so per-candidate plausibility re-checks below
        // resolve summaries via hash probe instead of a country scan.
        let player_lookup = PerformanceProfiler::stage("recs_player_lookup", 4, || {
            CountryPlayerLookup::build(country)
        });

        let window_mgr = TransferWindowManager::for_country(country.id, &country.code, date);
        let tick = AdviceTick {
            date,
            is_january: MarketCadence::is_mid_season_window_for(&country.code, date),
            price_level: country.settings.pricing.price_level,
            current_window: window_mgr.current_window_dates(country.id, date),
        };

        // Per-country scoring-chart + recent-award lookup, built once for
        // the whole pass so the breakout score on each snapshot is cheap.
        let performance_lookup = LeaguePerformanceLookup::build(country);
        let all_snapshots = SnapshotPass::build(country, tick, &performance_lookup);

        // Pass 1b (PARALLEL): each club's recommendation scan reads only
        // the shared snapshots / lookup / country and stages its actions
        // locally, so the clubs fan out across the pool. Ordered collect
        // keeps the applied sequence identical to the serial loop; RNG
        // draws move to the executing worker's thread-seeded stream — the
        // same order-of-execution dependence the tick already has at
        // country granularity.
        let scan_stage = PerformanceProfiler::stage_scope("recs_club_scan", 4);
        let actions: Vec<Vec<RecommendationAction>> = {
            let country: &Country = country;
            let snapshots = all_snapshots.as_slice();
            let lookup = &player_lookup;
            country
                .clubs
                .par_iter()
                .map(|club| {
                    ClubAdviceScan::open(country, club, snapshots, lookup, tick)
                        .map(|scan| scan.run())
                        .unwrap_or_default()
                })
                .collect::<Vec<Vec<RecommendationAction>>>()
        };
        drop(scan_stage);

        AdviceCommit::apply(country, actions, date);
    }
}
