//! The borrowing half of the loan market: who is available, and which club
//! asks about them.
//!
//! Three stages, which the comments inside the old 1 082-line
//! `scan_loan_market` already named as "Pass 1 read" and "Pass 2". The board
//! is read once for the whole country — the players their clubs listed, plus
//! the ones a bigger club never listed but would let go — and then every club
//! that has appetite is offered it, in a rotated order so registration id is
//! not first refusal on the whole market.
//!
//! A borrower looks four times, in this order, each stopping at the same
//! per-club scan cap: against its own open requests, opportunistically if it
//! is small or short, again in January if it is neither, and finally cold — a
//! player his club never loan-listed at all. The order is load-bearing: the
//! cap and the per-group dedupe carry across all four.

use chrono::{Datelike, NaiveDate, Weekday};
use log::debug;

use crate::club::team::squad::SquadAssetContext;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::deal::offer::{PersonalTermsOffer, TransferClause, TransferOffer};
use crate::transfers::deal::reason::TransferReason;
use crate::transfers::loan::interest::{
    BorrowerTaste, GroupPressure, InterestDraw, LoanCandidateProfile,
};
use crate::transfers::market::{
    TransferListing, TransferListingOrigin, TransferListingStatus, TransferListingType,
};
use crate::transfers::pipeline::TransferRequestStatus;
use crate::transfers::pipeline::processor::PipelineProcessor;
use crate::transfers::pipeline::trace::{MarketSwitches, TransferTrace};
use crate::transfers::squad::minutes::LoanPromise;
use crate::transfers::value::PlayerValuationCalculator;
use crate::utils::FormattingUtils;
use crate::{
    Club, Country, Person, PlayerFieldPositionGroup, PlayerSquadStatus, ReputationLevel, TeamType,
};
use std::collections::HashMap;

use super::*;
/// A player the market is offering on loan — listed by his club, or one a
/// bigger club never listed but would let go.
// Collect available loan listings (Pass 1 read)
struct LoanListing {
    player_id: u32,
    club_id: u32,
    asking_price: f64,
    ability: u8,
    age: u8,
    position_group: PlayerFieldPositionGroup,
    /// Parent club's main-team world reputation — drives the
    /// reputation-drop realism gate on the borrower side.
    parent_rep: u16,
    /// Best CA at the player's position group on the parent's
    /// main roster. A player far below it is "very raw" and
    /// tolerates a much bigger reputation drop.
    parent_best_in_group: u8,
    /// Parent listed this player via the development pathway —
    /// the loan exists to buy minutes, so the borrower-side
    /// expected-minutes gate runs at its stricter bar.
    is_development: bool,
    /// Reputation of the competition the parent plays in — the
    /// standard the loanee is dropping down from. Drives the
    /// division-level gate ([`Self::loan_league_level_ok`]).
    parent_league_rep: u16,
    /// The asset's own price on this move — what he is worth
    /// against a borrower's year and what his wage costs it.
    /// `None` only when the parent side could not be read at all
    /// (no contract, no squad), which stands the guard down.
    guard: Option<LoanAssetGuard>,
}

/// A loan approach one club decided to make. The scan reads the whole
/// country and cannot write, so every sweep stages here.
struct LoanScanAction {
    club_id: u32,
    player_id: u32,
    selling_club_id: u32,
    offer_amount: f64,
    reason: TransferReason,
    /// Cold approach for a player his club never loan-listed: Pass 2
    /// fabricates a synthetic listing and tags the negotiation
    /// unsolicited so the resolver withholds the "advertised" bonus.
    is_unsolicited: bool,
    /// Seller-side asking that backs that synthetic listing. Equal to
    /// the real listing's asking for listed targets (then unused).
    seller_asking: f64,
    /// The loan exists to buy minutes rather than to fill a hole —
    /// the borrower cleared `would_get_loan_minutes` at its stricter
    /// bar. Decides the shirt the offer promises ([`LoanPromise`]).
    is_development: bool,
}

impl LoanOptionContext for LoanScanAction {
    fn player_id(&self) -> u32 {
        self.player_id
    }
    fn selling_club_id(&self) -> u32 {
        self.selling_club_id
    }
    fn is_unsolicited(&self) -> bool {
        self.is_unsolicited
    }
}

/// The country's loan board, read once for the whole pass.
struct LoanBoard {
    listed: Vec<LoanListing>,
    unsolicited: Vec<LoanListing>,
}

impl LoanBoard {
    fn read(country: &Country, date: NaiveDate) -> Self {
        LoanBoard {
            listed: Self::listed(country, date),
            unsolicited: Self::unsolicited(country, date),
        }
    }

    fn is_empty(&self) -> bool {
        self.listed.is_empty() && self.unsolicited.is_empty()
    }

    /// Players their own club put on the market with the `Loa` badge.
    fn listed(country: &Country, date: NaiveDate) -> Vec<LoanListing> {
        let mut loan_listings: Vec<LoanListing> = Vec::new();

        for listing in &country.transfer_market.listings {
            if listing.listing_type != TransferListingType::Loan {
                continue;
            }
            if listing.status != TransferListingStatus::Available {
                continue;
            }
            // Synthetic rows fabricated to back a cold approach are NOT
            // seller loan availability. A failed unsolicited negotiation
            // reopened its synthetic listing, and this scan then treated
            // an unlisted prospect as permanently loan-listed at asking 0
            // — every club in the country could "sign" him off a listing
            // his parent never made.
            if !listing.is_seller_advertised() {
                continue;
            }
            if let Some(player) =
                PipelineProcessor::find_player_in_country(country, listing.player_id)
            {
                // Skip players already on loan — can't re-loan
                if player.is_on_loan() {
                    continue;
                }
                let group = player.position().position_group();
                let parent_club = country.clubs.iter().find(|c| c.id == listing.club_id);
                let parent_team =
                    parent_club.and_then(|c| c.teams.main().or(c.teams.teams.first()));
                let parent_rep = parent_team.map(|t| t.reputation.world).unwrap_or(0);
                let parent_best_in_group = parent_team
                    .map(|t| {
                        t.players
                            .iter()
                            .filter(|p| p.position().position_group() == group)
                            .map(|p| p.player_attributes.current_ability)
                            .max()
                            .unwrap_or(0)
                    })
                    .unwrap_or(0);
                let guard = parent_club
                    .and_then(|c| PipelineProcessor::loan_guard_for(country, c, player, date));
                // Treat as a "development" move (stricter minutes gate so he
                // actually plays, relaxed reputation/level floors so he can
                // drop a level or two to do so) either when the loan is
                // game-time-driven (development pathway, blocked prospect,
                // needs-minutes) or when the player is genuinely below his
                // own club's level. "Under 23" alone used to be enough,
                // which handed the whole allowance to a teenager who was
                // already his club's first-choice. Pure older-surplus /
                // financial loans keep the looser cover bar.
                let is_development =
                    PipelineProcessor::is_development_loan(guard.as_ref(), player.age(date))
                        || parent_club
                            .map(|c| {
                                c.transfer_plan.loan_out_candidates.iter().any(|cand| {
                                    cand.player_id == listing.player_id
                                        && cand.reason.expects_guaranteed_minutes()
                                })
                            })
                            .unwrap_or(false);
                loan_listings.push(LoanListing {
                    player_id: listing.player_id,
                    club_id: listing.club_id,
                    asking_price: listing.asking_price.amount,
                    ability: player.player_attributes.current_ability,
                    age: player.age(date),
                    position_group: group,
                    parent_rep,
                    parent_best_in_group,
                    is_development,
                    parent_league_rep: parent_club
                        .map(|c| PipelineProcessor::club_league_reputation(country, c))
                        .unwrap_or(0),
                    guard,
                });
            }
        }
        loan_listings
    }

