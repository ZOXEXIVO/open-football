use chrono::NaiveDate;
use log::{debug, info};
use super::types::{TransferActivitySummary, can_club_accept_player};
use crate::country::result::CountryResult;
use crate::{Country, Person, PlayerFieldPositionGroup, PlayerStatusType};
use crate::transfers::negotiation::{NegotiationPhase, NegotiationStatus, TransferNegotiation};
use crate::transfers::pipeline::TransferRequest;
use crate::transfers::pipeline_processor::PipelineProcessor;
use crate::transfers::staff_resolver::StaffResolver;

impl CountryResult {
    /// Handle expiring contracts and free agent signings.
    /// Releases players with expired contracts and matches soon-to-expire players to clubs.
    pub(crate) fn handle_free_agents(country: &mut Country, date: NaiveDate, summary: &mut TransferActivitySummary) {
        struct FreeAgentCandidate {
            player_id: u32,
            player_name: String,
            club_id: u32,
            club_name: String,
            ability: u8,
            potential: u8,
            age: u8,
            position_group: PlayerFieldPositionGroup,
            days_to_expiry: i64,
        }

        // Pass 1: Find players with expiring contracts (< 180 days)
        // and release players whose contracts have already expired
        let mut candidates: Vec<FreeAgentCandidate> = Vec::new();
        let mut expired_player_ids: Vec<u32> = Vec::new();

        for club in &country.clubs {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    let contract = match &player.contract {
                        Some(c) => c,
                        None => continue,
                    };

                    // Skip loan players
                    if player.is_on_loan() {
                        continue;
                    }

                    let days_left = (contract.expiration - date).num_days();

                    // Contract already expired — release player
                    if days_left <= 0 {
                        expired_player_ids.push(player.id);
                        continue;
                    }

                    // Contract expiring within 6 months
                    if days_left <= 180 {
                        // Skip if already listed or in negotiation
                        let statuses = player.statuses.get();
                        if statuses.contains(&PlayerStatusType::Lst)
                            || statuses.contains(&PlayerStatusType::Trn)
                            || statuses.contains(&PlayerStatusType::Bid)
                        {
                            continue;
                        }

                        candidates.push(FreeAgentCandidate {
                            player_id: player.id,
                            player_name: player.full_name.to_string(),
                            club_id: club.id,
                            club_name: club.name.clone(),
                            ability: player.player_attributes.current_ability,
                            potential: player.player_attributes.potential_ability,
                            age: player.age(date),
                            position_group: player.position().position_group(),
                            days_to_expiry: days_left,
                        });
                    }
                }
            }
        }

        // Release players with expired contracts
        for player_id in expired_player_ids {
            for club in &mut country.clubs {
                for team in &mut club.teams.teams {
                    if let Some(player) = team.players.players.iter_mut().find(|p| p.id == player_id) {
                        debug!("Contract expired: player {} ({}) released from {}",
                              player.full_name, player_id, club.name);
                        player.contract = None;
                        break;
                    }
                }
            }
        }

        if candidates.is_empty() {
            return;
        }

        // Pass 2: Match candidates to clubs with needs
        struct FreeAgentSigning {
            player_id: u32,
            player_name: String,
            from_club_id: u32,
            from_club_name: String,
            to_club_id: u32,
            reason: String,
        }

        let mut signings: Vec<FreeAgentSigning> = Vec::new();
        let max_signings_per_day = 3;

        for club in &country.clubs {
            if signings.len() >= max_signings_per_day {
                break;
            }

            if club.teams.teams.is_empty() {
                continue;
            }

            // Skip clubs that have reached their squad cap
            if !can_club_accept_player(club) {
                continue;
            }

            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }
            
            // Check unfulfilled transfer requests
            let unfulfilled: Vec<&TransferRequest> = plan
                .transfer_requests
                .iter()
                .filter(|r| {
                    r.status != crate::transfers::pipeline::TransferRequestStatus::Fulfilled
                        && r.status != crate::transfers::pipeline::TransferRequestStatus::Abandoned
                })
                .collect();

            for request in &unfulfilled {
                if signings.len() >= max_signings_per_day {
                    break;
                }

                // Find a matching free agent candidate
                if let Some(best) = candidates
                    .iter()
                    .filter(|c| {
                        c.club_id != club.id
                            && c.position_group == request.position.position_group()
                            && c.ability >= request.min_ability.saturating_sub(5)
                            && c.age <= 33
                            && c.days_to_expiry <= 90
                            && !signings.iter().any(|s| s.player_id == c.player_id)
                    })
                    .max_by_key(|c| c.ability as u16 + c.potential as u16)
                {
                    let reason = PipelineProcessor::transfer_need_reason_text(&request.reason).to_string();

                    signings.push(FreeAgentSigning {
                        player_id: best.player_id,
                        player_name: best.player_name.clone(),
                        from_club_id: best.club_id,
                        from_club_name: best.club_name.clone(),
                        to_club_id: club.id,
                        reason,
                    });
                }
            }
        }

        // Pass 3: Execute signings as free transfers with negotiation records
        for signing in &signings {
            // Resolve negotiator staff from buying club
            let negotiator_staff_id = country.clubs.iter()
                .find(|c| c.id == signing.to_club_id)
                .and_then(|c| c.teams.teams.first())
                .and_then(|t| StaffResolver::resolve(&t.staffs).negotiator.map(|s| s.id));

            // Create a negotiation record (immediately accepted) to track staff involvement
            let neg_id = country.transfer_market.next_negotiation_id;
            country.transfer_market.next_negotiation_id += 1;

            let offer = crate::transfers::offer::TransferOffer::new(
                crate::shared::CurrencyValue::new(0.0, crate::shared::Currency::Usd),
                signing.to_club_id,
                date,
            );

            let mut negotiation = TransferNegotiation::new(
                neg_id,
                signing.player_id,
                0, // no listing index
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
            country.transfer_market.negotiations.insert(neg_id, negotiation);
        }

        for signing in signings {
            let to_club_name = country.clubs.iter()
                .find(|c| c.id == signing.to_club_id)
                .map(|c| c.name.clone())
                .unwrap_or_default();

            // Create transfer history record with reason
            country.transfer_market.transfer_history.push(
                crate::transfers::CompletedTransfer::new(
                    signing.player_id,
                    signing.player_name,
                    signing.from_club_id,
                    0,
                    signing.from_club_name,
                    signing.to_club_id,
                    to_club_name,
                    date,
                    crate::shared::CurrencyValue::new(0.0, crate::shared::Currency::Usd),
                    crate::transfers::TransferType::Free,
                ).with_reason(signing.reason),
            );

            super::execution::execute_player_transfer(
                country,
                signing.player_id,
                signing.from_club_id,
                signing.to_club_id,
                0.0, // Free transfer
                date,
            );
            summary.completed_transfers += 1;

            debug!(
                "Free agent signing: player {} from club {} to club {}",
                signing.player_id, signing.from_club_id, signing.to_club_id
            );
        }
    }
}
