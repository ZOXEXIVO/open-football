use chrono::NaiveDate;
use crate::utils::IntegerUtils;
use log::{debug, info};
use super::types::{TransferActivitySummary, can_club_accept_player};
use crate::country::result::CountryResult;
use crate::{Country, Person, PlayerFieldPositionGroup, PlayerStatusType};
use crate::transfers::negotiation::{NegotiationPhase, NegotiationStatus, TransferNegotiation};
use crate::transfers::pipeline::TransferRequest;
use crate::transfers::pipeline::PipelineProcessor;
use crate::transfers::staff_resolver::StaffResolver;

impl CountryResult {
    /// Handle expiring contracts and free agent signings.
    ///
    /// Signing probability depends on player quality:
    ///   - Elite players (ability 140+): ~25% daily chance → signed within days
    ///   - Good players (100-140):       ~5-10% daily → signed within weeks
    ///   - Average players (60-100):     ~1-3% daily  → may take months
    ///   - Low quality (<60):            ~0.2-0.5%    → can sit 1-2 seasons
    ///
    /// This creates realistic free agent markets where low-quality players
    /// linger while stars get snapped up immediately.
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

        // Pass 1: Find players with expiring contracts (< 90 days) or already expired
        let mut candidates: Vec<FreeAgentCandidate> = Vec::new();
        let mut expired_player_ids: Vec<u32> = Vec::new();

        for club in &country.clubs {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    // Also consider players with no contract (already released)
                    let days_left = match &player.contract {
                        Some(c) => {
                            // Skip loan players
                            if player.is_on_loan() {
                                continue;
                            }
                            (c.expiration - date).num_days()
                        }
                        None => 0, // already a free agent
                    };

                    // Contract already expired — release player
                    if days_left <= 0 && player.contract.is_some() {
                        expired_player_ids.push(player.id);
                        // Still add as candidate (will be available after release below)
                    }

                    // Available for free agent signing: expired, no contract, or expiring within 90 days
                    if days_left <= 90 {
                        // Skip if already in negotiation
                        let statuses = player.statuses.get();
                        if statuses.contains(&PlayerStatusType::Trn)
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

        // Pass 2: Match candidates to clubs with needs, using probability-based signing
        struct FreeAgentSigning {
            player_id: u32,
            player_name: String,
            from_club_id: u32,
            from_club_name: String,
            to_club_id: u32,
            reason: String,
        }

        let mut signings: Vec<FreeAgentSigning> = Vec::new();
        let max_signings_per_day = 2;

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
                            && !signings.iter().any(|s| s.player_id == c.player_id)
                    })
                    .max_by_key(|c| c.ability as u16 + c.potential as u16)
                {
                    // Probability-based signing: better players get signed faster
                    // Daily chance based on ability and age:
                    //   ability 160+ → 25% daily (elite, signed in days)
                    //   ability 130  → 10% daily (signed in ~2 weeks)
                    //   ability 100  → 3% daily  (signed in ~1 month)
                    //   ability 70   → 0.8% daily (signed in ~4 months)
                    //   ability 40   → 0.2% daily (may take 1-2 seasons)
                    let ability_f = best.ability as f32;
                    let base_chance = if ability_f >= 160.0 {
                        25.0
                    } else if ability_f >= 130.0 {
                        5.0 + (ability_f - 130.0) / 30.0 * 20.0
                    } else if ability_f >= 100.0 {
                        1.5 + (ability_f - 100.0) / 30.0 * 3.5
                    } else if ability_f >= 60.0 {
                        0.3 + (ability_f - 60.0) / 40.0 * 1.2
                    } else {
                        0.1 + (ability_f / 60.0) * 0.2
                    };

                    // Age penalty: older players are harder to place
                    let age_factor = match best.age {
                        0..=29 => 1.0,
                        30..=31 => 0.8,
                        32..=33 => 0.5,
                        34..=35 => 0.3,
                        _ => 0.15,
                    };

                    // Young players with high potential get a boost
                    let potential_boost = if best.age < 24 && best.potential > best.ability + 20 {
                        1.5
                    } else {
                        1.0
                    };

                    let daily_chance = (base_chance * age_factor * potential_boost).clamp(0.1, 30.0);

                    // Roll the dice
                    let roll = IntegerUtils::random(1, 1000) as f32 / 10.0; // 0.1 to 100.0
                    if roll > daily_chance {
                        continue; // Not today — player stays on the market
                    }

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
            let negotiator_staff_id = country.clubs.iter()
                .find(|c| c.id == signing.to_club_id)
                .and_then(|c| c.teams.teams.first())
                .and_then(|t| StaffResolver::resolve(&t.staffs).negotiator.map(|s| s.id));

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
            country.transfer_market.negotiations.insert(neg_id, negotiation);
        }

        for signing in signings {
            let to_club_name = country.clubs.iter()
                .find(|c| c.id == signing.to_club_id)
                .map(|c| c.name.clone())
                .unwrap_or_default();

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

            super::execution::execute_transfer_within_country(
                country,
                signing.player_id,
                signing.from_club_id,
                signing.to_club_id,
                0.0,
                date,
            );
            PipelineProcessor::clear_player_interest(country, signing.player_id);
            summary.completed_transfers += 1;

            debug!(
                "Free agent signing: player {} from club {} to club {}",
                signing.player_id, signing.from_club_id, signing.to_club_id
            );
        }
    }
}
