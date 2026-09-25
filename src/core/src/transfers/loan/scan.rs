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

use crate::club::staff::perception::AbilityEstimator;
use crate::transfers::loan::LoanPipeline;
use crate::transfers::market::window::MarketCadence;
use crate::transfers::view::club::ClubView;
use crate::transfers::view::player::PlayerView;
use chrono::NaiveDate;
use log::debug;

use crate::club::player::mind::{CareerPlanView, MindClock};
use crate::club::team::squad::SquadAssetContext;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::deal::offer::{PersonalTermsOffer, TransferClause, TransferOffer};
use crate::transfers::deal::reason::TransferReason;
use crate::transfers::gate::fit::{ForeignSlotCount, SquadRegistrationLimits};
use crate::transfers::loan::interest::{
    BorrowerTaste, GroupPressure, InterestDraw, LoanCandidateProfile,
};
use crate::transfers::market::{
    TransferListing, TransferListingOrigin, TransferListingStatus, TransferListingType,
};
use crate::transfers::pipeline::TransferRequest;
use crate::transfers::pipeline::TransferRequestStatus;
use crate::transfers::pipeline::trace::TransferTrace;
use crate::transfers::squad::minutes::LoanPromise;
use crate::transfers::value::PlayerValuationCalculator;
use crate::utils::FormattingUtils;
use crate::{
    Club, Country, PathwayStage, Person, PlayerFieldPositionGroup, PlayerSquadStatus,
    ReputationLevel, TeamType,
};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::HashMap;

use super::*;
use crate::transfers::loan::legacy::{LegacyDomesticGate, LoanDestinationLevel};
/// A player the market is offering on loan — listed by his club, or one a
/// bigger club never listed but would let go.
// Collect available loan listings (Pass 1 read)
struct LoanListing {
    player_id: u32,
    club_id: u32,
    /// His passport — what the borrower's registration quota counts.
    nationality_country_id: u32,
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
    /// What the parent makes of sending him anywhere at all — read once
    /// per listing rather than per candidate borrower, because it is a
    /// property of the club and the man, not of the pair.
    willingness: ParentWillingness,
    /// The arc he is living out, so the consent term can tell a drop he
    /// meant to take from one he is being talked into.
    plan: CareerPlanView,
    /// What the parent is willing to keep paying of his wage, 0..1 —
    /// the subsidy that decides whether a poorer borrower can carry
    /// him at all.
    parent_subsidy: f32,
    /// Where the parent's pathway has him, and the band the club means
    /// this spell to put him at when it has said — the two fallbacks the
    /// agreement reads for a man with no plan of his own.
    stage: PathwayStage,
    club_band_target: Option<f32>,
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
            if let Some(player) = PlayerView::find_player_in_country(country, listing.player_id) {
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
                    .and_then(|c| LoanPipeline::loan_guard_for(country, c, player, date));
                // Treat as a "development" move (stricter minutes gate so he
                // actually plays, relaxed reputation/level floors so he can
                // drop a level or two to do so) either when the loan is
                // game-time-driven (development pathway, blocked prospect,
                // needs-minutes) or when the player is genuinely below his
                // own club's level — never on a birth year alone, which
                // hands the whole allowance to a teenager who is already
                // his club's first choice. Pure older-surplus /
                // financial loans keep the looser cover bar.
                let is_development =
                    LoanPipeline::is_development_loan(guard.as_ref(), player.age(date))
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
                    nationality_country_id: player.country_id,
                    asking_price: listing.asking_price.amount,
                    ability: AbilityEstimator::observable_level(player),
                    age: player.age(date),
                    position_group: group,
                    parent_rep,
                    parent_best_in_group,
                    is_development,
                    parent_league_rep: parent_club
                        .map(|c| LoanPipeline::club_league_reputation(country, c))
                        .unwrap_or(0),
                    guard,
                    willingness: parent_club
                        .map(|c| LoanAssetGuard::willingness_for(c, player, date))
                        .unwrap_or_else(ParentWillingness::open),
                    plan: player.mind.career.plan_view(MindClock::day(date)),
                    parent_subsidy: player.loan_subsidy(),
                    stage: player.pathway_stage(),
                    club_band_target: parent_club
                        .and_then(|c| {
                            c.transfer_plan
                                .loan_out_candidates
                                .iter()
                                .find(|cand| cand.player_id == player.id)
                        })
                        .and_then(|cand| cand.band_target),
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
        if !LoanPipeline::is_market_day(date) {
            return Vec::new();
        }
        // Each club's targets are read off its own roster against
        // country-constant facts, so the clubs are read in parallel; the
        // ordered flatten keeps the board in club order.
        country
            .clubs
            .par_iter()
            .map(|club| {
                let mut unsolicited_targets: Vec<LoanListing> = Vec::new();
                let parent_team = match club.teams.main().or_else(|| club.teams.teams.first()) {
                    Some(t) => t,
                    None => return unsolicited_targets,
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
                        let willingness = LoanAssetGuard::willingness_for(club, player, date);
                        let parent_holds = willingness.score < ParentWillingness::ENTERTAINS;
                        if TransferTrace::is(player.id) {
                            TransferTrace::line(
                                player.id,
                                "loan",
                                format!(
                                    "parent={} squad={:?} label={:?} asset={} rank={} \
                             standing={:.2} first_choice={} development={} \
                             parent_holds={parent_holds} willingness={:.2}",
                                    club.name,
                                    team.team_type,
                                    player
                                        .contract
                                        .as_ref()
                                        .map(|c| c.squad_status.clone())
                                        .unwrap_or(PlayerSquadStatus::NotYetSet),
                                    asset_class.label(),
                                    PlayerView::position_group_rank(
                                        club,
                                        player.id,
                                        player.position().position_group(),
                                    ),
                                    guard.map(|g| g.standing()).unwrap_or(0.0),
                                    guard.map(|g| g.first_choice()).unwrap_or(false),
                                    guard.map(|g| g.is_development()).unwrap_or(false),
                                    willingness.score,
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
                                dev && LoanPipeline::is_development_loan(guard.as_ref(), age)
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
                            nationality_country_id: player.country_id,
                            asking_price,
                            ability: AbilityEstimator::observable_level(player),
                            age,
                            position_group: group,
                            parent_rep,
                            parent_best_in_group,
                            is_development,
                            parent_league_rep: seller_league_rep,
                            guard,
                            willingness,
                            plan: player.mind.career.plan_view(MindClock::day(date)),
                            parent_subsidy: player.loan_subsidy(),
                            stage: player.pathway_stage(),
                            club_band_target: club
                                .transfer_plan
                                .loan_out_candidates
                                .iter()
                                .find(|cand| cand.player_id == player.id)
                                .and_then(|cand| cand.band_target),
                        });
                    }
                }
                unsolicited_targets
            })
            .collect::<Vec<Vec<LoanListing>>>()
            .into_iter()
            .flatten()
            .collect()
    }
}