    /// Players nobody listed. A lower- or other-league club can approach a
    /// bigger one about a player who has NOT been loan-listed — the badge is
    /// not a precondition for loan demand. Built weekly so the squad-wide scan
    /// stays cheap, while the listed market above is read daily.
    fn unsolicited(country: &Country, date: NaiveDate) -> Vec<LoanListing> {
        // ── Unsolicited loan targets (no `Loa` required) ─────────────
        //
        // A lower- or other-league club can approach a bigger club to take a
        // player on loan even when that player has NOT been loan-listed — the
        // `Loa` badge is not a precondition for loan demand. Built once per
        // pass on a weekly (Monday) cadence so the squad-wide scan stays
        // cheap; the listed-market scan above stays daily. Eligibility is the
        // central squad-asset classifier's job (see `UnsolicitedLoanTarget`):
        // young prospects and rotation players go as development loans and
        // genuine surplus goes at any age up to the loan cap — but a
        // first-team contributor is never cold-approached.
        let scan_unsolicited = date.weekday() == Weekday::Mon;
        let mut unsolicited_targets: Vec<LoanListing> = Vec::new();
        if scan_unsolicited {
            for club in &country.clubs {
                let parent_team = match club.teams.main().or_else(|| club.teams.teams.first()) {
                    Some(t) => t,
                    None => continue,
                };
                let parent_rep = parent_team.reputation.world;
                let asset_ctx = SquadAssetContext::build(club, date);
                let (seller_league_rep, seller_club_rep) =
                    PlayerValuationCalculator::seller_context(country, club);

                for team in &club.teams.teams {
                    for player in team.players.iter() {
                        let age = player.age(date);
                        let asset_class = asset_ctx.classify_in_squad(player, date, team.team_type);
                        // The parent's own veto, read before anybody asks:
                        // a club does not entertain a cold call about the
                        // man who starts for it.
                        let guard = LoanAssetGuard::for_player(
                            club,
                            player,
                            date,
                            seller_league_rep,
                            seller_club_rep,
                        );
                        let parent_holds = guard.map(|g| g.parent_holds()).unwrap_or(false)
                            && !MarketSwitches::loan_guard_off();
                        if TransferTrace::is(player.id) {
                            TransferTrace::line(
                                player.id,
                                "loan",
                                format!(
                                    "parent={} squad={:?} label={:?} asset={} rank={} \
                             standing={:.2} first_choice={} development={} \
                             parent_holds={parent_holds} reach={}",
                                    club.name,
                                    team.team_type,
                                    player
                                        .contract
                                        .as_ref()
                                        .map(|c| c.squad_status.clone())
                                        .unwrap_or(PlayerSquadStatus::NotYetSet),
                                    asset_class.label(),
                                    PipelineProcessor::position_group_rank(
                                        club,
                                        player.id,
                                        player.position().position_group(),
                                    ),
                                    guard.map(|g| g.standing()).unwrap_or(0.0),
                                    guard.map(|g| g.first_choice()).unwrap_or(false),
                                    guard.map(|g| g.is_development()).unwrap_or(false),
                                    guard.map(|g| g.parent_reach().label()).unwrap_or("unknown"),
                                ),
                            );
                        }
                        let is_development = match UnsolicitedLoanTarget::classify(
                            player,
                            age,
                            MAX_LOAN_TARGET_AGE,
                            asset_class,
                            parent_holds,
                        ) {
                            // A development loan is one the player NEEDS —
                            // below his club's own level, not merely young.
                            // The asset class says whether he is
                            // approachable; the guard says whether the
                            // development allowances belong to him.
                            Some(dev) => {
                                dev && PipelineProcessor::is_development_loan(guard.as_ref(), age)
                            }
                            None => continue,
                        };

                        let group = player.position().position_group();

                        // A YOUTH side's only keeper is never a cold-approach
                        // target — losing him leaves that playing side with
                        // zero goalkeepers. Mirrors the board loan-out
                        // listing rule, and stays scoped to youth teams:
                        // the club's senior depth covers a Main/B/Reserve
                        // keeper's departure (a reserve keeper going out on
                        // a development loan is exactly the intended
                        // pipeline).
                        if group == PlayerFieldPositionGroup::Goalkeeper
                            && matches!(
                                team.team_type,
                                TeamType::U18
                                    | TeamType::U19
                                    | TeamType::U20
                                    | TeamType::U21
                                    | TeamType::U23
                            )
                        {
                            let team_keepers = team
                                .players
                                .iter()
                                .filter(|p| {
                                    p.position().position_group()
                                        == PlayerFieldPositionGroup::Goalkeeper
                                })
                                .count();
                            if team_keepers <= 1 {
                                continue;
                            }
                        }
                        let parent_best_in_group = parent_team
                            .players
                            .iter()
                            .filter(|p| p.position().position_group() == group)
                            .map(|p| p.player_attributes.current_ability)
                            .max()
                            .unwrap_or(0);
                        // Development loans go out FREE: the parent wants the
                        // player developed, and a 10%-of-value fee would price
                        // a valuable prospect out of exactly the smaller,
                        // poorer clubs that actually have minutes for him (a
                        // low `max_loan_fee` was silently filtering them). This
                        // matches the main-squad board loan listing, which also
                        // asks zero. Older cover loans keep the nominal fee.
                        let asking_price = if is_development {
                            0.0
                        } else {
                            let value = player.value(date, seller_league_rep, seller_club_rep);
                            FormattingUtils::round_fee(value * 0.10)
                        };

                        unsolicited_targets.push(LoanListing {
                            player_id: player.id,
                            club_id: club.id,
                            asking_price,
                            ability: player.player_attributes.current_ability,
                            age,
                            position_group: group,
                            parent_rep,
                            parent_best_in_group,
                            is_development,
                            parent_league_rep: seller_league_rep,
                            guard,
                        });
                    }
                }
            }
        }
        unsolicited_targets
    }
}

