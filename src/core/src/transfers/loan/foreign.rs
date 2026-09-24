//! The cross-border half of the loan market.
//!
//! Same three stages as [`super::scan`] — read the board, offer it to every
//! club with appetite, commit what they asked for — but from a different kind
//! of input. This side never resolves a `Player`: it works entirely from
//! `&[&PlayerSummary]`, the snapshot the world pass builds of everyone outside
//! this country. That is a **data-shape** fork rather than a reach fork, which
//! is why it is still its own file: collapsing it onto the domestic scan needs
//! one target view both a live `Player` and a `PlayerSummary` can present, and
//! the eligibility here is genuinely different besides — region prestige,
//! market visibility, scout regions, and the compatriot sweep have no domestic
//! counterpart.

use crate::transfers::loan::LoanPipeline;
use crate::transfers::market::window::MarketCadence;
use crate::transfers::view::club::ClubView;
use chrono::NaiveDate;
use log::debug;

use crate::shared::{Currency, CurrencyValue};
use crate::transfers::ScoutingRegion;
use crate::transfers::deal::offer::{PersonalTermsOffer, TransferClause, TransferOffer};
use crate::transfers::deal::reason::TransferReason;
use crate::transfers::gate::TransferPlausibilityVerdict;
use crate::transfers::gate::build::{BuyerPlausibilityContext, TransferPlausibilityBuilder};
use crate::transfers::gate::fit::{ForeignSlotCount, SquadRegistrationLimits};
use crate::transfers::gate::stance::PlayerStanceBuilder;
use crate::transfers::loan::interest::{
    BorrowerTaste, GroupPressure, InterestDraw, LoanCandidateProfile,
};
use crate::transfers::market::knowledge::{ClubMarketKnowledge, PlacementReachIndex};
use crate::transfers::market::{TransferListing, TransferListingOrigin, TransferListingType};
use crate::transfers::pipeline::processor::PlayerSummary;
use crate::transfers::pipeline::trace::{MarketSwitches, TransferTrace};
use crate::transfers::pipeline::{TransferRequest, TransferRequestStatus};
use crate::transfers::squad::minutes::LoanPromise;
use crate::transfers::{MarketAffinity, MarketAffinityInputs, MarketMap, MoveKind};
use crate::utils::FormattingUtils;
use crate::{Club, Country, Language, PlayerFieldPositionGroup, ReputationLevel, Team};
use std::collections::HashMap;

use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::transfers::pipeline::ClubTransferPlan;

use super::*;
use crate::transfers::loan::legacy::{LegacyForeignGate, LegacyLoanGuard};

/// Everyone outside this country who could plausibly be borrowed, plus the
/// geography that decides who can even be seen from here.
struct ForeignLoanBoard<'a> {
    loans: Vec<&'a PlayerSummary>,
    compatriots: Vec<&'a PlayerSummary>,
    by_group: [Vec<&'a PlayerSummary>; PlayerFieldPositionGroup::COUNT],
    mid_season_window: bool,
    /// Loan corridor per `(passport, league he plays in)`, memoised while the
    /// board was filtered. The same number gates and weights: a route this
    /// country barely works is not a candidate it takes as readily as one it
    /// works every window.
    visibility: HashMap<(u32, u32), f32>,
}

impl ForeignLoanBoard<'_> {
    /// How plausible borrowing this man is from here, 0..1. Every player on
    /// the board was scored to get here, so the lookup always hits.
    fn visibility(&self, p: &PlayerSummary) -> f32 {
        self.visibility
            .get(&(p.nationality_country_id, p.country_id))
            .copied()
            .unwrap_or(1.0)
    }
}

/// One borrowing club's cross-border turn, resolved once.
struct ForeignBorrower<'a> {
    country_id: u32,
    club_region: ScoutingRegion,
    /// Languages spoken where he would be playing — the third of the
    /// three things that make a place familiar to him.
    country_language_mask: u64,
    club: &'a Club,
    plan: &'a ClubTransferPlan,
    team: &'a Team,
    team_rep: u16,
    ordinary_foreign_scan: bool,
    compatriot_sweep: bool,
    borrower_league_rep: u16,
    max_loan_fee: f64,
    avg_ability: u8,
    scout_regions: Vec<ScoutingRegion>,
    max_scans: usize,
    borrower_position_depth: BorrowerPositionDepth,
    foreign_borrower_profile: Option<LoanBorrowerProfile>,
    buyer_loan_ctx: BuyerPlausibilityContext,
    taste: BorrowerTaste,
    /// Its position under its league's foreigner quota, and the slots its
    /// in-flight approaches have already spent.
    foreign_slots: ForeignSlotCount,
    pending_foreign: u32,
    /// Countries this club's scouts cover, and how well — the walk that
    /// answers "how well do we know that market?" for both the reach and
    /// the knowledge memo below.
    scout_countries: Vec<(u32, u8)>,
    /// What the club knows of each source country, memoised: the board
    /// memoises the CORRIDOR per (passport, league), and knowledge is the
    /// other half of the reach and belongs to the club rather than the
    /// country.
    knowledge: std::cell::RefCell<FxHashMap<u32, f32>>,
    market_map: &'a MarketMap,
    /// Every lending club's placement map — the lender's own half of the
    /// geography, which this country's borrow cannot read live.
    placement_reach: &'a PlacementReachIndex,
    date: NaiveDate,
    /// What the club has actually asked for — the band, not just the
    /// shirt.
    open_requests: Vec<TransferRequest>,
}