/// What every borrower's turn reads of the rest of the market, taken once for
/// the pass: the scan only stages actions, so the negotiation set is frozen.
struct LoanScanLoad {
    pending_loans: HashMap<u32, Vec<(PlayerFieldPositionGroup, u8)>>,
    pending_foreign: FxHashMap<u32, u32>,
    active_counts: FxHashMap<u32, u32>,
    active_pairs: FxHashSet<(u32, u32)>,
}

/// One sweep's candidates, scored, before the pass's claims are taken out.
type ScoredSlate<'a> = Vec<(&'a LoanListing, f32)>;

/// One open request's slate.
struct RequestSlate<'a> {
    group: PlayerFieldPositionGroup,
    reason: &'static str,
    candidates: ScoredSlate<'a>,
}

/// One borrowing club, resolved once: its appetite, its depth, its taste, and
/// what its year and payroll can carry.
struct BorrowerScan<'a> {
    club: &'a Club,
    date: NaiveDate,
    mid_season_window: bool,
    scan_unsolicited: bool,
    rep_level: ReputationLevel,
    max_loan_fee: f64,
    max_scans: usize,
    avg_ability: u8,
    borrower_depth: BorrowerPositionDepth,
    borrower_world_rep: u16,
    borrower_league_rep: u16,
    borrower_profile: Option<LoanBorrowerProfile>,
    taste: BorrowerTaste,
    /// The club's position under its league's foreigner quota, and the
    /// slots its in-flight loan approaches have already spent.
    foreign_slots: ForeignSlotCount,
    pending_foreign: u32,
    /// What the club has actually asked for — the band, not just the
    /// shirt, so a request for a first-team defender does not read as an
    /// invitation for anybody who plays there.
    open_requests: Vec<TransferRequest>,
    active_pairs: &'a FxHashSet<(u32, u32)>,
}