/// What carries across one club's four sweeps: how many looks it has spent,
/// and which position groups it has already claimed somebody for.
struct ScanState {
    scans_this_club: usize,
    scanned_position_groups: Vec<PlayerFieldPositionGroup>,
}

/// One borrowing club, resolved once: its appetite, its depth, its taste, and
/// what its year and payroll can carry.
struct BorrowerScan<'a> {
    country: &'a Country,
    club: &'a Club,
    date: NaiveDate,
    is_january: bool,
    scan_unsolicited: bool,
    rep_level: ReputationLevel,
    has_critical_shortage: bool,
    max_loan_fee: f64,
    max_scans: usize,
    avg_ability: u8,
    borrower_depth: BorrowerPositionDepth,
    borrower_world_rep: u16,
    borrower_league_rep: u16,
    borrower_profile: Option<LoanBorrowerProfile>,
    taste: BorrowerTaste,
    open_request_groups: Vec<PlayerFieldPositionGroup>,
}

impl<'a> BorrowerScan<'a> {
    /// `None` when this club is not in the market at all — no squad, no plan,
    /// no appetite, or already at its concurrent-negotiation ceiling.
    fn open(
        country: &'a Country,
        club_idx: usize,
        tick: LoanScanTick,
        pending_loans: &HashMap<u32, Vec<(PlayerFieldPositionGroup, u8)>>,
    ) -> Option<Self> {
        let date = tick.date;
        let is_january = tick.is_january;
        let club = &country.clubs[club_idx];
        if club.teams.teams.is_empty() {
            return None;
        }

        let team = &club.teams.teams[0];
        let rep_level = team.reputation.level();

        let appetite = LoanBorrowerAppetite::assess(club, team, is_january);
        let has_critical_shortage = appetite.critical_shortage;

        if !appetite.scans {
            return None;
        }

        let plan = &club.transfer_plan;
        if !plan.initialized {
            return None;
        }

        // Respect concurrent negotiation limits
        let actual_active = country
            .transfer_market
            .active_negotiation_count_for_club(club.id);
        if actual_active >= plan.max_concurrent_negotiations {
            return None;
        }

        let balance = club.finance.balance.balance;
        let max_loan_fee = if balance < 0 {
            50_000.0
        } else {
            balance as f64 * 0.20
        };

        let max_scans: usize = match rep_level {
            ReputationLevel::Local | ReputationLevel::Amateur => 4,
            ReputationLevel::Regional => 3,
            ReputationLevel::National => 2,
            _ => 1,
        };

        let avg_ability = {
            let avg = team.players.current_ability_avg();
            if avg == 0 { 50 } else { avg }
        };

        // Track position groups already targeted in this scan pass
        // to avoid starting multiple negotiations for the same position

        // Borrower-side depth snapshot — shared with the foreign
        // scan. A club with 3 mediocre GKs should still loan a
        // world-class GK, but not a 4th mediocre one.
        let borrower_depth = BorrowerPositionDepth::snapshot(team)
            .with_pending_loans(pending_loans.get(&club.id).map_or(&[], |v| v.as_slice()));
        let borrower_world_rep = team.reputation.world;
        // Standard of football on offer here — the division gate reads
        // this against the parent's own competition.
        let borrower_league_rep = PipelineProcessor::club_league_reputation(country, club);
        // What this club's own year and payroll can carry — read once,
        // then folded per candidate with the group he plays in.
        let borrower_profile = LoanBorrowerProfile::of(club, date, borrower_league_rep);

        // What this club actually wants, as opposed to what is simply
        // available. Composed from the board's own recruitment policy —
        // which already existed and which the loan market read none of, so
        // every club ranked the market identically and converged on the
        // same name. `interest_in` runs only on candidates the gates have
        // already passed; it decides preference, never eligibility.
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
        // Scarcity pressure per group, so a club a man short at the back
        // wants a defender more than it wants the best name on the list.

        Some(BorrowerScan {
            country,
            club,
            date,
            is_january,
            scan_unsolicited: tick.scan_unsolicited,
            rep_level,
            has_critical_shortage,
            max_loan_fee,
            max_scans,
            avg_ability,
            borrower_depth,
            borrower_world_rep,
            borrower_league_rep,
            borrower_profile,
            taste,
            open_request_groups,
        })
    }

    /// This club's carrying capacity folded with the group a candidate plays in.
    fn borrower_for(&self, group: PlayerFieldPositionGroup) -> Option<LoanBorrowerProfile> {
        self.borrower_profile
            .map(|p| p.with_best_in_group(self.borrower_depth.best_in_group(group)))
    }

    /// True when the squad has no room for another body in that group at that
    /// level — three mediocre keepers should still take a world-class one, but
    /// never a fourth mediocre one.
    fn should_skip_loan(
        &self,
        group: PlayerFieldPositionGroup,
        loan_ability: u8,
        development: bool,
    ) -> bool {
        !self
            .borrower_depth
            .has_room_for(group, loan_ability, development)
    }

    /// A small club looks for a loan all year round, not just in January.
    fn is_small_club(&self) -> bool {
        matches!(
            self.rep_level,
            ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur
        )
    }