impl ForeignBorrower<'_> {
    /// Floor under the knowledge half of the reach — a club may LOOK at a
    /// market it does not work. The same floor
    /// [`crate::transfers::gate::TransferMovePlausibility`] applies to the
    /// buy side's knowledge term, folded in here because the summary path
    /// hands the gate the two of them already multiplied.
    const KNOWLEDGE_FLOOR: f32 = 0.5;

    /// How well this club knows one foreign market, 0..1 — its ledger, its
    /// scouts and its country's card, exactly as a permanent approach
    /// reads it. Memoised per country: a pass scores thousands of
    /// candidates against a few dozen source markets.
    fn knowledge_of(&self, country_id: u32) -> f32 {
        if let Some(cached) = self.knowledge.borrow().get(&country_id) {
            return *cached;
        }
        let best_scout_level = self
            .scout_countries
            .iter()
            .find(|(id, _)| *id == country_id)
            .map(|(_, level)| *level)
            .unwrap_or(0);
        let value = ClubMarketKnowledge::knowledge(
            self.market_map,
            self.country_id,
            &self.club.market_ledger,
            best_scout_level,
            country_id,
            self.date,
        );
        self.knowledge.borrow_mut().insert(country_id, value);
        value
    }

    /// How far this club's market reaches to ONE candidate abroad — the
    /// loan corridor times what the club knows of the market he is in.
    ///
    /// The same product a permanent cross-border approach is measured on
    /// (`market_geography`), so `thresholds::MARKET_REACH_FLOOR` means the
    /// same thing on a loan as it does on a purchase: the club may watch
    /// him, and that is all. The MAX over the lending country and the
    /// passport for the same reason the buy side takes it — a club with a
    /// Brazil man can see a Brazilian at Porto.
    fn market_reach(&self, board: &ForeignLoanBoard<'_>, p: &PlayerSummary) -> f32 {
        if MarketSwitches::loan_reach_off() || self.market_map.is_silent() {
            return 1.0;
        }
        let knowledge = self
            .knowledge_of(p.country_id)
            .max(self.knowledge_of(p.nationality_country_id));
        (board.visibility(p) * knowledge.max(Self::KNOWLEDGE_FLOOR)).clamp(0.0, 1.0)
    }

    /// How far the PARENT's own placement network reaches into this
    /// country, 0..1.
    fn placement_trust(&self, p: &PlayerSummary) -> f32 {
        if MarketSwitches::loan_placement_off() || self.market_map.is_silent() {
            return 1.0;
        }
        self.placement_reach.trust(
            self.market_map,
            p.club_id,
            p.country_id,
            self.country_id,
            self.date,
        )
    }

    /// How familiar this country is to HIM — his own corridor, his
    /// diaspora, or a language he speaks.
    fn familiarity(&self, p: &PlayerSummary) -> f32 {
        if MarketSwitches::loan_familiarity_off() || self.market_map.is_silent() {
            return 1.0;
        }
        MarketAffinity::player_affinity(
            self.market_map,
            p.nationality_country_id,
            self.country_id,
            p.language_profile.affinity_for(self.country_language_mask),
        )
    }

    /// Registration room this passport has here, with the club's in-flight
    /// approaches already counted against the quota.
    fn slot_room(&self, p: &PlayerSummary) -> f32 {
        if MarketSwitches::loan_slots_off() {
            return 1.0;
        }
        self.foreign_slots
            .room_after(p.nationality_country_id, self.pending_foreign)
    }

    /// What the three sides would agree on for this cross-border pair,
    /// 0..1 — the same four terms the domestic scan prices, off the
    /// staged summary.
    ///
    /// The parent's own willingness, the arc the player is living out
    /// and the wage his club would keep paying all travel on the
    /// summary: this country cannot reach into his club's squad to read
    /// any of them, which is the same reason `leave_pressure` travels.
    fn agreement_for(&self, board: &ForeignLoanBoard<'_>, p: &PlayerSummary) -> Option<f32> {
        let Some((inputs, _)) = self.priced(p, board.mid_season_window) else {
            return self.legacy_allows(p);
        };
        if TransferTrace::is(p.player_id) {
            TransferTrace::line(
                p.player_id,
                "loan",
                format!(
                    "borrower={} reach={:.3} {}",
                    self.club.name,
                    self.market_reach(board, p),
                    LoanAgreement::explain_inputs(&inputs),
                ),
            );
        }
        LoanAgreement::price(&inputs)
    }

    /// The same pair, as the SELLER will be asked about it.
    ///
    /// The parent of a cross-border loan sits in another country's borrow
    /// by the time the room opens, so the verdict it would give is stamped
    /// here — where both clubs are readable — exactly as the player's own
    /// stance and the sporting drop already are. The willingness on it is
    /// the destination-aware one: what the parent thinks of THIS move,
    /// which is the whole of what the room needs from it.
    fn stamped_verdict(
        &self,
        board: &ForeignLoanBoard<'_>,
        p: &PlayerSummary,
    ) -> Option<LoanGuardVerdict> {
        let (inputs, verdict) = self.priced(p, board.mid_season_window)?;
        let (parent, _, _, money) = LoanAgreement::terms(&inputs);
        Some(verdict?.about(parent.score, money.affordability))
    }

    /// The `OF_LOAN_AGREEMENT_OFF` arm: the conjunctive gate stack the
    /// four terms replaced.
    fn legacy_allows(&self, p: &PlayerSummary) -> Option<f32> {
        LegacyLoanGuard::foreign_allows(&LegacyForeignGate {
            summary: p,
            borrower_rep: self.team_rep,
            borrower_league_rep: self.borrower_league_rep,
            depth: &self.borrower_position_depth,
            borrower: self.foreign_borrower_profile.map(|profile| {
                profile.with_best_in_group(
                    self.borrower_position_depth.best_in_group(p.position_group),
                )
            }),
        })
        .then_some(1.0)
    }

    /// The pair's inputs and the guard's reading of it, built once so the
    /// score and the verdict the room reads can never describe different
    /// deals. `None` on the legacy arm, which prices nothing.
    fn priced(
        &self,
        p: &PlayerSummary,
        mid_season_window: bool,
    ) -> Option<(AgreementInputs, Option<LoanGuardVerdict>)> {
        if LoanAgreement::disarmed() {
            return None;
        }
        let group = p.position_group;
        let borrower = self.foreign_borrower_profile.map(|profile| {
            profile.with_best_in_group(self.borrower_position_depth.best_in_group(group))
        });
        let guard = LoanAssetGuard::from_summary(
            p.skill_ability,
            p.age,
            group,
            p.club_world_reputation,
            p.club_best_in_group,
            p.seller_ctx.league_reputation,
            p.estimated_value,
            p.salary,
            p.is_loan_listed,
            EffectivePlayerReputation::compute(
                p.world_reputation,
                p.current_reputation,
                p.home_reputation,
                false,
            ),
        )
        .with_plan(p.career_plan);
        let verdict = borrower
            .as_ref()
            .map(|b| guard.assess(b, p.loan_willingness.score, p.parent_subsidy));
        let inputs = AgreementInputs {
            parent: p.loan_willingness,
            parent_rep: p.club_world_reputation.max(0) as u16,
            parent_league_rep: p.seller_ctx.league_reputation,
            parent_best_in_group: p.club_best_in_group,
            parent_subsidy: p.parent_subsidy,
            borrower_tier: self.team.reputation.level(),
            borrower_rep: self.team_rep,
            borrower_league_rep: self.borrower_league_rep,
            group,
            count: self.borrower_position_depth.headcount(group),
            best_here: self.borrower_position_depth.best_in_group(group),
            clearly_better_ahead: self
                .borrower_position_depth
                .clearly_better_ahead(group, p.skill_ability),
            need: self.need_for(p).score(),
            slot_room: self.slot_room(p),
            mid_season_window,
            candidate: p.observable_level,
            is_development: p.is_development,
            stage: p.pathway_stage,
            // A summary carries no loan-out row: the club's own band is
            // one of the things a borrowing country cannot see.
            club_band_target: None,
            plan: p.career_plan,
            renown_gap: verdict.map(|v| v.renown_gap).unwrap_or(0.0),
            renown_band: guard.renown_band(),
            resignation: p.seller_ctx.market_resignation,
            // He is being asked to come back to the country his
            // passport is from — the one thing a cross-border loan can
            // offer that a domestic one cannot.
            going_home: p.nationality_country_id != 0
                && p.nationality_country_id == self.country_id,
            placement_trust: self.placement_trust(p),
            familiarity: self.familiarity(p),
            weight: verdict.map(|v| v.weight).unwrap_or(0.0),
            carry: verdict.map(|v| v.carry).unwrap_or(0.0),
            asking: p.estimated_value * 0.1,
            max_loan_fee: self.max_loan_fee,
        };
        Some((inputs, verdict))
    }

    /// How badly this club wants a body in that shirt — the same
    /// reading the domestic scan and the seller broadcast take.
    fn need_for(&self, p: &PlayerSummary) -> BorrowerNeed {
        let group = p.position_group;
        let request = self
            .open_requests
            .iter()
            .find(|r| r.position.position_group() == group);
        let ideal = group.ideal_squad_depth();
        let held = self.borrower_position_depth.headcount(group);
        BorrowerNeed {
            requested: request.is_some(),
            level_shortfall: request
                .map(|r| r.min_ability as i16 - p.observable_level as i16)
                .unwrap_or(0),
            age_excess: request
                .map(|r| p.age as i16 - r.preferred_age_max as i16)
                .unwrap_or(0),
            vacancy: ((ideal as f32 - held as f32) / ideal.max(1) as f32).clamp(0.0, 1.0),
        }
    }
}

