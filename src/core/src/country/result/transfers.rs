use chrono::NaiveDate;
use log::{debug, info};
use std::collections::HashMap;
use super::CountryResult;
use crate::league::Season;
use crate::simulator::SimulatorData;
use crate::{
    Club, Country, Person, PlayerClubContract,
    PlayerFieldPositionGroup, PlayerPositionType, PlayerSquadStatus,
    PlayerStatistics, PlayerStatisticsHistoryItem, PlayerStatusType,
};
use crate::shared::CurrencyValue;
use crate::transfers::{TransferListing, TransferListingType, TransferListingStatus, TransferWindowManager};
use crate::transfers::negotiation::{NegotiationPhase, NegotiationRejectionReason};
use crate::transfers::pipeline::TransferRequest;
use crate::transfers::pipeline_processor::PipelineProcessor;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(super) struct TransferActivitySummary {
    pub(super) total_listings: u32,
    pub(super) active_negotiations: u32,
    pub(super) completed_transfers: u32,
    pub(super) total_fees_exchanged: f64,
}

impl TransferActivitySummary {
    pub(super) fn new() -> Self {
        TransferActivitySummary {
            total_listings: 0,
            active_negotiations: 0,
            completed_transfers: 0,
            total_fees_exchanged: 0.0,
        }
    }

