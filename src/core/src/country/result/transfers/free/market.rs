//! The free-agent market, as one country runs it on one day.
//!
//! Eight passes, which the comments inside the old 935-line
//! `handle_free_agents` already numbered: find who is out of contract, add the
//! ones the rest of the world is letting go, honour any staged pre-contract,
//! fill an emergency hole, match everybody else to a club that needs them,
//! turn the staged depth offers into real ones, clear what the market has left
//! sitting, and finally execute.
//!
//! Its ten parameters are two ideas: what the pass reads from outside the
//! country ([`FreeAgentWorld`]) and what it reports back to the tick
//! ([`FreeAgentLedger`]). Both were spelled out at every call site.

use super::depth::{
    DepthNegotiationAction, EmergencyDepthRequestPlanner, FreeAgentNegotiationStager,
};
use super::pricing::{FreeAgentMarketCalculator, FreeAgentOfferPricing};
use crate::club::player::transfer::FreeAgentBlockReason;
use crate::club::staff::perception::PotentialEstimator;
use crate::country::result::CountryResult;
use crate::country::result::transfers::config::TransferConfig;
use crate::country::result::transfers::execution::{DevelopmentLoanPathway, TransferExecutor};
use crate::country::result::transfers::types::{
    CountryRoster, DeferredTransfer, TransferActivitySummary,
};
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::MarketMap;
use crate::transfers::deal::negotiation::{
    NegotiationPhase, NegotiationStatus, TransferNegotiation,
};
use crate::transfers::deal::offer::TransferOffer;
use crate::transfers::deal::reason::TransferReason;
use crate::transfers::gate::fit::SquadRegistrationLimits;
use crate::transfers::market::region::ScoutingRegion;
use crate::transfers::pipeline::{
    PipelineProcessor, TransferNeedReason, TransferRequest, TransferRequestStatus,
};
use crate::transfers::view::club::ClubView;
use crate::transfers::{CompletedTransfer, TransferType};
use crate::utils::IntegerUtils;
use crate::{Club, Country, Person, PlayerStatusType, Team};
use chrono::NaiveDate;
use log::debug;
use std::collections::HashSet;

use super::*;

/// What the free-agent pass reads from outside this country: the pool the rest
/// of the world is letting go, the geography between here and there, and the
/// market's own tuning.
pub(in crate::country::result) struct FreeAgentWorld<'a> {
    pub global_pool: &'a [GlobalFreeAgentSummary],
    pub market_map: &'a MarketMap,
    pub config: &'a TransferConfig,
}

/// What the pass reports back to the tick. Every field is a bucket the world
/// pass drains after the country is done.
pub(in crate::country::result) struct FreeAgentLedger<'a> {
    pub summary: &'a mut TransferActivitySummary,
    pub domestic_signed_ids: &'a mut Vec<u32>,
    pub global_offered_ids: &'a mut Vec<u32>,
    pub global_rejected_ids: &'a mut Vec<u32>,
    pub global_blocked: &'a mut Vec<(u32, FreeAgentBlockReason)>,
}

/// What expiry day did to this country's rosters: who ran out of contract, and
/// who the club talked into staying before the release sweep could clear them.
struct ExpiryOutcome {
    expired: Vec<u32>,
    renewed: HashSet<u32>,
}

/// The market as this country sees it today: who is free, who can see whom,
/// what the registration rules allow, and the caps the tuning sets. Read once
/// for the whole tick, because every one of these is a property of the market
/// rather than of the club doing the asking.
struct FreeAgentMarket<'a> {
    country: &'a Country,
    candidates: &'a [FreeAgentCandidate],
    visibility: &'a FreeAgentMarketVisibility,
    registration: &'a SquadRegistrationLimits,
    config: &'a TransferConfig,
    max_signings_per_day: usize,
    ability_slack: u8,
    buyer_country_reputation: u16,
    buyer_continent_id: u32,
    buyer_region_prestige: f32,
}

/// The matching in progress. Every club reads what the clubs before it did —
/// the day's signing cap, who has already been approached today — so this is
/// one accumulator rather than a per-club return value.
struct MatchState<'a> {
    signings: Vec<FreeAgentSigning>,
    approached_today: HashSet<(u32, u32)>,
    depth_offers: Vec<DepthNegotiationAction>,
    recorder: BlockReasonRecorder,
    /// What the emergency pass already signed, so the day's cap counts only
    /// what the request matcher adds on top.
    emergency_signing_count: usize,
    global_offered_ids: &'a mut Vec<u32>,
    global_rejected_ids: &'a mut Vec<u32>,
}

/// One club as the free-agent market reads it: what it can pay, who negotiates
/// for it, and how much room it has left under the foreigner quota.
struct FreeAgentBuyer<'a> {
    club: &'a Club,
    /// Resolved by type, not by position: a club main team is not always the
    /// first entry in its team list.
    main_team: Option<&'a Team>,
    club_score: f32,
    league_reputation: u16,
    negotiator_skill: u8,
    foreign_slots: ForeignSlotCount,
    benefactor: f32,
}

/// The shirt being filled, as one candidate's turn reads it.
struct RequestBrief<'a> {
    request: &'a TransferRequest,
    group: PlayerFieldPositionGroup,
    is_depth_request: bool,
    urgency_bonus: f32,
}

/// Why one candidate's turn ended.
enum CandidateOutcome {
    /// He said no, or could not be asked. Try the next name on the slate.
    Next,
    /// The shirt is spoken for — signed outright, or a depth pursuit staged.
    Filled,
}

/// Why one request's turn ended.
enum RequestOutcome {
    /// Move on to the club's next request.
    Next,
    /// The day's signing cap is spent; the club is done, and so is every club
    /// after it.
    DayFull,
}

/// Why one club's turn ended.
enum ClubOutcome {
    /// Move on to the next club.
    Next,
    /// The day's signing cap is spent; no club after this one gets a turn.
    DayFull,
}

/// The pass itself.
pub(in crate::country::result::transfers) struct FreeAgentMarketPass;