    /// Scarcity pressure per group, so a club a man short at the back wants a
    /// defender more than it wants the best name on the list.
    fn thinness_of(&self, group: PlayerFieldPositionGroup) -> f32 {
        GroupPressure::thinness(self.borrower_depth.headcount(group), group)
    }

    fn profile_of(&self, l: &LoanListing, fee: f64) -> LoanCandidateProfile {
        LoanCandidateProfile {
            player_id: l.player_id,
            true_ability: l.ability,
            age: l.age,
            is_development: l.is_development,
            fee,
            group_thinness: self.thinness_of(l.position_group),
            answers_open_request: self.open_request_groups.contains(&l.position_group),
        }
    }

    /// The four sweeps, in the order the old body ran them. Each stops at the
    /// same per-club cap, and each sees what the ones before it claimed.
    fn run(&self, board: &LoanBoard, actions: &mut Vec<LoanScanAction>) {
        let mut state = ScanState {
            scans_this_club: 0,
            scanned_position_groups: Vec::new(),
        };
        self.against_open_requests(&board.listed, &mut state, actions);
        self.opportunistically(&board.listed, &mut state, actions);
        self.in_january(&board.listed, &mut state, actions);
        self.cold(&board.unsolicited, &mut state, actions);
    }

    /// Against the club's own open requests first — a loan that answers a brief
    /// the club has already written. Emergency free-agent depth requests are
    /// excluded: those are the free-agent matcher's, never a loan's.
    fn against_open_requests(
        &self,
        loan_listings: &[LoanListing],
        state: &mut ScanState,
        actions: &mut Vec<LoanScanAction>,
    ) {
        let country = self.country;
        let club = self.club;
        let plan = &self.club.transfer_plan;
        let date = self.date;
        let _rep_level = self.rep_level.clone();
        let max_loan_fee = self.max_loan_fee;
        let max_scans = self.max_scans;
        let borrower_depth = &self.borrower_depth;
        let borrower_world_rep = self.borrower_world_rep;
        let borrower_league_rep = self.borrower_league_rep;
        let taste = &self.taste;
        let borrower_for = |group: PlayerFieldPositionGroup| -> Option<LoanBorrowerProfile> {
            self.borrower_for(group)
        };
        let should_skip_loan =
            |group: PlayerFieldPositionGroup, loan_ability: u8, development: bool| -> bool {
                self.should_skip_loan(group, loan_ability, development)
            };
        let profile_of =
            |l: &LoanListing, fee: f64| -> LoanCandidateProfile { self.profile_of(l, fee) };
        let mut scans_this_club = state.scans_this_club;
        let mut scanned_position_groups = std::mem::take(&mut state.scanned_position_groups);

        // Check unfulfilled transfer requests first. Emergency
        // free-agent depth requests are excluded — they're
        // serviced by the free-agent matcher only, not by loans.
        let unfulfilled = plan.transfer_requests.iter().filter(|r| {
            r.status != TransferRequestStatus::Fulfilled
                && r.status != TransferRequestStatus::Abandoned
                && !r.is_emergency_free_agent_depth()
        });

        for request in unfulfilled {
            if scans_this_club >= max_scans {
                break;
            }

            // Only scan once per position group — multiple requests for the same
            // group (e.g. FormationGap + DepthCover for GK) should not each trigger
            // a separate loan negotiation
            let pos_group = request.position.position_group();
            if scanned_position_groups.contains(&pos_group) {
                continue;
            }

            // Skip if position is full AND loan wouldn't be an upgrade.
            // Request-driven cover uses the strict bar — a club with a
            // full line asked for depth elsewhere, not a keeper prospect.
            if should_skip_loan(pos_group, request.min_ability, false) {
                continue;
            }

            // Relaxed thresholds: min_ability - 5, age_max + 3
            let relaxed_min = request.min_ability.saturating_sub(5);
            let relaxed_age_max = request
                .preferred_age_max
                .saturating_add(3)
                .min(MAX_LOAN_TARGET_AGE);

            let qualified: Vec<&LoanListing> = loan_listings
                .iter()
                .filter(|l| {
                    l.club_id != club.id
                && !club.is_rival(l.club_id) // no loans from rivals
                && l.position_group == pos_group
                && l.ability >= relaxed_min
                && l.age <= relaxed_age_max
                && l.age >= request.preferred_age_min
                && l.asking_price * 0.8 <= max_loan_fee
                && !plan.is_loan_approach_barred(l.player_id, date)
                && !country
                    .transfer_market
                    .has_active_negotiation_for(l.player_id, club.id)
                && !actions.iter().any(|a| a.player_id == l.player_id)
                // Every destination gate's own reading, taken before any
                // of them can short-circuit it away — the funnel is a
                // table, not a re-derivation.
                && PipelineProcessor::trace_loan_destination(
                    l.player_id,
                    &club.name,
                    l.position_group,
                    l.ability,
                    l.is_development,
                    l.parent_best_in_group,
                    &LoanDestinationLevel {
                        ability: l.ability,
                        parent_best_in_group: l.parent_best_in_group,
                        parent_rep: l.parent_rep,
                        borrower_rep: borrower_world_rep,
                        parent_league_rep: l.parent_league_rep,
                        borrower_league_rep,
                        is_development: l.is_development,
                    },
                    &borrower_depth,
                )
                // Room check with the CANDIDATE's real ability
                // and dev flag — the request-level pre-gate
                // above judged the room bar at the request's
                // min_ability, letting a `relaxed_min`
                // candidate into a genuinely full line.
                && borrower_depth.has_room_for(
                    l.position_group,
                    l.ability,
                    l.is_development,
                )
                // Development realism: the move must buy
                // minutes, and the reputation drop from the
                // parent must stay plausible.
                && borrower_depth.would_get_loan_minutes(
                    l.position_group,
                    l.ability,
                    l.is_development,
                    l.parent_best_in_group,
                )
                && LoanDestinationLevel {
                    ability: l.ability,
                    parent_best_in_group: l.parent_best_in_group,
                    parent_rep: l.parent_rep,
                    borrower_rep: borrower_world_rep,
                    parent_league_rep: l.parent_league_rep,
                    borrower_league_rep,
                    is_development: l.is_development,
                }
                .is_plausible()
                && PipelineProcessor::loan_guard_allows(
                    l.guard.as_ref(),
                    borrower_for(l.position_group).as_ref(),
                    l.player_id,
                )
                })
                .collect();

            // Everything left has cleared every gate. Which of them the
            // club goes for is a matter of preference, so it is drawn in
            // proportion to interest rather than taken as the top row of an
            // ability sort — the argmax made this a fixed pairing that
            // repeated until the listing disappeared.
            let weighted: Vec<(u32, f32)> = qualified
                .iter()
                .enumerate()
                .filter_map(|(i, l)| {
                    taste
                        .interest_in(&profile_of(l, l.asking_price * 0.8))
                        .map(|score| (i as u32, score))
                })
                .collect();

            if let Some(best) = InterestDraw::pick(&weighted).map(|i| qualified[i as usize]) {
                let reason = TransferReason::key(request.reason.as_signing_reason_key());
                actions.push(LoanScanAction {
                    club_id: club.id,
                    player_id: best.player_id,
                    selling_club_id: best.club_id,
                    offer_amount: FormattingUtils::round_fee(best.asking_price * 0.8),
                    reason,
                    is_unsolicited: false,
                    seller_asking: best.asking_price,
                    is_development: best.is_development,
                });
                scanned_position_groups.push(pos_group);
                scans_this_club += 1;
            }
        }

        state.scans_this_club = scans_this_club;
        state.scanned_position_groups = scanned_position_groups;
    }