impl<'a> BorrowerScan<'a> {
    /// `None` when this club is not in the market at all — no squad, no plan,
    /// no appetite, or already at its concurrent-negotiation ceiling.
    fn open(
        country: &'a Country,
        club_idx: usize,
        tick: LoanScanTick,
        load: &'a LoanScanLoad,
    ) -> Option<Self> {
        let date = tick.date;
        let mid_season_window = tick.mid_season_window;
        let club = &country.clubs[club_idx];
        if club.teams.teams.is_empty() {
            return None;
        }

        let team = &club.teams.teams[0];
        let rep_level = team.reputation.level();

        // Every club looks. How much a club of this standing actually
        // wants a loanee is priced by the appetite term, not decided
        // here — an Elite club that "never loans in August" was the
        // refusal that walked other clubs' prospects down two
        // divisions, a fortnight at a time.

        let plan = &club.transfer_plan;
        if !plan.initialized {
            return None;
        }

        // Respect concurrent negotiation limits
        let actual_active = load.active_counts.get(&club.id).copied().unwrap_or(0);
        if actual_active >= plan.max_concurrent_negotiations {
            return None;
        }

        let balance = club.finance.balance.balance;
        let max_loan_fee = if balance < 0 {
            50_000.0
        } else {
            balance as f64 * 0.20
        };

        // Looks a club spends per pass. Raised across the board now
        // that a look is a priced candidate rather than a gate pass:
        // most of them find nothing, and the budget is what decides how
        // much of the board a club actually reads.
        let max_scans: usize = match rep_level {
            ReputationLevel::Local | ReputationLevel::Amateur => 6,
            ReputationLevel::Regional => 4,
            ReputationLevel::National => 3,
            _ => 2,
        };

        let avg_ability = {
            let avg = team.players.current_ability_avg();
            if avg == 0 { 50 } else { avg }
        };

        // Borrower-side depth snapshot — shared with the foreign
        // scan. A club with 3 mediocre GKs should still loan a
        // world-class GK, but not a 4th mediocre one.
        let borrower_depth = BorrowerPositionDepth::snapshot(team).with_pending_loans(
            load.pending_loans
                .get(&club.id)
                .map_or(&[], |v| v.as_slice()),
        );
        let borrower_world_rep = team.reputation.world;
        // Standard of football on offer here — the division gate reads
        // this against the parent's own competition.
        let borrower_league_rep = LoanPipeline::club_league_reputation(country, club);
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
        let open_requests: Vec<TransferRequest> = plan
            .transfer_requests
            .iter()
            .filter(|r| {
                r.status != TransferRequestStatus::Fulfilled
                    && r.status != TransferRequestStatus::Abandoned
            })
            .cloned()
            .collect();
        let foreign_slots =
            SquadRegistrationLimits::new(country.id, &country.regulations).count(club);
        let pending_foreign = load.pending_foreign.get(&club.id).copied().unwrap_or(0);

        Some(BorrowerScan {
            club,
            date,
            mid_season_window,
            scan_unsolicited: tick.scan_unsolicited,
            rep_level,
            max_loan_fee,
            max_scans,
            avg_ability,
            borrower_depth,
            borrower_world_rep,
            borrower_league_rep,
            borrower_profile,
            taste,
            foreign_slots,
            pending_foreign,
            open_requests,
            active_pairs: &load.active_pairs,
        })
    }