/// What one club's turn needs from the rest of the market: who already has a
/// pursuit in flight, and how many negotiations each club is carrying.
struct MarketLoad {
    pending_loans: HashMap<u32, Vec<(PlayerFieldPositionGroup, u8)>>,
    /// Incoming loan approaches already spending a foreign registration
    /// slot, per borrowing club.
    pending_foreign: FxHashMap<u32, u32>,
    active_counts: FxHashMap<u32, u32>,
    active_pairs: FxHashSet<(u32, u32)>,
}

/// One open request's slate: every foreign candidate for that shirt, scored.
struct RequestSlate<'p> {
    group: PlayerFieldPositionGroup,
    reason: &'static str,
    candidates: Vec<(&'p PlayerSummary, f32)>,
}

/// One club's cross-border turn, scored before any club has claimed anybody.
struct ForeignTurn<'c, 'p> {
    borrower: ForeignBorrower<'c>,
    requests: Vec<RequestSlate<'p>>,
    prospects: Vec<(&'p PlayerSummary, f32)>,
}

impl<'c, 'p> ForeignTurn<'c, 'p> {
    fn score(
        borrower: ForeignBorrower<'c>,
        board: &ForeignLoanBoard<'p>,
        date: NaiveDate,
        active_pairs: &FxHashSet<(u32, u32)>,
    ) -> Self {
        let requests = ForeignLoanScan::request_slates(&borrower, board, date, active_pairs);
        let prospects = ForeignLoanScan::prospect_slate(&borrower, board, date, active_pairs);
        ForeignTurn {
            borrower,
            requests,
            prospects,
        }
    }

    /// Take the turn against what earlier clubs have already claimed: the
    /// requests first, each a position group the club then stops scanning,
    /// and the proactive pickup with whatever scan budget is left.
    fn claim(
        &self,
        board: &ForeignLoanBoard<'_>,
        date: NaiveDate,
        claimed: &mut FxHashSet<u32>,
        actions: &mut Vec<ForeignLoanAction>,
    ) {
        let borrower = &self.borrower;
        let mut scans = 0usize;
        // Track position groups already targeted to avoid multiple
        // negotiations for the same position (e.g. FormationGap + DepthCover
        // for GK).
        let mut scanned_position_groups: Vec<PlayerFieldPositionGroup> = Vec::new();

        for request in &self.requests {
            if scans >= borrower.max_scans {
                break;
            }
            if scanned_position_groups.contains(&request.group) {
                continue;
            }
            if let Some(best) = Self::draw(request.candidates.iter(), claimed) {
                actions.push(self.approach(
                    board,
                    best,
                    date,
                    TransferReason::key(request.reason),
                    false,
                ));
                claimed.insert(best.player_id);
                scanned_position_groups.push(request.group);
                scans += 1;
            }
        }

        if scans < borrower.max_scans {
            // Same-region prospects still circulate locally first — that
            // preference lives in the weight, so a genuinely better fit on
            // another continent is reachable instead of unreachable.
            let open = self
                .prospects
                .iter()
                .filter(|(p, _)| !scanned_position_groups.contains(&p.position_group));
            if let Some(best) = Self::draw(open, claimed) {
                actions.push(self.approach(
                    board,
                    best,
                    date,
                    TransferReason::key("signing_reason_loan_foreign_prospect"),
                    borrower.compatriot_sweep,
                ));
                claimed.insert(best.player_id);
            }
        }
    }

    /// Draw one candidate nobody earlier in the pass has claimed.
    fn draw<'s>(
        slate: impl Iterator<Item = &'s (&'p PlayerSummary, f32)>,
        claimed: &FxHashSet<u32>,
    ) -> Option<&'p PlayerSummary>
    where
        'p: 's,
    {
        let open: Vec<&(&'p PlayerSummary, f32)> = slate
            .filter(|(p, _)| !claimed.contains(&p.player_id))
            .collect();
        let weighted: Vec<(u32, f32)> = open
            .iter()
            .enumerate()
            .map(|(i, (_, score))| (i as u32, *score))
            .collect();
        InterestDraw::pick(&weighted).map(|i| open[i as usize].0)
    }

    fn approach(
        &self,
        board: &ForeignLoanBoard<'_>,
        best: &PlayerSummary,
        date: NaiveDate,
        reason: TransferReason,
        from_compatriot_sweep: bool,
    ) -> ForeignLoanAction {
        let borrower = &self.borrower;
        let staged = ForeignLoanStance::read(&borrower.buyer_loan_ctx, best, date);
        ForeignLoanAction {
            club_id: borrower.club.id,
            player: best.clone(),
            offer_amount: FormattingUtils::round_fee(best.estimated_value * 0.1 * 0.8),
            reason,
            is_development: ForeignUnsolicitedLoanTarget::is_development(best.age),
            from_compatriot_sweep,
            player_importance: staged.0,
            sporting_drop: staged.1,
            loan_verdict: borrower.stamped_verdict(board, best),
        }
    }
}