    /// Small clubs always look for a deal, not just in January; National clubs
    /// join them when the squad has a genuine shortage.
    fn opportunistically(
        &self,
        loan_listings: &[LoanListing],
        state: &mut ScanState,
        actions: &mut Vec<LoanScanAction>,
    ) {
        let country = self.country;
        let club = self.club;
        let plan = &self.club.transfer_plan;
        let date = self.date;
        let rep_level = self.rep_level.clone();
        let max_loan_fee = self.max_loan_fee;
        let max_scans = self.max_scans;
        let borrower_depth = &self.borrower_depth;
        let borrower_world_rep = self.borrower_world_rep;
        let borrower_league_rep = self.borrower_league_rep;
        let taste = &self.taste;
        let has_critical_shortage = self.has_critical_shortage;
        let avg_ability = self.avg_ability;
        let borrower_for = |group: PlayerFieldPositionGroup| -> Option<LoanBorrowerProfile> {
            self.borrower_for(group)
        };
        let should_skip_loan =
            |group: PlayerFieldPositionGroup, loan_ability: u8, development: bool| -> bool {
                self.should_skip_loan(group, loan_ability, development)
            };
        let profile_of =
            |l: &LoanListing, fee: f64| -> LoanCandidateProfile { self.profile_of(l, fee) };
        let mut scans_this_club = state.scans_this_club;
        let mut scanned_position_groups = std::mem::take(&mut state.scanned_position_groups);

        // Opportunistic scan: small clubs always look for deals,
        // not just in January. National clubs join in too when
        // their squad has a genuine shortage — the critical-need
        // override above already let them in past `should_scan`,
        // but the opportunistic branch was small-club-only and
        // would otherwise leave them empty-handed.
        let is_small_club = matches!(
            rep_level,
            ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur
        );

        // Small clubs (always) and National clubs in critical
        // shortage scan for available loan players.
        if (is_small_club || has_critical_shortage) && scans_this_club < max_scans {
            let opps: Vec<&LoanListing> = loan_listings
                .iter()
                .filter(|l| {
                    l.club_id != club.id
                && !club.is_rival(l.club_id)
                && l.age <= MAX_LOAN_TARGET_AGE
                // Squad-average floor is for cover loans; a youth
                // match-practice loan leans on the minutes gate.
                && (l.is_development || l.ability >= avg_ability.saturating_sub(5))
                && l.asking_price * 0.8 <= max_loan_fee
                && !plan.is_loan_approach_barred(l.player_id, date)
                && !country
                    .transfer_market
                    .has_active_negotiation_for(l.player_id, club.id)
                && !actions.iter().any(|a| a.player_id == l.player_id)
                // Every destination gate's own reading, taken before any
                // of them can short-circuit it away — the funnel is a
                // table, not a re-derivation.
                && PipelineProcessor::trace_loan_destination(
                    l.player_id,
                    &club.name,
                    l.position_group,
                    l.ability,
                    l.is_development,
                    l.parent_best_in_group,
                    &LoanDestinationLevel {
                        ability: l.ability,
                        parent_best_in_group: l.parent_best_in_group,
                        parent_rep: l.parent_rep,
                        borrower_rep: borrower_world_rep,
                        parent_league_rep: l.parent_league_rep,
                        borrower_league_rep,
                        is_development: l.is_development,
                    },
                    &borrower_depth,
                )
                && !scanned_position_groups.contains(&l.position_group)
                && !should_skip_loan(l.position_group, l.ability, l.is_development)
                && borrower_depth.would_get_loan_minutes(
                    l.position_group,
                    l.ability,
                    l.is_development,
                    l.parent_best_in_group,
                )
                && LoanDestinationLevel {
                    ability: l.ability,
                    parent_best_in_group: l.parent_best_in_group,
                    parent_rep: l.parent_rep,
                    borrower_rep: borrower_world_rep,
                    parent_league_rep: l.parent_league_rep,
                    borrower_league_rep,
                    is_development: l.is_development,
                }
                .is_plausible()
                && PipelineProcessor::loan_guard_allows(
                    l.guard.as_ref(),
                    borrower_for(l.position_group).as_ref(),
                    l.player_id,
                )
                })
                .collect();

            // Weighted sample without replacement, in place of "sort by
            // ability, take the top N" — which returned the same N in the
            // same order for as long as the listings stood.
            let weighted: Vec<(u32, f32)> = opps
                .iter()
                .enumerate()
                .filter_map(|(i, l)| {
                    taste
                        .interest_in(&profile_of(l, l.asking_price * 0.8))
                        .map(|score| (i as u32, score))
                })
                .collect();

            // Draw a surplus and walk it: the slate is filtered once, so
            // without a running group check one pass could open three
            // negotiations for the same position — which the scan's own
            // `scanned_position_groups` bookkeeping exists to prevent and
            // the old sort-and-take quietly allowed. Over-drawing keeps the
            // club's full scan budget usable despite the extra constraint.
            let wanted = max_scans - scans_this_club;
            for idx in InterestDraw::pick_several(&weighted, (wanted * 4).min(weighted.len())) {
                if scans_this_club >= max_scans {
                    break;
                }
                let opp = opps[idx as usize];
                if scanned_position_groups.contains(&opp.position_group) {
                    continue;
                }
                actions.push(LoanScanAction {
                    club_id: club.id,
                    player_id: opp.player_id,
                    selling_club_id: opp.club_id,
                    offer_amount: FormattingUtils::round_fee(opp.asking_price * 0.8),
                    reason: TransferReason::key("signing_reason_loan_opportunistic_upgrade"),
                    is_unsolicited: false,
                    seller_asking: opp.asking_price,
                    is_development: opp.is_development,
                });
                scanned_position_groups.push(opp.position_group);
                scans_this_club += 1;
            }
        }

        state.scans_this_club = scans_this_club;
        state.scanned_position_groups = scanned_position_groups;
    }