    /// The agreement these two clubs and this player would reach, 0..1.
    ///
    /// One number in place of the five-gate cluster every sweep used to
    /// repeat: the parent's willingness, the borrower's appetite, the
    /// player's consent and whether the money works, multiplied. Used
    /// twice at each call site — as the floor that keeps the candidate
    /// list finite, and as the weight the draw picks on — so a thin
    /// agreement is rare rather than forbidden.
    ///
    /// On the `OF_LOAN_AGREEMENT_OFF` arm this is the HEAD gate stack
    /// instead, unweighted, so a census can price one model against the
    /// other in the same tree.
    fn agreement_for(&self, l: &LoanListing) -> Option<f32> {
        if LoanAgreement::disarmed() {
            let level = LoanDestinationLevel {
                ability: l.ability,
                parent_best_in_group: l.parent_best_in_group,
                parent_rep: l.parent_rep,
                borrower_rep: self.borrower_world_rep,
                parent_league_rep: l.parent_league_rep,
                borrower_league_rep: self.borrower_league_rep,
                is_development: l.is_development,
            };
            return LegacyLoanGuard::domestic_allows(&LegacyDomesticGate {
                player_id: l.player_id,
                borrower_name: &self.club.name,
                group: l.position_group,
                ability: l.ability,
                is_development: l.is_development,
                parent_best_in_group: l.parent_best_in_group,
                level: &level,
                depth: &self.borrower_depth,
                guard: l.guard.as_ref(),
                borrower: self.borrower_for(l.position_group),
            })
            .then_some(1.0);
        }
        let group = l.position_group;
        let borrower = self.borrower_for(group);
        let verdict = l
            .guard
            .as_ref()
            .zip(borrower.as_ref())
            .map(|(g, b)| g.assess(b, l.willingness.score, l.parent_subsidy));
        let inputs = AgreementInputs {
            parent: l.willingness,
            parent_rep: l.parent_rep,
            parent_league_rep: l.parent_league_rep,
            parent_best_in_group: l.parent_best_in_group,
            parent_subsidy: l.parent_subsidy,
            borrower_tier: self.rep_level,
            borrower_rep: self.borrower_world_rep,
            borrower_league_rep: self.borrower_league_rep,
            group,
            count: self.borrower_depth.headcount(group),
            best_here: self.borrower_depth.best_in_group(group),
            clearly_better_ahead: self.borrower_depth.clearly_better_ahead(group, l.ability),
            need: self.need_for(l).score(),
            slot_room: self
                .foreign_slots
                .room_after(l.nationality_country_id, self.pending_foreign),
            mid_season_window: self.mid_season_window,
            candidate: l.ability,
            is_development: l.is_development,
            stage: l.stage,
            club_band_target: l.club_band_target,
            plan: l.plan,
            renown_gap: verdict.map(|v| v.renown_gap).unwrap_or(0.0),
            renown_band: l.guard.as_ref().map(|g| g.renown_band()).unwrap_or(0.0),
            resignation: l
                .guard
                .as_ref()
                .map(|g| g.listing_resignation())
                .unwrap_or(0.0),
            going_home: false,
            // Both sides of this deal are in one country: the parent is
            // placing him where it already lives, and he is not moving
            // anywhere he does not already play.
            placement_trust: 1.0,
            familiarity: 1.0,
            weight: verdict.map(|v| v.weight).unwrap_or(0.0),
            carry: verdict.map(|v| v.carry).unwrap_or(0.0),
            asking: l.asking_price,
            max_loan_fee: self.max_loan_fee,
        };
        if TransferTrace::is(l.player_id) {
            TransferTrace::line(
                l.player_id,
                "loan",
                format!(
                    "borrower={} {}",
                    self.club.name,
                    LoanAgreement::explain_inputs(&inputs)
                ),
            );
        }
        LoanAgreement::price(&inputs)
    }

