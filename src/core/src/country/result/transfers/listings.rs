use chrono::NaiveDate;
use log::debug;
use std::collections::HashMap;
use super::types::{SquadAnalysis, TransferActivitySummary};
use crate::country::result::CountryResult;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::{TransferListing, TransferListingType};
use crate::transfers::TransferWindowManager;
use crate::{
    Club, Country, Person, Player, PlayerFieldPositionGroup, PlayerPositionType,
    PlayerSquadStatus, PlayerStatusType, ReputationLevel,
};

enum ListingDecision {
    Keep,
    Transfer { reason: String },
    Loan { reason: String },
    FreeTransfer,
}

struct PendingListing {
    player_id: u32,
    club_id: u32,
    team_id: u32,
    asking_price: CurrencyValue,
    listing_type: TransferListingType,
    reason: String,
    decided_by: String,
}

impl CountryResult {
    /// List players for transfer based on pipeline decisions and staff evaluations.
    pub(crate) fn list_players_from_pipeline(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
    ) {
        let mut listings_to_add: Vec<PendingListing> = Vec::new();
        let price_level = country.settings.pricing.price_level;
        let window_mgr = TransferWindowManager::new();
        let current_window = window_mgr.current_window_dates(country.id, date);

        for club in &country.clubs {
            let squad_analysis = Self::analyze_squad_needs(club, date);

            if club.teams.teams.is_empty() {
                continue;
            }

            let main_team = &club.teams.teams[0];
            let league_reputation = main_team.league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);
            let club_reputation = main_team.reputation.world;
            let decided_by = main_team.staffs.head_coach().full_name.to_string();

            for player in &main_team.players.players {
                match Self::evaluate_player_listing(player, &squad_analysis, club, date, current_window) {
                    ListingDecision::Keep => {}
                    ListingDecision::FreeTransfer => {
                        let free_price = CurrencyValue { amount: 0.0, currency: Currency::Usd };
                        listings_to_add.push(PendingListing {
                            player_id: player.id,
                            club_id: club.id,
                            team_id: main_team.id,
                            asking_price: free_price,
                            listing_type: TransferListingType::EndOfContract,
                            reason: "dec_reason_under16_release".to_string(),
                            decided_by: decided_by.clone(),
                        });
                    }
                    ListingDecision::Transfer { reason } => {
                        let asking_price = Self::calculate_asking_price(player, club, date, price_level, league_reputation, club_reputation);
                        listings_to_add.push(PendingListing {
                            player_id: player.id,
                            club_id: club.id,
                            team_id: main_team.id,
                            asking_price,
                            listing_type: TransferListingType::Transfer,
                            reason,
                            decided_by: decided_by.clone(),
                        });
                    }
                    ListingDecision::Loan { reason } => {
                        listings_to_add.push(PendingListing {
                            player_id: player.id,
                            club_id: club.id,
                            team_id: main_team.id,
                            asking_price: CurrencyValue { amount: 0.0, currency: Currency::Usd },
                            listing_type: TransferListingType::Loan,
                            reason,
                            decided_by: decided_by.clone(),
                        });
                    }
                }
            }
        }

        if !listings_to_add.is_empty() {
            debug!("Transfer market: listing {} players for transfer/loan", listings_to_add.len());
        }

