use chrono::Datelike;
use chrono::NaiveDate;
use log::{debug, info};
use std::collections::HashMap;
use crate::league::{LeagueResult, Season};
use crate::simulator::SimulatorData;
use crate::{Club, ClubResult, ClubTransferStrategy, Country, Person, PlayerClubContract,
            PlayerFieldPositionGroup, PlayerPositionType, PlayerSquadStatus, PlayerStatistics,
            PlayerStatisticsHistoryItem, PlayerStatusType, SimulationResult};
use crate::club::staff::result::ScoutRecommendation;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::{TransferListing, TransferListingType, TransferWindowManager};

pub struct CountryResult {
    pub country_id: u32,
    pub leagues: Vec<LeagueResult>,
    pub clubs: Vec<ClubResult>,
}

impl CountryResult {
    pub fn new(country_id: u32, leagues: Vec<LeagueResult>, clubs: Vec<ClubResult>) -> Self {
        CountryResult { country_id, leagues, clubs }
    }

    pub fn process(self, data: &mut SimulatorData, result: &mut SimulationResult) {
        let current_date = data.date.date();
        let country_id = self.get_country_id(data);

        // Phase 3: Pre-season activities (if applicable)
        if Self::is_preseason(current_date) {
            self.simulate_preseason_activities(data, country_id, current_date);
        }

        // Phase 4: Transfer Market Activities
        let _transfer_activities = self.simulate_transfer_market(data, country_id, current_date);

        // Phase 5: International Competitions
        self.simulate_international_competitions(data, country_id, current_date);

        // Phase 6: Economic Updates
        self.update_economic_factors(data, country_id, current_date);

        // Phase 7: Media and Public Interest
        self.simulate_media_coverage(data, country_id, &self.leagues);

        // Phase 8: End of Period Processing
        self.process_end_of_period(data, country_id, current_date, &self.clubs);

        // Phase 9: Country Reputation Update
        self.update_country_reputation(data, country_id, &self.leagues, &self.clubs);

        // Phase 1: Process league results
        for league_result in self.leagues {
            league_result.process(data, result);
        }

        // Phase 2: Process club results
        for club_result in self.clubs {
            club_result.process(data, result);
        }
    }

    // Helper methods

    fn get_country_id(&self, _data: &SimulatorData) -> u32 {
        self.country_id
    }

    fn is_preseason(date: NaiveDate) -> bool {
        let month = date.month();
        // Preseason typically June-July in Europe
        month == 6 || month == 7
    }

    fn simulate_preseason_activities(&self, data: &mut SimulatorData, country_id: u32, date: NaiveDate) {
        info!("⚽ Running preseason activities...");

        if let Some(country) = data.country_mut(country_id) {
            // Schedule friendlies between clubs
            Self::schedule_friendly_matches(country, date);

            // Training camps and tours
            Self::organize_training_camps(country);

            // Preseason tournaments
            Self::organize_preseason_tournaments(country);
        }
    }

    fn simulate_transfer_market(
        &self,
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

        info!("💰 Transfer window is OPEN - simulating market activity");

        // Get country to access its data
        if let Some(country) = data.country_mut(country_id) {
            // Phase 1: Resolve pending negotiations from previous days
            Self::resolve_pending_negotiations(country, current_date, &mut summary);

            // Phase 2: Clubs list players for transfer
            Self::list_players_for_transfer(country, current_date, &mut summary);

            // Phase 3: Generate interest and start new negotiations
            Self::negotiate_transfers(country, current_date, &mut summary);

            // Phase 4: Start loan deal negotiations
            Self::process_loan_deals(country, current_date, &mut summary);

            // Phase 5: Free agents and contract expirations
            Self::handle_free_agents(country, current_date, &mut summary);

            // Phase 6: Expire stale negotiations
            country.transfer_market.update(current_date);

            debug!(
                "Transfer Activity - Listings: {}, Negotiations: {}, Completed: {}",
                summary.total_listings, summary.active_negotiations, summary.completed_transfers
            );
        }

        summary
    }

