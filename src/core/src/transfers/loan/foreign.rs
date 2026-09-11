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

use chrono::NaiveDate;
use log::debug;

use crate::shared::{Currency, CurrencyValue};
use crate::transfers::ScoutingRegion;
use crate::transfers::deal::offer::{PersonalTermsOffer, TransferClause, TransferOffer};
use crate::transfers::deal::reason::TransferReason;
use crate::transfers::gate::stance::PlayerStanceBuilder;
use crate::transfers::gate::{
    BuyerPlausibilityContext, TransferPlausibilityBuilder, TransferPlausibilityVerdict,
};
use crate::transfers::loan::interest::{
    BorrowerTaste, GroupPressure, InterestDraw, LoanCandidateProfile,
};
use crate::transfers::market::{TransferListing, TransferListingOrigin, TransferListingType};
use crate::transfers::pipeline::TransferRequestStatus;
use crate::transfers::pipeline::processor::{PipelineProcessor, PlayerSummary};
use crate::transfers::pipeline::trace::MarketSwitches;
use crate::transfers::squad::minutes::LoanPromise;
use crate::transfers::{MarketAffinity, MarketAffinityInputs, MarketMap, MoveKind};
use crate::utils::FormattingUtils;
use crate::{Club, Country, PlayerFieldPositionGroup, ReputationLevel, Team};
use std::collections::HashMap;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::transfers::pipeline::ClubTransferPlan;

use super::*;

/// Everyone outside this country who could plausibly be borrowed, plus the
/// geography that decides who can even be seen from here.
struct ForeignLoanBoard<'a> {
    loans: Vec<&'a PlayerSummary>,
    compatriots: Vec<&'a PlayerSummary>,
    by_group: [Vec<&'a PlayerSummary>; PlayerFieldPositionGroup::COUNT],
    is_january: bool,
}

/// One borrowing club's cross-border turn, resolved once.
struct ForeignBorrower<'a> {
    country_id: u32,
    club_region: ScoutingRegion,
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
    open_request_groups: Vec<PlayerFieldPositionGroup>,
}

/// What carries across one club's two cross-border sweeps.
struct ForeignScanState {
    scans: usize,
    scanned_position_groups: Vec<PlayerFieldPositionGroup>,
}

/// What one club's turn needs from the rest of the market: who already has a
/// pursuit in flight, and how many negotiations each club is carrying.
struct MarketLoad {
    pending_loans: HashMap<u32, Vec<(PlayerFieldPositionGroup, u8)>>,
    active_counts: FxHashMap<u32, u32>,
    active_pairs: FxHashSet<(u32, u32)>,
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
}

