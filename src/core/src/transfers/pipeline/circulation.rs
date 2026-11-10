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
//!     interest recorded, which resets the staleness streak;
//!   * a player no plausible buyer wants accumulates a dominant
//!     [`crate::club::player::transfer::AvailabilityBlockReason`] — asking
//!     price too high, wage too high, reputation too high, no affordable
//!     squad need, won't step down, country/region blocked — so a quality
//!     player never just sits with no interest and no explanation.
//!
//! The pass never signs anyone and never bypasses a gate: it only reads
//! the same plausibility + listed-target evaluators the live pipeline
//! uses and writes diagnostics. The circulation lift is applied where
//! those evaluators run, driven by the state this pass maintains.

use crate::transfers::market::window::MarketCadence;
use crate::transfers::pipeline::PlayerSummary;
use crate::transfers::scouting::judgement::ScoutJudgement;
use crate::transfers::squad::bands::TierBands;
use crate::transfers::view::player::{PlayerSummaryBuilder, PlayerView};
use rustc_hash::{FxHashMap, FxHashSet};

use chrono::NaiveDate;
use log::debug;

use crate::club::player::transfer::AvailabilityBlockReason;
use crate::transfers::MarketMap;
use crate::transfers::deal::negotiation::NegotiationStatus;
use crate::transfers::gate::build::Suitor;
use crate::transfers::gate::fit::{SquadFitSnapshot, SquadRegistrationLimits};
use crate::transfers::gate::{
    MoveTerms, PursuitAction, TransferMovePlausibility, TransferPlausibilityReason,
};
use crate::transfers::pipeline::TransferRequestStatus;
use crate::transfers::pipeline::advice::{
    BuyerContext, ListedTargetScreen, ListedTargetVerdict, ListedTargetView,
};
use crate::transfers::scouting::breakout::LeaguePerformanceLookup;
use crate::transfers::scouting::exposure::MarketDiscoveryDiagnosis;
use crate::transfers::value::PlayerValuationCalculator;
use crate::{Club, Country, Person, Player, PlayerFieldPositionGroup, PlayerStatusType};

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
/// year-round breakout watch ([`crate::transfers::scouting::watch`]) so both passes
/// derive the buyer's [`BuyerContext`] from one place.
pub(in crate::transfers) struct BuyerScan {
    rep_score: f32,
    plan_total_budget: f64,
    best_in_group: FxHashMap<PlayerFieldPositionGroup, u8>,
    open_request_groups: FxHashSet<PlayerFieldPositionGroup>,
    aging_groups: FxHashSet<PlayerFieldPositionGroup>,
    fit_by_group: FxHashMap<PlayerFieldPositionGroup, SquadFitSnapshot>,
}

impl BuyerScan {
    pub(in crate::transfers) fn build(
        country: &Country,
        club: &Club,
        date: NaiveDate,
    ) -> Option<BuyerScan> {
        let team = club.teams.main().or_else(|| club.teams.teams.first())?;
        let registration = SquadRegistrationLimits::new(country.id, &country.regulations);
        let rep_score = team.reputation.overall_score();
        let plan_total_budget = club.transfer_plan.total_budget;

        let mut best_in_group: FxHashMap<PlayerFieldPositionGroup, u8> = FxHashMap::default();
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

        let mut open_request_groups: FxHashSet<PlayerFieldPositionGroup> = FxHashSet::default();
        for r in club.transfer_plan.transfer_requests.iter() {
            if r.status != TransferRequestStatus::Fulfilled
                && r.status != TransferRequestStatus::Abandoned
            {
                open_request_groups.insert(r.position.position_group());
            }
        }

        let mut aging_groups: FxHashSet<PlayerFieldPositionGroup> = FxHashSet::default();
        for group in [
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ] {
            let baseline = TierBands::tier_starter_ca_score(rep_score, group);
            let has_aging = team.players.players.iter().any(|p| {
                p.position().position_group() == group
                    && p.age(date) >= 30
                    && p.player_attributes.current_ability + 5 >= baseline
            });
            if has_aging {
                aging_groups.insert(group);
            }
        }

        let fit_by_group: FxHashMap<PlayerFieldPositionGroup, SquadFitSnapshot> = [
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ]
        .into_iter()
        .map(|g| (g, SquadFitSnapshot::build(club, g, date, registration)))
        .collect();

        Some(BuyerScan {
            rep_score,
            plan_total_budget,
            best_in_group,
            open_request_groups,
            aging_groups,
            fit_by_group,
        })
    }