    /// The mid-season window, where even a National club that is not short
    /// goes looking.
    fn in_january(
        &self,
        loan_listings: &[LoanListing],
        state: &mut ScanState,
        actions: &mut Vec<LoanScanAction>,
    ) {
        let country = self.country;
        let club = self.club;
        let plan = &self.club.transfer_plan;
        let date = self.date;
        let _rep_level = self.rep_level.clone();
        let max_loan_fee = self.max_loan_fee;
        let max_scans = self.max_scans;
        let borrower_depth = &self.borrower_depth;
        let borrower_world_rep = self.borrower_world_rep;
        let borrower_league_rep = self.borrower_league_rep;
        let taste = &self.taste;
        let is_january = self.is_january;
        let avg_ability = self.avg_ability;
        let is_small_club = self.is_small_club();
        let borrower_for = |group: PlayerFieldPositionGroup| -> Option<LoanBorrowerProfile> {
            self.borrower_for(group)
        };
        let should_skip_loan =
            |group: PlayerFieldPositionGroup, loan_ability: u8, development: bool| -> bool {
                self.should_skip_loan(group, loan_ability, development)
            };
        let profile_of =
            |l: &LoanListing, fee: f64| -> LoanCandidateProfile { self.profile_of(l, fee) };
        let scans_this_club = state.scans_this_club;
        let scanned_position_groups = std::mem::take(&mut state.scanned_position_groups);

        // January extra: even National clubs look for opportunistic loans
        if is_january && scans_this_club < max_scans && !is_small_club {
            let mid_season: Vec<&LoanListing> = loan_listings
                .iter()
                .filter(|l| {
                    l.club_id != club.id
                && !club.is_rival(l.club_id)
                && l.age <= MAX_LOAN_TARGET_AGE
                // Squad-average floor is for cover loans; a youth
                // match-practice loan leans on the minutes gate.
                && (l.is_development || l.ability >= avg_ability.saturating_sub(8))
                && l.asking_price * 0.8 <= max_loan_fee
                && !plan.is_loan_approach_barred(l.player_id, date)
                && !country
                    .transfer_market
                    .has_active_negotiation_for(l.player_id, club.id)
                && !actions.iter().any(|a| a.player_id == l.player_id)
                // Every destination gate's own reading, taken before any
                // of them can short-circuit it away — the funnel is a
                // table, not a re-derivation.
                && PipelineProcessor::trace_loan_destination(
                    l.player_id,
                    &club.name,
                    l.position_group,
                    l.ability,
                    l.is_development,
                    l.parent_best_in_group,
                    &LoanDestinationLevel {
                        ability: l.ability,
                        parent_best_in_group: l.parent_best_in_group,
                        parent_rep: l.parent_rep,
                        borrower_rep: borrower_world_rep,
                        parent_league_rep: l.parent_league_rep,
                        borrower_league_rep,
                        is_development: l.is_development,
                    },
                    &borrower_depth,
                )
                && !scanned_position_groups.contains(&l.position_group)
                && !should_skip_loan(l.position_group, l.ability, l.is_development)
                && borrower_depth.would_get_loan_minutes(
                    l.position_group,
                    l.ability,
                    l.is_development,
                    l.parent_best_in_group,
                )
                && LoanDestinationLevel {
                    ability: l.ability,
                    parent_best_in_group: l.parent_best_in_group,
                    parent_rep: l.parent_rep,
                    borrower_rep: borrower_world_rep,
                    parent_league_rep: l.parent_league_rep,
                    borrower_league_rep,
                    is_development: l.is_development,
                }
                .is_plausible()
                && PipelineProcessor::loan_guard_allows(
                    l.guard.as_ref(),
                    borrower_for(l.position_group).as_ref(),
                    l.player_id,
                )
                })
                .collect();

            let weighted: Vec<(u32, f32)> = mid_season
                .iter()
                .enumerate()
                .filter_map(|(i, l)| {
                    taste
                        .interest_in(&profile_of(l, l.asking_price * 0.8))
                        .map(|score| (i as u32, score))
                })
                .collect();

            if let Some(opp) = InterestDraw::pick(&weighted).map(|i| mid_season[i as usize]) {
                actions.push(LoanScanAction {
                    club_id: club.id,
                    player_id: opp.player_id,
                    selling_club_id: opp.club_id,
                    offer_amount: FormattingUtils::round_fee(opp.asking_price * 0.8),
                    reason: TransferReason::key("signing_reason_loan_midseason_reinforcement"),
                    is_unsolicited: false,
                    seller_asking: opp.asking_price,
                    is_development: opp.is_development,
                });
            }
        }

        state.scans_this_club = scans_this_club;
        state.scanned_position_groups = scanned_position_groups;
    }