        // Apply listings
        for listing_data in listings_to_add {
            let status_type = match listing_data.listing_type {
                TransferListingType::Loan => PlayerStatusType::Loa,
                TransferListingType::EndOfContract => PlayerStatusType::Frt,
                _ => PlayerStatusType::Lst,
            };

            let movement = match listing_data.listing_type {
                TransferListingType::Loan => "dec_loan_listed",
                TransferListingType::EndOfContract => "dec_free_transfer_listed",
                _ => "dec_transfer_listed",
            };

            let listing = TransferListing::new(
                listing_data.player_id,
                listing_data.club_id,
                listing_data.team_id,
                listing_data.asking_price,
                date,
                listing_data.listing_type,
            );

            country.transfer_market.add_listing(listing);
            summary.total_listings += 1;

            for club in &mut country.clubs {
                for team in &mut club.teams.teams {
                    if let Some(player) = team.players.players.iter_mut().find(|p| p.id == listing_data.player_id) {
                        if !player.statuses.get().contains(&status_type) {
                            player.statuses.add(date, status_type);
                        }
                        player.decision_history.add(
                            date,
                            movement.to_string(),
                            listing_data.reason.clone(),
                            listing_data.decided_by.clone(),
                        );
                    }
                }
            }
        }
    }

    pub(crate) fn analyze_squad_needs(club: &Club, current_date: NaiveDate) -> SquadAnalysis {
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

    fn evaluate_player_listing(
        player: &Player,
        analysis: &SquadAnalysis,
        club: &Club,
        date: NaiveDate,
        current_window: Option<(NaiveDate, NaiveDate)>,
    ) -> ListingDecision {
        // Loan players belong to another club — cannot be listed by the loan club
        if player.is_on_loan() {
            return ListingDecision::Keep;
        }

        // Same-window protection: signed during this open window → can't be listed
        if let (Some(transfer_date), Some((window_start, window_end))) =
            (player.last_transfer_date, current_window)
        {
            if transfer_date >= window_start && transfer_date <= window_end {
                return ListingDecision::Keep;
            }
        }

        let statuses = player.statuses.get();

        // Already listed
        if statuses.contains(&PlayerStatusType::Lst) || statuses.contains(&PlayerStatusType::Loa) || statuses.contains(&PlayerStatusType::Frt) {
            return ListingDecision::Keep;
        }

        // Club signing plan: the club bought this player with intent.
        if let Some(ref plan) = player.plan {
            let total_apps = player.statistics.played + player.statistics.played_subs;
            if !plan.is_evaluated(date, total_apps) && !plan.is_expired(date) {
                return ListingDecision::Keep;
            }
        }

        let age = player.age(date);
        let ca = player.player_attributes.current_ability;
        let pa = player.player_attributes.potential_ability;
        let ca_i = ca as i16;
        let avg = analysis.quality_level as i16;

        let rep_level = club.teams.teams.first()
            .map(|t| t.reputation.level())
            .unwrap_or(ReputationLevel::Amateur);

        // Check if evaluation pipeline already identified as loan candidate
        let loan_candidate = club.transfer_plan.loan_out_candidates
            .iter()
            .find(|c| c.player_id == player.id);

        if let Some(candidate) = loan_candidate {
            let reason = match &candidate.reason {
                crate::transfers::pipeline::LoanOutReason::NeedsGameTime =>
                    "dec_reason_needs_game_time",
                crate::transfers::pipeline::LoanOutReason::BlockedByBetterPlayer =>
                    "dec_reason_blocked_by_better",
                crate::transfers::pipeline::LoanOutReason::Surplus =>
                    "dec_reason_surplus_tactical",
                crate::transfers::pipeline::LoanOutReason::FinancialRelief =>
                    "dec_reason_financial_relief",
                crate::transfers::pipeline::LoanOutReason::LackOfPlayingTime =>
                    "dec_reason_lack_playing_time",
                crate::transfers::pipeline::LoanOutReason::PostInjuryFitness =>
                    "dec_reason_post_injury_fitness",
            };
            return ListingDecision::Loan { reason: reason.to_string() };
        }

        // Player-initiated departures
        if let Some(ref contract) = player.contract {
            if matches!(contract.squad_status, PlayerSquadStatus::NotNeeded) {
                return Self::decide_listing_type(player, &rep_level, avg, date,
                    "dec_reason_surplus_squad".to_string());
            }
            if contract.is_transfer_listed {
                return ListingDecision::Transfer { reason: "dec_reason_club_listed".to_string() };
            }
        }

        if statuses.contains(&PlayerStatusType::Req) {
            return ListingDecision::Transfer { reason: "dec_reason_player_requested".to_string() };
        }

        if statuses.contains(&PlayerStatusType::Unh) {
            return ListingDecision::Transfer { reason: "dec_reason_player_unhappy".to_string() };
        }

        let is_promising_youth = age <= 23 && pa > ca + 10;

        // Wealth-aware quality gap threshold
        let quality_gap_threshold: i16 = match rep_level {
            ReputationLevel::Elite => 25,
            ReputationLevel::Continental => 20,
            ReputationLevel::National => 15,
            ReputationLevel::Regional => 12,
            _ => 10,
        };

        // Well below squad average
        if analysis.quality_level > 15 && ca_i < avg - quality_gap_threshold && !is_promising_youth {
            if !Self::position_group_has_depth(club, player, date) {
                return ListingDecision::Keep;
            }
            return Self::decide_listing_type(player, &rep_level, avg, date,
                "dec_reason_well_below_avg".to_string());
        }

        // Surplus position and below average
        let player_group = player.position().position_group();
        for surplus_pos in &analysis.surplus_positions {
            if surplus_pos.position_group() == player_group {
                if ca_i < avg && !is_promising_youth {
                    return Self::decide_listing_type(player, &rep_level, avg, date,
                        "dec_reason_below_avg_surplus".to_string());
                }
            }
        }

        // Aging players past their prime
        if age >= 32 && ca_i < avg + 5 {
            return ListingDecision::Transfer {
                reason: "dec_reason_aging_declining".to_string(),
            };
        }

        // Below-average players in large squads — wealth-aware threshold
        let squad_size = club.teams.teams.first().map(|t| t.players.players.len()).unwrap_or(0);
        let max_comfortable_squad = match rep_level {
            ReputationLevel::Elite => 45,
            ReputationLevel::Continental => 40,
            ReputationLevel::National => 32,
            ReputationLevel::Regional => 26,
            _ => 22,
        };

        if squad_size > max_comfortable_squad
            && ca_i < avg - 10
            && !is_promising_youth
        {
            return Self::decide_listing_type(player, &rep_level, avg, date,
                "dec_reason_squad_oversized".to_string());
        }

        // Contract expiring within 12 months
        if let Some(ref contract) = player.contract {
            let days_remaining = (contract.expiration - date).num_days();
            if days_remaining < 365 && days_remaining > 0 {
                return ListingDecision::Transfer {
                    reason: "dec_reason_contract_expiring".to_string(),
                };
            }
        }

        ListingDecision::Keep
    }

    /// Decide between Transfer and Loan based on player profile and club context.
    fn decide_listing_type(
        player: &Player,
        rep_level: &ReputationLevel,
        avg: i16,
        date: NaiveDate,
        base_reason: String,
    ) -> ListingDecision {
        let age = player.age(date);
        let ca = player.player_attributes.current_ability;
        let pa = player.player_attributes.potential_ability;

        // Under 16: free transfer
        if age < 16 {
            return ListingDecision::FreeTransfer;
        }

        // Young with development potential → loan for match practice
        if age <= 23 && pa > ca + 10 {
            return ListingDecision::Loan {
                reason: "dec_reason_young_needs_practice".to_string(),
            };
        }

        // At wealthy club, young enough and decent quality → loan to preserve asset
        if age <= 25
            && matches!(rep_level, ReputationLevel::Elite | ReputationLevel::Continental)
            && (ca as i16) >= avg - 20
        {
            return ListingDecision::Loan {
                reason: "dec_reason_blocked_top_club".to_string(),
            };
        }

        // Aging or peaked → transfer
        if age >= 30 || pa <= ca {
            return ListingDecision::Transfer {
                reason: "dec_reason_peaked_declining".to_string(),
            };
        }

        // Mid-career at wealthy club → loan to preserve value
        if age <= 27 && matches!(rep_level, ReputationLevel::Elite | ReputationLevel::Continental) {
            return ListingDecision::Loan {
                reason: "dec_reason_loan_playing_time".to_string(),
            };
        }

        // Default: transfer
        ListingDecision::Transfer { reason: base_reason }
    }

    /// Returns true if the player's position group already has enough players.
    fn position_group_has_depth(
        club: &Club,
        player: &Player,
        _date: NaiveDate,
    ) -> bool {
        let team = match club.teams.teams.first() {
            Some(t) => t,
            None => return false,
        };

        let group = player.position().position_group();
        let group_count = team.players.iter()
            .filter(|p| p.position().position_group() == group)
            .count();

        let min_to_keep = match group {
            PlayerFieldPositionGroup::Goalkeeper => 2,
            PlayerFieldPositionGroup::Defender => 4,
            PlayerFieldPositionGroup::Midfielder => 4,
            PlayerFieldPositionGroup::Forward => 2,
        };

        group_count > min_to_keep
    }

    fn calculate_asking_price(
        player: &Player,
        club: &Club,
        date: NaiveDate,
        price_level: f32,
        league_reputation: u16,
        club_reputation: u16,
    ) -> CurrencyValue {
        use crate::transfers::window::PlayerValuationCalculator;

        let base_value = PlayerValuationCalculator::calculate_value_with_price_level(player, date, price_level, league_reputation, club_reputation);

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
}
