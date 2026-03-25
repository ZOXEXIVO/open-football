use chrono::NaiveDate;
use log::debug;
use std::collections::HashMap;
use super::types::{SquadAnalysis, TransferActivitySummary};
use crate::country::result::CountryResult;
use crate::shared::CurrencyValue;
use crate::{
    Club, Country, Person, PlayerFieldPositionGroup, PlayerPositionType,
    PlayerSquadStatus, PlayerStatusType,
};
use crate::transfers::{TransferListing, TransferListingType};

impl CountryResult {
    /// List players for transfer based on pipeline decisions and staff evaluations.
    pub(crate) fn list_players_from_pipeline(
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

            let main_team = &club.teams.teams[0];
            let league_reputation = main_team.league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);
            let club_reputation = main_team.reputation.world;

            for player in &main_team.players.players {
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
                        let asking_price = Self::calculate_asking_price(player, club, date, price_level, league_reputation, club_reputation);
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

    fn should_list_player(
        player: &crate::Player,
        analysis: &SquadAnalysis,
        club: &Club,
        date: NaiveDate,
    ) -> bool {
        // Loan players belong to another club — cannot be listed by the loan club
        if player.is_on_loan() {
            return false;
        }

        // Recently transferred players get a settling-in period — prevents
        // unrealistic chains where a player is bought and immediately re-listed
        if let Some(transfer_date) = player.last_transfer_date {
            let days_since = (date - transfer_date).num_days();
            if days_since < 120 {
                return false;
            }
        }

        let statuses = player.statuses.get();

        // Already listed
        if statuses.contains(&PlayerStatusType::Lst) || statuses.contains(&PlayerStatusType::Loa) || statuses.contains(&PlayerStatusType::Frt) {
            return false;
        }

        // Player-initiated departures override the plan — if a player wants
        // out or has been explicitly marked NotNeeded, respect that.
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

        // Club signing plan: the club bought this player with intent.
        // Don't list them until they've had a fair chance (enough time + games).
        if let Some(ref plan) = player.plan {
            let total_apps = player.statistics.played + player.statistics.played_subs;
            if !plan.is_evaluated(date, total_apps) && !plan.is_expired(date) {
                return false;
            }
        }

        let age = player.age(date);
        let ca = player.player_attributes.current_ability as i16;
        let avg = analysis.quality_level as i16;

        // Young players with good development potential should be kept or loaned,
        // not sold — even if their current ability is below squad average.
        let is_promising_youth = age <= 23
            && player.player_attributes.potential_ability > player.player_attributes.current_ability + 10;

        // Well below squad average — club would accept offers.
        // But only for non-promising players: a 19yo prospect with CA 55 at a
        // club with avg 75 shouldn't be sold, that's why they were signed.
        if analysis.quality_level > 15 && ca < avg - 15 && !is_promising_youth {
            // Still check position group depth: if the group is thin, keep the player
            if !Self::position_group_has_depth(club, player, date) {
                return false;
            }
            return true;
        }

        // Surplus position and below average — but only if the surplus is real:
        // having 8 midfielders when you play 4 is surplus,
        // having 5 midfielders when you play 4 is depth.
        let player_group = player.position().position_group();
        for surplus_pos in &analysis.surplus_positions {
            if surplus_pos.position_group() == player_group {
                if ca < avg && !is_promising_youth {
                    return true;
                }
            }
        }

        // Aging players past their prime — clubs willing to sell
        if age >= 32 && ca < avg + 5 {
            return true;
        }

        // Below-average players in large squads — natural transfer candidates.
        // But require a bigger gap and skip promising youth.
        let squad_size = club.teams.teams.first().map(|t| t.players.players.len()).unwrap_or(0);
        if squad_size > 25
            && ca < avg - 10
            && !is_promising_youth
        {
            return true;
        }

        // Contract expiring within 12 months — club lists for sale early to get real transfer fees
        if let Some(ref contract) = player.contract {
            let days_remaining = (contract.expiration - date).num_days();
            if days_remaining < 365 && days_remaining > 0 {
                return true;
            }
        }

        false
    }

    /// Returns true if the player's position group already has enough players,
    /// i.e. selling one wouldn't leave a gap.
    fn position_group_has_depth(
        club: &Club,
        player: &crate::Player,
        date: NaiveDate,
    ) -> bool {
        let team = match club.teams.teams.first() {
            Some(t) => t,
            None => return false,
        };

        let group = player.position().position_group();
        let group_count = team.players.players.iter()
            .filter(|p| p.position().position_group() == group)
            .count();

        // Minimum depth per group: losing one player shouldn't leave a gap
        let min_to_keep = match group {
            PlayerFieldPositionGroup::Goalkeeper => 2,
            PlayerFieldPositionGroup::Defender => 4,
            PlayerFieldPositionGroup::Midfielder => 4,
            PlayerFieldPositionGroup::Forward => 2,
        };

        // Only has "depth" (safe to sell) if above minimum after selling
        group_count > min_to_keep
    }

    fn calculate_asking_price(
        player: &crate::Player,
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