    /// How badly this club wants a body in that shirt, for one
    /// candidate. The same reading the broadcast and the cross-border
    /// scan take, so the three of them describe one borrower.
    fn need_for(&self, l: &LoanListing) -> BorrowerNeed {
        let group = l.position_group;
        let request = self
            .open_requests
            .iter()
            .find(|r| r.position.position_group() == group);
        let ideal = group.ideal_squad_depth();
        let held = self.borrower_depth.headcount(group);
        BorrowerNeed {
            requested: request.is_some(),
            level_shortfall: request
                .map(|r| r.min_ability as i16 - l.ability as i16)
                .unwrap_or(0),
            age_excess: request
                .map(|r| l.age as i16 - r.preferred_age_max as i16)
                .unwrap_or(0),
            vacancy: ((ideal as f32 - held as f32) / ideal.max(1) as f32).clamp(0.0, 1.0),
        }
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
            answers_open_request: self
                .open_requests
                .iter()
                .any(|r| r.position.position_group() == l.position_group),
        }
    }

    /// Score every sweep's slate. Nothing here depends on what any other club
    /// claims, so clubs are scored in parallel; [`BorrowerTurn::claim`] runs
    /// the sweeps against the claims afterwards.
    fn score<'b>(&self, board: &'b LoanBoard) -> BorrowerTurn<'b> {
        // The same listing sits in several sweeps' slates and prices the same
        // in each, so each is priced once.
        let mut listed_memo: Vec<Option<Option<f32>>> = vec![None; board.listed.len()];
        let requests = self.request_slates(&board.listed, &mut listed_memo);
        let avg_ability = self.avg_ability;
        // Every club reads the loan market. How many looks it spends is
        // the scan budget — a bigger club simply has fewer, because
        // fewer of the names on it are for it. Reserving the whole
        // opportunistic sweep for small clubs and emergencies was a
        // category gate in front of an appetite term that already prices
        // the same fact.
        let opportunistic = self.slate(&board.listed, &mut listed_memo, |l| {
            l.age <= MAX_LOAN_TARGET_AGE
            // Squad-average floor is for cover loans; a youth
            // match-practice loan leans on the minutes gate.
            && (l.is_development || l.ability >= avg_ability.saturating_sub(5))
        });
        // The mid-season window is a second look for everybody: the
        // shape of a season is known by then and a hole is a hole.
        let january = if self.mid_season_window {
            self.slate(&board.listed, &mut listed_memo, |l| {
                l.age <= MAX_LOAN_TARGET_AGE
                // Squad-average floor is for cover loans; a youth
                // match-practice loan leans on the minutes gate.
                && (l.is_development || l.ability >= avg_ability.saturating_sub(8))
            })
        } else {
            Vec::new()
        };
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
        // Every tier cold-calls, Elite included: the agreement prices
        // the distance between two clubs continuously, and a second
        // hard reading of it is a gate doing a price's job.
        let cold = if self.scan_unsolicited {
            let borrower_world_rep = self.borrower_world_rep;
            let mut cold_memo: Vec<Option<Option<f32>>> = vec![None; board.unsolicited.len()];
            self.slate(&board.unsolicited, &mut cold_memo, |l| {
                // Approach "up": only a club below the parent
                // borrows the player for minutes.
                l.parent_rep > borrower_world_rep && l.age <= MAX_LOAN_TARGET_AGE
            })
        } else {
            Vec::new()
        };

        BorrowerTurn {
            club_id: self.club.id,
            max_scans: self.max_scans,
            mid_season_window: self.mid_season_window,
            scan_unsolicited: self.scan_unsolicited,
            requests,
            opportunistic,
            january,
            cold,
        }
    }

    /// Against the club's own open requests first — a loan that answers a brief
    /// the club has already written. Emergency free-agent depth requests are
    /// excluded: those are the free-agent matcher's, never a loan's.
    fn request_slates<'b>(
        &self,
        listings: &'b [LoanListing],
        memo: &mut [Option<Option<f32>>],
    ) -> Vec<RequestSlate<'b>> {
        self.club
            .transfer_plan
            .transfer_requests
            .iter()
            .filter(|r| {
                r.status != TransferRequestStatus::Fulfilled
                    && r.status != TransferRequestStatus::Abandoned
                    && !r.is_emergency_free_agent_depth()
            })
            // Skip if position is full AND loan wouldn't be an upgrade.
            // Request-driven cover uses the strict bar — a club with a
            // full line asked for depth elsewhere, not a keeper prospect.
            .filter(|r| !self.should_skip_loan(r.position.position_group(), r.min_ability, false))
            .map(|request| {
                let pos_group = request.position.position_group();
                // Relaxed thresholds: min_ability - 5, age_max + 3
                let relaxed_min = request.min_ability.saturating_sub(5);
                let relaxed_age_max = request
                    .preferred_age_max
                    .saturating_add(3)
                    .min(MAX_LOAN_TARGET_AGE);
                RequestSlate {
                    group: pos_group,
                    reason: request.reason.as_signing_reason_key(),
                    candidates: self.slate(listings, memo, |l| {
                        l.position_group == pos_group
                            && l.ability >= relaxed_min
                            && l.age <= relaxed_age_max
                            && l.age >= request.preferred_age_min
                    }),
                }
            })
            .collect()
    }

    /// Every listing that clears this club's standing gates and the sweep's
    /// own, priced. Which of them the club goes for is a matter of preference,
    /// so the claim draws in proportion to interest rather than taking the
    /// top row of an ability sort — the argmax made this a fixed pairing that
    /// repeated until the listing disappeared.
    fn slate<'b>(
        &self,
        listings: &'b [LoanListing],
        memo: &mut [Option<Option<f32>>],
        sweep: impl Fn(&LoanListing) -> bool,
    ) -> ScoredSlate<'b> {
        let club = self.club;
        let plan = &club.transfer_plan;
        listings
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                l.club_id != club.id
                && !club.is_rival(l.club_id) // no loans from rivals
                && l.asking_price * 0.8 <= self.max_loan_fee
                && sweep(l)
                && !plan.is_loan_approach_barred(l.player_id, self.date)
                && !self.active_pairs.contains(&(l.player_id, club.id))
            })
            .filter_map(|(i, l)| {
                let score = *memo[i].get_or_insert_with(|| self.scored(l));
                score.map(|s| (l, s))
            })
            .collect()
    }

    /// The agreement times the club's own interest in him. Priced once per
    /// pair: the score IS the filter, and running the agreement twice emitted
    /// the trace line twice for every candidate a club looked at.
    fn scored(&self, l: &LoanListing) -> Option<f32> {
        let agreement = self.agreement_for(l)?;
        self.taste
            .interest_in(&self.profile_of(l, l.asking_price * 0.8))
            .map(|score| score * agreement)
    }
}