/// The pass itself.
pub(in crate::transfers::loan) struct ForeignLoanScan;

/// A cross-border loan approach one club decided to make.
struct ForeignLoanAction {
    club_id: u32,
    player: PlayerSummary,
    offer_amount: f64,
    reason: TransferReason,
    /// Age-classified development loan — the borrower cleared the
    /// stricter minutes gate, so the offer promises a regular
    /// shirt ([`LoanPromise`]).
    is_development: bool,
    /// This approach came from the compatriot sweep rather than
    /// the ordinary foreign scan — one per club per window.
    from_compatriot_sweep: bool,
    /// What he is to his current club, and how far the move
    /// falls — both read from the staged plausibility model here,
    /// because at resolution time his club is abroad.
    player_importance: f32,
    sporting_drop: f32,
    /// What the parent would say about THIS destination, priced where
    /// both clubs are readable. The room reads it in place of the live
    /// guard, which a cross-border seller has never had.
    loan_verdict: Option<LoanGuardVerdict>,
}

impl ForeignLoanScan {
    pub(in crate::transfers::loan) fn run(
        country: &mut Country,
        foreign_players: &[&PlayerSummary],
        date: NaiveDate,
        market_map: &MarketMap,
        placement_reach: &PlacementReachIndex,
    ) {
        if foreign_players.is_empty() {
            return;
        }
        let Some(board) = Self::board(country, foreign_players, date, market_map) else {
            return;
        };

        let load = MarketLoad {
            pending_loans: LoanPipeline::pending_incoming_loans_by_club(country),
            pending_foreign: LoanPipeline::pending_foreign_registrations_by_club(country),
            active_counts: country.transfer_market.active_negotiation_counts(),
            active_pairs: country.transfer_market.active_negotiation_pairs(),
        };

        // Scoring a club's turn reads only the country, the board and the
        // club itself, so every turn is scored in parallel. What one club
        // claims narrows what the next may take, so the claims and the draws
        // stay serial, in visit order.
        let country_ref: &Country = country;
        let turns: Vec<Option<ForeignTurn<'_, '_>>> = (0..country_ref.clubs.len())
            .into_par_iter()
            .map(|club_idx| {
                Self::borrower(
                    country_ref,
                    club_idx,
                    &board,
                    date,
                    &load,
                    market_map,
                    placement_reach,
                )
                .map(|borrower| ForeignTurn::score(borrower, &board, date, &load.active_pairs))
            })
            .collect();

        let mut actions: Vec<ForeignLoanAction> = Vec::new();
        let mut claimed: FxHashSet<u32> = FxHashSet::default();
        for club_idx in InterestDraw::visit_order(turns.len()) {
            if let Some(turn) = &turns[club_idx] {
                turn.claim(&board, date, &mut claimed, &mut actions);
            }
        }
        drop(turns);

        Self::commit(country, date, actions);
    }

    /// Who out there could be borrowed, and how visible each of them is from
    /// here. The corridor is memoised on the PAIR (passport, league he plays
    /// in): keyed on the league alone, a Brazilian at Porto and a Portuguese at
    /// Porto shared one entry and the first of the two scored decided the
    /// geography for both.
    fn board<'a>(
        country: &Country,
        foreign_players: &[&'a PlayerSummary],
        date: NaiveDate,
        market_map: &MarketMap,
    ) -> Option<ForeignLoanBoard<'a>> {
        let mid_season_window = MarketCadence::is_mid_season_window_for(&country.code, date);

        // The scanning country's own region, which blocks loans from
        // clearly more prestigious ones (Paraguay can't loan from
        // England).
        let country_id = country.id;
        let club_region = ScoutingRegion::from_country(country.continent_id, &country.code);
        let club_region_prestige = club_region.league_prestige();

        // How plausible each source market is for THIS country, memoised on
        // the PAIR `(passport, league he plays in)`. The borrowing side of
        // the market has the same geography as the buying side: a Turkish
        // club takes loans from the leagues Turkish clubs deal with, not
        // from wherever a loanable body happens to be listed.
        //
        // Keyed on the league alone (the first cut) a Brazilian at Porto and
        // a Portuguese at Porto shared one entry, and the first of the two
        // scored decided the geography for both.
        let mut visibility_cache: HashMap<(u32, u32), f32> = HashMap::new();
        let mut market_visibility = |p: &PlayerSummary| -> f32 {
            let key = (p.nationality_country_id, p.country_id);
            if let Some(cached) = visibility_cache.get(&key) {
                return *cached;
            }
            let affinity = if market_map.is_silent() {
                1.0
            } else {
                MarketAffinity::loan_affinity(
                    market_map,
                    MarketAffinityInputs {
                        buyer_country_id: country_id,
                        nationality_country_id: key.0,
                        current_country_id: key.1,
                        kind: MoveKind::Talent,
                        // Scanned per country, one answer for the league.
                        benefactor: 0.0,
                    },
                )
            };
            visibility_cache.insert(key, affinity);
            affinity
        };