    /// A cold approach: a player his club never loan-listed. The badge is not
    /// a precondition for loan demand — the move only has to be credible, so
    /// the parent must be bigger, the player must actually get minutes, and
    /// the reputation drop must be plausible.
    fn cold(
        &self,
        unsolicited_targets: &[LoanListing],
        state: &mut ScanState,
        actions: &mut Vec<LoanScanAction>,
    ) {
        let country = self.country;
        let club = self.club;
        let plan = &self.club.transfer_plan;
        let date = self.date;
        let rep_level = self.rep_level.clone();
        let max_loan_fee = self.max_loan_fee;
        let max_scans = self.max_scans;
        let borrower_depth = &self.borrower_depth;
        let borrower_world_rep = self.borrower_world_rep;
        let borrower_league_rep = self.borrower_league_rep;
        let taste = &self.taste;
        let scan_unsolicited = self.scan_unsolicited;
        let avg_ability = self.avg_ability;
        let borrower_for = |group: PlayerFieldPositionGroup| -> Option<LoanBorrowerProfile> {
            self.borrower_for(group)
        };
        let should_skip_loan =
            |group: PlayerFieldPositionGroup, loan_ability: u8, development: bool| -> bool {
                self.should_skip_loan(group, loan_ability, development)
            };
        let profile_of =
            |l: &LoanListing, fee: f64| -> LoanCandidateProfile { self.profile_of(l, fee) };
        let scans_this_club = state.scans_this_club;
        let scanned_position_groups = std::mem::take(&mut state.scanned_position_groups);

        // ── Unsolicited loan approach (no `Loa` required) ─────────
        //
        // A lower-/mid-tier club asks a bigger club to take a player who
        // is NOT loan-listed. The badge is not required; the move only
        // has to be a credible loan — the parent is bigger (approach
        // "up"), the player would actually play, and the reputation drop
        // is plausible. First-team contributors were already excluded
        // when the target pool was built, so this never strips a club of
        // a key player.
        //
        // Continental clubs join the branch for PEER-LEVEL targets
        // only. They used to be excluded outright, so the first tier
        // that ever cold-called a big club's near-ready youngster was,
        // by construction, the one below the top flight — the exclusion
        // was itself a reason the boy ended up two divisions down.
        let cold_peer_only = matches!(rep_level, ReputationLevel::Continental);
        if scan_unsolicited
            && scans_this_club < max_scans
            && matches!(
                rep_level,
                ReputationLevel::Continental
                    | ReputationLevel::National
                    | ReputationLevel::Regional
                    | ReputationLevel::Local
                    | ReputationLevel::Amateur
            )
        {
            let cold: Vec<&LoanListing> = unsolicited_targets
                .iter()
                .filter(|l| {
                    l.club_id != club.id
                && !club.is_rival(l.club_id)
                // Approach "up": only a club below the parent
                // borrows the player for minutes.
                && l.parent_rep > borrower_world_rep
                && l.age <= MAX_LOAN_TARGET_AGE
                && l.asking_price * 0.8 <= max_loan_fee
                && !plan.is_loan_approach_barred(l.player_id, date)
                && !country
                    .transfer_market
                    .has_active_negotiation_for(l.player_id, club.id)
                && !actions.iter().any(|a| a.player_id == l.player_id)
                // Every destination gate's own reading, taken before any
                // of them can short-circuit it away — the funnel is a
                // table, not a re-derivation.
                && PipelineProcessor::trace_loan_destination(
                    l.player_id,
                    &club.name,
                    l.position_group,
                    l.ability,
                    l.is_development,
                    l.parent_best_in_group,
                    &LoanDestinationLevel {
                        ability: l.ability,
                        parent_best_in_group: l.parent_best_in_group,
                        parent_rep: l.parent_rep,
                        borrower_rep: borrower_world_rep,
                        parent_league_rep: l.parent_league_rep,
                        borrower_league_rep,
                        is_development: l.is_development,
                    },
                    &borrower_depth,
                )
                && !scanned_position_groups.contains(&l.position_group)
                // Will he actually play here? Position-aware, and
                // for keepers the strict plausible-#1 rule — this
                // is the realism check for a development loan.
                && !should_skip_loan(l.position_group, l.ability, l.is_development)
                && borrower_depth.would_get_loan_minutes(
                    l.position_group,
                    l.ability,
                    l.is_development,
                    l.parent_best_in_group,
                )
                // Squad-average / reputation-drop floors apply to
                // cover loans only; development loans lean on the
                // minutes gate above so a young keeper can drop to
                // a club where he STARTS (see `clears_level_gate`).
                && UnsolicitedLoanTarget::clears_level_gate(
                    avg_ability,
                    &LoanDestinationLevel {
                        ability: l.ability,
                        parent_best_in_group: l.parent_best_in_group,
                        parent_rep: l.parent_rep,
                        borrower_rep: borrower_world_rep,
                        parent_league_rep: l.parent_league_rep,
                        borrower_league_rep,
                        is_development: l.is_development,
                    },
                )
                && PipelineProcessor::loan_guard_allows(
                    l.guard.as_ref(),
                    borrower_for(l.position_group).as_ref(),
                    l.player_id,
                )
                && (!cold_peer_only
                    || PipelineProcessor::loan_guard_reach(
                        l.guard.as_ref(),
                        borrower_for(l.position_group).as_ref(),
                    ) == Some(LoanReach::PeerLevel))
                })
                .collect();

            let weighted: Vec<(u32, f32)> = cold
                .iter()
                .enumerate()
                .filter_map(|(i, l)| {
                    taste
                        .interest_in(&profile_of(l, l.asking_price * 0.8))
                        .map(|score| (i as u32, score))
                })
                .collect();

            // Terminal branch for this club: the draw is already capped at
            // the per-club scan budget, and nothing after this reads the
            // counter or the scanned-groups set, so the only bookkeeping
            // left is the within-draw group guard.
            let mut cold_groups: Vec<PlayerFieldPositionGroup> = Vec::new();
            let wanted = max_scans - scans_this_club;
            for idx in InterestDraw::pick_several(&weighted, (wanted * 4).min(weighted.len())) {
                if cold_groups.len() >= wanted {
                    break;
                }
                let tgt = cold[idx as usize];
                if cold_groups.contains(&tgt.position_group) {
                    continue;
                }
                cold_groups.push(tgt.position_group);
                actions.push(LoanScanAction {
                    club_id: club.id,
                    player_id: tgt.player_id,
                    selling_club_id: tgt.club_id,
                    offer_amount: FormattingUtils::round_fee(tgt.asking_price * 0.8),
                    reason: TransferReason::key("signing_reason_loan_development_approach"),
                    is_unsolicited: true,
                    seller_asking: tgt.asking_price,
                    is_development: tgt.is_development,
                });
            }
        }

        state.scans_this_club = scans_this_club;
        state.scanned_position_groups = scanned_position_groups;
    }
}

/// The single writer: turns the staged approaches into open negotiations.
struct LoanScanCommit;