    /// Build the per-player [`BuyerContext`] the listed-target evaluator
    /// expects, reading the precomputed per-group signals. `form_discovery`
    /// is `false` for the availability-driven diagnosis / listed sweep and
    /// `true` for the year-round breakout watch (which may surface a player
    /// who has not advertised availability).
    pub(in crate::transfers) fn buyer_context(
        &self,
        group: PlayerFieldPositionGroup,
        form_discovery: bool,
    ) -> BuyerContext {
        BuyerContext {
            buyer_rep_score: self.rep_score,
            plan_total_budget: self.plan_total_budget,
            buyer_best_in_group: self.best_in_group.get(&group).copied().unwrap_or(0),
            has_open_request: self.open_request_groups.contains(&group),
            has_aging_starter: self.aging_groups.contains(&group),
            form_discovery_mode: form_discovery,
            fit: self
                .fit_by_group
                .get(&group)
                .copied()
                .unwrap_or_else(SquadFitSnapshot::disabled),
        }
    }
}

/// Every club in the country that could take a listed man, read once.
struct DomesticBuyers<'s, 'c> {
    scans: &'s FxHashMap<u32, BuyerScan>,
    suitors: &'s FxHashMap<u32, Suitor<'c>>,
    market_map: &'s MarketMap,
}

/// The weekly circulation / diagnosis pass.
pub struct MarketCirculation;

impl MarketCirculation {
    /// Weekly market-circulation / diagnosis pass. Runs on the same
    /// cadence as the recommendation sweep and right after it, so the
    /// interest the sweep generated this tick is already visible.
    pub fn circulate_available_players(
        country: &mut Country,
        market_map: &MarketMap,
        date: NaiveDate,
    ) {
        if !MarketCadence::should_evaluate_for(&country.code, date) {
            return;
        }

        let interested = Self::concretely_pursued(country);

        // ── Precompute buyer snapshots once, keyed by club id. ──
        let buyer_scans: FxHashMap<u32, BuyerScan> = country
            .clubs
            .iter()
            .filter_map(|c| BuyerScan::build(country, c, date).map(|s| (c.id, s)))
            .collect();
        let suitors: FxHashMap<u32, Suitor<'_>> = country
            .clubs
            .iter()
            .filter(|c| buyer_scans.contains_key(&c.id))
            .map(|c| (c.id, Suitor::of(country, c, date)))
            .collect();

        // Scoring-chart + recent-award lookup, built once for the country.
        let performance_lookup = LeaguePerformanceLookup::build(country);

        let price_level = country.settings.pricing.price_level;

        // ── Phase 1: collect per-player actions (immutable). ──
        let mut actions: FxHashMap<u32, CirculationAction> = FxHashMap::default();
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

                    let view = Self::listed_target_view(
                        player,
                        seller,
                        date,
                        price_level,
                        seller_league_rep,
                        seller_club_rep,
                        &performance_lookup,
                        days,
                    );

                    let (plausible, reasons) = Self::scan_domestic_buyers(
                        country,
                        seller,
                        player,
                        &view,
                        &DomesticBuyers {
                            scans: &buyer_scans,
                            suitors: &suitors,
                            market_map,
                        },
                        date,
                    );

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

        Self::apply_circulation(country, date, &actions, to_clear);
    }

    /// Which players is any club already concretely pursuing? Monitoring rows,
    /// shortlist candidates, staff recommendations and live negotiations all
    /// count as "the market has shown interest recently".
    fn concretely_pursued(country: &Country) -> FxHashSet<u32> {
        // ── Which players is any club already concretely pursuing? ──
        // Monitoring rows, shortlist candidates, staff recommendations,
        // and live negotiations all count as "the market has shown
        // interest recently".
        let mut interested: FxHashSet<u32> = FxHashSet::default();
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

        interested
    }