        // Collect loan-listed foreign players
        // Only consider players from countries with equal or lower reputation,
        // and whose home region isn't far above the scanning club's region.
        let country_rep = country.reputation;
        let foreign_loans: Vec<&PlayerSummary> = foreign_players
            .iter()
            .copied()
            .filter(|p| {
                // Either the parent advertised the loan, or the player is a
                // credible cold target his club isn't building around — the
                // `Loa` badge is not required for a loan approach. Importance
                // is gated below by the staged (unsolicited) plausibility.
                let approachable = p.is_loan_listed
                    || p.home_return_wanted
                    || (p.age <= MAX_LOAN_TARGET_AGE
                        && ForeignUnsolicitedLoanTarget::looks_loanable(
                            p.age,
                            p.skill_ability,
                            p.club_best_in_group,
                        ));
                if !approachable {
                    return false;
                }
                // A player is never a stranger in his own country. The
                // level gates below read truth and stay exactly as they
                // were — what changes is that his own federation is not
                // "a smaller country" to him, and his own continent is a
                // smaller step than a stranger's (L7.3).
                let is_home_country =
                    p.nationality_country_id != 0 && p.nationality_country_id == country_id;
                let is_home_region = !is_home_country && p.nationality_region == Some(club_region);
                // Country-reputation step-down. A player from a more
                // prestigious footballing nation isn't a realistic loan-in
                // for a smaller country — EXCEPT development-profile
                // youngsters, who routinely drop a national tier for
                // guaranteed senior minutes (Russia → Belarus, an Argentine
                // prospect → a smaller league). The region-prestige gate
                // below and the club-rep reality band downstream still bound
                // how far the move can fall.
                if !HomeLoanGates::country_rep_ok(
                    LoanPipeline::foreign_loan_country_rep_ok(
                        p.country_reputation,
                        country_rep,
                        ForeignUnsolicitedLoanTarget::is_development(p.age),
                    ),
                    is_home_country,
                ) {
                    return false;
                }
                // Cross-region prestige step-down. A settled player won't loan
                // down into a clearly smaller ecosystem for a bit-part role, but
                // a development youngster drops abroad for guaranteed minutes
                // (an Italian U18 → Romania). See `foreign_loan_region_ok`.
                // `p.region` is precomputed at pool-build time.
                if !HomeLoanGates::region_ok(
                    LoanPipeline::foreign_loan_region_ok(
                        p.region.league_prestige(),
                        club_region_prestige,
                        ForeignUnsolicitedLoanTarget::is_development(p.age),
                    ),
                    p.region.league_prestige(),
                    club_region_prestige,
                    is_home_country,
                    is_home_region,
                ) {
                    return false;
                }
                // Corridor visibility, read as a LOAN — the route between the
                // two clubs' leagues as well as the player's own corridor.
                // A borrowing club looks in the markets its league works,
                // the same as a buying one, but a loan is business between
                // the two CLUBS and the passport alone cannot see that. Home
                // is exempt by construction (affinity 1.0), which is what
                // keeps the loan-home pathway untouched.
                market_visibility(p) >= FOREIGN_LOAN_VISIBILITY_FLOOR
            })
            .collect();

        if foreign_loans.is_empty() {
            return None;
        }

        // Posted compatriots, hoisted out of the per-club loop: the ONLY
        // foreign loan market an Elite or Continental club opens, and a
        // country-wide fact that has no business being recomputed per club.
        let compatriots: Vec<&PlayerSummary> = foreign_loans
            .iter()
            .copied()
            .filter(|p| p.home_return_wanted && p.nationality_country_id == country_id)
            .collect();

        // Position-group partition for the request loop below, so a
        // request does not walk the whole filtered pool rejecting the
        // other three groups candidate by candidate. The proactive
        // development pickup after the request loop crosses groups and
        // keeps using the flat `foreign_loans`. Selection is a weighted
        // draw over the whole qualified slate either way, so the
        // partition is a cost saving and nothing more — it preserves no
        // ordering a tie-break could land on.
        let mut foreign_loans_by_group: [Vec<&PlayerSummary>; PlayerFieldPositionGroup::COUNT] =
            Default::default();
        for p in &foreign_loans {
            foreign_loans_by_group[p.position_group.index()].push(*p);
        }

