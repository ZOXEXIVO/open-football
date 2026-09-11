//! The domestic half of opening a negotiation.
//!
//! Three passes, the same shape as [`super::foreign`]: read every club's
//! shortlists and decide an offer for the candidate at the cursor, write the
//! negotiations, then mark the candidates the realism gate refused so their
//! shortlist cursor advances instead of stalling on a dud.
//!
//! The offer is [`super::ApproachBuilder`], shared with the cross-border pass.
//! What stays here is what a single country can answer for itself: who is
//! already negotiating for the man, whether he is on loan or protected, and
//! which of the buyer's own shortlists is next in line.

use crate::transfers::loan::LoanPipeline;
use crate::transfers::pipeline::approach::ApproachPass;
use crate::transfers::view::club::ClubView;
use crate::transfers::view::player::PlayerView;
use chrono::NaiveDate;
use log::debug;

use crate::Country;
use crate::transfers::pipeline::ShortlistCandidate;

use super::*;

/// One buying club, resolved once: what it can spend, what its wage bill will
/// bear, and how many negotiations it may still open.
struct DomesticBuyer<'a> {
    country: &'a Country,
    club: &'a Club,
    team: &'a Team,
    plan: &'a ClubTransferPlan,
    rep_level: ReputationLevel,
    buying_rep_score: f32,
    buying_league_reputation: u16,
    avg_ability: u8,
    budget: f64,
    wage_headroom: Option<f64>,
    slots_available: usize,
}

/// The man at a shortlist cursor, as his prospective buyer reads him — plus
/// the buy / loan / loan-with-option call the director of football made.
struct DomesticTarget<'a> {
    player: &'a Player,
    selling_club: &'a Club,
    selling_club_id: u32,
    selling_rep_score: f32,
    selling_league_reputation: u16,
    is_rival: bool,
    monitoring: Option<&'a ScoutPlayerMonitoring>,
    scouting_report: Option<&'a DetailedScoutingReport>,
    approach: TransferApproach,
    is_loan: bool,
    has_option_to_buy: bool,
    is_prospect_purchase: bool,
}

/// What reading the cursor produced.
enum TargetOutcome<'a> {
    Pursue(DomesticTarget<'a>),
    /// Outside the request's age band: stage a reject so the cursor advances
    /// past him rather than stalling on him every tick.
    Reject,
    /// Nothing to stage — he cannot be read, or his club no longer holds him.
    Skip,
}

/// The pass itself: decide, write, then clear the duds.
pub(in crate::transfers::pipeline) struct DomesticApproachPass;

impl DomesticApproachPass {
    pub(in crate::transfers::pipeline) fn run(country: &mut Country, date: NaiveDate) {
        let mut actions: Vec<NegotiationAction> = Vec::new();
        let mut plausibility_rejected: Vec<PlausibilityReject> = Vec::new();
        let price_level = country.settings.pricing.price_level;
        let window_mgr = TransferWindowManager::for_country(country.id, &country.code, date);
        let current_window = window_mgr.current_window_dates(country.id, date);

        for club in &country.clubs {
            let Some(buyer) = Self::buyer(country, club, date) else {
                continue;
            };
            Self::pursue(
                &buyer,
                date,
                price_level,
                current_window,
                &mut actions,
                &mut plausibility_rejected,
            );
        }

        Self::commit(country, date, actions);
        Self::clear_rejects(country, plausibility_rejected);

        LoanPipeline::process_loan_out_listings(country, date);
    }