    fn list_players_for_transfer(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
    ) {
        // Collect player listing data to avoid borrow conflicts
        // Tuple: (player_id, club_id, team_id, asking_price, listing_type)
        let mut listings_to_add = Vec::new();
        let price_level = country.settings.pricing.price_level;

        for club in &country.clubs {
            // Analyze squad and determine transfer needs
            let squad_analysis = Self::analyze_squad_needs(club);

            // List surplus players
            for player in &club.teams.teams[0].players.players {
                if Self::should_loan_player(player, &squad_analysis, date) {
                    let asking_price = Self::calculate_asking_price(player, club, date, price_level);
                    listings_to_add.push((
                        player.id,
                        club.id,
                        club.teams.teams[0].id,
                        asking_price,
                        TransferListingType::Loan,
                    ));
                } else if Self::should_list_player(player, &squad_analysis, club) {
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

        // Now add all listings and set statuses
        for (player_id, club_id, team_id, asking_price, listing_type) in listings_to_add {
            let status_type = match listing_type {
                TransferListingType::Loan => PlayerStatusType::Loa,
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

            // Set status on player
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

    fn negotiate_transfers(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
    ) {
        let mut negotiations_to_process = Vec::new();

        // First pass: collect all potential negotiations
        for buying_club in &country.clubs {
            // Use real club budget instead of hardcoded value
            let budget = buying_club.finance.transfer_budget.clone().unwrap_or(CurrencyValue {
                amount: (buying_club.finance.balance.balance.max(0) as f64) * 0.3,
                currency: Currency::Usd,
            });

            let analysis = Self::analyze_squad_needs(buying_club);

            let strategy = ClubTransferStrategy {
                club_id: buying_club.id,
                budget: Some(budget),
                selling_willingness: 0.5,
                buying_aggressiveness: Self::calculate_buying_aggressiveness(buying_club),
                target_positions: analysis.needed_positions,
                reputation_level: analysis.quality_level as u16,
            };

            let available_listings: Vec<_> = country
                .transfer_market
                .get_available_listings()
                .into_iter()
                .filter(|listing| listing.club_id != buying_club.id)
                .cloned()
                .collect();

            for listing in available_listings {
                if let Some(player) = Self::find_player_in_country(country, listing.player_id) {
                    if strategy.decide_player_interest(player) {
                        let offer = strategy.calculate_initial_offer(
                            player,
                            &listing.asking_price,
                            date,
                        );

                        negotiations_to_process.push((
                            listing.player_id,
                            buying_club.id,
                            listing.club_id,
                            offer,
                            false, // listed player, normal acceptance rate
                        ));
                    }
                }
            }
        }

        // Scouted player negotiations: pursue players with Wnt status from scouting
        let scouting_interests: Vec<_> = country.scouting_interests
            .iter()
            .filter(|i| i.recommendation == ScoutRecommendation::Sign)
            .map(|i| (i.player_id, i.interested_club_id))
            .collect();

        for (player_id, interested_club_id) in scouting_interests {
            // Skip if already in negotiations
            if negotiations_to_process.iter().any(|(pid, bid, _, _, _)| *pid == player_id && *bid == interested_club_id) {
                continue;
            }

            // Find which club owns the player
            let selling_club_id = country.clubs.iter()
                .find(|c| {
                    c.teams.teams.iter().any(|t| t.players.players.iter().any(|p| p.id == player_id))
                })
                .map(|c| c.id);

            let selling_club_id = match selling_club_id {
                Some(id) if id != interested_club_id => id,
                _ => continue,
            };

            if let Some(buying_club) = country.clubs.iter().find(|c| c.id == interested_club_id) {
                let budget = buying_club.finance.transfer_budget.clone().unwrap_or(CurrencyValue {
                    amount: (buying_club.finance.balance.balance.max(0) as f64) * 0.3,
                    currency: Currency::Usd,
                });

                let analysis = Self::analyze_squad_needs(buying_club);
                let strategy = ClubTransferStrategy {
                    club_id: buying_club.id,
                    budget: Some(budget),
                    selling_willingness: 0.5,
                    buying_aggressiveness: Self::calculate_buying_aggressiveness(buying_club),
                    target_positions: analysis.needed_positions,
                    reputation_level: analysis.quality_level as u16,
                };

                if let Some(player) = Self::find_player_in_country(country, player_id) {
                    if strategy.decide_player_interest(player) {
                        // Use player valuation as asking price (seller didn't list)
                        let asking_price = Self::calculate_asking_price(
                            player,
                            country.clubs.iter().find(|c| c.id == selling_club_id).unwrap(),
                            date,
                            country.settings.pricing.price_level,
                        );

                        let offer = strategy.calculate_initial_offer(player, &asking_price, date);

                        negotiations_to_process.push((
                            player_id,
                            interested_club_id,
                            selling_club_id,
                            offer,
                            true, // scouted player, lower acceptance rate
                        ));
                    }
                }
            }
        }

        // Second pass: start negotiations (leave as Pending — resolved on future days)
        for (player_id, buying_club_id, _selling_club_id, offer, is_scouted) in negotiations_to_process {
            // Skip if this player already has a pending negotiation
            let already_negotiating = country.transfer_market.negotiations.values()
                .any(|n| n.player_id == player_id && n.buying_club_id == buying_club_id
                    && (n.status == crate::transfers::NegotiationStatus::Pending
                        || n.status == crate::transfers::NegotiationStatus::Countered));
            if already_negotiating {
                continue;
            }

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                player_id,
                buying_club_id,
                offer,
                date,
            ) {
                // Tag the negotiation with metadata for later resolution
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_unsolicited = is_scouted;
                }
                summary.active_negotiations += 1;
            }
        }
    }

    fn resolve_pending_negotiations(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
    ) {
        // Collect negotiations ready to resolve
        let ready_to_resolve: Vec<(u32, bool, bool)> = country
            .transfer_market
            .negotiations
            .values()
            .filter(|n| n.is_ready_to_resolve(date))
            .map(|n| (n.id, n.is_unsolicited, n.is_loan))
            .collect();

        for (neg_id, is_unsolicited, is_loan) in ready_to_resolve {
            let (player_id, selling_club_id, buying_club_id, offer_amount) = {
                let n = match country.transfer_market.negotiations.get(&neg_id) {
                    Some(n) => n,
                    None => continue,
                };
                (n.player_id, n.selling_club_id, n.buying_club_id, n.current_offer.base_fee.amount)
            };

            // Determine acceptance based on negotiation type
            let accepted = if is_loan {
                Self::simulate_negotiation_outcome(neg_id, selling_club_id, buying_club_id, false)
                    || Self::simulate_negotiation_outcome(neg_id, selling_club_id, buying_club_id, false)
                // ~64% chance (two 40% rolls combined) for loans
            } else {
                Self::simulate_negotiation_outcome(neg_id, selling_club_id, buying_club_id, is_unsolicited)
            };

            if accepted {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.accept();
                }

                let player_name = Self::find_player_in_country(country, player_id)
                    .map(|p| p.full_name.to_string())
                    .unwrap_or_default();
                let from_team_name = country.clubs.iter()
                    .find(|c| c.id == selling_club_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_default();
                let to_team_name = country.clubs.iter()
                    .find(|c| c.id == buying_club_id)
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

                    if is_loan {
                        Self::execute_loan_transfer(
                            country,
                            player_id,
                            selling_club_id,
                            buying_club_id,
                            offer_amount,
                            date,
                        );
                    } else {
                        Self::execute_player_transfer(
                            country,
                            player_id,
                            selling_club_id,
                            buying_club_id,
                            offer_amount,
                            date,
                        );
                    }
                }
            } else {
                // Rejected — selling club says no
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.reject();
                }
            }
        }
    }

    fn simulate_international_competitions(
        &self,
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) {
        if let Some(country) = data.country_mut(country_id) {
            // Simulate continental competitions (e.g., Champions League, Europa League)
            for competition in &mut country.international_competitions {
                competition.simulate_round(date);
            }
        }
    }

    fn update_economic_factors(
        &self,
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) {
        // Update country's economic indicators monthly
        if date.day() == 1 {
            if let Some(country) = data.country_mut(country_id) {
                country.economic_factors.monthly_update();
            }
        }
    }

    fn simulate_media_coverage(
        &self,
        data: &mut SimulatorData,
        country_id: u32,
        league_results: &[LeagueResult],
    ) {
        if let Some(country) = data.country_mut(country_id) {
            country.media_coverage.update_from_results(league_results);
            country.media_coverage.generate_weekly_stories(&country.clubs);
        }
    }

    fn process_end_of_period(
        &self,
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
        club_results: &[ClubResult],
    ) {
        // End of season processing
        if date.month() == 5 && date.day() == 31 {
            info!("📅 End of season processing");

            if let Some(country) = data.country_mut(country_id) {
                // Award ceremonies
                Self::process_season_awards(country, club_results);

                // Contract expirations
                Self::process_contract_expirations(country);

                // Retirements
                Self::process_player_retirements(country, date);
            }
        }

        // End of year processing
        if date.month() == 12 && date.day() == 31 {
            if let Some(country) = data.country_mut(country_id) {
                Self::process_year_end_finances(country);
            }
        }
    }

    fn update_country_reputation(
        &self,
        data: &mut SimulatorData,
        country_id: u32,
        _league_results: &[crate::league::LeagueResult],
        _club_results: &[ClubResult],
    ) {
        if let Some(country) = data.country_mut(country_id) {
            // Base reputation change
            let mut reputation_change: i16 = 0;

            // Factor 1: League competitiveness
            for league in &country.leagues.leagues {
                let competitiveness = Self::calculate_league_competitiveness(league);
                reputation_change += (competitiveness * 5.0) as i16;
            }

            // Factor 2: International competition performance
            let international_success = Self::calculate_international_success(country);
            reputation_change += international_success as i16;

            // Factor 3: Transfer market activity
            let transfer_reputation = Self::calculate_transfer_market_reputation(country);
            reputation_change += transfer_reputation as i16;

            // Apply change with bounds
            let new_reputation = (country.reputation as i16 + reputation_change).clamp(0, 1000) as u16;

            if new_reputation != country.reputation {
                debug!(
                    "Country {} reputation changed: {} -> {} ({})",
                    country.name,
                    country.reputation,
                    new_reputation,
                    if reputation_change > 0 {
                        format!("+{}", reputation_change)
                    } else {
                        reputation_change.to_string()
                    }
                );
                country.reputation = new_reputation;
            }
        }
    }

    // Helper methods

    fn analyze_squad_needs(club: &Club) -> SquadAnalysis {
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

        // Count players by position group
        let mut group_counts: HashMap<PlayerFieldPositionGroup, u32> = HashMap::new();
        let mut total_ability: u32 = 0;
        let mut total_age: u32 = 0;
        let now = chrono::Local::now().naive_local().date();

        for player in players {
            let group = player.position().position_group();
            *group_counts.entry(group).or_insert(0) += 1;
            total_ability += player.player_attributes.current_ability as u32;
            total_age += player.age(now) as u32;
        }

        let avg_ability = (total_ability / players.len() as u32) as u8;
        let avg_age = total_age as f32 / players.len() as f32;

        // Ideal squad: 2 GK, 6 DEF, 6 MID, 4 FWD = 18 minimum
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

    fn should_loan_player(
        player: &crate::Player,
        analysis: &SquadAnalysis,
        date: NaiveDate,
    ) -> bool {
        let statuses = player.statuses.get();

        // Already listed for transfer or loan
        if statuses.contains(&PlayerStatusType::Lst) || statuses.contains(&PlayerStatusType::Loa) {
            return false;
        }

        // Player explicitly requested transfer — sell, don't loan
        if statuses.contains(&PlayerStatusType::Req) || statuses.contains(&PlayerStatusType::Unh) {
            return false;
        }

        let age = player.age(date);
        let ability = player.player_attributes.current_ability;
        let potential = player.player_attributes.potential_ability;

        // Young players (under 21) with high potential but below squad level → loan for development
        if age <= 21 && potential > ability + 5 && (ability as i16) < analysis.quality_level as i16 {
            return true;
        }

        // Surplus position players who are decent but not the weakest → loan rather than sell
        let player_group = player.position().position_group();
        for surplus_pos in &analysis.surplus_positions {
            if surplus_pos.position_group() == player_group {
                // Close to squad level — loan, don't sell
                let diff = (ability as i16) - (analysis.quality_level as i16);
                if diff >= -5 && diff < 0 {
                    return true;
                }
            }
        }

        false
    }

    fn should_list_player(
        player: &crate::Player,
        analysis: &SquadAnalysis,
        _club: &Club,
    ) -> bool {
        let statuses = player.statuses.get();

        // Already listed for transfer or loan
        if statuses.contains(&PlayerStatusType::Lst) || statuses.contains(&PlayerStatusType::Loa) {
            return false;
        }

        // Players marked as NotNeeded in their contract
        if let Some(ref contract) = player.contract {
            if matches!(contract.squad_status, PlayerSquadStatus::NotNeeded) {
                return true;
            }
            if contract.is_transfer_listed {
                return true;
            }
        }

        // Players who have requested a transfer
        if player.statuses.get().contains(&PlayerStatusType::Req) {
            return true;
        }

        // Unhappy players for more than a short period
        if player.statuses.get().contains(&PlayerStatusType::Unh) {
            return true;
        }

        // Low-rated players relative to squad average (more than 15 points below)
        if analysis.quality_level > 15 &&
            (player.player_attributes.current_ability as i16) < (analysis.quality_level as i16 - 15) {
            return true;
        }

        // Surplus position players
        let player_group = player.position().position_group();
        for surplus_pos in &analysis.surplus_positions {
            if surplus_pos.position_group() == player_group {
                // Only list the weakest players from surplus positions
                if (player.player_attributes.current_ability as i16) < analysis.quality_level as i16 {
                    return true;
                }
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
            0.9 // Financial pressure
        } else {
            1.1 // No pressure
        };

        CurrencyValue {
            amount: base_value.amount * multiplier,
            currency: base_value.currency,
        }
    }

    fn calculate_buying_aggressiveness(club: &Club) -> f32 {
        let balance = club.finance.balance.balance as f64;
        if balance > 10_000_000.0 {
            0.8
        } else if balance > 1_000_000.0 {
            0.6
        } else if balance > 0.0 {
            0.4
        } else {
            0.2
        }
    }

    #[allow(dead_code)]
    fn identify_target_positions(club: &Club) -> Vec<crate::PlayerPositionType> {
        let analysis = Self::analyze_squad_needs(club);
        analysis.needed_positions
    }

    fn find_player_in_country(country: &Country, player_id: u32) -> Option<&crate::Player> {
        for club in &country.clubs {
            for team in &club.teams.teams {
                if let Some(player) = team.players.players.iter().find(|p| p.id == player_id) {
                    return Some(player);
                }
            }
        }
        None
    }

    fn simulate_negotiation_outcome(
        _negotiation_id: u32,
        _selling_club: u32,
        _buying_club: u32,
        is_scouted: bool,
    ) -> bool {
        use crate::utils::IntegerUtils;
        if is_scouted {
            IntegerUtils::random(0, 100) > 75 // 25% success rate for unsolicited bids
        } else {
            IntegerUtils::random(0, 100) > 60 // 40% success rate for listed players
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
        // Find selling club and take the player out
        let mut player = None;
        let mut selling_team_name = String::new();
        let mut selling_team_slug = String::new();

        if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
            selling_team_name = selling_club.name.clone();

            for team in &mut selling_club.teams.teams {
                if let Some(p) = team.players.take_player(&player_id) {
                    player = Some(p);
                    selling_team_slug = team.slug.clone();
                    // Remove from team transfer list too
                    team.transfer_list.remove(player_id);
                    break;
                }
            }

            // Selling club receives fee
            selling_club.finance.add_transfer_income(fee);
        }

        if let Some(mut player) = player {
            // Archive current season stats
            let season_year = if date.month() >= 8 {
                date.year() as u16
            } else {
                (date.year() - 1) as u16
            };

            let old_stats = std::mem::take(&mut player.statistics);
            player.statistics_history.items.push(PlayerStatisticsHistoryItem {
                season: Season::OneYear(season_year),
                team_name: selling_team_name,
                team_slug: selling_team_slug,
                is_loan: false,
                statistics: old_stats,
            });

            // Reset current stats
            player.statistics = PlayerStatistics::default();

            // Clear transfer-related statuses
            player.statuses.remove(PlayerStatusType::Lst);
            player.statuses.remove(PlayerStatusType::Req);
            player.statuses.remove(PlayerStatusType::Unh);
            player.statuses.remove(PlayerStatusType::Trn);
            player.statuses.remove(PlayerStatusType::Bid);
            player.statuses.remove(PlayerStatusType::Wnt);
            player.statuses.remove(PlayerStatusType::Sct);

            // Create new contract with buying club
            let contract_years = if player.age(date) < 24 { 5 }
            else if player.age(date) < 28 { 4 }
            else if player.age(date) < 32 { 3 }
            else { 2 };

            let expiry = date.checked_add_signed(chrono::Duration::days(contract_years * 365))
                .unwrap_or(date);

            let salary = (fee / 200.0).max(500.0) as u32; // Rough weekly salary from fee

            player.contract = Some(PlayerClubContract::new(salary, expiry));

            // Add player to buying club's first team
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
        // Take player from selling club
        let mut player = None;
        let mut selling_team_name = String::new();
        let mut selling_team_slug = String::new();

        if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
            selling_team_name = selling_club.name.clone();

            for team in &mut selling_club.teams.teams {
                if let Some(p) = team.players.take_player(&player_id) {
                    player = Some(p);
                    selling_team_slug = team.slug.clone();
                    team.transfer_list.remove(player_id);
                    break;
                }
            }

            // Selling club receives loan fee
            selling_club.finance.add_transfer_income(loan_fee);
        }

        if let Some(mut player) = player {
            // Archive current stats with old club
            let season_year = if date.month() >= 8 {
                date.year() as u16
            } else {
                (date.year() - 1) as u16
            };

            let old_stats = std::mem::take(&mut player.statistics);
            player.statistics_history.items.push(PlayerStatisticsHistoryItem {
                season: Season::OneYear(season_year),
                team_name: selling_team_name,
                team_slug: selling_team_slug,
                is_loan: false,
                statistics: old_stats,
            });

            // Add loan history entry for the new club
            let mut buying_club_name = String::new();
            let mut buying_team_slug = String::new();
            if let Some(buying_club) = country.clubs.iter().find(|c| c.id == buying_club_id) {
                buying_club_name = buying_club.name.clone();
                if let Some(first_team) = buying_club.teams.teams.first() {
                    buying_team_slug = first_team.slug.clone();
                }
            }

            player.statistics_history.items.push(PlayerStatisticsHistoryItem {
                season: Season::OneYear(season_year),
                team_name: buying_club_name,
                team_slug: buying_team_slug,
                is_loan: true,
                statistics: PlayerStatistics::default(),
            });

            // Reset current stats
            player.statistics = PlayerStatistics::default();

            // Clear loan/transfer statuses
            player.statuses.remove(PlayerStatusType::Loa);
            player.statuses.remove(PlayerStatusType::Lst);
            player.statuses.remove(PlayerStatusType::Wnt);
            player.statuses.remove(PlayerStatusType::Sct);

            // Create a loan contract (6 months default)
            let loan_end = date
                .checked_add_signed(chrono::Duration::days(180))
                .unwrap_or(date);

            let salary = (loan_fee / 50.0).max(200.0) as u32;
            player.contract = Some(PlayerClubContract::new_loan(salary, loan_end));

            // Add player to buying club
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

    fn schedule_friendly_matches(_country: &mut Country, _date: NaiveDate) {
        debug!("Scheduling preseason friendlies");
    }

    fn organize_training_camps(_country: &mut Country) {
        debug!("Organizing training camps");
    }

    fn organize_preseason_tournaments(_country: &mut Country) {
        debug!("Organizing preseason tournaments");
    }

    fn process_loan_deals(country: &mut Country, date: NaiveDate, summary: &mut TransferActivitySummary) {
        let mut loan_negotiations = Vec::new();

        // First pass: find clubs interested in loan-listed players
        for buying_club in &country.clubs {
            let budget = buying_club.finance.transfer_budget.clone().unwrap_or(CurrencyValue {
                amount: (buying_club.finance.balance.balance.max(0) as f64) * 0.1,
                currency: Currency::Usd,
            });

            let analysis = Self::analyze_squad_needs(buying_club);

            let strategy = ClubTransferStrategy {
                club_id: buying_club.id,
                budget: Some(budget),
                selling_willingness: 0.5,
                buying_aggressiveness: Self::calculate_buying_aggressiveness(buying_club),
                target_positions: analysis.needed_positions,
                reputation_level: analysis.quality_level as u16,
            };

            let loan_listings: Vec<_> = country
                .transfer_market
                .get_available_listings()
                .into_iter()
                .filter(|l| l.listing_type == TransferListingType::Loan && l.club_id != buying_club.id)
                .cloned()
                .collect();

            for listing in loan_listings {
                if let Some(player) = Self::find_player_in_country(country, listing.player_id) {
                    if strategy.decide_player_interest(player) {
                        // Loan fee is typically much lower than transfer fee
                        let loan_fee = CurrencyValue {
                            amount: listing.asking_price.amount * 0.1,
                            currency: listing.asking_price.currency.clone(),
                        };

                        let offer = strategy.calculate_initial_offer(player, &loan_fee, date);

                        loan_negotiations.push((
                            listing.player_id,
                            buying_club.id,
                            listing.club_id,
                            offer,
                        ));
                    }
                }
            }
        }

        // Second pass: start loan negotiations (leave as Pending — resolved by resolve_pending_negotiations)
        for (player_id, buying_club_id, _selling_club_id, offer) in loan_negotiations {
            // Skip if already negotiating
            let already_negotiating = country.transfer_market.negotiations.values()
                .any(|n| n.player_id == player_id && n.buying_club_id == buying_club_id
                    && (n.status == crate::transfers::NegotiationStatus::Pending
                        || n.status == crate::transfers::NegotiationStatus::Countered));
            if already_negotiating {
                continue;
            }

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                player_id,
                buying_club_id,
                offer,
                date,
            ) {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_loan = true;
                }
                summary.active_negotiations += 1;
            }
        }
    }

    fn handle_free_agents(_country: &mut Country, _date: NaiveDate, _summary: &mut TransferActivitySummary) {
        debug!("Handling free agents");
    }

    fn calculate_league_competitiveness(_league: &crate::league::League) -> f32 {
        0.5 // Simplified
    }

    fn calculate_international_success(_country: &Country) -> i16 {
        0 // Simplified
    }

    fn calculate_transfer_market_reputation(_country: &Country) -> i16 {
        0 // Simplified
    }

    fn process_season_awards(_country: &mut Country, _club_results: &[ClubResult]) {
        debug!("Processing season awards");
    }

    fn process_contract_expirations(_country: &mut Country) {
        debug!("Processing contract expirations");
    }

    fn process_player_retirements(_country: &mut Country, _date: NaiveDate) {
        debug!("Processing player retirements");
    }

    fn process_year_end_finances(_country: &mut Country) {
        debug!("Processing year-end finances");
    }
}

// Supporting structures (keep these in country.rs)

#[allow(dead_code)]
#[derive(Debug)]
struct TransferActivitySummary {
    total_listings: u32,
    active_negotiations: u32,
    completed_transfers: u32,
    total_fees_exchanged: f64,
}

impl TransferActivitySummary {
    fn new() -> Self {
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
#[derive(Debug)]
struct SquadAnalysis {
    surplus_positions: Vec<crate::PlayerPositionType>,
    needed_positions: Vec<crate::PlayerPositionType>,
    average_age: f32,
    quality_level: u8,
}