impl FreeAgentMarketPass {
    pub(in crate::country::result::transfers) fn run(
        country: &mut Country,
        date: NaiveDate,
        world: &FreeAgentWorld<'_>,
        ledger: &mut FreeAgentLedger<'_>,
    ) -> Vec<GlobalFreeAgentSigning> {
        let market_map = world.market_map;
        let config = world.config;
        let global_blocked = &mut *ledger.global_blocked;
        let global_offered_ids = &mut *ledger.global_offered_ids;
        let global_rejected_ids = &mut *ledger.global_rejected_ids;

        let candidates = Self::candidates(country, date, world);

        if candidates.is_empty() {
            return Vec::new();
        }

        // How visible each candidate is to THIS market — the corridor from
        // his passport and his last league, the market's own familiarity
        // with both, how long he has been available, and whether this is a
        // league that buys names. Computed once here because the answer is a
        // property of the market, not of the club doing the asking.
        let visibility = FreeAgentMarketVisibility::build(country.id, market_map, &candidates);

        // The league's foreigner quota, resolved once for the whole pass.
        // Every free-agent door — request matcher, emergency fill, both
        // clearing tiers — asks it before it signs anybody, which is what
        // the paid paths have always done.
        let registration = SquadRegistrationLimits::new(country.id, &country.regulations);

        // Why each global-pool candidate was skipped today, highest-rank
        // reason per player. Drained into `global_blocked` at the end of
        // the tick; Phase C stamps it onto the player's market state.
        // Created BEFORE the emergency pass so that pass can explain its
        // own rejections through the same channel — it is the door the
        // geography complaint was actually about, and it was the one that
        // recorded nothing.
        let mut recorder = BlockReasonRecorder::new();

        // Pass 2: Match candidates to clubs with needs, using probability-based signing
        let mut signings: Vec<FreeAgentSigning> = Vec::new();

        // ── Pass 2-pre: honour staged pre-contracts ─────────────────
        // A player who agreed a pre-contract while running his deal down
        // now has an expired contract (cleared by the release sweep
        // above). Route the agreed free transfer to his future club
        // FIRST — pushed ahead of the emergency / request / clearing
        // passes so their `signings.iter().any(...)` dedup leaves him be.
        // Pass 3 executes it through the ordinary in-country path.
        CountryResult::collect_pre_contract_signings(country, &mut signings);

        // ── Pass 2a (NEW): Emergency squad fill ─────────────────────
        // Runs BEFORE the request-driven matcher so clubs sitting
        // under MIN_FIRST_TEAM_SQUAD don't have to wait for the
        // scouting/shortlist pipeline. Pushes into the same
        // `signings` vec so Pass 3 executes them through the existing
        // path and the normal matcher's `signings.iter().any(...)`
        // dedup naturally skips already-claimed candidates.
        //
        // Depth shortfalls are NOT signed here — the pass returns them
        // as intents and they become DepthCover pipeline requests
        // below, serviced through the staged-negotiation flow like any
        // other recruitment need. Only the "cannot field a side /
        // group below minimum" rescue slots keep the direct path.
        let depth_intents = CountryResult::handle_free_agents_emergency_pass(
            country,
            &candidates,
            config,
            &visibility,
            &mut signings,
            global_offered_ids,
            global_rejected_ids,
            &mut recorder,
        );
        EmergencyDepthRequestPlanner::stage_requests(country, &depth_intents);

        // Peak post-season window (Jun–Aug) lifts the request-driven cap
        // so summer free-agent business isn't throttled to the off-season
        // trickle.
        let max_signings_per_day = config.max_free_agent_signings_for(date);
        let ability_slack = config.free_agent_ability_slack;
        let buyer_country_reputation = country.reputation;
        let buyer_continent_id = country.continent_id;
        // Mirrors `scan_foreign_loan_market`: same region the country sits
        // in, used as the prestige anchor for cross-region gating.
        let buyer_region = ScoutingRegion::from_country(country.continent_id, &country.code);
        let buyer_region_prestige = buyer_region.league_prestige();
        // (club, player) pairs already approached this tick — a player
        // who turned this club down under one request must not be
        // re-asked the same day under another.
        let approached_today: HashSet<(u32, u32)> = HashSet::new();
        // Depth-type requests (DepthCover / SquadPadding) never sign
        // instantly — they collect staged offers here and the stager
        // below turns each one into a real Pending negotiation that
        // resolves over the following days via
        // `resolve_pending_negotiations` (personal terms → medical).
        let depth_offers: Vec<DepthNegotiationAction> = Vec::new();
        // Snapshot the emergency-pass headcount so the normal cap
        // measures only ITS own signings — otherwise an emergency
        // pass that already added 5 picks would starve every
        // request-driven match for the rest of the tick.
        let emergency_signing_count = signings.len();

        let mut state = MatchState {
            signings,
            approached_today,
            depth_offers,
            recorder,
            emergency_signing_count,
            global_offered_ids,
            global_rejected_ids,
        };
        Self::match_to_clubs(
            country,
            &FreeAgentMarket {
                country,
                candidates: &candidates,
                visibility: &visibility,
                registration: &registration,
                config,
                max_signings_per_day,
                ability_slack,
                buyer_country_reputation,
                buyer_continent_id,
                buyer_region_prestige,
            },
            &mut state,
        );
        let MatchState {
            mut signings,
            depth_offers,
            mut recorder,
            ..
        } = state;

        // Pass 2b: turn the staged depth offers into real Pending
        // negotiations (PersonalTerms phase). Runs after the matcher
        // loop because creating a negotiation needs the mutable
        // country borrow the loop's club iteration holds immutably.
        let staged_depth_ids: HashSet<u32> = depth_offers.iter().map(|d| d.player_id).collect();
        FreeAgentNegotiationStager::stage(country, depth_offers, date, global_offered_ids);

        // Pass 2c: long-term market clearing. Free agents past the
        // pressure / days-free thresholds stop waiting for an explicit
        // transfer request — they take a modest squad-role deal at a
        // lower-tier club with open roster room. Runs last so it only
        // touches the long tail the emergency and request-driven
        // passes left behind.
        CountryResult::handle_free_agents_market_clearing_pass(
            country,
            &candidates,
            config,
            date,
            &visibility,
            &staged_depth_ids,
            &mut signings,
            global_offered_ids,
            global_rejected_ids,
            &mut recorder,
            market_map,
        );

        // Surface the tick's skip reasons; Phase C stamps them onto
        // the pool players' market state outside the country borrow.
        recorder.drain_into(global_blocked);

        // Split signings: in-country (player still has a from-club row)
        // versus global pool (player lives in `sim.free_agents`, signaled

        Self::commit(country, date, signings, ledger)
    }