    /// `None` when this club is not opening anything: no plan, squad full,
    /// already at its concurrent-negotiation ceiling, or no squad at all.
    fn buyer<'a>(
        country: &'a Country,
        club: &'a Club,
        date: NaiveDate,
    ) -> Option<DomesticBuyer<'a>> {
        let plan = &club.transfer_plan;

        if !plan.initialized || !plan.can_start_negotiation() {
            return None;
        }

        // Skip clubs that have reached their squad cap. Use the same
        // `ClubView::can_accept_player` predicate the executor enforces: it
        // resolves the Main team by TeamType, not `teams[0]`. The old
        // `teams.first()` count gated against whatever team happened to
        // sit first (often a reserve/B roster), so a club whose Main was
        // already full kept agreeing deals the executor then refused —
        // re-pursuing the same target every evaluation cycle.
        if !ClubView::can_accept_player(club) {
            return None;
        }

        let actual_active = country
            .transfer_market
            .active_negotiation_count_for_club(club.id);
        if actual_active >= plan.max_concurrent_negotiations {
            return None;
        }

        // Same FFP discipline the evaluation pass applies when it
        // sizes allocations — the raw read here fed the offer /
        // escalation strategy full spending power while the plan
        // itself was operating on half, so the two layers disagreed
        // about the same sanction.
        let raw_budget = club
            .finance
            .transfer_budget
            .as_ref()
            .map(|b| b.amount)
            .unwrap_or_else(|| (club.finance.balance.balance.max(0) as f64) * 0.3);
        let budget = if club.finance.is_ffp_breach(date) {
            raw_budget * 0.5
        } else {
            raw_budget
        };

        if club.teams.teams.is_empty() {
            return None;
        }

        let team = &club.teams.teams[0];
        let rep_level = team.reputation.level();
        let buying_rep_score = team.reputation.overall_score();
        let buying_league_reputation = team
            .league_id
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);

        let avg_ability = {
            let avg = team.players.current_ability_avg();
            if avg == 0 { 50 } else { avg }
        };

        // Board wage mandate headroom — annual wages committed across
        // all squads vs the season wage budget. None when no mandate
        // has been set (fresh worlds, test fixtures).
        let committed_wages: f64 = club
            .teams
            .iter()
            .map(|t| t.get_annual_salary() as f64)
            .sum();
        let wage_headroom = club
            .board
            .season_targets
            .as_ref()
            .map(|t| (t.wage_budget.max(0) as f64 - committed_wages).max(0.0));

        let slots_available = plan
            .max_concurrent_negotiations
            .saturating_sub(actual_active) as usize;

        Some(DomesticBuyer {
            country,
            club,
            team,
            plan,
            rep_level,
            buying_rep_score,
            buying_league_reputation,
            avg_ability,
            budget,
            wage_headroom,
            slots_available,
        })
    }

    /// Walk this club's shortlists, and for each one decide an offer for the
    /// candidate sitting at the cursor.
    fn pursue(
        buyer: &DomesticBuyer<'_>,
        date: NaiveDate,
        price_level: f32,
        current_window: Option<(NaiveDate, NaiveDate)>,
        actions: &mut Vec<NegotiationAction>,
        plausibility_rejected: &mut Vec<PlausibilityReject>,
    ) {
        let country = buyer.country;
        let club = buyer.club;
        let team = buyer.team;
        let plan = buyer.plan;
        let buying_rep_score = buyer.buying_rep_score;
        let buying_league_reputation = buyer.buying_league_reputation;
        let avg_ability = buyer.avg_ability;
        let budget = buyer.budget;
        let slots_available = buyer.slots_available;
        let mut negotiations_this_club = 0usize;

        for shortlist in &plan.shortlists {
            if negotiations_this_club >= slots_available {
                break;
            }

            if shortlist.has_pursuing_candidate() {
                continue;
            }

            if shortlist.all_exhausted() {
                continue;
            }

            // The owning request must still be live. A board veto
            // stamps `board_approved = Some(false)` + Abandoned; a
            // need already filled elsewhere (FA instant signing, a
            // won race) stamps Fulfilled. This loop used to ignore
            // both — vetoed targets were approached the same tick
            // the chairman blocked them, and the negotiation open
            // then overwrote the veto's Abandoned back to
            // Negotiating.
            let request = plan
                .transfer_requests
                .iter()
                .find(|r| r.id == shortlist.transfer_request_id);
            let request_live = request
                .map(|r| {
                    r.status != TransferRequestStatus::Abandoned
                        && r.status != TransferRequestStatus::Fulfilled
                        && r.board_approved != Some(false)
                })
                .unwrap_or(true);
            if !request_live {
                continue;
            }

            let candidate = match shortlist.current_candidate() {
                Some(c) if c.status == ShortlistCandidateStatus::Available => c,
                _ => continue,
            };

            let player_id = candidate.player_id;

            if country
                .transfer_market
                .has_active_negotiation_for(player_id, club.id)
            {
                continue;
            }

            // Skip players on loan contracts — they belong to another club
            // Skip recently signed players — their club has a plan for them
            let (is_on_loan, is_protected) = PlayerView::find_player_in_country(country, player_id)
                .map(|p| {
                    (
                        p.is_on_loan(),
                        p.is_transfer_protected(date, current_window),
                    )
                })
                .unwrap_or((false, false));
            if is_on_loan || is_protected {
                continue;
            }

            let selling_club_id = country
                .clubs
                .iter()
                .find(|c| c.teams.contains_player(player_id))
                .map(|c| c.id);

            let selling_club_id = match selling_club_id {
                Some(id) if id != club.id => id,
                _ => continue, // Foreign players handled by initiate_foreign_negotiations
            };

            let target = match Self::target(buyer, candidate, request, selling_club_id, date) {
                TargetOutcome::Pursue(target) => target,
                TargetOutcome::Reject => {
                    plausibility_rejected.push(PlausibilityReject {
                        club_id: club.id,
                        player_id,
                        shortlist_request_id: shortlist.transfer_request_id,
                    });
                    continue;
                }
                TargetOutcome::Skip => continue,
            };
            let DomesticTarget {
                player,
                selling_club,
                selling_club_id,
                selling_rep_score,
                selling_league_reputation,
                is_rival,
                monitoring,
                scouting_report,
                approach,
                is_loan,
                has_option_to_buy,
                is_prospect_purchase,
            } = target;

            let outcome = ApproachBuilder::build(
                &ApproachBuyer {
                    club,
                    team,
                    plan,
                    rep_score: buying_rep_score,
                    league_reputation: buying_league_reputation,
                    avg_ability,
                    budget,
                },
                &ApproachTarget {
                    player,
                    selling_club,
                    selling_club_id,
                    selling_rep_score,
                    selling_league_reputation,
                    is_rival,
                    monitoring,
                    scouting_report,
                },
                request,
                &ApproachContext {
                    // One country, so it is both reaches at once.
                    buy_country: country,
                    sell_country: country,
                    market_map: &MarketMap::default(),
                    price_level,
                    date,
                    shortlist_request_id: shortlist.transfer_request_id,
                    approach: approach.clone(),
                    is_loan,
                    has_option_to_buy,
                    is_prospect_purchase,
                },
                &ApproachDrift {
                    allocated_budget: shortlist.allocated_budget.min(budget),
                    valuation_reputation: team.reputation.market_value_score(),
                    shortlist_rank: shortlist
                        .candidates
                        .iter()
                        .position(|c| c.player_id == player_id)
                        .map(|p| p as u8),
                    competition_count: Some(
                        country
                            .transfer_market
                            .active_rival_bids(player_id, club.id)
                            .min(u8::MAX as u32) as u8,
                    ),
                    loan_appearance_fee: true,
                    generic_reason_fallback: false,
                    final_gate_fee: Some(candidate.estimated_fee),
                },
            );

            match outcome {
                ApproachOutcome::Refused => {
                    plausibility_rejected.push(PlausibilityReject {
                        club_id: club.id,
                        player_id,
                        shortlist_request_id: shortlist.transfer_request_id,
                    });
                    continue;
                }
                ApproachOutcome::Approach(action) => actions.push(*action),
            }

            negotiations_this_club += 1;
        }
    }

    /// What the club believes about the man at the shortlist cursor, and what
    /// its director of football decided to do about him.
    fn target<'a>(
        buyer: &DomesticBuyer<'a>,
        candidate: &ShortlistCandidate,
        request: Option<&TransferRequest>,
        selling_club_id: u32,
        date: NaiveDate,
    ) -> TargetOutcome<'a> {
        let country = buyer.country;
        let club = buyer.club;
        let plan = buyer.plan;
        let rep_level = buyer.rep_level.clone();
        let buying_rep_score = buyer.buying_rep_score;
        let buying_league_reputation = buyer.buying_league_reputation;
        let budget = buyer.budget;
        let wage_headroom = buyer.wage_headroom;
        let player_id = candidate.player_id;

        // Rivalry is a deal friction, not an absolute block. A weaker
        // rival approaching a giant has essentially no chance; a club
        // at parity or above can still force the move through by
        // paying a premium or on a reputation-gap flinch. The penalty
        // is applied during resolve_initial_approach via is_rival flag.
        let is_rival = club.is_rival(selling_club_id);

        // ──────────────────────────────────────────────────
        // SMART BUY/LOAN DECISION
        // The DoF decides the approach based on context:
        // - Club reputation tier
        // - Budget vs player value
        // - Transfer request reason
        // - Whether the player is loan-listed
        // - Player age and potential
        // ──────────────────────────────────────────────────

        // Scout-side context for this candidate — believed
        // ability/potential from monitoring rows or reports.
        // Drives both the buy/loan decision and (further down)
        // the offer strategy. Hidden PA is never consulted.
        let monitoring = plan
            .scout_monitoring
            .iter()
            .find(|m| m.player_id == player_id);
        let scouting_report = plan
            .scouting_reports
            .iter()
            .find(|r| r.player_id == player_id);
        let scout_assessed = monitoring
            .map(|m| (m.current_assessed_ability, m.current_assessed_potential))
            .or_else(|| scouting_report.map(|r| (r.assessed_ability, r.assessed_potential)));
        let scout_confidence = monitoring
            .map(|m| m.confidence)
            .or_else(|| scouting_report.map(|r| r.confidence));

        let target = PlayerView::find_player_in_country(country, player_id);
        let player_age = target.map(|p| p.age(date)).unwrap_or(25);

        // Stale-row guard: a candidate outside the request's age
        // band (inserted before the shortlist-side band gates
        // existed, or aged across a window boundary) must not
        // carry the request's motive into a deal — the
        // "32-year-old signed as a young prospect" reason bug.
        // Same relaxed band as every insertion path: min strict,
        // max + 3. Staged as a reject so the cursor advances to
        // the next candidate instead of stalling.
        if let Some(req) = request {
            if player_age < req.preferred_age_min
                || player_age > req.preferred_age_max.saturating_add(3)
            {
                return TargetOutcome::Reject;
            }
        }
        // "Gettable" signals: a peer/bigger seller only parts with
        // a prospect who is listed, wants out, or barely plays.
        let target_available = target
            .map(|p| {
                p.statuses.has(PlayerStatusType::Lst)
                    || p.statuses.has(PlayerStatusType::Loa)
                    || p.statuses.has(PlayerStatusType::Req)
                    || p.statuses.has(PlayerStatusType::Unh)
                    || (p.statistics.played + p.statistics.played_subs) < 10
            })
            .unwrap_or(false);
        let expected_wage = target
            .map(|p| {
                WageCalculator::expected_annual_wage(
                    p,
                    player_age,
                    buying_rep_score,
                    buying_league_reputation,
                )
            })
            .unwrap_or(0);
        let selling_rep_score = country
            .clubs
            .iter()
            .find(|c| c.id == selling_club_id)
            .and_then(|c| c.teams.teams.first())
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.3);
        let selling_league_reputation = country
            .clubs
            .iter()
            .find(|c| c.id == selling_club_id)
            .and_then(|c| c.teams.teams.first())
            .and_then(|t| t.league_id)
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);

        let prospect_ctx = ProspectSigningContext {
            scout_assessed,
            scout_confidence,
            prospect_slots_used: plan
                .prospect_buys_this_window
                .saturating_add(plan.prospect_pursuits_active),
            seller_rep_score: selling_rep_score,
            buyer_rep_score: buying_rep_score,
            target_available,
            wage_headroom,
            expected_wage,
        };

        let approach = ApproachPass::determine_transfer_approach(
            &rep_level,
            budget,
            candidate.estimated_fee,
            request,
            player_age,
            date,
            club.finance.balance.balance,
            &club.philosophy,
            &prospect_ctx,
        );

        let is_loan = !matches!(approach, TransferApproach::PermanentTransfer);
        let has_option_to_buy = matches!(approach, TransferApproach::LoanWithOption);
        let is_prospect_purchase = !is_loan
            && matches!(
                request.map(|r| &r.reason),
                Some(TransferNeedReason::DevelopmentSigning)
            );

        let Some(player) = PlayerView::find_player_in_country(country, player_id) else {
            return TargetOutcome::Skip;
        };
        let Some(selling_club) = country.clubs.iter().find(|c| c.id == selling_club_id) else {
            return TargetOutcome::Skip;
        };

        TargetOutcome::Pursue(DomesticTarget {
            player,
            selling_club,
            selling_club_id,
            selling_rep_score,
            selling_league_reputation,
            is_rival,
            monitoring,
            scouting_report,
            approach,
            is_loan,
            has_option_to_buy,
            is_prospect_purchase,
        })
    }

    /// Pass 2 — write: open a negotiation for every offer the clubs decided on.
    fn commit(country: &mut Country, date: NaiveDate, actions: Vec<NegotiationAction>) {
        // Pass 2: Start negotiations
        for action in actions {
            let selling_rep = ClubView::get_club_reputation(country, action.selling_club_id);
            let buying_rep = ClubView::get_club_reputation(country, action.club_id);
            let (p_age, p_ambition) =
                PlayerView::get_player_negotiation_data(country, action.player_id, date);

            let has_listing = country
                .transfer_market
                .get_listing_by_player(action.player_id)
                .is_some();

            if !has_listing {
                let listing_type = if action.is_loan {
                    TransferListingType::Loan
                } else {
                    TransferListingType::Transfer
                };

                let selling_team_id = country
                    .clubs
                    .iter()
                    .find(|c| c.id == action.selling_club_id)
                    .and_then(|c| c.teams.teams.first())
                    .map(|t| t.id)
                    .unwrap_or(0);

                // The synthetic listing advertises the SELLER's asking price,
                // not the buyer's budget-capped offer. Pricing it off the
                // offer let a cash-poor club define the seller's valuation and
                // walk away with a first-team player for a fraction of his
                // worth; the seller's own asking keeps the acceptance ratio
                // honest (an unaffordable bid now reads as the lowball it is).
                let asking = SyntheticListingPrice::for_unsolicited(&action.seller_asking);

                // Tag this as synthetic — the parent club did not list
                // the player; the negotiation resolver must not grant
                // the "is_listed" acceptance bonus to bids backed by it.
                let listing = TransferListing::new_with_origin(
                    action.player_id,
                    action.selling_club_id,
                    selling_team_id,
                    asking,
                    date,
                    listing_type,
                    TransferListingOrigin::SyntheticUnsolicited,
                );
                country.transfer_market.add_listing(listing);
            }

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                action.player_id,
                action.club_id,
                action.offer,
                date,
                selling_rep,
                buying_rep,
                p_age,
                p_ambition,
            ) {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_loan = action.is_loan;
                    negotiation.has_option_to_buy = action.has_option_to_buy;
                    negotiation.is_unsolicited = !has_listing;
                    negotiation.negotiator_staff_id = action.negotiator_staff_id;
                    negotiation.reason = action.reason.clone();
                    negotiation.player_name = action.player_name.clone();
                    negotiation.selling_club_name = action.selling_club_name.clone();
                    negotiation.player_sold_from = action.player_sold_from.clone();
                    negotiation.open_salary_at(action.offered_annual_wage);
                    negotiation.buying_league_reputation = action.buying_league_reputation;
                    negotiation.selling_league_reputation = action.selling_league_reputation;
                    negotiation.player_stage_inclination = action.player_stage_inclination;
                    negotiation.buyer_ceiling_fee = action.buyer_ceiling_fee;
                    negotiation.brief_tier = action.brief_tier;
                    negotiation.reason.rival = action.is_rival;
                }

                if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                    let plan = &mut club.transfer_plan;

                    if let Some(shortlist) = plan
                        .shortlists
                        .iter_mut()
                        .find(|s| s.transfer_request_id == action.shortlist_request_id)
                    {
                        if let Some(candidate) = shortlist.current_candidate_mut() {
                            if candidate.player_id == action.player_id {
                                candidate.status = ShortlistCandidateStatus::CurrentlyPursuing;
                            }
                        }
                    }

                    if let Some(req) = plan
                        .transfer_requests
                        .iter_mut()
                        .find(|r| r.id == action.shortlist_request_id)
                    {
                        req.status = TransferRequestStatus::Negotiating;
                    }

                    plan.active_negotiation_count += 1;
                    if action.is_prospect_purchase {
                        // Pursuit slot taken; converted into a completed
                        // buy (or released) in on_negotiation_resolved.
                        plan.prospect_pursuits_active =
                            plan.prospect_pursuits_active.saturating_add(1);
                    }
                }

                debug!(
                    "Pipeline: Club {} started negotiation for player {} ({})",
                    action.club_id,
                    action.player_id,
                    if action.is_loan { "loan" } else { "transfer" }
                );
            }
        }
    }

    /// Mark every candidate the realism gate refused as unavailable and
    /// advance its shortlist past the dud ONCE.
    ///
    /// These never opened a negotiation, so routing them through
    /// `on_negotiation_resolved` was wrong on three counts: it advanced the
    /// cursor a second time (silently skipping the next viable candidate),
    /// decremented the active-negotiation slot counter for a slot never taken,
    /// and charged the manager a failed-bid morale hit for a bid never made.
    fn clear_rejects(country: &mut Country, plausibility_rejected: Vec<PlausibilityReject>) {
        // Apply plausibility/band rejects: mark each shortlist candidate
        // as unavailable, advance the shortlist past the dud ONCE, and
        // update monitoring + request status inline. These never opened a
        // negotiation, so routing them through `on_negotiation_resolved`
        // was wrong on three counts: it advanced the cursor a second time
        // (silently skipping the next viable candidate), decremented the
        // active-negotiation slot counter for a slot never taken, and
        // charged the manager a failed-bid morale hit for a bid that was
        // never made.
        for reject in plausibility_rejected {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == reject.club_id) {
                let plan = &mut club.transfer_plan;
                plan.set_monitoring_status_for_player(
                    reject.player_id,
                    ScoutMonitoringStatus::Lost,
                );
                if let Some(shortlist) = plan
                    .shortlists
                    .iter_mut()
                    .find(|s| s.transfer_request_id == reject.shortlist_request_id)
                {
                    if let Some(candidate) = shortlist
                        .candidates
                        .iter_mut()
                        .find(|c| c.player_id == reject.player_id)
                    {
                        candidate.status = ShortlistCandidateStatus::Unavailable;
                    }
                    shortlist.advance_to_next();
                    let exhausted = shortlist.all_exhausted();
                    if let Some(req) = plan
                        .transfer_requests
                        .iter_mut()
                        .find(|r| r.id == reject.shortlist_request_id)
                    {
                        // Never resurrect a need that was filled or
                        // vetoed while this candidate sat on the list.
                        let request_live = req.status != TransferRequestStatus::Fulfilled
                            && req.status != TransferRequestStatus::Abandoned;
                        if request_live {
                            req.status = if !exhausted {
                                TransferRequestStatus::Shortlisted
                            } else if req.priority == TransferNeedPriority::Critical {
                                TransferRequestStatus::Pending
                            } else {
                                TransferRequestStatus::Abandoned
                            };
                        }
                    }
                }
            }
        }
    }
}