impl LoanScanCommit {
    fn apply(country: &mut Country, actions: Vec<LoanScanAction>, date: NaiveDate) {
        // Pass 2: Start loan negotiations
        for action in actions {
            // Unsolicited approaches target players their club never loan-
            // listed, so the market has no listing to negotiate against.
            // Mirror the permanent-pipeline pattern: fabricate a synthetic
            // loan listing (tagged so the resolver withholds the
            // "seller-advertised" acceptance bonus), priced at the seller's
            // asking. `start_negotiation` requires a listing, so this must
            // come first.
            if action.is_unsolicited
                && country
                    .transfer_market
                    .get_listing_by_player(action.player_id)
                    .is_none()
            {
                let selling_team_id = country
                    .clubs
                    .iter()
                    .find(|c| c.id == action.selling_club_id)
                    .and_then(|c| c.teams.teams.first())
                    .map(|t| t.id)
                    .unwrap_or(0);
                country
                    .transfer_market
                    .add_listing(TransferListing::new_with_origin(
                        action.player_id,
                        action.selling_club_id,
                        selling_team_id,
                        CurrencyValue {
                            amount: FormattingUtils::round_fee(action.seller_asking.max(0.0)),
                            currency: Currency::Usd,
                        },
                        date,
                        TransferListingType::Loan,
                        TransferListingOrigin::SyntheticUnsolicited,
                    ));
            }

            let selling_rep =
                PipelineProcessor::get_club_reputation(country, action.selling_club_id);
            let buying_rep = PipelineProcessor::get_club_reputation(country, action.club_id);
            let (p_age, p_ambition) =
                PipelineProcessor::get_player_negotiation_data(country, action.player_id, date);

            let mut clauses = Vec::new();

            // Option to buy. A loan without one is a season the borrowing
            // club spends developing somebody else's player and hands him
            // straight back — which is why no AI loan in the world ever
            // became a permanent move: the conversion machinery
            // (`decide_loan_buyout`) is driven entirely by
            // `loan_future_fee`, and nothing on this path had ever written
            // one. In real football most loans of players with a future
            // carry an option, and a good loan spell is one of the main
            // routes a squad player finds a permanent home.
            if let Some(option_fee) = PipelineProcessor::loan_option_fee(country, &action, date) {
                clauses.push(TransferClause::LoanOptionToBuy(CurrencyValue {
                    amount: option_fee,
                    currency: Currency::Usd,
                }));
            }

            // Add appearance fee clause for high-reputation selling clubs
            let selling_rep_level =
                PipelineProcessor::get_club_reputation_level(country, action.selling_club_id);
            match selling_rep_level {
                ReputationLevel::Elite => {
                    clauses.push(TransferClause::AppearanceFee(
                        CurrencyValue {
                            amount: FormattingUtils::round_fee(action.offer_amount * 0.30),
                            currency: Currency::Usd,
                        },
                        10,
                    ));
                }
                ReputationLevel::Continental => {
                    clauses.push(TransferClause::AppearanceFee(
                        CurrencyValue {
                            amount: FormattingUtils::round_fee(action.offer_amount * 0.20),
                            currency: Currency::Usd,
                        },
                        15,
                    ));
                }
                _ => {}
            }

            // Loans run to the season end — set the explicit duration field
            // (months) rather than the permanent-contract years field so
            // the market history doesn't double-encode "1" as both 1 year
            // and 1 month.
            let offer = TransferOffer {
                base_fee: CurrencyValue {
                    amount: action.offer_amount,
                    currency: Currency::Usd,
                },
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
                action.player_id,
                action.club_id,
                offer,
                date,
                selling_rep,
                buying_rep,
                p_age,
                p_ambition,
            ) {
                // Resolve player and club names
                let (p_name, sc_name) = PipelineProcessor::resolve_player_and_club_name(
                    country,
                    action.player_id,
                    action.selling_club_id,
                );

                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_loan = true;
                    negotiation.is_unsolicited = action.is_unsolicited;
                    negotiation.reason = action.reason.clone();
                    negotiation.player_name = p_name;
                    negotiation.selling_club_name = sc_name;
                }

                // The club has made its move on this target; it does not
                // reopen the file next Monday. Stamped whatever the outcome —
                // if the bid succeeds the player is on loan and filtered out
                // anyway, and if it fails this is exactly the repeat approach
                // that made one club look welded to one player.
                if let Some(buyer) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                    buyer
                        .transfer_plan
                        .record_loan_approach(action.player_id, date);
                }

                debug!(
                    "Loan scan: Club {} started loan negotiation for player {}",
                    action.club_id, action.player_id
                );
            }
        }
    }
}

/// The tick every stage runs against.
#[derive(Clone, Copy)]
struct LoanScanTick {
    date: NaiveDate,
    is_january: bool,
    /// The cold-approach sweep is weekly; the listed market is read daily.
    scan_unsolicited: bool,
}

/// The pass itself: read the board, offer it to every club with appetite, then
/// commit what they asked for.
pub(in crate::transfers::loan) struct LoanMarketScan;

impl LoanMarketScan {
    pub(in crate::transfers::loan) fn run(country: &mut Country, date: NaiveDate) {
        let tick = LoanScanTick {
            date,
            is_january: PipelineProcessor::is_mid_season_window_for(country, date),
            scan_unsolicited: date.weekday() == Weekday::Mon,
        };

        // Age out expired approach standoffs and stale loan-placement rows
        // before anything reads them. `prune_rejected` had no caller at all, so
        // the scouting reject list it also trims grew for the life of the world
        // — the entries expired logically but never left the vector.
        for club in &mut country.clubs {
            club.transfer_plan.prune_rejected(date);
        }

        let board = LoanBoard::read(country, date);
        if board.is_empty() {
            return;
        }

        let mut actions: Vec<LoanScanAction> = Vec::new();
        let pending_loans = PipelineProcessor::pending_incoming_loans_by_club(country);

        // Rotate who looks first. The per-pass dedup below is "has anybody
        // claimed him yet", so registration order was first refusal on the
        // whole market — the lowest-id club took the pick of every listing,
        // every tick, forever.
        for club_idx in InterestDraw::visit_order(country.clubs.len()) {
            if let Some(scan) = BorrowerScan::open(country, club_idx, tick, &pending_loans) {
                scan.run(&board, &mut actions);
            }
        }

        LoanScanCommit::apply(country, actions, date);
    }
}