    /// Passes 1 and 1b — who is actually free today.
    fn candidates(
        country: &mut Country,
        date: NaiveDate,
        world: &FreeAgentWorld<'_>,
    ) -> Vec<FreeAgentCandidate> {
        let (mut candidates, expired, renewed) = Self::expiring_here(country, date);
        Self::add_global_pool(
            country,
            world,
            &mut candidates,
            &ExpiryOutcome { expired, renewed },
        );
        candidates
    }

    /// Contracts running out or already lapsed on a roster in this country.
    /// The owning club gets one last renewal attempt before the release sweep
    /// clears anybody — real clubs do not watch a player they want walk out on
    /// expiry day without a final offer.
    fn expiring_here(
        country: &mut Country,
        date: NaiveDate,
    ) -> (Vec<FreeAgentCandidate>, Vec<u32>, HashSet<u32>) {
        // Pass 1: Find players with expiring contracts (< 90 days) or already expired
        let mut candidates: Vec<FreeAgentCandidate> = Vec::new();
        let mut expired_player_ids: Vec<u32> = Vec::new();

        for club in &country.clubs {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    // Loaned-in players belong to their parent club regardless
                    // of whether the local record has a `contract` field set.
                    // Check in both branches so a stale None-contract on a loan
                    // can't accidentally mark the player as free.
                    if player.is_on_loan() {
                        continue;
                    }

                    let days_left = match &player.contract {
                        Some(c) => (c.expiration - date).num_days(),
                        None => 0, // already a free agent
                    };

                    // Contract already expired — release player
                    if days_left <= 0 && player.contract.is_some() {
                        expired_player_ids.push(player.id);
                        // Still add as candidate (will be available after release below)
                    }

                    // Available for free agent signing: contract expired or
                    // the player has no contract at all. A player with a
                    // running contract — even one expiring next week —
                    // stays at his current club until it actually ends;
                    // otherwise we fabricate "free transfers" of players
                    // who were still under contract, which is the exact
                    // move real leagues prohibit. Pre-contract agreements
                    // (signed now, effective at contract end) would need
                    // their own deferred-execution flow, not this path.
                    if days_left <= 0 {
                        // Skip if already mid-negotiation — but a staged
                        // pre-contract also wears the `Trn` badge, and
                        // for him THIS scan is the execution path (the
                        // expiry routing honours the agreed club), so he
                        // must pass through.
                        if (player.statuses.has(PlayerStatusType::Trn)
                            || player.statuses.has(PlayerStatusType::Bid))
                            && player.pending_pre_contract().is_none()
                        {
                            continue;
                        }

                        let last_salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
                        candidates.push(FreeAgentCandidate {
                            player_id: player.id,
                            player_name: player.full_name.to_string(),
                            club_id: club.id,
                            club_name: club.name.clone(),
                            ability: player.player_attributes.current_ability,
                            // Signing decisions read the observable
                            // ceiling, never hidden biological PA.
                            potential: PotentialEstimator::observable_ceiling(player, date),
                            age: player.age(date),
                            position_group: player.position().position_group(),
                            days_to_expiry: days_left,
                            // In-country candidates are by definition at a
                            // club in this country, so the country-rep gate
                            // always passes — record `country.reputation`
                            // directly. Same for the region gate: the
                            // candidate sits in `country`, so the buyer's
                            // own region is its own reference point.
                            nationality_country_reputation: country.reputation,
                            nationality_region: ScoutingRegion::from_country(
                                country.continent_id,
                                &country.code,
                            ),
                            // For in-country expiring contracts we don't
                            // hydrate the player's true nationality here
                            // (would need a SimulatorData lookup we don't
                            // have). They're treated as domestic for
                            // emergency-fill purposes — which is the
                            // common case anyway and skews preference
                            // mildly toward local journeymen.
                            nationality_country_code: country.code.clone(),
                            nationality_continent_id: country.continent_id,
                            // Treated as domestic for the same reason: the
                            // true passport is not reachable from inside
                            // this borrow, and the affinity of a man
                            // already playing here is 1.0 either way.
                            nationality_country_id: country.id,
                            // Expiring contracts haven't entered the
                            // market yet — pressure is zero, the player
                            // is just transitioning. The new gates fall
                            // back to the original behaviour for them
                            // because `reference_reputation` matches
                            // the buyer's country rep exactly.
                            career_pressure: 0.0,
                            days_free: 0,
                            reference_reputation: country.reputation,
                            last_salary,
                            last_country_reputation: country.reputation,
                            last_league_reputation: country.reputation,
                            world_reputation: player.player_attributes.world_reputation,
                            current_reputation: player.player_attributes.current_reputation,
                            professionalism_norm: (player.attributes.professionalism / 20.0)
                                .clamp(0.0, 1.0),
                            // Expiring-contract candidates aren't on the
                            // open market yet — no accumulated pity.
                            failed_approach_streak: 0,
                            is_global_pool: false,
                            // An expiring domestic contract is, by
                            // construction, a man playing right here.
                            last_country_id: country.id,
                        });
                    }
                }
            }
        }

        // Final-chance renewal: before the release sweep clears expired
        // contracts, the owning club makes one synchronous renewal attempt
        // (real clubs don't watch a player they want walk out on expiry day
        // without a last offer). Accepted players carry a fresh contract and
        // leave the free-agent flow entirely; rejected ones continue into
        // the release sweep unchanged.
        let renewed_player_ids =
            CountryResult::run_expiry_day_renewals(country, date, &expired_player_ids);
        candidates.retain(|c| !renewed_player_ids.contains(&c.player_id));

        (candidates, expired_player_ids, renewed_player_ids)
    }

    /// The players the rest of the world has put in its "move on a free" pool,
    /// plus the release sweep over whoever here is still expired.
    fn add_global_pool(
        country: &mut Country,
        world: &FreeAgentWorld<'_>,
        candidates: &mut Vec<FreeAgentCandidate>,
        expiry: &ExpiryOutcome,
    ) {
        let global_pool = world.global_pool;
        let expired_player_ids = &expiry.expired;
        let renewed_player_ids = &expiry.renewed;

        // Pass 1b: Include the global "Move on Free" pool — players who live
        // outside any country's roster in `sim.free_agents`. Without this
        // step, manually-released players are invisible to club AI: only
        // contract-expiry candidates above would ever get signed. Use
        // club_id=0 / club_name="Free Agent" as the synthetic "from" so the
        // matching filter in Pass 2 (`c.club_id != club.id`) and the Pass 3
        // splitter (`from_club_id == 0` → defer to caller) both work.
        for fa in global_pool {
            candidates.push(FreeAgentCandidate {
                player_id: fa.player_id,
                player_name: fa.player_name.clone(),
                club_id: 0,
                club_name: "Free Agent".to_string(),
                ability: fa.ability,
                potential: fa.potential,
                age: fa.age,
                position_group: fa.position_group,
                days_to_expiry: 0,
                nationality_country_reputation: fa.nationality_country_reputation,
                nationality_region: ScoutingRegion::from_country(
                    fa.nationality_continent_id,
                    &fa.nationality_country_code,
                ),
                nationality_country_code: fa.nationality_country_code.clone(),
                nationality_continent_id: fa.nationality_continent_id,
                nationality_country_id: fa.nationality_country_id,
                career_pressure: fa.career_pressure,
                days_free: fa.days_free,
                reference_reputation: fa.reference_reputation,
                last_salary: fa.last_salary,
                last_country_reputation: fa.last_country_reputation,
                last_league_reputation: fa.last_league_reputation,
                world_reputation: fa.world_reputation,
                current_reputation: fa.current_reputation,
                professionalism_norm: fa.professionalism_norm,
                failed_approach_streak: fa.failed_approach_streak,
                is_global_pool: true,
                last_country_id: fa.last_country_id,
            });
        }

        // Release players with expired contracts. Players who accepted the
        // expiry-day renewal above are no longer expired — skip them, and
        // keep their shortlist/scouting interest intact (they're still
        // legitimate transfer targets under contract).
        for &player_id in expired_player_ids {
            if renewed_player_ids.contains(&player_id) {
                continue;
            }
            for club in &mut country.clubs {
                for team in &mut club.teams.teams {
                    if let Some(player) =
                        team.players.players.iter_mut().find(|p| p.id == player_id)
                    {
                        debug!(
                            "Contract expired: player {} ({}) released from {}",
                            player.full_name, player_id, club.name
                        );
                        player.contract = None;
                        break;
                    }
                }
            }
            // A freshly-released player is no longer a transfer target at his
            // old club, and he cannot be on any other club's loan-out list —
            // drop shortlist, scouting, and loan-out entries everywhere.
            PipelineProcessor::clear_player_interest(country, player_id);
        }
    }

    /// Pass 2 — offer the market to every club that has an unfilled request.
    fn match_to_clubs(country: &Country, market: &FreeAgentMarket<'_>, state: &mut MatchState<'_>) {
        for club in &country.clubs {
            match Self::serve_club(country, club, market, state) {
                ClubOutcome::Next => {}
                ClubOutcome::DayFull => break,
            }
        }
    }

    /// One club's turn: read what it can pay and register, then work its
    /// requests in order until the day's cap is spent.
    fn serve_club(
        country: &Country,
        club: &Club,
        market: &FreeAgentMarket<'_>,
        state: &mut MatchState<'_>,
    ) -> ClubOutcome {
        let registration = market.registration;
        let max_signings_per_day = market.max_signings_per_day;
        let emergency_signing_count = state.emergency_signing_count;
        let signings = &state.signings;

        if signings.len() - emergency_signing_count >= max_signings_per_day {
            return ClubOutcome::DayFull;
        }

        if club.teams.teams.is_empty() {
            return ClubOutcome::Next;
        }

        // Skip clubs that have reached their squad cap
        if !ClubView::can_accept_player(club) {
            return ClubOutcome::Next;
        }

        let plan = &club.transfer_plan;
        if !plan.initialized {
            return ClubOutcome::Next;
        }

        // Check unfulfilled transfer requests
        let unfulfilled: Vec<&TransferRequest> = plan
            .transfer_requests
            .iter()
            .filter(|r| {
                r.status != TransferRequestStatus::Fulfilled
                    && r.status != TransferRequestStatus::Abandoned
            })
            .collect();

        // Pre-compute the buyer's tier anchors. Used for role
        // inference and the quality-fit band — the same numbers
        // every rolling-CA gate in the project relies on. The tier
        // anchor curves are calibrated for `overall_score()` (home /
        // national / world blend); reading raw `world` — usually the
        // lowest of the three — understated every buyer's band and
        // biased the whole pool toward `AboveMaximumAbility`.
        let main_team = club.teams.main().or_else(|| club.teams.teams.first());
        let buyer_club_score = main_team
            .map(|t| t.reputation.overall_score().clamp(0.0, 1.0))
            .unwrap_or(0.0);
        let buyer_league_reputation = main_team
            .and_then(|t| t.league_id)
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);
        // Man-management is the closest analogue to "negotiator
        // skill" in the staff attribute schema. Staff skills run
        // 0..20, scaled to 0..100 here so it slots into the
        // calculator's percentage-style negotiation factor.
        let buyer_negotiator_skill = main_team
            .and_then(|t| t.staffs.find_negotiator())
            .map(|s| (s.staff_attributes.mental.man_management as u32 * 5).min(100) as u8)
            .unwrap_or(50);
        // The quota is a squad fact, so it is counted once per club and
        // not once per open request.
        let buyer_foreign_slots = registration.count(club);
        let buyer_benefactor = club.board.ownership.benefactor;

        let buyer = FreeAgentBuyer {
            club,
            main_team,
            club_score: buyer_club_score,
            league_reputation: buyer_league_reputation,
            negotiator_skill: buyer_negotiator_skill,
            foreign_slots: buyer_foreign_slots,
            benefactor: buyer_benefactor,
        };
        for request in &unfulfilled {
            match Self::serve_request(country, &buyer, request, market, state) {
                RequestOutcome::Next => {}
                RequestOutcome::DayFull => return ClubOutcome::DayFull,
            }
        }

        ClubOutcome::Next
    }

    /// One request's turn: rank who is visible and plausible for the shirt,
    /// then work down the slate until somebody says yes or the attempts run out.
    fn serve_request(
        country: &Country,
        buyer: &FreeAgentBuyer<'_>,
        request: &TransferRequest,
        market: &FreeAgentMarket<'_>,
        state: &mut MatchState<'_>,
    ) -> RequestOutcome {
        let club = buyer.club;

        let buyer_club_score = buyer.club_score;
        let buyer_league_reputation = buyer.league_reputation;
        let buyer_negotiator_skill = buyer.negotiator_skill;
        let buyer_foreign_slots = buyer.foreign_slots;
        let buyer_benefactor = buyer.benefactor;
        let candidates = market.candidates;
        let visibility = market.visibility;
        let config = market.config;
        let max_signings_per_day = market.max_signings_per_day;
        let ability_slack = market.ability_slack;
        let buyer_country_reputation = market.buyer_country_reputation;
        let buyer_continent_id = market.buyer_continent_id;
        let buyer_region_prestige = market.buyer_region_prestige;
        let emergency_signing_count = state.emergency_signing_count;
        let signings_so_far = state.signings.len();

        let depth_offers = &mut state.depth_offers;
        let recorder = &mut state.recorder;

        if signings_so_far - emergency_signing_count >= max_signings_per_day {
            return RequestOutcome::DayFull;
        }

        let group = request.position.position_group();

        // Emergency-planner depth requests route through the
        // staged-negotiation flow below instead of instant
        // signing. Explicitly marker-driven: a normal evaluated
        // DepthCover / SquadPadding request keeps the legacy
        // instant path.
        let is_depth_request = request.is_emergency_free_agent_depth();
        // One pursuit in flight per request, for EVERY source:
        // Negotiating means a live paid negotiation (or staged
        // FA pursuit) already owns this need. Instant-signing on
        // top of it delivered two players for one hole — the FA
        // journeyman landed, `mark_group_fulfilled` stamped the
        // request Fulfilled, and the paid negotiation still
        // completed for the same position.
        if request.status == TransferRequestStatus::Negotiating {
            return RequestOutcome::Next;
        }

        let buyer_ctx = RequestBuyerContext {
            club_score: buyer_club_score,
            league_reputation: buyer_league_reputation,
            negotiator_skill: buyer_negotiator_skill,
            country_reputation: buyer_country_reputation,
            continent_id: buyer_continent_id,
            region_prestige: buyer_region_prestige,
            visibility: &visibility,
            foreign_slots: buyer_foreign_slots,
            benefactor: buyer_benefactor,
        };
        let nominal_floor = request.min_ability.saturating_sub(ability_slack);

        // Gate pass — the same sliding career-pressure
        // tolerances as before (quality band, country rep,
        // cross-continent, region prestige), but every
        // passing candidate is collected instead of only the
        // single best, and each gate failure is recorded so
        // the diagnosis layer can explain long sits.
        let mut ranked: Vec<(&FreeAgentCandidate, f32)> = Vec::new();
        for c in candidates.iter() {
            if c.club_id == club.id {
                continue;
            }
            if c.position_group != group {
                continue;
            }
            if state.signings.iter().any(|s| s.player_id == c.player_id)
                || depth_offers.iter().any(|d| d.player_id == c.player_id)
            {
                continue;
            }
            match RequestCandidateGates::evaluate(
                c,
                &buyer_ctx,
                group,
                is_depth_request,
                nominal_floor,
            ) {
                Ok(()) => {
                    let priority = RequestCandidateOrdering::priority(c, &buyer_ctx, group);
                    ranked.push((c, priority));
                }
                Err(reason) => {
                    if c.is_global_pool {
                        recorder.record(c.player_id, reason);
                    }
                }
            }
        }
        if ranked.is_empty() {
            return RequestOutcome::Next;
        }
        // Combined score replaces the legacy raw-quality
        // `max_by_key`: quality fit, locality, rep closeness,
        // career pressure, and wage affordability together
        // decide the order, so a realistic willing journeyman
        // outranks a stronger player who will never accept.
        ranked.sort_by(RequestCandidateOrdering::cmp);

        // Daily probability of this club making an offer today.
        // Urgency reflects how badly the request matters; for
        // free agents the unfulfilled-request reason maps to a
        // urgency bonus.
        let urgency_bonus = match request.reason {
            TransferNeedReason::SquadPadding => 10.0,
            TransferNeedReason::FormationGap => 7.0,
            TransferNeedReason::DepthCover => 5.0,
            TransferNeedReason::CheapReinforcement => 4.0,
            TransferNeedReason::QualityUpgrade => 3.0,
            _ => 2.0,
        };

        // Fallback attempts: walk the ranked list until a
        // candidate signs (or stages), or the per-request
        // attempt cap runs out. The legacy single-candidate
        // behaviour skipped the whole request when the one
        // pick failed a roll, which let an unrealistic strong
        // candidate starve every signable player behind them.
        let mut attempts = 0usize;
        for (best, _priority) in ranked {
            if attempts >= config.free_agent_attempts_per_request {
                break;
            }
            if state.signings.len() - emergency_signing_count >= max_signings_per_day {
                break;
            }
            // One pursuit in flight per (club, player) pair.
            if is_depth_request
                && country
                    .transfer_market
                    .has_active_negotiation_for(best.player_id, club.id)
            {
                continue;
            }
            // One approach per (club, player) per tick — a
            // player this club already tried today under
            // another request must not be re-asked.
            if !state.approached_today.insert((club.id, best.player_id)) {
                continue;
            }
            attempts += 1;

            match Self::try_candidate(
                buyer,
                &RequestBrief {
                    request,
                    group,
                    is_depth_request,
                    urgency_bonus,
                },
                best,
                market,
                state,
            ) {
                CandidateOutcome::Next => {}
                CandidateOutcome::Filled => break,
            }
        }

        RequestOutcome::Next
    }

    /// One name on the slate: roll the daily approach chance, and if he answers
    /// either stage a depth pursuit, surface a global-pool intent, or sign him.
    fn try_candidate(
        buyer: &FreeAgentBuyer<'_>,
        brief: &RequestBrief<'_>,
        best: &FreeAgentCandidate,
        market: &FreeAgentMarket<'_>,
        state: &mut MatchState<'_>,
    ) -> CandidateOutcome {
        let country = market.country;
        let club = buyer.club;
        let main_team = buyer.main_team;
        let buyer_club_score = buyer.club_score;
        let buyer_league_reputation = buyer.league_reputation;
        let buyer_negotiator_skill = buyer.negotiator_skill;
        let request = brief.request;
        let group = brief.group;
        let is_depth_request = brief.is_depth_request;
        let urgency_bonus = brief.urgency_bonus;
        let config = market.config;
        let buyer_country_reputation = market.buyer_country_reputation;

        let daily_chance = if best.is_global_pool {
            // Pity bonus lifts the daily chance for a
            // structurally-signable player who keeps losing
            // the approach roll, so a real squad need isn't
            // left unfilled for months purely on dice. The
            // fresh-high-ability bonus makes a good player who
            // just came free move quickly when a club already
            // has a matching open request for him.
            FreeAgentMarketCalculator::daily_signing_chance(
                best.career_pressure,
                best.ability,
                urgency_bonus
                    + FreeAgentMarketCalculator::pity_bonus(best.failed_approach_streak)
                    + config.fresh_high_ability_bonus(best.days_free, best.ability),
            )
        } else {
            // In-country expiring-contract candidates keep the
            // tuned tier-table behaviour — they're not on the
            // open market yet, just transitioning. Falling back
            // to the pressure curve here would cut elite-player
            // signings (CA 160 + pressure 0 = ~7%, vs the 25%
            // the existing balance assumes).
            config.daily_signing_chance(best.ability, best.potential, best.age)
        };

        // Roll the dice — a miss moves on to the next-ranked
        // candidate instead of abandoning the request.
        let roll = IntegerUtils::random(1, 1000) as f32 / 10.0; // 0.1 to 100.0
        if roll > daily_chance {
            if best.is_global_pool {
                state
                    .recorder
                    .record(best.player_id, FreeAgentBlockReason::DailyChanceRollFailed);
            }
            return CandidateOutcome::Next;
        }

        // Depth-type request: stage a real negotiation instead
        // of an instant signing. The player's acceptance is NOT
        // rolled here — `resolve_personal_terms` owns it when
        // the PersonalTerms phase matures, exactly like any
        // pipeline pursuit. Wage / role / contract length are
        // staged now so the offer the player evaluates is the
        // offer that gets installed on completion.
        if is_depth_request {
            let pricing = FreeAgentOfferPricing::compute(
                best,
                group,
                buyer_club_score,
                buyer_league_reputation,
                buyer_negotiator_skill,
                buyer_country_reputation,
            );
            let terms = pricing.signed_terms(best);
            // Player-side anchor for the rep-diff logic in
            // `resolve_personal_terms`: in-country candidates
            // use their current club's standing, pool players
            // their own reference reputation — a big name at a
            // tiny buyer reads as a downward move and resists.
            let selling_rep = if best.is_global_pool {
                (best.reference_reputation as f32 / 10_000.0).clamp(0.0, 1.0)
            } else {
                country
                    .clubs
                    .iter()
                    .find(|c| c.id == best.club_id)
                    .and_then(|c| c.teams.teams.first())
                    .map(|t| t.reputation.overall_score().clamp(0.0, 1.0))
                    .unwrap_or(0.3)
            };
            let player_ambition = if best.is_global_pool {
                0.5
            } else {
                CountryRoster::find(country, best.player_id)
                    .map(|p| p.attributes.ambition)
                    .unwrap_or(0.5)
            };
            let negotiator_staff_id =
                main_team.and_then(|t| t.staffs.find_negotiator().map(|s| s.id));

            state.depth_offers.push(DepthNegotiationAction {
                player_id: best.player_id,
                player_name: best.player_name.clone(),
                from_club_id: best.club_id,
                from_club_name: best.club_name.clone(),
                to_club_id: club.id,
                request_id: request.id,
                terms,
                selling_rep,
                buying_rep: buyer_club_score,
                buying_league_reputation: buyer_league_reputation,
                negotiator_staff_id,
                player_age: best.age,
                player_ambition,
                is_global_pool: best.is_global_pool,
                reason: TransferReason::key(request.reason.as_signing_reason_key()),
            });
            // One staged pursuit per request — the resolver
            // owns it from here.
            return CandidateOutcome::Filled;
        }

        // Acceptance: would the player actually sign this
        // particular offer? Wage / role / prestige / quality
        // fit weighted into a single score, sigmoid against a
        // pressure-decayed threshold. Skipped for in-country
        // expiring contracts (no career pressure; pre-decay
        // behaviour keeps the existing balance).
        if best.is_global_pool {
            if let Some(outcome) = Self::global_pool_answer(buyer, brief, best, market, state) {
                return outcome;
            }
        }

        let reason = TransferReason::key(request.reason.as_signing_reason_key());

        // Stage stage-aware contract terms so the installed deal
        // matches the free agent's market stage (a long-unemployed
        // or older player signs a short trial, not a multi-year
        // deal off the generic age-band default) and carries the
        // role / pressure-decayed wage. Mirrors the depth and
        // global-pool pricing so the contract-length policy can't
        // drift between the free-agent entry points.
        let terms = FreeAgentOfferPricing::compute(
            best,
            group,
            buyer_club_score,
            buyer_league_reputation,
            buyer_negotiator_skill,
            buyer_country_reputation,
        )
        .signed_terms(best);

        state.signings.push(FreeAgentSigning {
            player_id: best.player_id,
            player_name: best.player_name.clone(),
            from_club_id: best.club_id,
            from_club_name: best.club_name.clone(),
            to_club_id: club.id,
            reason,
            terms: Some(terms),
            fills_group: Some(group),
        });
        return CandidateOutcome::Filled;
    }

    /// Pass 3 — execute every staged signing as a free transfer, with the

    /// A name from the pool the rest of the world is letting go. He is priced
    /// and rolled against here, and every roll counts as an offer received.
    ///
    /// `None` means he said yes — and deliberately does NOT sign him: a
    /// global-pool player is signed on the ordinary path below, so the terms
    /// and the history record come out identical however he was found.
    fn global_pool_answer(
        buyer: &FreeAgentBuyer<'_>,
        brief: &RequestBrief<'_>,
        best: &FreeAgentCandidate,
        market: &FreeAgentMarket<'_>,
        state: &mut MatchState<'_>,
    ) -> Option<CandidateOutcome> {
        let buyer_club_score = buyer.club_score;
        let buyer_league_reputation = buyer.league_reputation;
        let buyer_negotiator_skill = buyer.negotiator_skill;
        let group = brief.group;
        let buyer_country_reputation = market.buyer_country_reputation;
        let recorder = &mut state.recorder;
        let global_offered_ids = &mut *state.global_offered_ids;
        let global_rejected_ids = &mut *state.global_rejected_ids;

        let pricing = FreeAgentOfferPricing::compute(
            best,
            group,
            buyer_club_score,
            buyer_league_reputation,
            buyer_negotiator_skill,
            buyer_country_reputation,
        );
        let rep_drop = FreeAgentMarketCalculator::rep_drop_allowed(
            best.career_pressure,
            best.age,
            best.ability,
        );
        let min_ca = FreeAgentMarketCalculator::min_acceptable_ca(
            buyer_club_score,
            group,
            best.career_pressure,
        );
        let max_ca = FreeAgentMarketCalculator::max_acceptable_ca(
            buyer_club_score,
            group,
            best.career_pressure,
        );
        let wage_fit =
            FreeAgentMarketCalculator::wage_score(pricing.offer_wage, pricing.reservation_wage);
        let score = FreeAgentMarketCalculator::acceptance_score(
            wage_fit,
            FreeAgentMarketCalculator::role_score(pricing.role),
            FreeAgentMarketCalculator::prestige_score(
                buyer_country_reputation,
                best.reference_reputation,
                rep_drop,
            ),
            FreeAgentMarketCalculator::quality_fit_score(best.ability, min_ca, max_ca),
            best.career_pressure,
        );
        let threshold = FreeAgentMarketCalculator::acceptance_threshold(best.career_pressure);
        let prob = FreeAgentMarketCalculator::acceptance_probability(score, threshold);
        let acceptance_roll = IntegerUtils::random(1, 1000) as f32 / 1000.0;
        // Every roll is an "offer received" — the player
        // got a concrete approach today. Track separately
        // whether they accepted so the pool-side state can
        // bump `offers_rejected_total` only on declines.
        global_offered_ids.push(best.player_id);
        if acceptance_roll > prob {
            global_rejected_ids.push(best.player_id);
            // A clearly-underwater wage is the most
            // informative cause; otherwise it was the
            // overall composition.
            recorder.record(
                best.player_id,
                if wage_fit < 0.35 {
                    FreeAgentBlockReason::WageReservationMismatch
                } else {
                    FreeAgentBlockReason::AcceptanceRollFailed
                },
            );
            return Some(CandidateOutcome::Next);
        }

        None
    }

    /// Pass 3 — turn every staged signing into a real move.
    fn commit(
        country: &mut Country,
        date: NaiveDate,
        signings: Vec<FreeAgentSigning>,
        ledger: &mut FreeAgentLedger<'_>,
    ) -> Vec<GlobalFreeAgentSigning> {
        Self::record_negotiations(country, date, &signings);
        Self::execute(country, date, signings, ledger)
    }

    /// Every free signing gets a negotiation record, so a move that never went
    /// through the paid pipeline still reads like one in history.
    fn record_negotiations(country: &mut Country, date: NaiveDate, signings: &[FreeAgentSigning]) {
        for signing in signings {
            if signing.from_club_id == 0 {
                continue;
            }
            let negotiator_staff_id = country
                .clubs
                .iter()
                .find(|c| c.id == signing.to_club_id)
                .and_then(|c| c.teams.teams.first())
                .and_then(|t| t.staffs.find_negotiator().map(|s| s.id));

            let neg_id = country.transfer_market.next_negotiation_id;
            country.transfer_market.next_negotiation_id += 1;

            let offer = TransferOffer::new(
                CurrencyValue::new(0.0, Currency::Usd),
                signing.to_club_id,
                date,
            );

            let mut negotiation = TransferNegotiation::new(
                neg_id,
                signing.player_id,
                0,
                signing.from_club_id,
                signing.to_club_id,
                offer,
                date,
                0.0,
                0.0,
                0,
                0.0,
            );
            negotiation.negotiator_staff_id = negotiator_staff_id;
            negotiation.reason = signing.reason.clone();
            negotiation.status = NegotiationStatus::Accepted;
            negotiation.phase = NegotiationPhase::MedicalAndFinalization { started: date };
            country
                .transfer_market
                .negotiations
                .insert(neg_id, negotiation);
        }
    }

    /// The moves themselves. A signing whose seller is club 0 came from the
    /// global pool and cannot be executed here — the country pass has no reach
    /// into `sim.free_agents` — so it surfaces as an intent and the world tick
    /// adjudicates first-come.
    fn execute(
        country: &mut Country,
        date: NaiveDate,
        signings: Vec<FreeAgentSigning>,
        ledger: &mut FreeAgentLedger<'_>,
    ) -> Vec<GlobalFreeAgentSigning> {
        let summary = &mut *ledger.summary;
        let domestic_signed_ids = &mut *ledger.domestic_signed_ids;
        let mut global_signings: Vec<GlobalFreeAgentSigning> = Vec::new();
        let country_id = country.id;

        for signing in signings {
            if signing.from_club_id == 0 {
                // Global pool signing — the caller must execute against
                // `sim.free_agents`. We surface intent only; first-come-
                // first-served dedup happens at execution time when the
                // player may have already been claimed by another country.
                let to_club_id = signing.to_club_id;
                let fills_group = signing.fills_group;
                global_signings.push(GlobalFreeAgentSigning {
                    player_id: signing.player_id,
                    player_name: signing.player_name,
                    buying_country_id: country_id,
                    buying_club_id: to_club_id,
                    reason: signing.reason,
                    terms: signing.terms,
                });
                // Even though execution is deferred, the buying club's
                // open request for the same group is conceptually
                // serviced — mark fulfilled now so weekly re-evaluation
                // doesn't re-emit it. The actual roster mutation may
                // still fail at Phase C (player taken by another
                // country first); the request mark is conservative —
                // worst case a later tick re-emits it.
                if let Some(group) = fills_group {
                    TransferPlanSync::mark_group_fulfilled(country, to_club_id, group);
                }
                continue;
            }

            let to_club_name = country
                .clubs
                .iter()
                .find(|c| c.id == signing.to_club_id)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            // Captured before `signing.reason` is moved into the history
            // row below, so the monthly diagnostics can split pre-contract
            // moves from ordinary domestic-expiry signings.
            let is_pre_contract = signing.reason.key == "pre_contract";

            // Execute first — a failed move (squad full, player not found
            // at claimed origin) must NOT leave a phantom transfer-history
            // row. The club-transfers page reads this list directly, so
            // any entry written here is visible whether or not the player
            // actually moved.
            let buying_league_reputation = country
                .clubs
                .iter()
                .find(|c| c.id == signing.to_club_id)
                .and_then(|c| c.teams.teams.first())
                .and_then(|t| t.league_id)
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);
            // Translate staged emergency terms (if any) into the
            // executor's wage + personal-terms inputs. Without this
            // the in-country free-agent path silently falls back to
            // the calculator default and any short-deal pitch made
            // during the emergency offer evaporates.
            let agreed_annual_wage = signing.terms.map(|t| t.annual_wage);
            let personal_terms = signing.terms.map(|t| t.to_personal_terms());
            let deferred = DeferredTransfer {
                player_id: signing.player_id,
                selling_country_id: country.id,
                selling_club_id: signing.from_club_id,
                buying_country_id: country.id,
                buying_club_id: signing.to_club_id,
                fee: 0.0,
                is_loan: false,
                has_option_to_buy: false,
                agreed_annual_wage,
                buying_league_reputation,
                sell_on_percentage: None,
                loan_future_fee: None,
                personal_terms,
                // Free-agent signings carry no transfer-fee clauses
                // (no fee, no sell-on, no installments).
                offer_clauses: Vec::new(),
            };
            // The country IS the world here: this pass runs inside Phase A
            // on one country borrow, and a free signing off a domestic
            // expiry never leaves it. The executor is the same one the
            // cross-border path uses — see
            // [`crate::transfers::view::world::MarketWorld`].
            let executed = TransferExecutor::permanent(country, &deferred, date);

            if !executed {
                debug!(
                    "Free agent signing rejected: player {} from club {} to club {}",
                    signing.player_id, signing.from_club_id, signing.to_club_id
                );
                continue;
            }

            // A young free signing at a big club is development material
            // too — same pathway as paid prospect purchases. Foreign
            // loanee count is unavailable from a single-country borrow;
            // the domestic count still enforces the cap.
            DevelopmentLoanPathway::stage_after_purchase(
                country,
                signing.to_club_id,
                signing.player_id,
                None,
                date,
                0,
            );

            country.transfer_market.transfer_history.push(
                CompletedTransfer::new(
            signing.player_id,
            signing.player_name,
            signing.from_club_id,
            0,
            signing.from_club_name,
            signing.to_club_id,
            to_club_name,
            date,
            CurrencyValue::new(0.0, Currency::Usd),
            TransferType::Free,
        )
        .with_reason(signing.reason)
        // A domestic expiry never leaves the country, and the row
        // has to say so: without an origin the corridor census
        // cannot tell it apart from a pool signing out of nowhere.
        .with_origin_country(country.id),
            );

            PipelineProcessor::clear_player_interest(country, signing.player_id);
            // Mirror the global-pool branch above: once a signing
            // actually lands, mark the matching group's open request as
            // fulfilled so the weekly re-evaluation doesn't generate a
            // duplicate. Done after execution so a failed move (squad
            // cap, lookup miss) doesn't silently fulfill a still-open
            // need.
            if let Some(group) = signing.fills_group {
                TransferPlanSync::mark_group_fulfilled(country, signing.to_club_id, group);
            }
            // Surface the signed id so the caller (which holds the full
            // simulator) can run the cross-country interest sweep once
            // the country mutable borrow ends.
            domestic_signed_ids.push(signing.player_id);
            summary.completed_transfers += 1;
            // Pre-contract moves are a subset of the in-country signings;
            // count them separately so Phase C can split the monthly
            // "pre-contract" vs "domestic-expiry" diagnostics.
            if is_pre_contract {
                summary.signed_pre_contract += 1;
            }

            debug!(
                "Free agent signing: player {} from club {} to club {}",
                signing.player_id, signing.from_club_id, signing.to_club_id
            );
        }

        global_signings
    }
}
