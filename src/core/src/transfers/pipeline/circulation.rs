//! Weekly market-circulation / diagnosis pass for signed, available
//! players. The buyer-centric "listed-star sweep" in
//! [`super::recommendations`] decides which available players a *given
//! club* should pursue; this pass inverts that view — for each available
//! player it asks "is anyone interested, and if not, why?" — and records
//! the answer on the player's durable
//! [`crate::club::player::transfer::AvailabilityMarketState`].
//!
//! The result is the "natural market reaction or a clear reason why the
//! market is blocked" the design calls for:
//!   * a player some club is monitoring / shortlisting / negotiating, or
//!     whom the plausibility-checked buyer scan would accept, has his
//!     interest recorded (which resets the staleness streak and feeds the
//!     softening curve back toward neutral);
//!   * a player no plausible buyer wants accumulates a dominant
//!     [`crate::club::player::transfer::AvailabilityBlockReason`] — asking
//!     price too high, wage too high, reputation too high, no affordable
//!     squad need, won't step down, country/region blocked — so a quality
//!     player never just sits with no interest and no explanation.
//!
//! The pass never signs anyone and never bypasses a gate: it only reads
//! the same plausibility + listed-target evaluators the live pipeline
//! uses and writes diagnostics. Behavioural feedback (price / wage
//! softening, circulation lift) is applied where those evaluators run,
//! driven by the state this pass maintains.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use log::debug;

use crate::club::player::transfer::AvailabilityBlockReason;
use crate::transfers::negotiation::NegotiationStatus;
use crate::transfers::pipeline::TransferRequestStatus;
use crate::transfers::pipeline::breakout::LeaguePerformanceLookup;
use crate::transfers::pipeline::exposure::MarketDiscoveryDiagnosis;
use crate::transfers::pipeline::plausibility::{
    TransferPlausibilityBuilder, TransferPlausibilityEvaluator, TransferPlausibilityVerdict,
};
use crate::transfers::pipeline::processor::PipelineProcessor;
use crate::transfers::pipeline::recommendations::{
    BuyerContext, ListedTargetVerdict, ListedTargetView, evaluate_listed_target,
};
use crate::transfers::window::PlayerValuationCalculator;
use crate::{Club, Country, Person, PlayerFieldPositionGroup, PlayerStatusType};

/// Minimum days on the market before the pass spends a buyer scan to
/// diagnose a no-interest player. Fresher than this and the market simply
/// hasn't had a fair look yet.
const DIAGNOSE_MIN_DAYS: i64 = 14;

/// Backstop on full buyer scans per country per pass — a pathological
/// guard for very large leagues, never hit in practice. Players beyond
/// the cap retain their previous diagnosis; the count is logged.
const DIAGNOSE_CAP: usize = 200;

/// What the pass decided for one available player.
enum CirculationAction {
    /// A plausible buyer is interested (existing pursuit or a fresh scan
    /// accept). Resets the staleness streak.
    Interest,
    /// On the market, but too freshly to draw a conclusion yet.
    TooEarly,
    /// No plausible buyer — record the dominant reason.
    Blocked(AvailabilityBlockReason),
}

/// Precomputed buyer snapshot reused across every available player so the
/// scan doesn't re-walk a club's reputation / wage / squad data per
/// candidate. Built once per club at the start of the pass. Shared with the
/// year-round breakout watch ([`super::breakout_watch`]) so both passes
/// derive the buyer's [`BuyerContext`] from one place.
pub(in crate::transfers::pipeline) struct BuyerScan {
    rep_score: f32,
    world_rep: i16,
    league_rep: u16,
    total_wages: u32,
    wage_budget: u32,
    plan_total_budget: f64,
    max_recommend_value: f64,
    best_in_group: HashMap<PlayerFieldPositionGroup, u8>,
    open_request_groups: HashSet<PlayerFieldPositionGroup>,
    aging_groups: HashSet<PlayerFieldPositionGroup>,
}

impl BuyerScan {
    pub(in crate::transfers::pipeline) fn build(
        country: &Country,
        club: &Club,
        date: NaiveDate,
    ) -> Option<BuyerScan> {
        let team = club.teams.teams.first()?;
        let rep_score = team.reputation.overall_score();
        let world_rep = team.reputation.world as i16;
        let league_rep = team
            .league_id
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);
        let total_wages: u32 = club.teams.iter().map(|t| t.get_annual_salary()).sum();
        let wage_budget: u32 = club
            .finance
            .wage_budget
            .as_ref()
            .map(|b| b.amount.max(0.0) as u32)
            .unwrap_or(total_wages.saturating_mul(11) / 10);
        let plan_total_budget = club.transfer_plan.total_budget;
        let max_recommend_value = plan_total_budget * 2.0;

        let mut best_in_group: HashMap<PlayerFieldPositionGroup, u8> = HashMap::new();
        for p in team.players.players.iter() {
            let g = p.position().position_group();
            let ca = p.player_attributes.current_ability;
            best_in_group
                .entry(g)
                .and_modify(|v| {
                    if ca > *v {
                        *v = ca
                    }
                })
                .or_insert(ca);
        }