/// One borrowing club's four sweeps, scored before any club has claimed
/// anybody.
struct BorrowerTurn<'a> {
    club_id: u32,
    max_scans: usize,
    mid_season_window: bool,
    scan_unsolicited: bool,
    requests: Vec<RequestSlate<'a>>,
    opportunistic: ScoredSlate<'a>,
    january: ScoredSlate<'a>,
    cold: ScoredSlate<'a>,
}

impl BorrowerTurn<'_> {
    /// The four sweeps, in the order the old body ran them. Each stops at the
    /// same per-club cap, and each sees what the ones before it — this club's
    /// and every club earlier in the visit order — claimed.
    fn claim(&self, claimed: &mut FxHashSet<u32>, actions: &mut Vec<LoanScanAction>) {
        let mut scans_this_club = 0usize;
        // Track position groups already targeted in this scan pass
        // to avoid starting multiple negotiations for the same position
        let mut scanned_position_groups: Vec<PlayerFieldPositionGroup> = Vec::new();

        for request in &self.requests {
            if scans_this_club >= self.max_scans {
                break;
            }
            // Only scan once per position group — multiple requests for the same
            // group (e.g. FormationGap + DepthCover for GK) should not each trigger
            // a separate loan negotiation
            if scanned_position_groups.contains(&request.group) {
                continue;
            }
            let open = Self::unclaimed(&request.candidates, claimed);
            if let Some(best) = InterestDraw::pick(&Self::weights(&open)).map(|i| open[i as usize].0)
            {
                self.stage(best, TransferReason::key(request.reason), false, claimed, actions);
                scanned_position_groups.push(request.group);
                scans_this_club += 1;
            }
        }

        if scans_this_club < self.max_scans {
            let open = Self::unclaimed(&self.opportunistic, claimed);
            let weighted = Self::weights(&open);
            // Draw a surplus and walk it: the slate is filtered once, so
            // without a running group check one pass could open three
            // negotiations for the same position — which the scan's own
            // `scanned_position_groups` bookkeeping exists to prevent and
            // the old sort-and-take quietly allowed. Over-drawing keeps the
            // club's full scan budget usable despite the extra constraint.
            let wanted = self.max_scans - scans_this_club;
            for idx in InterestDraw::pick_several(&weighted, (wanted * 4).min(weighted.len())) {
                if scans_this_club >= self.max_scans {
                    break;
                }
                let opp = open[idx as usize].0;
                if scanned_position_groups.contains(&opp.position_group) {
                    continue;
                }
                self.stage(
                    opp,
                    TransferReason::key("signing_reason_loan_opportunistic_upgrade"),
                    false,
                    claimed,
                    actions,
                );
                scanned_position_groups.push(opp.position_group);
                scans_this_club += 1;
            }
        }

        if self.mid_season_window && scans_this_club < self.max_scans {
            let open = Self::unclaimed(&self.january, claimed);
            if let Some(opp) = InterestDraw::pick(&Self::weights(&open)).map(|i| open[i as usize].0)
            {
                self.stage(
                    opp,
                    TransferReason::key("signing_reason_loan_midseason_reinforcement"),
                    false,
                    claimed,
                    actions,
                );
            }
        }

        if self.scan_unsolicited && scans_this_club < self.max_scans {
            let open = Self::unclaimed(&self.cold, claimed);
            let weighted = Self::weights(&open);
            // Terminal branch for this club: the draw is already capped at
            // the per-club scan budget, and nothing after this reads the
            // counter or the scanned-groups set, so the only bookkeeping
            // left is the within-draw group guard.
            let mut cold_groups: Vec<PlayerFieldPositionGroup> = Vec::new();
            let wanted = self.max_scans - scans_this_club;
            for idx in InterestDraw::pick_several(&weighted, (wanted * 4).min(weighted.len())) {
                if cold_groups.len() >= wanted {
                    break;
                }
                let tgt = open[idx as usize].0;
                if cold_groups.contains(&tgt.position_group) {
                    continue;
                }
                cold_groups.push(tgt.position_group);
                self.stage(
                    tgt,
                    TransferReason::key("signing_reason_loan_development_approach"),
                    true,
                    claimed,
                    actions,
                );
            }
        }
    }

    /// The slate as it stands when the sweep opens — a sweep does not re-read
    /// the claims its own draws make.
    fn unclaimed<'s>(
        slate: &'s [(&'s LoanListing, f32)],
        claimed: &FxHashSet<u32>,
    ) -> Vec<&'s (&'s LoanListing, f32)> {
        slate
            .iter()
            .filter(|(l, _)| !claimed.contains(&l.player_id))
            .collect()
    }

    fn weights(open: &[&(&LoanListing, f32)]) -> Vec<(u32, f32)> {
        open.iter()
            .enumerate()
            .map(|(i, (_, score))| (i as u32, *score))
            .collect()
    }

    fn stage(
        &self,
        l: &LoanListing,
        reason: TransferReason,
        is_unsolicited: bool,
        claimed: &mut FxHashSet<u32>,
        actions: &mut Vec<LoanScanAction>,
    ) {
        claimed.insert(l.player_id);
        actions.push(LoanScanAction {
            club_id: self.club_id,
            player_id: l.player_id,
            selling_club_id: l.club_id,
            offer_amount: FormattingUtils::round_fee(l.asking_price * 0.8),
            reason,
            is_unsolicited,
            seller_asking: l.asking_price,
            is_development: l.is_development,
        });
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

            let selling_rep = ClubView::get_club_reputation(country, action.selling_club_id);
            let buying_rep = ClubView::get_club_reputation(country, action.club_id);
            let (p_age, p_ambition) =
                PlayerView::get_player_negotiation_data(country, action.player_id, date);

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
            if let Some(option_fee) = LoanPipeline::loan_option_fee(country, &action, date) {
                clauses.push(TransferClause::LoanOptionToBuy(CurrencyValue {
                    amount: option_fee,
                    currency: Currency::Usd,
                }));
            }

            // Add appearance fee clause for high-reputation selling clubs
            let selling_rep_level =
                ClubView::get_club_reputation_level(country, action.selling_club_id);
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
                let (p_name, sc_name) = PlayerView::resolve_player_and_club_name(
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
    mid_season_window: bool,
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
            mid_season_window: MarketCadence::is_mid_season_window_for(&country.code, date),
            scan_unsolicited: LoanPipeline::is_market_day(date),
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

        let load = LoanScanLoad {
            pending_loans: LoanPipeline::pending_incoming_loans_by_club(country),
            pending_foreign: LoanPipeline::pending_foreign_registrations_by_club(country),
            active_counts: country.transfer_market.active_negotiation_counts(),
            active_pairs: country.transfer_market.active_negotiation_pairs(),
        };

        let country_ref: &Country = country;
        let turns: Vec<Option<BorrowerTurn<'_>>> = (0..country_ref.clubs.len())
            .into_par_iter()
            .map(|club_idx| {
                BorrowerScan::open(country_ref, club_idx, tick, &load).map(|scan| scan.score(&board))
            })
            .collect();

        // Rotate who looks first. The per-pass dedup is "has anybody claimed
        // him yet", so registration order was first refusal on the whole
        // market — the lowest-id club took the pick of every listing, every
        // tick, forever.
        let mut actions: Vec<LoanScanAction> = Vec::new();
        let mut claimed: FxHashSet<u32> = FxHashSet::default();
        for club_idx in InterestDraw::visit_order(turns.len()) {
            if let Some(turn) = &turns[club_idx] {
                turn.claim(&mut claimed, &mut actions);
            }
        }
        drop(turns);

        LoanScanCommit::apply(country, actions, date);
    }
}