impl ForeignLoanScan {
    pub(in crate::transfers::loan) fn run(
        country: &mut Country,
        foreign_players: &[&PlayerSummary],
        date: NaiveDate,
        market_map: &MarketMap,
    ) {
        if foreign_players.is_empty() {
            return;
        }
        let Some(board) = Self::board(country, foreign_players, date, market_map) else {
            return;
        };

        let mut actions: Vec<ForeignLoanAction> = Vec::new();
        let load = MarketLoad {
            pending_loans: PipelineProcessor::pending_incoming_loans_by_club(country),
            active_counts: country.transfer_market.active_negotiation_counts(),
            active_pairs: country.transfer_market.active_negotiation_pairs(),
        };

        for club_idx in InterestDraw::visit_order(country.clubs.len()) {
            let Some(borrower) = Self::borrower(country, club_idx, &board, date, &load) else {
                continue;
            };
            let mut state = ForeignScanState {
                scans: 0,
                scanned_position_groups: Vec::new(),
            };
            Self::against_requests(
                country,
                &borrower,
                &board,
                date,
                &load.active_pairs,
                &mut state,
                &mut actions,
            );
            Self::proactively(
                country,
                &borrower,
                &board,
                date,
                &load.active_pairs,
                &mut state,
                &mut actions,
            );
        }

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
        let is_january = PipelineProcessor::is_mid_season_window_for(country, date);

        // The scanning country's own region — used to block loans from
        // clearly more prestigious regions (Paraguay can't loan from England).
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
            if market_map.is_silent() {
                return 1.0;
            }
            let key = (p.nationality_country_id, p.country_id);
            if let Some(cached) = visibility_cache.get(&key) {
                return *cached;
            }
            let affinity = MarketAffinity::affinity(
                market_map,
                MarketAffinityInputs {
                    buyer_country_id: country_id,
                    nationality_country_id: key.0,
                    current_country_id: key.1,
                    kind: MoveKind::Talent,
                    // Scanned per country, one answer for the league.
                    benefactor: 0.0,
                },
            );
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
                    PipelineProcessor::foreign_loan_country_rep_ok(
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
                    PipelineProcessor::foreign_loan_region_ok(
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
                // Corridor visibility. A borrowing club looks in the
                // markets its league works, the same as a buying one — the
                // loan market is not a separate geography. Home is exempt
                // by construction (affinity 1.0), which is what keeps the
                // loan-home pathway untouched.
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

        // Position-group partition for the request loop below — each
        // request used to walk the whole filtered pool to reject the
        // other three groups candidate by candidate. The proactive
        // development pickup after the request loop crosses groups and
        // keeps using the flat `foreign_loans`. Selection is a weighted
        // draw over the whole qualified slate either way, so the
        // partition is a cost saving and nothing more — it no longer
        // has to preserve an ordering for a tie-break to land on.
        let mut foreign_loans_by_group: [Vec<&PlayerSummary>; PlayerFieldPositionGroup::COUNT] =
            Default::default();
        for p in &foreign_loans {
            foreign_loans_by_group[p.position_group.index()].push(*p);
        }

        Some(ForeignLoanBoard {
            loans: foreign_loans,
            compatriots,
            by_group: foreign_loans_by_group,
            is_january,
        })
    }

    /// `None` when this club is not in the cross-border market at all.
    fn borrower<'a>(
        country: &'a Country,
        club_idx: usize,
        board: &ForeignLoanBoard<'_>,
        date: NaiveDate,
        load: &MarketLoad,
    ) -> Option<ForeignBorrower<'a>> {
        let is_january = board.is_january;
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
            ReputationLevel::Continental => is_january && club.finance.balance.balance < 0,
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
        // It used to be a country-wide `any()` that simply flipped
        // `should_scan_foreign`, after which both branches iterated
        // the FULL foreign pool with no compatriot restriction: one
        // posted English 21-year-old at Ajax had every Elite English
        // club running the ordinary foreign scan every pass for as
        // long as he stayed posted.
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
        let borrower_league_rep = PipelineProcessor::club_league_reputation(country, club);

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

        // Check unfulfilled transfer requests. Emergency
        // free-agent depth requests are excluded — they're
        // serviced by the free-agent matcher only, not by loans.
        let _scans = 0usize;
        let max_scans: usize = match rep_level {
            ReputationLevel::Elite => 3,
            ReputationLevel::Continental => 2,
            _ => 1,
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
        let open_request_groups: Vec<PlayerFieldPositionGroup> = plan
            .transfer_requests
            .iter()
            .filter(|r| {
                r.status != TransferRequestStatus::Fulfilled
                    && r.status != TransferRequestStatus::Abandoned
            })
            .map(|r| r.position.position_group())
            .collect();
        // Cultural proximity stays a preference, as it was — but as a

        // The request path walks the whole foreign pool by position
        // group, so it belongs to the ordinary scan alone — a

        Some(ForeignBorrower {
            country_id: country.id,
            club_region: ScoutingRegion::from_country(country.continent_id, &country.code),
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
            open_request_groups,
        })
    }

    /// What this club actually wants, as opposed to what is simply available.
    fn foreign_interest(
        borrower: &ForeignBorrower<'_>,
        p: &PlayerSummary,
        fee: f64,
    ) -> Option<f32> {
        let taste = &borrower.taste;
        let country_id = borrower.country_id;
        let club_region = borrower.club_region;
        let borrower_position_depth = &borrower.borrower_position_depth;
        let open_request_groups = &borrower.open_request_groups;
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
                answers_open_request: open_request_groups.contains(&p.position_group),
            })
            .map(|score| {
                // Local supply is real: a club looks in its own
                // backyard first. Kept exactly as it was — the
                // home pull competes with it rather than
                // replacing it, and the census decides.
                let local = if p.region == club_region { 1.45 } else { 1.0 };
                // …and a man's own country pulls him back, in
                // proportion to how much he wants it. Note this
                // reads his NATIONALITY region against the
                // borrower's, where `local` above reads his
                // CLUB's — the two used to be the same field, and
                // the same-region term therefore worked AGAINST a
                // return home.
                score
                    * local
                    * HomeLoanPull::factor(
                        p.nationality_country_id,
                        p.nationality_region,
                        country_id,
                        club_region,
                        p.return_home_desire,
                    )
            })
    }

    /// The club's own open requests, answered from abroad.
    fn against_requests(
        _country: &Country,
        borrower: &ForeignBorrower<'_>,
        board: &ForeignLoanBoard<'_>,
        date: NaiveDate,
        active_pairs: &FxHashSet<(u32, u32)>,
        state: &mut ForeignScanState,
        actions: &mut Vec<ForeignLoanAction>,
    ) {
        let club = borrower.club;
        let plan = borrower.plan;
        let team = borrower.team;
        let _team_rep = borrower.team_rep;
        let ordinary_foreign_scan = borrower.ordinary_foreign_scan;
        let _compatriot_sweep = borrower.compatriot_sweep;
        let borrower_league_rep = borrower.borrower_league_rep;
        let max_loan_fee = borrower.max_loan_fee;
        let avg_ability = borrower.avg_ability;
        let scout_regions = &borrower.scout_regions;
        let max_scans = borrower.max_scans;
        let borrower_position_depth = &borrower.borrower_position_depth;
        let buyer_loan_ctx = &borrower.buyer_loan_ctx;
        let _taste = &borrower.taste;
        let _open_request_groups = &borrower.open_request_groups;
        let club_region = borrower.club_region;
        let country_id = borrower.country_id;
        let _is_january = board.is_january;
        let _compatriots = &board.compatriots;
        let foreign_borrower_for =
            |group: PlayerFieldPositionGroup| -> Option<LoanBorrowerProfile> {
                borrower.foreign_borrower_profile.map(|p| {
                    p.with_best_in_group(borrower.borrower_position_depth.best_in_group(group))
                })
            };
        let foreign_interest = |p: &PlayerSummary, fee: f64| -> Option<f32> {
            Self::foreign_interest(borrower, p, fee)
        };
        let scans = &mut state.scans;
        let scanned_position_groups = &mut state.scanned_position_groups;
        let foreign_loans_by_group = &board.by_group;
        let unfulfilled = plan.transfer_requests.iter().filter(|r| {
            r.status != TransferRequestStatus::Fulfilled
                && r.status != TransferRequestStatus::Abandoned
                && !r.is_emergency_free_agent_depth()
        });

        for request in unfulfilled.filter(|_| ordinary_foreign_scan) {
            if *scans >= max_scans {
                break;
            }

            let pos_group = request.position.position_group();
            if scanned_position_groups.contains(&pos_group) {
                continue;
            }

            let relaxed_min = request.min_ability.saturating_sub(5);

            // Filter foreign loan players: must match position, ability,
            // be in a scout's known region, and be a realistic move
            // (players don't go from Serie A to the Nigerian league)
            let team_rep = team.reputation.world;
            let qualified: Vec<&&PlayerSummary> = foreign_loans_by_group[pos_group.index()]
                .iter()
                .filter(|p| {
                    !club.is_rival(p.club_id)
                && !plan.is_loan_approach_barred(p.player_id, date)
                && p.position_group == request.position.position_group()
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
                && !actions.iter().any(|a| a.player.player_id == p.player_id)
                // Reputation reality check: players don't drop more than
                // ~40% in league level on loan. A rep-8000 player won't
                // go to a rep-2000 club. This prevents Serie A → Nigeria.
                && p.home_reputation <= (team_rep as f32 * 2.0) as i16
                && team_rep >= (p.home_reputation.max(0) as f32 * 0.35) as u16
                // `p.region` is stamped from the same
                // (continent, country-code) resolve at pool
                // build time — no per-candidate re-resolve.
                && HomeLoanGates::reach_ok(
                    scout_regions.contains(&p.region),
                    p.nationality_country_id,
                    p.nationality_region,
                    country_id,
                    club_region,
                    p.home_return_wanted,
                    ForeignUnsolicitedLoanTarget::is_development(p.age),
                )
                // Borrower-depth gate: if the borrower's
                // squad is already full at this position
                // group, accept only when the incoming
                // player is clearly better than what's
                // already there. Mirrors the domestic
                // `should_skip_loan` shape so we don't
                // import a 4th mid-tier GK on top of three.
                && borrower_position_depth.has_room_for(
                    p.position_group,
                    p.skill_ability,
                    ForeignUnsolicitedLoanTarget::is_development(p.age),
                )
                // The loan must buy minutes — not a bench
                // seat behind a wall of better players.
                // Young loanees (the development profile)
                // need the stricter expected-minutes bar.
                && borrower_position_depth.would_get_loan_minutes(
                    p.position_group,
                    p.skill_ability,
                    ForeignUnsolicitedLoanTarget::is_development(p.age),
                    p.club_best_in_group,
                )
                // Parent-club reputation drop — same realism
                // gate as the domestic scan, anchored on the
                // player's own club rather than his personal
                // reputation.
                && PipelineProcessor::loan_level_ok(
                    team_rep,
                    p.club_world_reputation.max(0) as u16,
                    p.skill_ability,
                    p.club_best_in_group,
                    ForeignUnsolicitedLoanTarget::is_development(p.age),
                    // Both competitions are in hand on a
                    // cross-border loan: his is on the
                    // summary, the borrower's is local (C12).
                    p.seller_ctx.league_reputation,
                    borrower_league_rep,
                )
                // Staged cross-border veto: an important player at
                // a much stronger club abroad isn't a credible
                // loan target even when loan-listed, unless his
                // availability genuinely opens the move. Mirrors
                // the permanent foreign gate.
                && !matches!(
                    TransferPlausibilityBuilder::evaluate_summary(
                        &buyer_loan_ctx,
                        p,
                        true,
                        true,
                        date,
                            None,
                    ),
                    Some(TransferPlausibilityVerdict::HardReject(_))
                )
                // The asset's own price on this destination —
                // the two money terms cross a border unchanged.
                && PipelineProcessor::foreign_loan_guard_allows(
                    p,
                    foreign_borrower_for(p.position_group).as_ref(),
                )
                })
                .collect();

            let weighted: Vec<(u32, f32)> = qualified
                .iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    foreign_interest(p, p.estimated_value * 0.1 * 0.8)
                        .map(|score| (i as u32, score))
                })
                .collect();

            if let Some(best) = InterestDraw::pick(&weighted).map(|i| *qualified[i as usize]) {
                let loan_fee = FormattingUtils::round_fee(best.estimated_value * 0.1 * 0.8);
                // Same shape as the domestic request scan — the empty
                // `format!` here dropped the request's "why" from every
                // foreign request-driven loan's history row.
                let reason = TransferReason::key(request.reason.as_signing_reason_key());
                let staged = ForeignLoanStance::read(&buyer_loan_ctx, best, date);
                actions.push(ForeignLoanAction {
                    club_id: club.id,
                    player: (*best).clone(),
                    offer_amount: loan_fee,
                    reason,
                    is_development: ForeignUnsolicitedLoanTarget::is_development(best.age),
                    from_compatriot_sweep: false,
                    player_importance: staged.0,
                    sporting_drop: staged.1,
                });
                scanned_position_groups.push(pos_group);
                *scans += 1;
            }
        }
    }

    /// A development pickup nobody asked for: a young player at a bigger club
    /// abroad who would get minutes here.
    fn proactively(
        _country: &Country,
        borrower: &ForeignBorrower<'_>,
        board: &ForeignLoanBoard<'_>,
        date: NaiveDate,
        active_pairs: &FxHashSet<(u32, u32)>,
        state: &mut ForeignScanState,
        actions: &mut Vec<ForeignLoanAction>,
    ) {
        let club = borrower.club;
        let plan = borrower.plan;
        let team = borrower.team;
        let _team_rep = borrower.team_rep;
        let ordinary_foreign_scan = borrower.ordinary_foreign_scan;
        let compatriot_sweep = borrower.compatriot_sweep;
        let borrower_league_rep = borrower.borrower_league_rep;
        let max_loan_fee = borrower.max_loan_fee;
        let _avg_ability = borrower.avg_ability;
        let scout_regions = &borrower.scout_regions;
        let max_scans = borrower.max_scans;
        let borrower_position_depth = &borrower.borrower_position_depth;
        let buyer_loan_ctx = &borrower.buyer_loan_ctx;
        let _taste = &borrower.taste;
        let _open_request_groups = &borrower.open_request_groups;
        let club_region = borrower.club_region;
        let country_id = borrower.country_id;
        let _is_january = board.is_january;
        let _compatriots = &board.compatriots;
        let foreign_borrower_for =
            |group: PlayerFieldPositionGroup| -> Option<LoanBorrowerProfile> {
                borrower.foreign_borrower_profile.map(|p| {
                    p.with_best_in_group(borrower.borrower_position_depth.best_in_group(group))
                })
            };
        let foreign_interest = |p: &PlayerSummary, fee: f64| -> Option<f32> {
            Self::foreign_interest(borrower, p, fee)
        };
        let scans = &mut state.scans;
        let scanned_position_groups = &mut state.scanned_position_groups;
        let foreign_loans = &board.loans;
        let compatriots = &board.compatriots;
        let scan_pool: &[&PlayerSummary] = if ordinary_foreign_scan {
            foreign_loans
        } else {
            compatriots
        };

        // ── Proactive foreign development pickup (no request needed) ──
        //
        // The request loop above only signs a foreign loanee when THIS
        // club already asked for the position — so a giant's loan-listed
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
        let team_rep = team.reputation.world;
        if *scans < max_scans {
            let prospects: Vec<&&PlayerSummary> = scan_pool
                .iter()
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
                && PipelineProcessor::home_pickup_age_ok(
                    p.age,
                    p.home_return_wanted,
                    p.nationality_country_id,
                    country_id,
                )
                && !club.is_rival(p.club_id)
                && !plan.is_loan_approach_barred(p.player_id, date)
                && !scanned_position_groups.contains(&p.position_group)
                && p.estimated_value * 0.1 <= max_loan_fee
                && !active_pairs.contains(&(p.player_id, club.id))
                && !actions.iter().any(|a| a.player.player_id == p.player_id)
                // Reputation reality band — identical to the
                // request path, so a prospect still can't drop
                // into a far smaller ecosystem than his club's.
                && p.home_reputation <= (team_rep as f32 * 2.0) as i16
                && team_rep >= (p.home_reputation.max(0) as f32 * 0.35) as u16
                && HomeLoanGates::reach_ok(
                    scout_regions.contains(&p.region),
                    p.nationality_country_id,
                    p.nationality_region,
                    country_id,
                    club_region,
                    p.home_return_wanted,
                    true,
                )
                // Development profile throughout (gated above), so
                // every borrower-side gate runs at its dev setting:
                // the relaxed keeper room check (Fix A) lets him
                // into a full-but-weak line, and the minutes gate
                // guarantees he competes rather than sits.
                && borrower_position_depth.has_room_for(
                    p.position_group,
                    p.skill_ability,
                    true,
                )
                && borrower_position_depth.would_get_loan_minutes(
                    p.position_group,
                    p.skill_ability,
                    true,
                    p.club_best_in_group,
                )
                && PipelineProcessor::loan_level_ok(
                    team_rep,
                    p.club_world_reputation.max(0) as u16,
                    p.skill_ability,
                    p.club_best_in_group,
                    true,
                    // Both competitions are in hand on a
                    // cross-border loan: his is on the
                    // summary, the borrower's is local (C12).
                    p.seller_ctx.league_reputation,
                    borrower_league_rep,
                )
                && !matches!(
                    TransferPlausibilityBuilder::evaluate_summary(
                        &buyer_loan_ctx,
                        p,
                        true,
                        true,
                        date,
                            None,
                    ),
                    Some(TransferPlausibilityVerdict::HardReject(_))
                )
                // The asset's own price on this destination —
                // the two money terms cross a border unchanged.
                && PipelineProcessor::foreign_loan_guard_allows(
                    p,
                    foreign_borrower_for(p.position_group).as_ref(),
                )
                })
                .collect();

            // Same-region prospects still circulate locally first — that
            // preference now lives in the weight, so a genuinely better fit
            // on another continent is reachable instead of unreachable.
            let weighted: Vec<(u32, f32)> = prospects
                .iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    foreign_interest(p, p.estimated_value * 0.1 * 0.8)
                        .map(|score| (i as u32, score))
                })
                .collect();

            if let Some(best) = InterestDraw::pick(&weighted).map(|i| *prospects[i as usize]) {
                // Terminal branch for this club: nothing after this reads
                // the scan counter or the scanned-groups set, so the loan
                // action is all that's needed.
                let loan_fee = FormattingUtils::round_fee(best.estimated_value * 0.1 * 0.8);
                let staged = ForeignLoanStance::read(&buyer_loan_ctx, best, date);
                actions.push(ForeignLoanAction {
                    club_id: club.id,
                    player: (*best).clone(),
                    offer_amount: loan_fee,
                    reason: TransferReason::key("signing_reason_loan_foreign_prospect"),
                    is_development: ForeignUnsolicitedLoanTarget::is_development(best.age),
                    player_importance: staged.0,
                    sporting_drop: staged.1,
                    from_compatriot_sweep: compatriot_sweep,
                });
            }
        }
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

            let buying_rep = PipelineProcessor::get_club_reputation(country, action.club_id);
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
                        action.player.estimated_value
                            * PipelineProcessor::LOAN_OPTION_VALUE_FRACTION,
                    ),
                    currency: Currency::Usd,
                }));
            }
            let offer = TransferOffer {
                base_fee: asking_price,
                clauses,
                contract_length_years: None,
                loan_duration_months: Some(PipelineProcessor::loan_duration_to_season_end(
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
                    negotiation.open_salary_at(action.player.salary);
                }

                // Same standoff the domestic scan stamps — a club that has
                // moved for a foreign target doesn't reopen the file the
                // following tick.
                if let Some(buyer) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                    buyer
                        .transfer_plan
                        .record_loan_approach(action.player.player_id, date);
                    if action.from_compatriot_sweep {
                        // One homecoming per window. The sweep is a door
                        // for the boy a league produced, not a licence to
                        // shop abroad.
                        buyer.transfer_plan.compatriot_sweeps_this_window = buyer
                            .transfer_plan
                            .compatriot_sweeps_this_window
                            .saturating_add(1);
                    }
                }

                debug!(
                    "Foreign loan scan: Club {} started foreign loan negotiation for player {} from country {}",
                    action.club_id, action.player.player_id, action.player.country_id
                );
            }
        }
    }
}