    /// Build the player view once — every buyer below is shown the same one.
    #[allow(clippy::too_many_arguments)]
    fn listed_target_view(
        player: &Player,
        seller: &Club,
        date: NaiveDate,
        price_level: f32,
        seller_league_rep: u16,
        seller_club_rep: u16,
        performance_lookup: &LeaguePerformanceLookup,
        days: i64,
    ) -> ListedTargetView {
        // Build the player view once (shared across buyers).
        let group = player.position().position_group();
        let ability = PlayerView::position_evaluation_ability(player);
        let player_age = player.age(date);
        let estimated_potential = ability
            + ScoutJudgement::estimate_growth_potential(
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
        let contract_months = player
            .contract
            .as_ref()
            .map(|c| ((c.expiration - date).num_days().max(0) / 30).min(i16::MAX as i64) as i16)
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

        ListedTargetView {
            ability,
            nationality_country_id: player.country_id,
            estimated_potential,
            age: player_age,
            estimated_value: value,
            position_group: group,
            is_listed: player.statuses.has(PlayerStatusType::Lst),
            is_transfer_requested: player.statuses.has(PlayerStatusType::Req),
            is_unhappy: player.statuses.has(PlayerStatusType::Unh),
            is_loan_listed: player.statuses.has(PlayerStatusType::Loa),
            breakout_score,
            world_reputation: player.player_attributes.world_reputation,
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
        }
    }

    /// Scan the plausible domestic buyers. One that clears both the cheap
    /// gates and the plausibility gate is enough for the player to read as
    /// circulating; everything the others refused on is the diagnosis.
    fn scan_domestic_buyers(
        country: &Country,
        seller: &Club,
        player: &Player,
        view: &ListedTargetView,
        buyers: &DomesticBuyers<'_, '_>,
        date: NaiveDate,
    ) -> (bool, Vec<AvailabilityBlockReason>) {
        let group = view.position_group;
        let mut target: Option<Option<PlayerSummary>> = None;

        // ── Scan plausible domestic buyers. ──
        let mut reasons: Vec<AvailabilityBlockReason> = Vec::new();
        let mut plausible = false;
        for buyer in country.clubs.iter() {
            if buyer.id == seller.id || seller.is_rival(buyer.id) {
                continue;
            }
            let (Some(scan), Some(suitor)) =
                (buyers.scans.get(&buyer.id), buyers.suitors.get(&buyer.id))
            else {
                continue;
            };
            let bctx = scan.buyer_context(group, false);
            match ListedTargetScreen::evaluate(view, &bctx) {
                ListedTargetVerdict::Reject(reason) => {
                    reasons.push(MarketDiscoveryDiagnosis::from_listed_reject(reason));
                }
                ListedTargetVerdict::Accept(_) => {
                    let Some(summary) = target
                        .get_or_insert_with(|| {
                            PlayerSummaryBuilder::one(country, seller, player, date)
                        })
                        .as_ref()
                    else {
                        break;
                    };
                    let reading = TransferMovePlausibility::read(
                        suitor,
                        summary,
                        MoveTerms::purchase(true),
                        buyers.market_map,
                    );
                    if reading.reaches(PursuitAction::FillNow.needs()) {
                        plausible = true;
                        break;
                    }
                    reasons.push(MarketDiscoveryDiagnosis::from_plausibility(
                        reading
                            .blocking_reason
                            .unwrap_or(TransferPlausibilityReason::NoSportingUpside),
                    ));
                }
            }
        }

        (plausible, reasons)
    }

    /// Phase 2 — apply what the read pass decided.
    fn apply_circulation(
        country: &mut Country,
        date: NaiveDate,
        actions: &FxHashMap<u32, CirculationAction>,
        to_clear: Vec<u32>,
    ) {
        // ── Phase 2: apply (mutable). ──
        let clear_set: FxHashSet<u32> = to_clear.into_iter().collect();
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