    #[allow(dead_code)]
    fn get_market_heat_index(&self) -> f32 {
        let activity = (self.active_negotiations as f32 + self.completed_transfers as f32) / 100.0;
        activity.min(1.0)
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct SquadAnalysis {
    surplus_positions: Vec<PlayerPositionType>,
    needed_positions: Vec<PlayerPositionType>,
    average_age: f32,
    quality_level: u8,
}

/// Internal data extracted from a negotiation for phase resolution
struct NegotiationData {
    player_id: u32,
    selling_club_id: u32,
    buying_club_id: u32,
    offer_amount: f64,
    is_loan: bool,
    is_unsolicited: bool,
    phase: NegotiationPhase,
    selling_rep: f32,
    buying_rep: f32,
    player_age: u8,
    player_ambition: f32,
    asking_price: f64,
    is_listed: bool,
}

impl CountryResult {
    // ============================================================
    // Pipeline-driven transfer market simulation
    // ============================================================

    pub(super) fn simulate_transfer_market(
        data: &mut SimulatorData,
        country_id: u32,
        current_date: NaiveDate,
    ) -> TransferActivitySummary {
        let mut summary = TransferActivitySummary::new();

        // Check if transfer window is open
        let window_manager = TransferWindowManager::new();
        if !window_manager.is_window_open(country_id, current_date) {
            return summary;
        }

        debug!("Transfer window is OPEN - simulating pipeline-driven market activity");

        if let Some(country) = data.country_mut(country_id) {
            // Step 1: Resolve pending negotiations from previous days [EXISTING - kept]
            Self::resolve_pending_negotiations(country, current_date, &mut summary);

            // Step 2: List players for transfer (must run before shortlists so market has candidates)
            Self::list_players_from_pipeline(country, current_date, &mut summary);

            // Step 3: Evaluate squads (periodic - not daily)
            PipelineProcessor::evaluate_squads(country, current_date);

            // Step 3.5: Staff proactively recommend players (weekly)
            PipelineProcessor::generate_staff_recommendations(country, current_date);

            // Step 3.75: Process staff recommendations into pipeline actions (weekly)
            PipelineProcessor::process_staff_recommendations(country, current_date);

            // Step 4: Assign scouts to pending requests
            PipelineProcessor::assign_scouts(country, current_date);

            // Step 4.5: Assign scouts to youth/reserve team matches
            PipelineProcessor::assign_scouts_to_matches(country, current_date);

            // Step 4.75: Process match-day scouting observations
            PipelineProcessor::process_match_scouting(country, current_date);

            // Step 5: Process scouting observations
            PipelineProcessor::process_scouting(country, current_date);

            // Step 6: Build shortlists from scouting + market listings
            PipelineProcessor::build_shortlists(country, current_date);

            // Step 7: Initiate negotiations from shortlists
            PipelineProcessor::initiate_negotiations(country, current_date);

            // Step 7.5: Small clubs proactively scan the loan market
            PipelineProcessor::scan_loan_market(country, current_date);

            // Step 8: Free agents and contract expirations [EXISTING - kept]
            Self::handle_free_agents(country, current_date, &mut summary);

            // Step 9: Expire stale negotiations [EXISTING - kept]
            country.transfer_market.update(current_date);

            debug!(
                "Transfer Activity - Listings: {}, Negotiations: {}, Completed: {}",
                summary.total_listings, summary.active_negotiations, summary.completed_transfers
            );
        }

        summary
    }

    /// List players for transfer based on pipeline decisions and staff evaluations.
    fn list_players_from_pipeline(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
    ) {
        let mut listings_to_add = Vec::new();
        let price_level = country.settings.pricing.price_level;

        for club in &country.clubs {
            let squad_analysis = Self::analyze_squad_needs(club, date);

            if club.teams.teams.is_empty() {
                continue;
            }

            for player in &club.teams.teams[0].players.players {
                // Use existing should_list_player logic for non-pipeline listings
                if Self::should_list_player(player, &squad_analysis, club, date) {
                    let age = player.age(date);

                    if age < 16 {
                        // Under-16: free transfer only, no transfer fee
                        let free_price = CurrencyValue { amount: 0.0, currency: crate::shared::Currency::Usd };
                        listings_to_add.push((
                            player.id,
                            club.id,
                            club.teams.teams[0].id,
                            free_price,
                            TransferListingType::EndOfContract,
                        ));
                    } else {
                        let asking_price = Self::calculate_asking_price(player, club, date, price_level);
                        listings_to_add.push((
                            player.id,
                            club.id,
                            club.teams.teams[0].id,
                            asking_price,
                            TransferListingType::Transfer,
                        ));
                    }
                }
            }
        }

        if !listings_to_add.is_empty() {
            debug!("Transfer market: listing {} players for transfer/loan", listings_to_add.len());
        }

        // Apply listings
        for (player_id, club_id, team_id, asking_price, listing_type) in listings_to_add {
            let status_type = match listing_type {
                TransferListingType::Loan => PlayerStatusType::Loa,
                TransferListingType::EndOfContract => PlayerStatusType::Frt,
                _ => PlayerStatusType::Lst,
            };

            let listing = TransferListing::new(
                player_id,
                club_id,
                team_id,
                asking_price,
                date,
                listing_type,
            );

            country.transfer_market.add_listing(listing);
            summary.total_listings += 1;

            for club in &mut country.clubs {
                for team in &mut club.teams.teams {
                    if let Some(player) = team.players.players.iter_mut().find(|p| p.id == player_id) {
                        if !player.statuses.get().contains(&status_type) {
                            player.statuses.add(date, status_type);
                        }
                    }
                }
            }
        }
    }

    // ============================================================
    // Resolve Pending Negotiations
    // ============================================================

    fn resolve_pending_negotiations(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
    ) {
        use crate::utils::FloatUtils;

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
                    }
                }
                None => continue,
            };

