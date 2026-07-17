//! Depth-fill routing for the free-agent market.
//!
//! `emergency_squad_fill_depth` slots never sign players directly. The
//! emergency pass in `free_agents.rs` surfaces them as
//! [`EmergencyDepthRequestIntent`]s; this module owns everything that
//! happens next:
//!
//! 1. [`EmergencyDepthRequestPlanner`] turns each intent into an open
//!    `DepthCover` pipeline request tagged
//!    [`TransferRequestSource::EmergencyFreeAgentDepth`] so the paid
//!    scouting / shortlist / loan paths skip it.
//! 2. The request-driven matcher in `handle_free_agents` picks a
//!    candidate, prices the offer through `FreeAgentOfferPricing`
//!    (in `free_agent_market_calc`), and collects a
//!    [`DepthNegotiationAction`].
//! 3. [`FreeAgentNegotiationStager`] creates the Pending negotiation
//!    (PersonalTerms phase) and wires the buying club's plan so
//!    `PipelineProcessor::on_negotiation_resolved` mirrors the outcome
//!    back onto the request like any pipeline pursuit.
//!
//! Resolution from there is the normal lifecycle in `negotiations.rs`:
//! `resolve_personal_terms` owns the player's decision, `resolve_medical`
//! completes (pool players via `NegotiationOutcomes.free_agent_signings`,
//! in-country players via the deferred-transfer queue).

use super::free_agents::EmergencySignedTerms;
use crate::Country;
use crate::PlayerFieldPositionGroup;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::market::{TransferListing, TransferListingType};
use crate::transfers::offer::TransferOffer;
use crate::transfers::pipeline::{
    PipelineProcessor, ShortlistCandidate, ShortlistCandidateStatus, TransferNeedPriority,
    TransferNeedReason, TransferRequest, TransferRequestSource, TransferRequestStatus,
    TransferShortlist,
};
use crate::transfers::squad_needs::EmergencyGroupSlot;
use chrono::NaiveDate;
use log::debug;

/// A depth shortfall the emergency planner refused to fill directly.
/// The caller turns each intent into a `DepthCover` pipeline request
/// via [`EmergencyDepthRequestPlanner::stage_requests`] so the need is
/// serviced through candidate filtering, staged negotiation, personal
/// terms, and medical like any other recruitment target.
pub(super) struct EmergencyDepthRequestIntent {
    pub club_id: u32,
    pub group: PlayerFieldPositionGroup,
}

/// Turns [`EmergencyDepthRequestIntent`]s into open `DepthCover`
/// transfer requests on the owning club's plan, tagged
/// `EmergencyFreeAgentDepth` so only the free-agent matcher services
/// them.
pub(super) struct EmergencyDepthRequestPlanner;

impl EmergencyDepthRequestPlanner {
    pub(super) fn stage_requests(country: &mut Country, intents: &[EmergencyDepthRequestIntent]) {
        for intent in intents {
            let Some(club) = country.clubs.iter_mut().find(|c| c.id == intent.club_id) else {
                continue;
            };
            // Same tier anchor every request in `evaluate_squads`
            // uses; an absent main team falls back to the first team
            // like the rest of the emergency pass.
            let rep_score = club
                .teams
                .main()
                .or_else(|| club.teams.teams.first())
                .map(|t| t.reputation.overall_score())
                .unwrap_or(0.0);
            let plan = &mut club.transfer_plan;
            // Dedup against any open request for the same group —
            // mirrors the weekly-evaluation dedup so the planner can
            // re-run every tick without flooding the plan. Requests in
            // Negotiating state count as open: the pursuit is live.
            let already_open = plan.transfer_requests.iter().any(|existing| {
                existing.position.position_group() == intent.group
                    && existing.status != TransferRequestStatus::Fulfilled
                    && existing.status != TransferRequestStatus::Abandoned
            });
            if already_open {
                continue;
            }
            let request = Self::build_request(intent.group, rep_score, plan.next_request_id());
            debug!(
                "Emergency depth → pipeline request: club {} needs {:?} (request {})",
                intent.club_id, intent.group, request.id
            );
            plan.transfer_requests.push(request);
        }
    }

    /// Build the `DepthCover` request for one group slot. Min / ideal
    /// ability read off the same tier-anchored baseline `evaluate_squads`
    /// uses for its own DepthCover needs, so the two entry points can't
    /// drift apart. Budget allocation is zero on purpose: the request is
    /// free-agent-only (encoded by the `EmergencyFreeAgentDepth` source)
    /// and never enters the paid scouting / shortlist / loan paths.
    fn build_request(
        group: PlayerFieldPositionGroup,
        rep_score: f32,
        request_id: u32,
    ) -> TransferRequest {
        let baseline = PipelineProcessor::tier_starter_ca_score(rep_score, group);
        let mut request = TransferRequest::new(
            request_id,
            EmergencyGroupSlot::representative_position(group),
            TransferNeedPriority::Optional,
            TransferNeedReason::DepthCover,
            baseline.saturating_sub(15),
            baseline.saturating_sub(5),
            0.0,
        );
        request.source = TransferRequestSource::EmergencyFreeAgentDepth;
        request
    }
}

