use chrono::NaiveDate;
use super::types::{DeferredTransfer, NegotiationData, TransferActivitySummary, find_player_in_country};
use crate::country::result::CountryResult;
use crate::utils::FloatUtils;
use crate::{Country, PlayerSquadStatus, PlayerStatusType};
use crate::transfers::TransferListingStatus;
use crate::transfers::negotiation::{NegotiationPhase, NegotiationRejectionReason};
use crate::transfers::pipeline::PipelineProcessor;

impl CountryResult {
    pub(crate) fn resolve_pending_negotiations(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
    ) -> Vec<DeferredTransfer> {
        let mut deferred: Vec<DeferredTransfer> = Vec::new();
        let country_id = country.id;

        let ready_to_resolve: Vec<u32> = country
            .transfer_market
            .negotiations
            .values()
            .filter(|n| n.is_phase_ready(date))
            .map(|n| n.id)
            .collect();

        for neg_id in ready_to_resolve {
            let neg_data = match country.transfer_market.negotiations.get(&neg_id) {
                Some(n) => {
                    let asking_price = country.transfer_market.listings
                        .get(n.listing_id as usize)
                        .map(|l| l.asking_price.amount)
                        .unwrap_or(0.0);
                    let is_listed = country.transfer_market.listings
                        .get(n.listing_id as usize)
                        .map(|l| l.status == TransferListingStatus::InNegotiation
                            || l.status == TransferListingStatus::Available)
                        .unwrap_or(false);
                    NegotiationData {
                        player_id: n.player_id,
                        selling_club_id: n.selling_club_id,
                        buying_club_id: n.buying_club_id,
                        offer_amount: n.current_offer.base_fee.amount,
                        is_loan: n.is_loan,
                        is_unsolicited: n.is_unsolicited,
                        phase: n.phase.clone(),
                        selling_rep: n.selling_club_reputation,
                        buying_rep: n.buying_club_reputation,
                        player_age: n.player_age,
                        player_ambition: n.player_ambition,
                        asking_price,
                        is_listed,
                        selling_country_id: n.selling_country_id,
                        player_name: n.player_name.clone(),
                        selling_club_name: n.selling_club_name.clone(),
                    }
                }
                None => continue,
            };

            match neg_data.phase {
                NegotiationPhase::InitialApproach { .. } => {
                    Self::resolve_initial_approach(country, neg_id, &neg_data, date);
                }
                NegotiationPhase::ClubNegotiation { round, .. } => {
                    Self::resolve_club_negotiation(country, neg_id, &neg_data, round, date);
                }
                NegotiationPhase::PersonalTerms { .. } => {
                    Self::resolve_personal_terms(country, neg_id, &neg_data, date);
                }
                NegotiationPhase::MedicalAndFinalization { .. } => {
                    Self::resolve_medical(country, country_id, neg_id, &neg_data, date, summary, &mut deferred);
                }
            }
        }

        deferred
    }