            match neg_data.phase {
                // === Phase 1: Initial Approach ===
                NegotiationPhase::InitialApproach { .. } => {
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

                // === Phase 2: Club Negotiation ===
                NegotiationPhase::ClubNegotiation { round, .. } => {
                    let mut chance: f32 = if neg_data.asking_price > 0.0 {
                        let ratio = neg_data.offer_amount / neg_data.asking_price;
                        if ratio >= 1.2 { 80.0 }
                        else if ratio >= 1.0 { 65.0 }
                        else if ratio >= 0.9 { 50.0 }
                        else if ratio >= 0.8 { 35.0 }
                        else if ratio >= 0.7 { 20.0 }
                        else { 10.0 }
                    } else {
                        50.0
                    };

                    let importance = Self::calculate_player_importance(
                        country, neg_data.player_id, neg_data.selling_club_id,
                    );
                    chance -= importance * 30.0;

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
                                negotiation.current_offer.base_fee.amount * 1.10
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

                // === Phase 3: Personal Terms ===
                NegotiationPhase::PersonalTerms { .. } => {
                    let mut chance: f32 = 50.0;

                    let rep_diff = neg_data.buying_rep - neg_data.selling_rep;
                    if rep_diff > 0.3 {
                        chance += 30.0;
                    } else if rep_diff > 0.15 {
                        chance += 15.0;
                    } else if rep_diff < -0.3 {
                        chance -= 30.0;
                    } else if rep_diff < -0.15 {
                        chance -= 15.0;
                    }

                    if let Some(player) = Self::find_player_in_country(country, neg_data.player_id) {
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

                // === Phase 4: Medical & Finalization ===
                NegotiationPhase::MedicalAndFinalization { .. } => {
                    // Verify the player is still at the selling club before finalizing.
                    // A player may have already moved (e.g., loaned elsewhere) since
                    // negotiations started — abort if they're no longer available.
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
                        PipelineProcessor::on_negotiation_resolved(
                            country,
                            neg_data.buying_club_id,
                            neg_data.player_id,
                            false,
                        );
                        continue;
                    }

                    let is_injured = Self::find_player_in_country(country, neg_data.player_id)
                        .map(|p| p.player_attributes.is_injured)
                        .unwrap_or(false);
                    let fail_chance = if is_injured { 15.0 } else { 5.0 };
                    let roll = FloatUtils::random(0.0, 100.0);

                    if roll >= fail_chance {
                        if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                            negotiation.accept();
                        }

                        let player_name = Self::find_player_in_country(country, neg_data.player_id)
                            .map(|p| p.full_name.to_string())
                            .unwrap_or_default();
                        let from_team_name = country.clubs.iter()
                            .find(|c| c.id == neg_data.selling_club_id)
                            .map(|c| c.name.clone())
                            .unwrap_or_default();
                        let to_team_name = country.clubs.iter()
                            .find(|c| c.id == neg_data.buying_club_id)
                            .map(|c| c.name.clone())
                            .unwrap_or_default();

                        if let Some(completed) = country.transfer_market.complete_transfer(
                            neg_id,
                            date,
                            player_name,
                            from_team_name,
                            to_team_name,
                        ) {
                            summary.completed_transfers += 1;
                            summary.total_fees_exchanged += completed.fee.amount;

                            if neg_data.is_loan {
                                Self::execute_loan_transfer(
                                    country,
                                    neg_data.player_id,
                                    neg_data.selling_club_id,
                                    neg_data.buying_club_id,
                                    neg_data.offer_amount,
                                    date,
                                );
                            } else {
                                Self::execute_player_transfer(
                                    country,
                                    neg_data.player_id,
                                    neg_data.selling_club_id,
                                    neg_data.buying_club_id,
                                    neg_data.offer_amount,
                                    date,
                                );
                            }

                            PipelineProcessor::on_negotiation_resolved(
                                country,
                                neg_data.buying_club_id,
                                neg_data.player_id,
                                true,
                            );
                        }
                    } else {
                        if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                            negotiation.reject_with_reason(NegotiationRejectionReason::MedicalFailed);
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
            }
        }
    }

    // ============================================================
    // Transfer helpers
    // ============================================================

    fn analyze_squad_needs(club: &Club, current_date: NaiveDate) -> SquadAnalysis {
        if club.teams.teams.is_empty() {
            return SquadAnalysis {
                surplus_positions: vec![],
                needed_positions: vec![],
                average_age: 25.0,
                quality_level: 50,
            };
        }

        let team = &club.teams.teams[0];
        let players = &team.players.players;

        if players.is_empty() {
            return SquadAnalysis {
                surplus_positions: vec![],
                needed_positions: vec![],
                average_age: 25.0,
                quality_level: 50,
            };
        }

        let mut group_counts: HashMap<PlayerFieldPositionGroup, u32> = HashMap::new();
        let mut total_ability: u32 = 0;
        let mut total_age: u32 = 0;
        for player in players {
            let group = player.position().position_group();
            *group_counts.entry(group).or_insert(0) += 1;
            total_ability += player.player_attributes.current_ability as u32;
            total_age += player.age(current_date) as u32;
        }

        let avg_ability = (total_ability / players.len() as u32) as u8;
        let avg_age = total_age as f32 / players.len() as f32;

        let gk = *group_counts.get(&PlayerFieldPositionGroup::Goalkeeper).unwrap_or(&0);
        let def = *group_counts.get(&PlayerFieldPositionGroup::Defender).unwrap_or(&0);
        let mid = *group_counts.get(&PlayerFieldPositionGroup::Midfielder).unwrap_or(&0);
        let fwd = *group_counts.get(&PlayerFieldPositionGroup::Forward).unwrap_or(&0);

        let mut surplus = Vec::new();
        let mut needed = Vec::new();

        if gk > 2 { surplus.push(PlayerPositionType::Goalkeeper); }
        if gk < 2 { needed.push(PlayerPositionType::Goalkeeper); }
        if def > 7 { surplus.push(PlayerPositionType::DefenderCenter); }
        if def < 4 { needed.push(PlayerPositionType::DefenderCenter); }
        if mid > 7 { surplus.push(PlayerPositionType::MidfielderCenter); }
        if mid < 4 { needed.push(PlayerPositionType::MidfielderCenter); }
        if fwd > 5 { surplus.push(PlayerPositionType::Striker); }
        if fwd < 2 { needed.push(PlayerPositionType::Striker); }

        SquadAnalysis {
            surplus_positions: surplus,
            needed_positions: needed,
            average_age: avg_age,
            quality_level: avg_ability,
        }
    }

    fn should_list_player(
        player: &crate::Player,
        analysis: &SquadAnalysis,
        club: &Club,
        date: NaiveDate,
    ) -> bool {
        // Loan players belong to another club — cannot be listed by the loan club
        if let Some(ref contract) = player.contract {
            if contract.contract_type == crate::ContractType::Loan {
                return false;
            }
        }

        let statuses = player.statuses.get();

        // Already listed
        if statuses.contains(&PlayerStatusType::Lst) || statuses.contains(&PlayerStatusType::Loa) || statuses.contains(&PlayerStatusType::Frt) {
            return false;
        }

        if let Some(ref contract) = player.contract {
            if matches!(contract.squad_status, PlayerSquadStatus::NotNeeded) {
                return true;
            }
            if contract.is_transfer_listed {
                return true;
            }
        }

        if statuses.contains(&PlayerStatusType::Req) {
            return true;
        }

        if statuses.contains(&PlayerStatusType::Unh) {
            return true;
        }

        // Well below squad average — club would accept offers
        if analysis.quality_level > 15 &&
            (player.player_attributes.current_ability as i16) < (analysis.quality_level as i16 - 15) {
            return true;
        }

        // Surplus position and below average
        let player_group = player.position().position_group();
        for surplus_pos in &analysis.surplus_positions {
            if surplus_pos.position_group() == player_group {
                if (player.player_attributes.current_ability as i16) < analysis.quality_level as i16 {
                    return true;
                }
            }
        }

        let age = player.age(date);

        // Aging players past their prime — clubs willing to sell
        if age >= 32 && (player.player_attributes.current_ability as i16) < analysis.quality_level as i16 + 5 {
            return true;
        }

        // Below-average players in large squads — natural transfer candidates
        let squad_size = club.teams.teams.first().map(|t| t.players.players.len()).unwrap_or(0);
        if squad_size > 23
            && (player.player_attributes.current_ability as i16) < analysis.quality_level as i16 - 5
        {
            return true;
        }

        // Contract expiring within 6 months — club prefers selling to losing for free
        if let Some(ref contract) = player.contract {
            let days_remaining = (contract.expiration - date).num_days();
            if days_remaining < 180 && days_remaining > 0 {
                return true;
            }
        }

        false
    }

    fn calculate_asking_price(
        player: &crate::Player,
        club: &Club,
        date: NaiveDate,
        price_level: f32,
    ) -> CurrencyValue {
        use crate::transfers::window::PlayerValuationCalculator;

        let base_value = PlayerValuationCalculator::calculate_value_with_price_level(player, date, price_level);

        let multiplier = if club.finance.balance.balance < 0 {
            0.9
        } else {
            1.1
        };

        CurrencyValue {
            amount: base_value.amount * multiplier,
            currency: base_value.currency,
        }
    }

    pub(super) fn find_player_in_country(country: &Country, player_id: u32) -> Option<&crate::Player> {
        for club in &country.clubs {
            for team in &club.teams.teams {
                if let Some(player) = team.players.players.iter().find(|p| p.id == player_id) {
                    return Some(player);
                }
            }
        }
        None
    }

    fn calculate_player_importance(country: &Country, player_id: u32, club_id: u32) -> f32 {
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

    fn reopen_listing_for_player(country: &mut Country, player_id: u32) {
        for listing in &mut country.transfer_market.listings {
            if listing.player_id == player_id
                && listing.status == TransferListingStatus::InNegotiation
            {
                listing.status = TransferListingStatus::Available;
                break;
            }
        }
    }

    fn execute_player_transfer(
        country: &mut Country,
        player_id: u32,
        selling_club_id: u32,
        buying_club_id: u32,
        fee: f64,
        date: NaiveDate,
    ) {
        let mut player = None;
        let mut selling_team_name = String::new();
        let mut selling_team_slug = String::new();
        let mut selling_team_reputation: u16 = 0;
        let mut selling_league_id = None;

        if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
            selling_team_name = selling_club.name.clone();

            for team in &mut selling_club.teams.teams {
                if let Some(p) = team.players.take_player(&player_id) {
                    player = Some(p);
                    selling_team_slug = team.slug.clone();
                    selling_team_reputation = team.reputation.world;
                    selling_league_id = team.league_id;
                    team.transfer_list.remove(player_id);
                    break;
                }
            }

            selling_club.finance.add_transfer_income(fee);
        }

        if let Some(mut player) = player {
            let season = Season::from_date(date);

            // If the team's league is friendly (reserves etc.), use the club's main league instead
            let (selling_league_name, selling_league_slug) = selling_league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .and_then(|l| {
                    if l.friendly {
                        // Fall back to first non-friendly league in the country
                        country.leagues.leagues.iter().find(|ml| !ml.friendly)
                    } else {
                        Some(l)
                    }
                })
                .map(|l| (l.name.clone(), l.slug.clone()))
                .unwrap_or_default();

            let old_stats = std::mem::take(&mut player.statistics);
            player.statistics_history.push_or_replace(PlayerStatisticsHistoryItem {
                season: season.clone(),
                team_name: selling_team_name,
                team_slug: selling_team_slug,
                team_reputation: selling_team_reputation,
                league_name: selling_league_name,
                league_slug: selling_league_slug,
                is_loan: false,
                transfer_fee: None,
                statistics: old_stats,
                created_at: date,
            });

            player.statistics = PlayerStatistics::default();

            let buying_info = country.clubs.iter()
                .find(|c| c.id == buying_club_id)
                .and_then(|c| {
                    let main_team = c.teams.teams.iter()
                        .find(|t| t.team_type == crate::TeamType::Main)
                        .or(c.teams.teams.first())?;
                    let league = main_team.league_id
                        .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                        .map(|l| (l.name.clone(), l.slug.clone()))
                        .unwrap_or_default();
                    Some((main_team.name.clone(), main_team.slug.clone(), main_team.reputation.world, league.0, league.1))
                });

            if let Some((buy_team_name, buy_team_slug, buy_team_rep, buy_league_name, buy_league_slug)) = buying_info {
                player.statistics_history.push_or_replace(PlayerStatisticsHistoryItem {
                    season: season.clone(),
                    team_name: buy_team_name,
                    team_slug: buy_team_slug,
                    team_reputation: buy_team_rep,
                    league_name: buy_league_name,
                    league_slug: buy_league_slug,
                    is_loan: false,
                    transfer_fee: Some(fee),
                    statistics: PlayerStatistics::default(),
                    created_at: date,
                });
            }

            player.last_transfer_date = Some(date);

            player.statuses.remove(PlayerStatusType::Lst);
            player.statuses.remove(PlayerStatusType::Req);
            player.statuses.remove(PlayerStatusType::Unh);
            player.statuses.remove(PlayerStatusType::Trn);
            player.statuses.remove(PlayerStatusType::Bid);
            player.statuses.remove(PlayerStatusType::Wnt);
            player.statuses.remove(PlayerStatusType::Sct);

            // Fresh start at new club — reset happiness to neutral
            player.happiness = crate::PlayerHappiness::new();

            let contract_years = if player.age(date) < 24 { 5 }
            else if player.age(date) < 28 { 4 }
            else if player.age(date) < 32 { 3 }
            else { 2 };

            let expiry = date.checked_add_signed(chrono::Duration::days(contract_years * 365))
                .unwrap_or(date);

            let salary = (fee / 200.0).max(500.0) as u32;

            player.contract = Some(PlayerClubContract::new(salary, expiry));

            if let Some(buying_club) = country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
                buying_club.finance.spend_from_transfer_budget(fee);

                if !buying_club.teams.teams.is_empty() {
                    buying_club.teams.teams[0].players.add(player);
                }
            }

            info!(
                "Transfer completed: player {} moved from club {} to club {} for {}",
                player_id, selling_club_id, buying_club_id, fee
            );
        }
    }

    fn execute_loan_transfer(
        country: &mut Country,
        player_id: u32,
        selling_club_id: u32,
        buying_club_id: u32,
        loan_fee: f64,
        date: NaiveDate,
    ) {
        let mut player = None;
        let mut selling_team_name = String::new();
        let mut selling_team_slug = String::new();
        let mut selling_team_reputation: u16 = 0;
        let mut selling_league_id = None;

        if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
            selling_team_name = selling_club.name.clone();

            for team in &mut selling_club.teams.teams {
                if let Some(p) = team.players.take_player(&player_id) {
                    player = Some(p);
                    selling_team_slug = team.slug.clone();
                    selling_team_reputation = team.reputation.world;
                    selling_league_id = team.league_id;
                    team.transfer_list.remove(player_id);
                    break;
                }
            }

            selling_club.finance.add_transfer_income(loan_fee);
        }

        if let Some(mut player) = player {
            let season = Season::from_date(date);

            // If the team's league is friendly (reserves etc.), use the club's main league instead
            let (selling_league_name, selling_league_slug) = selling_league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .and_then(|l| {
                    if l.friendly {
                        country.leagues.leagues.iter().find(|ml| !ml.friendly)
                    } else {
                        Some(l)
                    }
                })
                .map(|l| (l.name.clone(), l.slug.clone()))
                .unwrap_or_default();

            let old_stats = std::mem::take(&mut player.statistics);
            player.statistics_history.push_or_replace(PlayerStatisticsHistoryItem {
                season: season.clone(),
                team_name: selling_team_name,
                team_slug: selling_team_slug,
                team_reputation: selling_team_reputation,
                league_name: selling_league_name,
                league_slug: selling_league_slug,
                is_loan: false,
                transfer_fee: None,
                statistics: old_stats,
                created_at: date,
            });

            // Reset current stats for the new club — history entry for the loan spell
            // will be created when the player moves again or the season ends
            player.statistics = PlayerStatistics::default();

            let buying_info = country.clubs.iter()
                .find(|c| c.id == buying_club_id)
                .and_then(|c| {
                    let main_team = c.teams.teams.iter()
                        .find(|t| t.team_type == crate::TeamType::Main)
                        .or(c.teams.teams.first())?;
                    let league = main_team.league_id
                        .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                        .map(|l| (l.name.clone(), l.slug.clone()))
                        .unwrap_or_default();
                    Some((main_team.name.clone(), main_team.slug.clone(), main_team.reputation.world, league.0, league.1))
                });

            if let Some((buy_team_name, buy_team_slug, buy_team_rep, buy_league_name, buy_league_slug)) = buying_info {
                player.statistics_history.push_or_replace(PlayerStatisticsHistoryItem {
                    season: season.clone(),
                    team_name: buy_team_name,
                    team_slug: buy_team_slug,
                    team_reputation: buy_team_rep,
                    league_name: buy_league_name,
                    league_slug: buy_league_slug,
                    is_loan: true,
                    transfer_fee: Some(loan_fee),
                    statistics: PlayerStatistics::default(),
                    created_at: date,
                });
            }

            player.last_transfer_date = Some(date);

            player.statuses.remove(PlayerStatusType::Loa);
            player.statuses.remove(PlayerStatusType::Lst);
            player.statuses.remove(PlayerStatusType::Req);
            player.statuses.remove(PlayerStatusType::Unh);
            player.statuses.remove(PlayerStatusType::Trn);
            player.statuses.remove(PlayerStatusType::Bid);
            player.statuses.remove(PlayerStatusType::Wnt);
            player.statuses.remove(PlayerStatusType::Sct);

            // Fresh start at new club — reset happiness to neutral
            player.happiness = crate::PlayerHappiness::new();

            let loan_end = date
                .checked_add_signed(chrono::Duration::days(180))
                .unwrap_or(date);

            let salary = (loan_fee / 50.0).max(200.0) as u32;
            player.contract = Some(PlayerClubContract::new_loan(salary, loan_end, selling_club_id));

            if let Some(buying_club) = country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
                buying_club.finance.spend_from_transfer_budget(loan_fee);

                if !buying_club.teams.teams.is_empty() {
                    buying_club.teams.teams[0].players.add(player);
                }
            }

            info!(
                "Loan completed: player {} loaned from club {} to club {} (fee: {})",
                player_id, selling_club_id, buying_club_id, loan_fee
            );
        }
    }

    /// Handle expiring contracts and free agent signings.
    /// Releases players with expired contracts and matches soon-to-expire players to clubs.
    fn handle_free_agents(country: &mut Country, date: NaiveDate, summary: &mut TransferActivitySummary) {
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
                    if contract.contract_type == crate::club::player::contract::ContractType::Loan {
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
                        info!("Contract expired: player {} ({}) released from {}",
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
        }

        let mut signings: Vec<FreeAgentSigning> = Vec::new();
        let max_signings_per_day = 3; // Don't overwhelm with too many per day

        for club in &country.clubs {
            if signings.len() >= max_signings_per_day {
                break;
            }

            if club.teams.teams.is_empty() {
                continue;
            }

            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            let team = &club.teams.teams[0];
            let avg_ability: u8 = if !team.players.players.is_empty() {
                let total: u32 = team
                    .players
                    .players
                    .iter()
                    .map(|p| p.player_attributes.current_ability as u32)
                    .sum();
                (total / team.players.players.len() as u32) as u8
            } else {
                50
            };

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
                            // Don't poach from same-quality clubs actively
                            && c.days_to_expiry <= 90
                            // Not already being signed by another club this round
                            && !signings.iter().any(|s| s.player_id == c.player_id)
                    })
                    .max_by_key(|c| c.ability as u16 + c.potential as u16)
                {
                    signings.push(FreeAgentSigning {
                        player_id: best.player_id,
                        player_name: best.player_name.clone(),
                        from_club_id: best.club_id,
                        from_club_name: best.club_name.clone(),
                        to_club_id: club.id,
                    });
                }
            }
        }

        // Pass 3: Execute signings as free transfers
        for signing in signings {
            let to_club_name = country.clubs.iter()
                .find(|c| c.id == signing.to_club_id)
                .map(|c| c.name.clone())
                .unwrap_or_default();

            // Create transfer history record
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
                ),
            );

            Self::execute_player_transfer(
                country,
                signing.player_id,
                signing.from_club_id,
                signing.to_club_id,
                0.0, // Free transfer
                date,
            );
            summary.completed_transfers += 1;

            info!(
                "Free agent signing: player {} from club {} to club {}",
                signing.player_id, signing.from_club_id, signing.to_club_id
            );
        }
    }
}