/// One staged depth offer collected by the request-driven matcher in
/// `handle_free_agents`. Carries everything the stager needs to create
/// the negotiation once the mutable country borrow is available — the
/// matcher loop itself iterates `&country.clubs` and cannot touch the
/// market.
pub(super) struct DepthNegotiationAction {
    pub player_id: u32,
    pub player_name: String,
    /// 0 for global-pool free agents — doubles as the negotiation's
    /// `selling_club_id`, which marks the pool-completion path in
    /// `resolve_medical`.
    pub from_club_id: u32,
    pub from_club_name: String,
    pub to_club_id: u32,
    pub request_id: u32,
    pub terms: EmergencySignedTerms,
    pub selling_rep: f32,
    pub buying_rep: f32,
    pub buying_league_reputation: u16,
    pub negotiator_staff_id: Option<u32>,
    pub player_age: u8,
    pub player_ambition: f32,
    pub is_global_pool: bool,
    pub reason: String,
}

/// Creates the Pending negotiation for each staged depth offer and
/// wires the buying club's plan (shortlist candidate, request status,
/// active-negotiation count) so `PipelineProcessor::on_negotiation_resolved`
/// can mirror the outcome back onto the request like any pipeline
/// pursuit. The negotiation enters `PersonalTerms` directly — a free
/// agent has no selling club to haggle a fee with — and resolves over
/// the following days in `resolve_pending_negotiations`.
pub(super) struct FreeAgentNegotiationStager;

impl FreeAgentNegotiationStager {
    pub(super) fn stage(
        country: &mut Country,
        actions: Vec<DepthNegotiationAction>,
        date: NaiveDate,
        global_offered_ids: &mut Vec<u32>,
    ) {
        for action in actions {
            // A free agent is genuinely available — back the approach
            // with an EndOfContract listing (not a synthetic one) so
            // the resolver grants the availability bonus and the
            // completed transfer records as a free move.
            if country
                .transfer_market
                .get_listing_by_player(action.player_id)
                .is_none()
            {
                country.transfer_market.add_listing(TransferListing::new(
                    action.player_id,
                    action.from_club_id,
                    0,
                    CurrencyValue::new(0.0, Currency::Usd),
                    date,
                    TransferListingType::EndOfContract,
                ));
            }

            let offer = TransferOffer::new(
                CurrencyValue::new(0.0, Currency::Usd),
                action.to_club_id,
                date,
            )
            .with_personal_terms(action.terms.to_personal_terms());

            let Some(neg_id) = country.transfer_market.start_negotiation(
                action.player_id,
                action.to_club_id,
                offer,
                date,
                action.selling_rep,
                action.buying_rep,
                action.player_age,
                action.player_ambition,
            ) else {
                continue;
            };

            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                // No fee to negotiate — the player's decision is the
                // only open question, so skip straight to his phase.
                negotiation.advance_to_personal_terms(date);
                negotiation.offered_salary = Some(action.terms.annual_wage);
                // The staged wage was priced through the free-agent market
                // chain (role, decay, desperation) — it IS the reservation,
                // so the resolution-side wage check scores it at par
                // instead of skipping the money question entirely.
                negotiation.staged_reservation_wage = Some(action.terms.annual_wage);
                negotiation.negotiator_staff_id = action.negotiator_staff_id;
                negotiation.reason = action.reason.clone();
                negotiation.player_name = action.player_name.clone();
                negotiation.selling_club_name = action.from_club_name.clone();
                negotiation.buying_league_reputation = action.buying_league_reputation;
            }

            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.to_club_id) {
                let plan = &mut club.transfer_plan;
                let shortlist = match plan
                    .shortlists
                    .iter()
                    .position(|s| s.transfer_request_id == action.request_id)
                {
                    Some(idx) => &mut plan.shortlists[idx],
                    None => {
                        plan.shortlists
                            .push(TransferShortlist::new(action.request_id, 0.0));
                        plan.shortlists.last_mut().unwrap()
                    }
                };
                if let Some(pos) = shortlist
                    .candidates
                    .iter()
                    .position(|c| c.player_id == action.player_id)
                {
                    shortlist.candidates[pos].status = ShortlistCandidateStatus::CurrentlyPursuing;
                    shortlist.current_pursuit_index = pos;
                } else {
                    shortlist.candidates.push(ShortlistCandidate {
                        player_id: action.player_id,
                        score: 0.5,
                        estimated_fee: 0.0,
                        status: ShortlistCandidateStatus::CurrentlyPursuing,
                    });
                    shortlist.current_pursuit_index = shortlist.candidates.len() - 1;
                }
                if let Some(request) = plan
                    .transfer_requests
                    .iter_mut()
                    .find(|r| r.id == action.request_id)
                {
                    request.status = TransferRequestStatus::Negotiating;
                }
                plan.active_negotiation_count += 1;
            }

            // The player has now received a concrete negotiated offer
            // — bump the pool-side 30-day window counter. Rejection is
            // tracked later by `resolve_personal_terms` if he declines.
            if action.is_global_pool {
                global_offered_ids.push(action.player_id);
            }

            debug!(
                "Depth free-agent negotiation staged: club {} → player {} (request {}, wage {})",
                action.to_club_id, action.player_id, action.request_id, action.terms.annual_wage
            );
        }
    }
}