        let mut open_request_groups: HashSet<PlayerFieldPositionGroup> = HashSet::new();
        for r in club.transfer_plan.transfer_requests.iter() {
            if r.status != TransferRequestStatus::Fulfilled
                && r.status != TransferRequestStatus::Abandoned
            {
                open_request_groups.insert(r.position.position_group());
            }
        }

        let mut aging_groups: HashSet<PlayerFieldPositionGroup> = HashSet::new();
        for group in [
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ] {
            let baseline = PipelineProcessor::tier_starter_ca_score(rep_score, group);
            let has_aging = team.players.players.iter().any(|p| {
                p.position().position_group() == group
                    && p.age(date) >= 30
                    && p.player_attributes.current_ability + 5 >= baseline
            });
            if has_aging {
                aging_groups.insert(group);
            }
        }

        Some(BuyerScan {
            rep_score,
            world_rep,
            league_rep,
            total_wages,
            wage_budget,
            plan_total_budget,
            max_recommend_value,
            best_in_group,
            open_request_groups,
            aging_groups,
        })
    }

    /// Build the per-player [`BuyerContext`] the listed-target evaluator
    /// expects, reading the precomputed per-group signals. `form_discovery`
    /// is `false` for the availability-driven diagnosis / listed sweep and
    /// `true` for the year-round breakout watch (which may surface a player
    /// who has not advertised availability).
    pub(in crate::transfers::pipeline) fn buyer_context(
        &self,
        group: PlayerFieldPositionGroup,
        form_discovery: bool,
    ) -> BuyerContext {
        BuyerContext {
            buyer_rep_score: self.rep_score,
            buyer_world_rep: self.world_rep,
            buyer_league_reputation: self.league_rep,
            buyer_total_wages: self.total_wages,
            buyer_wage_budget: self.wage_budget,
            plan_total_budget: self.plan_total_budget,
            max_recommend_value: self.max_recommend_value,
            buyer_best_in_group: self.best_in_group.get(&group).copied().unwrap_or(0),
            has_open_request: self.open_request_groups.contains(&group),
            has_aging_starter: self.aging_groups.contains(&group),
            form_discovery_mode: form_discovery,
        }
    }
}

