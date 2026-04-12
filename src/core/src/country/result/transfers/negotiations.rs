use chrono::NaiveDate;
use super::types::{DeferredTransfer, NegotiationData, TransferActivitySummary, find_player_in_country};
use crate::country::result::CountryResult;
use crate::transfers::negotiation::{NegotiationPhase, NegotiationRejectionReason};
use crate::transfers::pipeline::PipelineProcessor;
use crate::transfers::scouting_region::ScoutingRegion;
use crate::transfers::TransferListingStatus;
use crate::utils::{FloatUtils, FormattingUtils};
use crate::transfers::TransferWindowManager;
use crate::{Country, PlayerSquadStatus, PlayerStatusType};

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
                        selling_continent_id: n.selling_continent_id,
                        selling_country_code: n.selling_country_code.clone(),
                        player_sold_from: n.player_sold_from.clone(),
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
        // Selling club refuses to negotiate for recently signed players —
        // they bought this player with a plan and won't sell immediately.
        // Check domestic players only (foreign players aren't in this country).
        if neg_data.selling_country_id.is_none() {
            let window_mgr = TransferWindowManager::new();
            let current_window = window_mgr.current_window_dates(country.id, date);
            if let Some(player) = find_player_in_country(country, neg_data.player_id) {
                if player.is_transfer_protected(date, current_window) {
                    if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                        negotiation.reject_with_reason(NegotiationRejectionReason::PlayerTooImportant);
                    }
                    Self::reopen_listing_for_player(country, neg_data.player_id);
                    PipelineProcessor::on_negotiation_resolved(
                        country,
                        neg_data.buying_club_id,
                        neg_data.player_id,
                        false,
                    );
                    return;
                }
            }
        }

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
                let new_amount = FormattingUtils::round_fee(
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

        let is_foreign = neg_data.selling_country_id.is_some();

        // Foreign loans start lower: unfamiliar country, language, likely
        // wage cut. Players don't jump across borders for a bit-part role
        // as readily as they change clubs within their own league.
        let mut chance: f32 = if is_foreign && neg_data.is_loan {
            45.0
        } else if is_foreign {
            50.0
        } else {
            60.0
        };

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

        // Age + reputation interaction: how players at different career stages
        // evaluate upward vs downward moves
        let age = neg_data.player_age;
        let ambition = neg_data.player_ambition;

        if age < 23 {
            // Young players dream big — they want to develop at the highest level.
            // Resist downward moves strongly; welcome upward moves for development.
            if rep_diff > 0.1 {
                chance += 5.0; // Happy to move up
            } else if rep_diff < -0.3 {
                chance -= 20.0; // "I'm not throwing away my career"
            } else if rep_diff < -0.1 {
                chance -= 12.0; // Reluctant to step down
            }
        } else if age <= 28 {
            // Prime years — players want the best competition and exposure.
            // Very resistant to downward moves; moving up is welcome.
            if rep_diff > 0.15 {
                chance += 5.0;
            } else if rep_diff < -0.3 {
                chance -= 15.0; // Strong resistance in peak years
            } else if rep_diff < -0.1 {
                chance -= 10.0;
            }
        } else {
            // Veteran players — pragmatic, value playing time and money.
            // Accept downward moves more easily, especially for salary.
            chance += 5.0;
        }

        // Ambition: ambitious players dream of top clubs and resist stepping down.
        // Low-ambition players are more content wherever they are.
        if ambition > 0.7 {
            if rep_diff > 0.1 {
                chance += 10.0; // Ambitious + moving up = eager
            } else if rep_diff < -0.1 {
                // Scale penalty with the gap: bigger drop = stronger refusal
                let penalty = if rep_diff < -0.3 { 20.0 }
                    else { 12.0 };
                chance -= penalty;
            }
        } else if ambition < 0.4 {
            // Low ambition: less bothered by prestige, more accepting
            if rep_diff < -0.1 {
                chance += 5.0;
            }
        }

        // For domestic, check salary and player-specific details
        if neg_data.selling_country_id.is_none() {
            if let Some(player) = find_player_in_country(country, neg_data.player_id) {
                let current_salary = player.contract.as_ref()
                    .map(|c| c.salary as f64)
                    .unwrap_or(500.0);
                let offered_salary = (neg_data.offer_amount / 200.0).max(500.0);
                let salary_ratio = offered_salary / current_salary;

                // Salary influence: money talks, but can't override everything.
                // For downward moves, salary can soften the blow.
                // For veterans, salary is a bigger motivator.
                if salary_ratio >= 2.0 {
                    if age >= 29 {
                        chance += 20.0; // Veterans: big payday is very tempting
                    } else {
                        chance += 12.0; // Younger: money helps but doesn't override ambition
                    }
                } else if salary_ratio >= 1.3 {
                    if age >= 29 {
                        chance += 12.0;
                    } else {
                        chance += 5.0;
                    }
                } else if salary_ratio < 0.8 {
                    chance -= 20.0; // Pay cut on top of prestige drop = very unattractive
                }

                let statuses = player.statuses.get();
                if statuses.contains(&PlayerStatusType::Req) {
                    chance += 25.0; // Wants out — will accept more
                } else if statuses.contains(&PlayerStatusType::Unh) {
                    chance += 20.0; // Unhappy — willing to move
                }
            }
        } else {
            // Foreign player — same age/ambition checks already applied above.
            // Add salary estimate since we can't access the actual contract.
            if rep_diff > 0.2 {
                chance += 10.0;
            }
        }

        // Player reluctance to return to a club that sold them.
        // The feeling of rejection is strong — but context matters:
        // - Sold cheaply → player felt undervalued → strong resentment
        // - Club is much bigger → prestige pull can overcome hurt pride
        // - Ambitious player → may want to prove themselves, reduces penalty
        // - Older player → more pragmatic, sentimental about returning "home"
        // - Player was unhappy/requested transfer → less resentment (they wanted out)
        if let Some((sold_club_id, sold_fee)) = &neg_data.player_sold_from {
            if *sold_club_id == neg_data.buying_club_id {
                // Base: player doesn't want to go back to club that rejected them
                let mut return_penalty: f32 = 25.0;

                // Sold cheaply relative to current offer → felt undervalued
                if *sold_fee > 0.0 && neg_data.offer_amount > sold_fee * 3.0 {
                    return_penalty += 10.0;
                }

                // Club is much bigger → prestige can overcome pride
                if rep_diff > 0.3 {
                    return_penalty -= 15.0;
                } else if rep_diff > 0.15 {
                    return_penalty -= 8.0;
                }

                // Ambitious players want to prove themselves at big clubs
                if neg_data.player_ambition > 0.7 && rep_diff > 0.1 {
                    return_penalty -= 8.0;
                }

                // Older players are more pragmatic
                if neg_data.player_age >= 30 {
                    return_penalty -= 5.0;
                }

                chance -= return_penalty.max(5.0);
            }
        }

        // Geographic preference: players resist moves to less prestigious regions
        if let Some(sell_continent_id) = neg_data.selling_continent_id {
            let buying_region = ScoutingRegion::from_country(country.continent_id, &country.code);
            let selling_region = ScoutingRegion::from_country(sell_continent_id, &neg_data.selling_country_code);

            if buying_region != selling_region {
                let buy_prestige = buying_region.league_prestige();
                let sell_prestige = selling_region.league_prestige();
                let prestige_drop = sell_prestige - buy_prestige;

                if prestige_drop > 0.0 {
                    // Moving to less prestigious region — players resist this.
                    // Previously 60× was too soft: a 0.55 drop (W.Europe→S.America)
                    // only cost −33, leaving ~27% acceptance for prime-age players.
                    let base_penalty = prestige_drop * 110.0;

                    // Ambitious players resist prestige drops more
                    let ambition_factor = if neg_data.player_ambition > 0.7 { 1.5 }
                        else if neg_data.player_ambition > 0.5 { 1.0 }
                        else { 0.7 };

                    // Veterans (30+) accept drops more easily for money/playing time,
                    // but a very large drop still stings regardless of age.
                    let age_factor = if prestige_drop > 0.4 {
                        if neg_data.player_age >= 32 { 0.7 }
                        else if neg_data.player_age >= 30 { 0.85 }
                        else { 1.0 }
                    } else if neg_data.player_age >= 32 { 0.3 }
                        else if neg_data.player_age >= 30 { 0.5 }
                        else { 1.0 };

                    chance -= base_penalty * ambition_factor * age_factor;
                } else if prestige_drop < -0.1 {
                    // Moving to more prestigious region — bonus
                    chance += (-prestige_drop) * 20.0;
                }
            }
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

        // Verify the player is still at the selling club (domestic) or not
        // already claimed by another deferred transfer (foreign)
        if is_foreign {
            // Reject if another negotiation for this player is already deferred
            if deferred.iter().any(|d| d.player_id == neg_data.player_id) {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.reject_with_reason(NegotiationRejectionReason::SellerRefusedToNegotiate);
                }
                PipelineProcessor::on_negotiation_resolved(country, neg_data.buying_club_id, neg_data.player_id, false);
                return;
            }
        } else {
            let player_at_selling_club = country.clubs.iter()
                .find(|c| c.id == neg_data.selling_club_id)
                .map(|c| c.teams.contains_player(neg_data.player_id))
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