        Some(ForeignLoanBoard {
            loans: foreign_loans,
            compatriots,
            by_group: foreign_loans_by_group,
            mid_season_window,
            visibility: visibility_cache,
        })
    }

    /// `None` when this club is not in the cross-border market at all.
    fn borrower<'a>(
        country: &'a Country,
        club_idx: usize,
        board: &ForeignLoanBoard<'_>,
        date: NaiveDate,
        load: &MarketLoad,
        market_map: &'a MarketMap,
        placement_reach: &'a PlacementReachIndex,
    ) -> Option<ForeignBorrower<'a>> {
        let mid_season_window = board.mid_season_window;
        let compatriots = &board.compatriots;
        let pending_loans = &load.pending_loans;
        let active_counts = &load.active_counts;
        let club = &country.clubs[club_idx];
        if club.teams.teams.is_empty() {
            return None;
        }

        let plan = &club.transfer_plan;
        if !plan.initialized {
            return None;
        }

        let team = &club.teams.teams[0];
        let rep_level = team.reputation.level();

        // Elite clubs buy — they don't scan loan markets.
        // Continental only in January with negative balance.
        // Local/Amateur don't have the scouting reach for foreign markets.
        // Who runs the ORDINARY foreign loan scan — the request path
        // plus the proactive pickup, over the whole foreign pool.
        let ordinary_foreign_scan = match rep_level {
            ReputationLevel::Elite => false,
            ReputationLevel::Continental => mid_season_window && club.finance.balance.balance < 0,
            ReputationLevel::National | ReputationLevel::Regional => true,
            _ => false, // Local/Amateur
        };
        // …and who runs the COMPATRIOT SWEEP instead.
        //
        // A compatriot the world has been told would come home is a
        // different proposition from a foreign loan market. Flamengo
        // and Palmeiras do not trawl Europe for loanees — and they do
        // sign the boy their own league produced when his club posts
        // him. So the sweep opens exactly that door: the proactive
        // branch alone, over the posted-compatriot slice alone, once
        // per club per window.
        //
        // Narrow on purpose. Flip `should_scan_foreign` country-wide
        // instead and both branches walk the FULL foreign pool with no
        // compatriot restriction — one posted English 21-year-old at
        // Ajax has every Elite English club running the ordinary foreign
        // scan every pass for as long as he stays posted.
        let compatriot_sweep = !ordinary_foreign_scan
            && !MarketSwitches::compatriot_sweep_off()
            && matches!(
                rep_level,
                ReputationLevel::Elite | ReputationLevel::Continental
            )
            && plan.compatriot_sweeps_this_window == 0
            && !compatriots.is_empty();
        if !ordinary_foreign_scan && !compatriot_sweep {
            return None;
        }
        // The slice each branch actually looks at.
        let borrower_league_rep = LoanPipeline::club_league_reputation(country, club);

        // Check concurrent negotiation limits
        let actual_active = active_counts.get(&club.id).copied().unwrap_or(0);
        if actual_active >= plan.max_concurrent_negotiations {
            return None;
        }

        let balance = club.finance.balance.balance;
        let max_loan_fee = if balance < 0 {
            30_000.0
        } else {
            (balance as f64 * 0.15).min(500_000.0)
        };

        let avg_ability = {
            let avg = team.players.current_ability_avg();
            if avg == 0 { 50 } else { avg }
        };

        // Get scout known regions for this club
        let scout_regions: Vec<ScoutingRegion> = club
            .teams
            .teams
            .iter()
            .flat_map(|t| t.staffs.iter())
            .flat_map(|s| s.staff_attributes.knowledge.known_regions.iter().copied())
            .collect();
        // …and the COUNTRY axis of the same department, which is the unit
        // market knowledge is measured in. Inverted for the reason the buy
        // side inverts it: a scout knows a handful of countries, so one
        // pass over the department answers every question the scan will
        // ask.
        let mut scout_countries: Vec<(u32, u8)> = Vec::new();
        for staff in club.teams.teams.iter().flat_map(|t| t.staffs.staffs.iter()) {
            for known in &staff.staff_attributes.knowledge.known_countries {
                match scout_countries
                    .iter_mut()
                    .find(|(id, _)| *id == known.country_id)
                {
                    Some(entry) => entry.1 = entry.1.max(known.level),
                    None => scout_countries.push((known.country_id, known.level)),
                }
            }
        }

        // Check unfulfilled transfer requests. Emergency
        // free-agent depth requests are excluded — they're
        // serviced by the free-agent matcher only, not by loans.
        let _scans = 0usize;
        let max_scans: usize = match rep_level {
            ReputationLevel::Elite => 3,
            _ => 2,
        };

        // Track position groups already targeted to avoid multiple negotiations
        // for the same position (e.g. FormationGap + DepthCover for GK)
        let _scanned_position_groups: Vec<PlayerFieldPositionGroup> = Vec::new();

        // Borrower-side depth snapshot — the same gate the
        // domestic scan uses to avoid loaning into an already-full
        // position. Building it once here keeps the filter inside
        // `foreign_loans.iter()` cheap.
        let borrower_position_depth = BorrowerPositionDepth::snapshot(team)
            .with_pending_loans(pending_loans.get(&club.id).map_or(&[], |v| v.as_slice()));
        // What this club's year and payroll can carry — the same two
        // money terms the domestic scan prices, which cross a border
        // unchanged: a Segunda club's revenue is a Segunda club's
        // revenue whoever the parent is.
        let foreign_borrower_profile = LoanBorrowerProfile::of(club, date, borrower_league_rep);
        let _foreign_borrower_for =
            |group: PlayerFieldPositionGroup| -> Option<LoanBorrowerProfile> {
                foreign_borrower_profile
                    .map(|p| p.with_best_in_group(borrower_position_depth.best_in_group(group)))
            };

        // Staged-plausibility buyer context, built once per club so the
        // foreign-loan filter can run the same cross-border veto the
        // scouting / permanent paths use. The candidate `PlayerSummary`
        // carries the seller-side context, so no selling-country ref is
        // needed here.
        let buyer_loan_ctx = BuyerPlausibilityContext::build(country, club, date);

        // Same preference model the domestic scan uses. The cross-border
        // branches ranked on `(same_region, skill_ability)`, which made the
        // continent's single strongest listed prospect the one target every
        // eligible club in the region opened with.
        let taste = BorrowerTaste::of(
            club,
            club.teams
                .teams
                .iter()
                .map(|t| {
                    t.staffs
                        .resolve_for_transfers()
                        .best_scout_judging_ability()
                })
                .max()
                .unwrap_or(5),
            avg_ability,
            max_loan_fee,
        );
        let open_requests: Vec<TransferRequest> = plan
            .transfer_requests
            .iter()
            .filter(|r| {
                r.status != TransferRequestStatus::Fulfilled
                    && r.status != TransferRequestStatus::Abandoned
            })
            .cloned()
            .collect();
        // Cultural proximity stays a preference, as it was — but as a

        // The request path walks the whole foreign pool by position
        // group, so it belongs to the ordinary scan alone — a

        Some(ForeignBorrower {
            country_id: country.id,
            club_region: ScoutingRegion::from_country(country.continent_id, &country.code),
            country_language_mask: Language::country_language_mask(&country.code),
            club,
            plan,
            team,
            team_rep: team.reputation.world,
            ordinary_foreign_scan,
            compatriot_sweep,
            borrower_league_rep,
            max_loan_fee,
            avg_ability,
            scout_regions,
            max_scans,
            borrower_position_depth,
            foreign_borrower_profile,
            buyer_loan_ctx,
            taste,
            foreign_slots: SquadRegistrationLimits::new(country.id, &country.regulations)
                .count(club),
            pending_foreign: load.pending_foreign.get(&club.id).copied().unwrap_or(0),
            scout_countries,
            knowledge: std::cell::RefCell::new(FxHashMap::default()),
            market_map,
            placement_reach,
            date,
            open_requests,
        })
    }

    /// What this club actually wants, as opposed to what is simply available.
    fn foreign_interest(
        borrower: &ForeignBorrower<'_>,
        board: &ForeignLoanBoard<'_>,
        p: &PlayerSummary,
        fee: f64,
    ) -> Option<f32> {
        let taste = &borrower.taste;
        let country_id = borrower.country_id;
        let club_region = borrower.club_region;
        let borrower_position_depth = &borrower.borrower_position_depth;
        let open_requests = &borrower.open_requests;
        taste
            .interest_in(&LoanCandidateProfile {
                player_id: p.player_id,
                true_ability: p.skill_ability,
                age: p.age,
                is_development: ForeignUnsolicitedLoanTarget::is_development(p.age),
                fee,
                group_thinness: GroupPressure::thinness(
                    borrower_position_depth.headcount(p.position_group),
                    p.position_group,
                ),
                answers_open_request: open_requests
                    .iter()
                    .any(|r| r.position.position_group() == p.position_group),
            })
            .map(|score| {
                // Local supply is real: a club looks in its own
                // backyard first. Kept exactly as it was — the
                // home pull competes with it rather than
                // replacing it, and the census decides.
                let local = if p.region == club_region { 1.45 } else { 1.0 };
                // …and a man's own country pulls him back, in
                // proportion to how much he wants it. Note this reads
                // his NATIONALITY region against the borrower's, where
                // `local` above reads his CLUB's: collapse the two into
                // one field and the same-region term works AGAINST a
                // return home.
                // …and the geography the board already scored. The floor only
                // removes what no club would look at; between the candidates
                // that clear it, a club borrows out of the leagues it deals
                // with far more often than out of the ones it can merely see.
                // The draw cubes this ([`InterestDraw::SHARPNESS`]), so the
                // 0.39 a Brazilian at Zenit reads against the same man at
                // Flamengo is a 17× difference in odds.
                score
                    * local
                    * board.visibility(p)
                    * HomeLoanPull::factor(
                        p.nationality_country_id,
                        p.nationality_region,
                        country_id,
                        club_region,
                        p.return_home_desire,
                    )
            })
    }

    /// The club's own open requests, answered from abroad: one scored slate
    /// per request, in the order the club files them. Nobody has been
    /// claimed yet — [`ForeignTurn::claim`] takes those out.
    fn request_slates<'p>(
        borrower: &ForeignBorrower<'_>,
        board: &ForeignLoanBoard<'p>,
        date: NaiveDate,
        active_pairs: &FxHashSet<(u32, u32)>,
    ) -> Vec<RequestSlate<'p>> {
        if !borrower.ordinary_foreign_scan {
            return Vec::new();
        }
        let club = borrower.club;
        let plan = borrower.plan;
        let max_loan_fee = borrower.max_loan_fee;
        let avg_ability = borrower.avg_ability;
        let scout_regions = &borrower.scout_regions;
        let buyer_loan_ctx = &borrower.buyer_loan_ctx;
        let club_region = borrower.club_region;
        let country_id = borrower.country_id;

        plan.transfer_requests
            .iter()
            .filter(|r| {
                r.status != TransferRequestStatus::Fulfilled
                    && r.status != TransferRequestStatus::Abandoned
                    && !r.is_emergency_free_agent_depth()
            })
            .map(|request| {
                let pos_group = request.position.position_group();
                let relaxed_min = request.min_ability.saturating_sub(5);

                // Filter foreign loan players: must match position, ability,
                // be in a scout's known region, and be a realistic move
                // (players don't go from Serie A to the Nigerian league)
                let candidates = board.by_group[pos_group.index()]
                    .iter()
                    .copied()
                    .filter(|p| {
                        !club.is_rival(p.club_id)
                    && !plan.is_loan_approach_barred(p.player_id, date)
                    && p.skill_ability >= relaxed_min
                    && p.age <= request
                        .preferred_age_max
                        .saturating_add(3)
                        .min(MAX_LOAN_TARGET_AGE)
                    && p.age >= request.preferred_age_min
                    && p.estimated_value * 0.1 <= max_loan_fee
                    // Squad-average floor is for cover loans; a youth
                    // match-practice loan leans on the minutes gate.
                    && (ForeignUnsolicitedLoanTarget::is_development(p.age)
                        || p.skill_ability >= avg_ability.saturating_sub(10))
                    && !active_pairs.contains(&(p.player_id, club.id))
                    // Reputation reality check: players don't drop more than
                    // ~40% in league level on loan. A rep-8000 player won't
                    // go to a rep-2000 club. This prevents Serie A → Nigeria.
                    // Where a scout can see him, and whether the move is
                    // a credible one for a man of his standing at all.
                    && HomeLoanGates::reach_ok(
                        scout_regions.contains(&p.region),
                        p.nationality_country_id,
                        p.nationality_region,
                        country_id,
                        club_region,
                        p.home_return_wanted,
                        ForeignUnsolicitedLoanTarget::is_development(p.age),
                    )
                    && !matches!(
                        TransferPlausibilityBuilder::evaluate_summary(
                            buyer_loan_ctx,
                            p,
                            true,
                            true,
                            date,
                            Some(borrower.market_reach(board, p)),
                        ),
                        Some(TransferPlausibilityVerdict::HardReject(_))
                    )
                    })
                    .filter_map(|p| Self::scored(borrower, board, p))
                    .collect();

                RequestSlate {
                    group: pos_group,
                    // Same shape as the domestic request scan — the
                    // request's "why" goes on the loan's history row.
                    reason: request.reason.as_signing_reason_key(),
                    candidates,
                }
            })
            .collect()
    }

    /// A development pickup nobody asked for: a young player at a bigger club
    /// abroad who would get minutes here. Scored before the request branch
    /// has spent any position group — [`ForeignTurn::claim`] takes those out.
    fn prospect_slate<'p>(
        borrower: &ForeignBorrower<'_>,
        board: &ForeignLoanBoard<'p>,
        date: NaiveDate,
        active_pairs: &FxHashSet<(u32, u32)>,
    ) -> Vec<(&'p PlayerSummary, f32)> {
        let club = borrower.club;
        let plan = borrower.plan;
        let max_loan_fee = borrower.max_loan_fee;
        let scout_regions = &borrower.scout_regions;
        let buyer_loan_ctx = &borrower.buyer_loan_ctx;
        let club_region = borrower.club_region;
        let country_id = borrower.country_id;
        let scan_pool: &[&'p PlayerSummary] = if borrower.ordinary_foreign_scan {
            &board.loans
        } else {
            &board.compatriots
        };

        // ── Proactive foreign development pickup (no request needed) ──
        //
        // The request loop only signs a foreign loanee when THIS club
        // already asked for the position — so a giant's loan-listed
        // prospect (a River Plate youth keeper no domestic club can field
        // as a #1) has no cross-border outlet, because the seller
        // broadcast is domestic-only. Mirror the domestic opportunistic /
        // unsolicited scan here: a National / Regional club proactively
        // takes a loan-listed DEVELOPMENT prospect from a same-region,
        // equal-or-higher-rep country when it would actually play him.
        // The country-rep / region step-downs (already lifted for
        // development when `foreign_loans` was built) plus the
        // borrower-depth minutes gate keep the drop plausible and
        // guarantee minutes, so prospects circulate across the continent
        // (Argentina → Uruguay / Chile / …) without landing on a bench.
        // Bounded by the same per-club scan budget as the request path.
        scan_pool
            .iter()
            .copied()
            .filter(|p| {
                // A posted compatriot is his own advert — his
                // parent has told the world he would come home,
                // which is exactly the row a loan listing is.
                //
                // …and he is not held to the 23 the ordinary cold
                // pickup uses. `UnsettledAbroadScan` posts men up
                // to 25, and so does the manager-talk route, so a
                // posted 24- or 25-year-old compatriot passed
                // every pool filter and was then invisible to
                // every Elite / Continental club in his own
                // country — reachable only by a National side
                // with an open request in his position.
                (p.is_loan_listed || p.home_return_wanted)
            && LoanPipeline::home_pickup_age_ok(
                p.age,
                p.home_return_wanted,
                p.nationality_country_id,
                country_id,
            )
            && !club.is_rival(p.club_id)
            && !plan.is_loan_approach_barred(p.player_id, date)
            && p.estimated_value * 0.1 <= max_loan_fee
            && !active_pairs.contains(&(p.player_id, club.id))
            // Reputation reality band — identical to the
            // request path, so a prospect still can't drop
            // into a far smaller ecosystem than his club's.
            // Where a scout can see him, and whether the move is
            // a credible one for a man of his standing at all.
            && HomeLoanGates::reach_ok(
                scout_regions.contains(&p.region),
                p.nationality_country_id,
                p.nationality_region,
                country_id,
                club_region,
                p.home_return_wanted,
                true,
            )
            && !matches!(
                TransferPlausibilityBuilder::evaluate_summary(
                    buyer_loan_ctx,
                    p,
                    true,
                    true,
                    date,
                    Some(borrower.market_reach(board, p)),
                ),
                Some(TransferPlausibilityVerdict::HardReject(_))
            )
            })
            .filter_map(|p| Self::scored(borrower, board, p))
            .collect()
    }

    /// What the three sides would agree on — the parent's willingness, the
    /// borrower's appetite, his own consent and the money — times what this
    /// club actually wants. `None` drops him from the slate.
    fn scored<'p>(
        borrower: &ForeignBorrower<'_>,
        board: &ForeignLoanBoard<'_>,
        p: &'p PlayerSummary,
    ) -> Option<(&'p PlayerSummary, f32)> {
        let agreed = borrower.agreement_for(board, p)?;
        Self::foreign_interest(borrower, board, p, p.estimated_value * 0.1 * 0.8)
            .map(|score| (p, score * agreed))
    }

    /// The single writer: open a negotiation for every approach the clubs made.
    fn commit(country: &mut Country, date: NaiveDate, actions: Vec<ForeignLoanAction>) {
        if actions.is_empty() {
            return;
        }
        if actions.is_empty() {
            return;
        }

        // Create listings and negotiations for foreign loan targets
        for action in actions {
            let asking_price = CurrencyValue {
                amount: action.offer_amount,
                currency: Currency::Usd,
            };

            // Unsolicited foreign approaches target players their club never
            // loan-listed: tag the backing listing synthetic so the resolver
            // withholds the "seller-advertised" acceptance bonus. A genuinely
            // loan-listed foreign target keeps the seller-advertised origin.
            let is_unsolicited = !action.player.is_loan_listed;
            let listing = TransferListing::new_with_origin(
                action.player.player_id,
                action.player.club_id,
                0, // Foreign team — no local team_id
                asking_price.clone(),
                date,
                TransferListingType::Loan,
                if is_unsolicited {
                    TransferListingOrigin::SyntheticUnsolicited
                } else {
                    TransferListingOrigin::LoanOutListed
                },
            );
            country.transfer_market.add_listing(listing);

            let buying_rep = ClubView::get_club_reputation(country, action.club_id);
            // Use a reasonable estimate for selling club rep
            let selling_rep = (action.player.skill_ability as f32 / 200.0).clamp(0.1, 0.9);

            // Same as the domestic loan path — explicit months-side
            // duration so the market history record matches the actual
            // loan length. The borrower is in the scanning country.
            // A boy going home comes with a buy option. It is how these
            // deals are actually written — Gerson, Kaio Jorge, Matheus
            // Pereira all converted — and it is where the ~30 % conversion
            // band in the census comes from. The option is withheld on an
            // ordinary cold approach (a club does not price a permanent it
            // has not asked about); a posted compatriot is not a cold
            // approach, it is a homecoming his parent advertised.
            let coming_home = action.player.home_return_wanted
                && action.player.nationality_country_id != 0
                && action.player.nationality_country_id == country.id;
            let mut clauses = Vec::new();
            if coming_home && action.player.estimated_value > 0.0 {
                clauses.push(TransferClause::LoanOptionToBuy(CurrencyValue {
                    amount: FormattingUtils::round_fee(
                        action.player.estimated_value * LoanPipeline::LOAN_OPTION_VALUE_FRACTION,
                    ),
                    currency: Currency::Usd,
                }));
            }
            let offer = TransferOffer {
                base_fee: asking_price,
                clauses,
                contract_length_years: None,
                loan_duration_months: Some(LoanPipeline::loan_duration_to_season_end(
                    country,
                    action.club_id,
                    date,
                )),
                personal_terms: Some(PersonalTermsOffer {
                    // A loan promises a shirt: minutes are what it is
                    // for (B4 / [`LoanPromise`]). No wage — the borrower
                    // picks up a share of the deal he already has.
                    squad_status_promise: LoanPromise::for_loan(action.is_development),
                    ..PersonalTermsOffer::default()
                }),
                offering_club_id: action.club_id,
                offered_date: date,
            };

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                action.player.player_id,
                action.club_id,
                offer,
                date,
                selling_rep,
                buying_rep,
                action.player.age,
                action.player.determination,
            ) {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_loan = true;
                    negotiation.has_option_to_buy = coming_home;
                    negotiation.is_unsolicited = is_unsolicited;
                    negotiation.reason = action.reason;
                    negotiation.selling_country_id = Some(action.player.country_id);
                    negotiation.selling_continent_id = Some(action.player.continent_id);
                    negotiation.selling_country_code = action.player.country_code.clone();
                    negotiation.player_name = action.player.player_name.clone();
                    negotiation.selling_club_name = action.player.club_name.clone();
                    // The borrower depth fold can't resolve a foreign
                    // player in-country — the stamped profile keeps this
                    // in-flight loan visible to the position cap.
                    negotiation.loan_target_profile =
                        Some((action.player.position_group, action.player.skill_ability));
                    // The player is in another country's borrow by the time
                    // he is asked, so his side of the appraisal is captured
                    // here from the market summary — which is exactly what
                    // this borrower actually knows about him. Without it a
                    // cross-border loan fell back to the bare prestige wall
                    // that made going home the least likely destination on
                    // the board (L7.6).
                    negotiation.staged_stance = Some(PlayerStanceBuilder::from_summary(
                        &action.player,
                        action.player_importance,
                        action.player.region,
                    ));
                    negotiation.staged_sporting_drop = Some(action.sporting_drop);
                    // The parent's own answer about this destination, and
                    // the passport the borrower's quota has to count while
                    // the approach is in flight.
                    negotiation.staged_loan_verdict = action.loan_verdict;
                    negotiation.loan_target_country = Some(action.player.nationality_country_id);
                    negotiation.open_salary_at(action.player.salary);
                }

                if let Some(buyer) = country.clubs.iter_mut().find(|c| c.id == action.club_id)
                    && action.from_compatriot_sweep
                {
                    // One homecoming per window. The sweep is a door
                    // for the boy a league produced, not a licence to
                    // shop abroad.
                    buyer.transfer_plan.compatriot_sweeps_this_window = buyer
                        .transfer_plan
                        .compatriot_sweeps_this_window
                        .saturating_add(1);
                }

                debug!(
                    "Foreign loan scan: Club {} started foreign loan negotiation for player {} from country {}",
                    action.club_id, action.player.player_id, action.player.country_id
                );
            }
        }
    }
}