impl PipelineProcessor {
    /// Weekly market-circulation / diagnosis pass. Runs on the same
    /// cadence as the recommendation sweep and right after it, so the
    /// interest the sweep generated this tick is already visible.
    pub fn circulate_available_players(country: &mut Country, date: NaiveDate) {
        if !Self::should_evaluate(date) {
            return;
        }

        // ── Which players is any club already concretely pursuing? ──
        // Monitoring rows, shortlist candidates, staff recommendations,
        // and live negotiations all count as "the market has shown
        // interest recently".
        let mut interested: HashSet<u32> = HashSet::new();
        for club in &country.clubs {
            let plan = &club.transfer_plan;
            for m in &plan.scout_monitoring {
                if m.is_active_interest() {
                    interested.insert(m.player_id);
                }
            }
            for s in &plan.shortlists {
                for c in &s.candidates {
                    interested.insert(c.player_id);
                }
            }
            for r in &plan.staff_recommendations {
                interested.insert(r.player_id);
            }
        }
        for n in country.transfer_market.negotiations.values() {
            if matches!(
                n.status,
                NegotiationStatus::Pending | NegotiationStatus::Countered
            ) {
                interested.insert(n.player_id);
            }
        }

        // ── Precompute buyer snapshots once, keyed by club id. ──
        let buyer_scans: HashMap<u32, BuyerScan> = country
            .clubs
            .iter()
            .filter_map(|c| BuyerScan::build(country, c, date).map(|s| (c.id, s)))
            .collect();

        // Scoring-chart + recent-award lookup, built once for the country.
        let performance_lookup = LeaguePerformanceLookup::build(country);

        let price_level = country.settings.pricing.price_level;

        // ── Phase 1: collect per-player actions (immutable). ──
        let mut actions: HashMap<u32, CirculationAction> = HashMap::new();
        let mut to_clear: Vec<u32> = Vec::new();
        let mut scans_done = 0usize;
        let mut scans_skipped_by_cap = 0usize;

        for seller in &country.clubs {
            let (seller_league_rep, seller_club_rep) =
                PlayerValuationCalculator::seller_context(country, seller);

            for team in &seller.teams.teams {
                for player in &team.players.players {
                    // Stale availability state on a no-longer-available
                    // player is collected for cleanup regardless.
                    if !player.is_market_available() {
                        if player.availability_market_state().is_some() {
                            to_clear.push(player.id);
                        }
                        continue;
                    }
                    if player.is_on_loan() {
                        continue;
                    }

                    let pid = player.id;

                    // Already being pursued → record interest, no scan.
                    if interested.contains(&pid) {
                        actions.insert(pid, CirculationAction::Interest);
                        continue;
                    }

                    let days = player.days_available(date);
                    if days < DIAGNOSE_MIN_DAYS {
                        actions.insert(pid, CirculationAction::TooEarly);
                        continue;
                    }

                    if scans_done >= DIAGNOSE_CAP {
                        scans_skipped_by_cap += 1;
                        continue;
                    }
                    scans_done += 1;

                    // Build the player view once (shared across buyers).
                    let group = player.position().position_group();
                    let ability = Self::position_evaluation_ability(player);
                    let player_age = player.age(date);
                    let estimated_potential = ability
                        + Self::estimate_growth_potential(
                            player_age,
                            player.skills.mental.determination,
                            player.skills.mental.work_rate,
                            player.skills.mental.composure,
                            player.skills.mental.anticipation,
                            ability,
                        );
                    let value = PlayerValuationCalculator::calculate_value_with_price_level(
                        player,
                        date,
                        price_level,
                        seller_league_rep,
                        seller_club_rep,
                    )
                    .amount;
                    let statuses = player.statuses.get();
                    let contract_months = player
                        .contract
                        .as_ref()
                        .map(|c| {
                            ((c.expiration - date).num_days().max(0) / 30).min(i16::MAX as i64)
                                as i16
                        })
                        .unwrap_or(0);
                    let interest_30d = player
                        .availability_market_state()
                        .map(|s| s.recent_interest(date))
                        .unwrap_or(0);
                    let failed_scans = player
                        .availability_market_state()
                        .map(|s| s.failed_scans)
                        .unwrap_or(0);

                    let breakout_score = performance_lookup
                        .breakout_for_player(
                            player,
                            player.statistics.total_games(),
                            player.statistics.average_rating_realistic(group),
                            player_age,
                            seller_league_rep,
                        )
                        .score;

                    let view = ListedTargetView {
                        ability,
                        estimated_potential,
                        age: player_age,
                        estimated_value: value,
                        position_group: group,
                        is_listed: statuses.contains(&PlayerStatusType::Lst),
                        is_transfer_requested: statuses.contains(&PlayerStatusType::Req),
                        is_unhappy: statuses.contains(&PlayerStatusType::Unh),
                        is_loan_listed: statuses.contains(&PlayerStatusType::Loa),
                        breakout_score,
                        world_reputation: player.player_attributes.world_reputation,
                        current_reputation: player.player_attributes.current_reputation,
                        ambition: player.attributes.ambition,
                        parent_club_score: seller
                            .teams
                            .main()
                            .map(|t| t.reputation.overall_score())
                            .unwrap_or(0.0),
                        parent_club_in_debt: seller.finance.balance.balance < 0,
                        days_available: days,
                        contract_months_remaining: contract_months,
                        low_usage: player.statistics.total_games() < 8,
                        recent_interest_count: interest_30d,
                        failed_scans,
                    };

                    // ── Scan plausible domestic buyers. ──
                    let mut reasons: Vec<AvailabilityBlockReason> = Vec::new();
                    let mut plausible = false;
                    for buyer in country.clubs.iter() {
                        if buyer.id == seller.id || seller.is_rival(buyer.id) {
                            continue;
                        }
                        let Some(scan) = buyer_scans.get(&buyer.id) else {
                            continue;
                        };
                        let bctx = scan.buyer_context(group, false);
                        match evaluate_listed_target(&view, &bctx) {
                            ListedTargetVerdict::Reject(reason) => {
                                reasons.push(MarketDiscoveryDiagnosis::from_listed_reject(reason));
                            }
                            ListedTargetVerdict::Accept(_) => {
                                // Cheap gates pass — confirm the realism
                                // (step-down / country-route) gates too.
                                let inputs = TransferPlausibilityBuilder::from_clubs(
                                    country, buyer, seller, player, value, false, true, date,
                                );
                                match TransferPlausibilityEvaluator::evaluate(&inputs) {
                                    TransferPlausibilityVerdict::HardReject(pr) => {
                                        reasons
                                            .push(MarketDiscoveryDiagnosis::from_plausibility(pr));
                                    }
                                    TransferPlausibilityVerdict::Allow(_) => {
                                        plausible = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    if plausible {
                        actions.insert(pid, CirculationAction::Interest);
                    } else {
                        actions.insert(
                            pid,
                            CirculationAction::Blocked(MarketDiscoveryDiagnosis::dominant(
                                &reasons,
                            )),
                        );
                    }
                }
            }
        }

        if scans_skipped_by_cap > 0 {
            debug!(
                "circulate_available_players: country {} hit the diagnosis cap; {} stale players retained their previous diagnosis this tick",
                country.id, scans_skipped_by_cap
            );
        }

        if actions.is_empty() && to_clear.is_empty() {
            return;
        }

        // ── Phase 2: apply (mutable). ──
        let clear_set: HashSet<u32> = to_clear.into_iter().collect();
        for club in &mut country.clubs {
            for team in &mut club.teams.teams {
                for player in &mut team.players.players {
                    if let Some(action) = actions.get(&player.id) {
                        match action {
                            CirculationAction::Interest => player.on_availability_interest(date),
                            CirculationAction::TooEarly => player.ensure_availability_state(date),
                            CirculationAction::Blocked(reason) => {
                                player.on_availability_blocked(date, *reason)
                            }
                        }
                    } else if clear_set.contains(&player.id) {
                        player.clear_availability_state();
                    }
                }
            }
        }
    }
}