    fn resolve_initial_approach(
        country: &mut Country,
        neg_id: u32,
        neg_data: &NegotiationData,
        date: NaiveDate,
    ) {

        let mut chance: f32 = if neg_data.is_listed {
            75.0
        } else if neg_data.is_unsolicited {
            45.0
        } else {
            55.0
        };

        if neg_data.asking_price > 0.0 {
            let ratio = neg_data.offer_amount / neg_data.asking_price;
            if ratio >= 1.0 {
                chance += 25.0;
            } else if ratio >= 0.8 {
                chance += 10.0;
            } else if ratio < 0.5 {
                chance -= 15.0;
            }
        }

        let rep_diff = neg_data.buying_rep - neg_data.selling_rep;
        if rep_diff > 0.2 {
            chance += 15.0;
        } else if rep_diff < -0.2 {
            chance -= 10.0;
        }

        chance = chance.clamp(5.0, 95.0);
        let roll = FloatUtils::random(0.0, 100.0);

        if roll < chance {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.advance_to_club_negotiation(date);
            }
        } else {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.reject_with_reason(NegotiationRejectionReason::SellerRefusedToNegotiate);
            }
            Self::reopen_listing_for_player(country, neg_data.player_id);
            PipelineProcessor::on_negotiation_resolved(
                country,
                neg_data.buying_club_id,
                neg_data.player_id,
                false,
            );
        }
    }

    fn resolve_club_negotiation(
        country: &mut Country,
        neg_id: u32,
        neg_data: &NegotiationData,
        round: u8,
        date: NaiveDate,
    ) {
        let mut chance: f32 = if neg_data.asking_price > 0.0 {
            let ratio = neg_data.offer_amount / neg_data.asking_price;
            if ratio >= 1.2 { 90.0 }
            else if ratio >= 1.0 { 75.0 }
            else if ratio >= 0.9 { 60.0 }
            else if ratio >= 0.8 { 50.0 }
            else if ratio >= 0.7 { 35.0 }
            else { 15.0 }
        } else {
            55.0
        };

        // For domestic transfers, check player importance
        if neg_data.selling_country_id.is_none() {
            let importance = Self::calculate_player_importance(
                country, neg_data.player_id, neg_data.selling_club_id,
            );
            chance -= importance * 20.0;
        }

        if let Some(selling_club) = country.clubs.iter().find(|c| c.id == neg_data.selling_club_id) {
            if selling_club.finance.balance.balance < 0 {
                chance += 15.0;
            }
        }

        let rep_diff = neg_data.buying_rep - neg_data.selling_rep;
        if rep_diff > 0.15 {
            chance += 10.0;
        }

        chance = chance.clamp(5.0, 95.0);
        let roll = FloatUtils::random(0.0, 100.0);

        if roll < chance {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.advance_to_personal_terms(date);
            }
        } else if round < 3 {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                let new_amount = crate::utils::FormattingUtils::round_fee(
                    negotiation.current_offer.base_fee.amount * 1.15
                );
                negotiation.current_offer.base_fee.amount = new_amount;
                negotiation.advance_club_negotiation_round(date);
            }
        } else {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.reject_with_reason(NegotiationRejectionReason::AskingPriceTooHigh);
            }
            Self::reopen_listing_for_player(country, neg_data.player_id);
            PipelineProcessor::on_negotiation_resolved(
                country,
                neg_data.buying_club_id,
                neg_data.player_id,
                false,
            );
        }
    }

    fn resolve_personal_terms(
        country: &mut Country,
        neg_id: u32,
        neg_data: &NegotiationData,
        date: NaiveDate,
    ) {

        let mut chance: f32 = 60.0;

        if neg_data.is_listed {
            chance += 10.0;
        }

        let rep_diff = neg_data.buying_rep - neg_data.selling_rep;
        if rep_diff > 0.3 {
            chance += 25.0;
        } else if rep_diff > 0.15 {
            chance += 15.0;
        } else if rep_diff < -0.3 {
            chance -= 20.0;
        } else if rep_diff < -0.15 {
            chance -= 10.0;
        }

        // For domestic, check player details; for foreign, use cached data
        if neg_data.selling_country_id.is_none() {
            if let Some(player) = find_player_in_country(country, neg_data.player_id) {
                let current_salary = player.contract.as_ref()
                    .map(|c| c.salary as f64)
                    .unwrap_or(500.0);
                let offered_salary = (neg_data.offer_amount / 200.0).max(500.0);
                let salary_ratio = offered_salary / current_salary;
                if salary_ratio >= 2.0 {
                    chance += 20.0;
                } else if salary_ratio >= 1.3 {
                    chance += 10.0;
                } else if salary_ratio < 0.8 {
                    chance -= 20.0;
                }

                let age = neg_data.player_age;
                if age < 23 {
                    if rep_diff > 0.4 {
                        chance -= 5.0;
                    }
                    if rep_diff > 0.1 {
                        chance += 5.0;
                    }
                } else if age <= 28 {
                    if rep_diff < -0.1 {
                        chance -= 10.0;
                    }
                } else {
                    if salary_ratio >= 1.5 {
                        chance += 10.0;
                    }
                    chance += 5.0;
                }

                let statuses = player.statuses.get();
                if statuses.contains(&PlayerStatusType::Req) {
                    chance += 25.0;
                } else if statuses.contains(&PlayerStatusType::Unh) {
                    chance += 20.0;
                }

                let ambition = neg_data.player_ambition;
                if ambition > 0.7 && rep_diff > 0.1 {
                    chance += 10.0;
                } else if ambition > 0.7 && rep_diff < -0.1 {
                    chance -= 10.0;
                }
            }
        } else {
            // Foreign player — use reputation diff as primary driver
            if rep_diff > 0.2 { chance += 15.0; }
        }

        chance = chance.clamp(5.0, 95.0);
        let roll = FloatUtils::random(0.0, 100.0);

        if roll < chance {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.advance_to_medical(date);
            }
        } else {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.reject_with_reason(NegotiationRejectionReason::PlayerRejectedPersonalTerms);
            }
            Self::reopen_listing_for_player(country, neg_data.player_id);
            PipelineProcessor::on_negotiation_resolved(
                country,
                neg_data.buying_club_id,
                neg_data.player_id,
                false,
            );
        }
    }

    fn resolve_medical(
        country: &mut Country,
        country_id: u32,
        neg_id: u32,
        neg_data: &NegotiationData,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
        deferred: &mut Vec<DeferredTransfer>,
    ) {
        let is_foreign = neg_data.selling_country_id.is_some();

        // For domestic: verify the player is still at the selling club
        if !is_foreign {
            let player_at_selling_club = country.clubs.iter()
                .find(|c| c.id == neg_data.selling_club_id)
                .map(|c| c.teams.teams.iter().any(|t|
                    t.players.players.iter().any(|p| p.id == neg_data.player_id)
                ))
                .unwrap_or(false);

            if !player_at_selling_club {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.reject_with_reason(NegotiationRejectionReason::SellerRefusedToNegotiate);
                }
                Self::reopen_listing_for_player(country, neg_data.player_id);
                PipelineProcessor::on_negotiation_resolved(country, neg_data.buying_club_id, neg_data.player_id, false);
                return;
            }
        }

        let is_injured = if is_foreign {
            false
        } else {
            find_player_in_country(country, neg_data.player_id)
                .map(|p| p.player_attributes.is_injured)
                .unwrap_or(false)
        };
        let fail_chance = if is_injured { 15.0 } else { 5.0 };
        let roll = FloatUtils::random(0.0, 100.0);

        if roll >= fail_chance {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.accept();
            }

            // Resolve names: domestic from country, foreign from cached names
            let player_name = if is_foreign {
                neg_data.player_name.clone()
            } else {
                find_player_in_country(country, neg_data.player_id)
                    .map(|p| p.full_name.to_string())
                    .unwrap_or_default()
            };
            let from_team_name = if is_foreign {
                neg_data.selling_club_name.clone()
            } else {
                country.clubs.iter()
                    .find(|c| c.id == neg_data.selling_club_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_default()
            };
            let to_team_name = country.clubs.iter()
                .find(|c| c.id == neg_data.buying_club_id)
                .map(|c| c.name.clone())
                .unwrap_or_default();

            if let Some(completed) = country.transfer_market.complete_transfer(
                neg_id, date, player_name, from_team_name, to_team_name,
            ) {
                summary.completed_transfers += 1;
                summary.total_fees_exchanged += completed.fee.amount;

                // All execution is deferred to SimulatorData level
                let selling_country_id = neg_data.selling_country_id.unwrap_or(country_id);
                deferred.push(DeferredTransfer {
                    player_id: neg_data.player_id,
                    selling_country_id,
                    selling_club_id: neg_data.selling_club_id,
                    buying_country_id: country_id,
                    buying_club_id: neg_data.buying_club_id,
                    fee: neg_data.offer_amount,
                    is_loan: neg_data.is_loan,
                });

                PipelineProcessor::on_negotiation_resolved(country, neg_data.buying_club_id, neg_data.player_id, true);
                PipelineProcessor::clear_player_interest(country, neg_data.player_id);
            }
        } else {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.reject_with_reason(NegotiationRejectionReason::MedicalFailed);
            }
            Self::reopen_listing_for_player(country, neg_data.player_id);
            PipelineProcessor::on_negotiation_resolved(country, neg_data.buying_club_id, neg_data.player_id, false);
        }
    }

    pub(crate) fn calculate_player_importance(country: &Country, player_id: u32, club_id: u32) -> f32 {
        if let Some(club) = country.clubs.iter().find(|c| c.id == club_id) {
            if club.teams.teams.is_empty() {
                return 0.5;
            }
            let team = &club.teams.teams[0];
            let players = &team.players.players;
            if players.is_empty() {
                return 0.5;
            }

            let avg_ability: f32 = players.iter()
                .map(|p| p.player_attributes.current_ability as f32)
                .sum::<f32>() / players.len() as f32;

            if let Some(player) = players.iter().find(|p| p.id == player_id) {
                let ability = player.player_attributes.current_ability as f32;
                let ratio = (ability - avg_ability) / avg_ability.max(1.0);
                let status_bonus = match &player.contract {
                    Some(c) => match c.squad_status {
                        PlayerSquadStatus::KeyPlayer => 0.3,
                        PlayerSquadStatus::FirstTeamRegular => 0.15,
                        _ => 0.0,
                    },
                    None => 0.0,
                };
                (ratio * 0.5 + 0.5 + status_bonus).clamp(0.0, 1.0)
            } else {
                0.5
            }
        } else {
            0.5
        }
    }

    pub(crate) fn reopen_listing_for_player(country: &mut Country, player_id: u32) {
        for listing in &mut country.transfer_market.listings {
            if listing.player_id == player_id
                && listing.status == TransferListingStatus::InNegotiation
            {
                listing.status = TransferListingStatus::Available;
                break;
            }
        }
    }
}
