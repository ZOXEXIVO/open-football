use chrono::NaiveDate;
use log::debug;

use crate::shared::{Currency, CurrencyValue};
use crate::transfers::market::{TransferListing, TransferListingStatus, TransferListingType};
use crate::transfers::offer::{TransferClause, TransferOffer};
use crate::transfers::pipeline::processor::{PipelineProcessor, PlayerSummary};
use crate::transfers::pipeline::{LoanOutStatus, TransferRequest, TransferRequestStatus};
use crate::transfers::window::PlayerValuationCalculator;
use crate::transfers::ScoutingRegion;
use crate::utils::FormattingUtils;
use crate::{
    ClubPhilosophy, Country, Person, PlayerFieldPositionGroup, PlayerStatusType, ReputationLevel,
};

impl PipelineProcessor {
    // ============================================================
    // Step 6.5: Scan Loan Market — Small clubs proactively seek loans
    // ============================================================

    pub fn scan_loan_market(country: &mut Country, date: NaiveDate) {
        let is_january = Self::is_january_window(date);

        // Collect available loan listings (Pass 1 read)
        struct LoanListing {
            player_id: u32,
            club_id: u32,
            asking_price: f64,
            ability: u8,
            age: u8,
            position_group: PlayerFieldPositionGroup,
        }

        let mut loan_listings: Vec<LoanListing> = Vec::new();

        for listing in &country.transfer_market.listings {
            if listing.listing_type != TransferListingType::Loan {
                continue;
            }
            if listing.status != TransferListingStatus::Available {
                continue;
            }
            if let Some(player) = Self::find_player_in_country(country, listing.player_id) {
                // Skip players already on loan — can't re-loan
                if player.is_on_loan() {
                    continue;
                }
                loan_listings.push(LoanListing {
                    player_id: listing.player_id,
                    club_id: listing.club_id,
                    asking_price: listing.asking_price.amount,
                    ability: player.player_attributes.current_ability,
                    age: player.age(date),
                    position_group: player.position().position_group(),
                });
            }
        }

        if loan_listings.is_empty() {
            return;
        }

        // Collect club scanning actions (Pass 1 read)
        struct LoanScanAction {
            club_id: u32,
            player_id: u32,
            selling_club_id: u32,
            offer_amount: f64,
            reason: String,
        }

        let mut actions: Vec<LoanScanAction> = Vec::new();

        for club in &country.clubs {
            if club.teams.teams.is_empty() {
                continue;
            }

            let team = &club.teams.teams[0];
            let rep_level = team.reputation.level();

            // Determine if club should scan — philosophy overrides reputation defaults.
            // LoanFocused clubs always scan; SignToCompete clubs almost never loan.
            let should_scan = match &club.philosophy {
                ClubPhilosophy::LoanFocused => true,
                ClubPhilosophy::SignToCompete => {
                    // Only loan as emergency cover in January
                    is_january && club.finance.balance.balance < 0
                }
                _ => match rep_level {
                    ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur => true,
                    ReputationLevel::National => is_january || club.finance.balance.balance < 0,
                    ReputationLevel::Continental => is_january && club.finance.balance.balance < 0,
                    ReputationLevel::Elite => false,
                },
            };

            if !should_scan {
                continue;
            }

            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            // Respect concurrent negotiation limits
            let actual_active = country
                .transfer_market
                .active_negotiation_count_for_club(club.id);
            if actual_active >= plan.max_concurrent_negotiations {
                continue;
            }

            let balance = club.finance.balance.balance;
            let max_loan_fee = if balance < 0 {
                50_000.0
            } else {
                balance as f64 * 0.20
            };

            let max_scans: usize = match rep_level {
                ReputationLevel::Local | ReputationLevel::Amateur => 4,
                ReputationLevel::Regional => 3,
                ReputationLevel::National => 2,
                _ => 1,
            };
            let mut scans_this_club = 0usize;

            let avg_ability = {
                let avg = team.players.current_ability_avg();
                if avg == 0 { 50 } else { avg }
            };

            // Track position groups already targeted in this scan pass
            // to avoid starting multiple negotiations for the same position
            let mut scanned_position_groups: Vec<PlayerFieldPositionGroup> = Vec::new();

            // Track position group depth + best ability to prevent bloat
            // while still allowing upgrades. A club with 3 mediocre GKs should
            // still loan a world-class GK, but not a 4th mediocre one.
            struct PositionDepth {
                group: PlayerFieldPositionGroup,
                count: usize,
                max: usize,
                best_ability: u8,
            }

            let position_depth: Vec<PositionDepth> = [
                (PlayerFieldPositionGroup::Goalkeeper, 3usize),
                (PlayerFieldPositionGroup::Defender, 8),
                (PlayerFieldPositionGroup::Midfielder, 8),
                (PlayerFieldPositionGroup::Forward, 6),
            ].iter().map(|&(group, max)| {
                let players_at_pos: Vec<u8> = team.players.iter()
                    .filter(|p| p.position().position_group() == group)
                    .map(|p| p.player_attributes.current_ability)
                    .collect();
                PositionDepth {
                    group,
                    count: players_at_pos.len(),
                    max,
                    best_ability: players_at_pos.iter().copied().max().unwrap_or(0),
                }
            }).collect();

            let should_skip_loan = |group: PlayerFieldPositionGroup, loan_ability: u8| -> bool {
                if let Some(depth) = position_depth.iter().find(|d| d.group == group) {
                    if depth.count < depth.max {
                        return false; // Still have room — always allow
                    }
                    // Position is full — only allow if the loan player is clearly
                    // better than the best we have (upgrade, not bloat)
                    loan_ability <= depth.best_ability
                } else {
                    false
                }
            };

            // Check unfulfilled transfer requests first
            let unfulfilled: Vec<&TransferRequest> = plan
                .transfer_requests
                .iter()
                .filter(|r| {
                    r.status != TransferRequestStatus::Fulfilled
                        && r.status != TransferRequestStatus::Abandoned
                })
                .collect();

            for request in &unfulfilled {
                if scans_this_club >= max_scans {
                    break;
                }

                // Only scan once per position group — multiple requests for the same
                // group (e.g. FormationGap + DepthCover for GK) should not each trigger
                // a separate loan negotiation
                let pos_group = request.position.position_group();
                if scanned_position_groups.contains(&pos_group) {
                    continue;
                }

                // Skip if position is full AND loan wouldn't be an upgrade
                if should_skip_loan(pos_group, request.min_ability) {
                    continue;
                }

                // Relaxed thresholds: min_ability - 5, age_max + 3
                let relaxed_min = request.min_ability.saturating_sub(5);
                let relaxed_age_max = request.preferred_age_max.saturating_add(3);

                if let Some(best) = loan_listings
                    .iter()
                    .filter(|l| {
                        l.club_id != club.id
                            && !club.is_rival(l.club_id) // no loans from rivals
                            && l.position_group == pos_group
                            && l.ability >= relaxed_min
                            && l.age <= relaxed_age_max
                            && l.age >= request.preferred_age_min
                            && l.asking_price * 0.8 <= max_loan_fee
                            && !country
                                .transfer_market
                                .has_active_negotiation_for(l.player_id, club.id)
                            && !actions.iter().any(|a| a.player_id == l.player_id)
                    })
                    .max_by_key(|l| l.ability)
                {
                    let reason = format!("Loan signing — {}", Self::transfer_need_reason_text(&request.reason));
                    actions.push(LoanScanAction {
                        club_id: club.id,
                        player_id: best.player_id,
                        selling_club_id: best.club_id,
                        offer_amount: FormattingUtils::round_fee(best.asking_price * 0.8),
                        reason,
                    });
                    scanned_position_groups.push(pos_group);
                    scans_this_club += 1;
                }
            }

            // Opportunistic scan: small clubs always look for deals, not just in January
            let is_small_club = matches!(
                rep_level,
                ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur
            );

            // Small clubs scan for available loan players to strengthen their squad
            if is_small_club && scans_this_club < max_scans {
                let mut opps: Vec<&LoanListing> = loan_listings
                    .iter()
                    .filter(|l| {
                        l.club_id != club.id
                            && !club.is_rival(l.club_id)
                            && l.ability >= avg_ability.saturating_sub(5)
                            && l.asking_price * 0.8 <= max_loan_fee
                            && !country
                                .transfer_market
                                .has_active_negotiation_for(l.player_id, club.id)
                            && !actions.iter().any(|a| a.player_id == l.player_id)
                            && !scanned_position_groups.contains(&l.position_group)
                            && !should_skip_loan(l.position_group, l.ability)
                    })
                    .collect();
                opps.sort_by(|a, b| b.ability.cmp(&a.ability));

                for opp in opps.iter().take(max_scans - scans_this_club) {
                    actions.push(LoanScanAction {
                        club_id: club.id,
                        player_id: opp.player_id,
                        selling_club_id: opp.club_id,
                        offer_amount: FormattingUtils::round_fee(opp.asking_price * 0.8),
                        reason: "Loan signing — opportunistic squad upgrade".to_string(),
                    });
                    scanned_position_groups.push(opp.position_group);
                    scans_this_club += 1;
                }
            }

            // January extra: even National clubs look for opportunistic loans
            if is_january && scans_this_club < max_scans && !is_small_club {
                if let Some(opp) = loan_listings
                    .iter()
                    .filter(|l| {
                        l.club_id != club.id
                            && !club.is_rival(l.club_id)
                            && l.ability >= avg_ability.saturating_sub(8)
                            && l.asking_price * 0.8 <= max_loan_fee
                            && !country
                                .transfer_market
                                .has_active_negotiation_for(l.player_id, club.id)
                            && !actions.iter().any(|a| a.player_id == l.player_id)
                            && !scanned_position_groups.contains(&l.position_group)
                            && !should_skip_loan(l.position_group, l.ability)
                    })
                    .max_by_key(|l| l.ability)
                {
                    actions.push(LoanScanAction {
                        club_id: club.id,
                        player_id: opp.player_id,
                        selling_club_id: opp.club_id,
                        offer_amount: FormattingUtils::round_fee(opp.asking_price * 0.8),
                        reason: "Loan signing — January window reinforcement".to_string(),
                    });
                }
            }
        }

        // Pass 2: Start loan negotiations
        for action in actions {
            let selling_rep = Self::get_club_reputation(country, action.selling_club_id);
            let buying_rep = Self::get_club_reputation(country, action.club_id);
            let (p_age, p_ambition) =
                Self::get_player_negotiation_data(country, action.player_id, date);

            let mut clauses = Vec::new();

            // Add appearance fee clause for high-reputation selling clubs
            let selling_rep_level = Self::get_club_reputation_level(country, action.selling_club_id);
            match selling_rep_level {
                ReputationLevel::Elite => {
                    clauses.push(TransferClause::AppearanceFee(
                        CurrencyValue {
                            amount: FormattingUtils::round_fee(action.offer_amount * 0.30),
                            currency: Currency::Usd,
                        },
                        10,
                    ));
                }
                ReputationLevel::Continental => {
                    clauses.push(TransferClause::AppearanceFee(
                        CurrencyValue {
                            amount: FormattingUtils::round_fee(action.offer_amount * 0.20),
                            currency: Currency::Usd,
                        },
                        15,
                    ));
                }
                _ => {}
            }

            let offer = TransferOffer {
                base_fee: CurrencyValue {
                    amount: action.offer_amount,
                    currency: Currency::Usd,
                },
                clauses,
                salary_contribution: None,
                contract_length: Some(1),
                offering_club_id: action.club_id,
                offered_date: date,
            };

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                action.player_id,
                action.club_id,
                offer,
                date,
                selling_rep,
                buying_rep,
                p_age,
                p_ambition,
            ) {
                // Resolve player and club names
                let (p_name, sc_name) = Self::resolve_player_and_club_name(country, action.player_id, action.selling_club_id);

                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_loan = true;
                    negotiation.is_unsolicited = false;
                    negotiation.reason = action.reason.clone();
                    negotiation.player_name = p_name;
                    negotiation.selling_club_name = sc_name;
                }

                debug!(
                    "Loan scan: Club {} started loan negotiation for player {}",
                    action.club_id, action.player_id
                );
            }
        }
    }

    // ============================================================
    // Step 7b: Loan Market Scanning (other countries)
    // ============================================================

    pub fn scan_foreign_loan_market(
        country: &mut Country,
        foreign_players: &[PlayerSummary],
        date: NaiveDate,
    ) {
        if foreign_players.is_empty() {
            return;
        }

        let is_january = Self::is_january_window(date);

        // The scanning country's own region — used to block loans from
        // clearly more prestigious regions (Paraguay can't loan from England).
        let club_region = ScoutingRegion::from_country(
            country.continent_id,
            &country.code,
        );
        let club_region_prestige = club_region.league_prestige();

        // Collect loan-listed foreign players
        // Only consider players from countries with equal or lower reputation,
        // and whose home region isn't far above the scanning club's region.
        let country_rep = country.reputation;
        let foreign_loans: Vec<&PlayerSummary> = foreign_players
            .iter()
            .filter(|p| {
                if !p.is_loan_listed || p.country_reputation > country_rep {
                    return false;
                }
                let player_region = ScoutingRegion::from_country(
                    p.continent_id,
                    &p.country_code,
                );
                // Block cross-region loans where the player's home region is
                // significantly more prestigious. Real players don't loan down
                // into a clearly smaller football ecosystem for a bit-part role.
                player_region.league_prestige() <= club_region_prestige + 0.20
            })
            .collect();

        if foreign_loans.is_empty() {
            return;
        }

        struct ForeignLoanAction {
            club_id: u32,
            player: PlayerSummary,
            offer_amount: f64,
            reason: String,
        }

        let mut actions: Vec<ForeignLoanAction> = Vec::new();

        for club in &country.clubs {
            if club.teams.teams.is_empty() {
                continue;
            }

            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            let team = &club.teams.teams[0];
            let rep_level = team.reputation.level();

            // Elite clubs buy — they don't scan loan markets.
            // Continental only in January with negative balance.
            // Local/Amateur don't have the scouting reach for foreign markets.
            let should_scan_foreign = match rep_level {
                ReputationLevel::Elite => false,
                ReputationLevel::Continental => {
                    is_january && club.finance.balance.balance < 0
                }
                ReputationLevel::National | ReputationLevel::Regional => true,
                _ => false, // Local/Amateur
            };
            if !should_scan_foreign {
                continue;
            }

            // Check concurrent negotiation limits
            let actual_active = country
                .transfer_market
                .active_negotiation_count_for_club(club.id);
            if actual_active >= plan.max_concurrent_negotiations {
                continue;
            }

            let balance = club.finance.balance.balance;
            let max_loan_fee = if balance < 0 {
                30_000.0
            } else {
                (balance as f64 * 0.15).min(500_000.0)
            };

            let avg_ability = {
                let avg = team.players.current_ability_avg();
                if avg == 0 { 50 } else { avg }
            };

            // Get scout known regions for this club
            let scout_regions: Vec<ScoutingRegion> = club
                .teams
                .teams
                .iter()
                .flat_map(|t| t.staffs.iter())
                .flat_map(|s| s.staff_attributes.knowledge.known_regions.iter().copied())
                .collect();

            // Check unfulfilled transfer requests
            let unfulfilled: Vec<&TransferRequest> = plan
                .transfer_requests
                .iter()
                .filter(|r| {
                    r.status != TransferRequestStatus::Fulfilled
                        && r.status != TransferRequestStatus::Abandoned
                })
                .collect();

            let mut scans = 0usize;
            let max_scans: usize = match rep_level {
                ReputationLevel::Elite => 3,
                ReputationLevel::Continental => 2,
                _ => 1,
            };

            // Track position groups already targeted to avoid multiple negotiations
            // for the same position (e.g. FormationGap + DepthCover for GK)
            let mut scanned_position_groups: Vec<PlayerFieldPositionGroup> = Vec::new();

            for request in &unfulfilled {
                if scans >= max_scans {
                    break;
                }

                let pos_group = request.position.position_group();
                if scanned_position_groups.contains(&pos_group) {
                    continue;
                }

                let relaxed_min = request.min_ability.saturating_sub(5);

                // Filter foreign loan players: must match position, ability,
                // be in a scout's known region, and be a realistic move
                // (players don't go from Serie A to the Nigerian league)
                let team_rep = team.reputation.world;
                if let Some(best) = foreign_loans
                    .iter()
                    .filter(|p| {
                        !club.is_rival(p.club_id)
                            && p.position_group == request.position.position_group()
                            && p.skill_ability >= relaxed_min
                            && p.age <= request.preferred_age_max.saturating_add(3)
                            && p.age >= request.preferred_age_min
                            && p.estimated_value * 0.1 <= max_loan_fee
                            && p.skill_ability >= avg_ability.saturating_sub(10)
                            && !country
                                .transfer_market
                                .has_active_negotiation_for(p.player_id, club.id)
                            && !actions.iter().any(|a| a.player.player_id == p.player_id)
                            // Reputation reality check: players don't drop more than
                            // ~40% in league level on loan. A rep-8000 player won't
                            // go to a rep-2000 club. This prevents Serie A → Nigeria.
                            && p.home_reputation <= (team_rep as f32 * 2.0) as i16
                            && team_rep >= (p.home_reputation.max(0) as f32 * 0.35) as u16
                            && {
                                let player_region = ScoutingRegion::from_country(
                                    p.continent_id, &p.country_code,
                                );
                                scout_regions.contains(&player_region)
                            }
                    })
                    .max_by_key(|p| p.skill_ability)
                {
                    let loan_fee = FormattingUtils::round_fee(
                        best.estimated_value * 0.1 * 0.8,
                    );
                    let reason = format!(
                        "Loan signing",
                    );
                    actions.push(ForeignLoanAction {
                        club_id: club.id,
                        player: PlayerSummary {
                            player_id: best.player_id,
                            club_id: best.club_id,
                            country_id: best.country_id,
                            continent_id: best.continent_id,
                            country_code: best.country_code.clone(),
                            player_name: best.player_name.clone(),
                            club_name: best.club_name.clone(),
                            position: best.position,
                            position_group: best.position_group,
                            age: best.age,
                            estimated_value: best.estimated_value,
                            is_listed: best.is_listed,
                            is_loan_listed: best.is_loan_listed,
                            skill_ability: best.skill_ability,
                            average_rating: best.average_rating,
                            goals: best.goals,
                            assists: best.assists,
                            appearances: best.appearances,
                            determination: best.determination,
                            work_rate: best.work_rate,
                            composure: best.composure,
                            anticipation: best.anticipation,
                            technical_avg: best.technical_avg,
                            mental_avg: best.mental_avg,
                            physical_avg: best.physical_avg,
                            current_reputation: best.current_reputation,
                            home_reputation: best.home_reputation,
                            world_reputation: best.world_reputation,
                            country_reputation: best.country_reputation,
                            is_injured: best.is_injured,
                            contract_months_remaining: best.contract_months_remaining,
                            salary: best.salary,
                        },
                        offer_amount: loan_fee,
                        reason,
                    });
                    scanned_position_groups.push(pos_group);
                    scans += 1;
                }
            }
        }

        if actions.is_empty() {
            return;
        }

        // Create listings and negotiations for foreign loan targets
        for action in actions {
            let asking_price = CurrencyValue {
                amount: action.offer_amount,
                currency: Currency::Usd,
            };

            let listing = TransferListing::new(
                action.player.player_id,
                action.player.club_id,
                0, // Foreign team — no local team_id
                asking_price.clone(),
                date,
                TransferListingType::Loan,
            );
            country.transfer_market.add_listing(listing);

            let buying_rep = Self::get_club_reputation(country, action.club_id);
            // Use a reasonable estimate for selling club rep
            let selling_rep = (action.player.skill_ability as f32 / 200.0).clamp(0.1, 0.9);

            let offer = TransferOffer {
                base_fee: asking_price,
                clauses: Vec::new(),
                salary_contribution: None,
                contract_length: Some(1),
                offering_club_id: action.club_id,
                offered_date: date,
            };

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                action.player.player_id,
                action.club_id,
                offer,
                date,
                selling_rep,
                buying_rep,
                action.player.age,
                action.player.determination,
            ) {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_loan = true;
                    negotiation.is_unsolicited = false;
                    negotiation.reason = action.reason;
                    negotiation.selling_country_id = Some(action.player.country_id);
                    negotiation.selling_continent_id = Some(action.player.continent_id);
                    negotiation.selling_country_code = action.player.country_code.clone();
                    negotiation.player_name = action.player.player_name.clone();
                    negotiation.selling_club_name = action.player.club_name.clone();
                }

                debug!(
                    "Foreign loan scan: Club {} started foreign loan negotiation for player {} from country {}",
                    action.club_id, action.player.player_id, action.player.country_id
                );
            }
        }
    }

    /// List loan-out candidates on the transfer market.
    pub(super) fn process_loan_out_listings(country: &mut Country, date: NaiveDate) {
        let price_level = country.settings.pricing.price_level;
        let mut listings_to_add: Vec<(u32, TransferListing)> = Vec::new();

        for club in &country.clubs {
            for candidate in &club.transfer_plan.loan_out_candidates {
                if candidate.status != LoanOutStatus::Identified {
                    continue;
                }

                if country
                    .transfer_market
                    .get_listing_by_player(candidate.player_id)
                    .is_some()
                {
                    continue;
                }

                if let Some(player) = Self::find_player_in_club(club, candidate.player_id) {
                    if player.is_on_loan() {
                        continue;
                    }

                    let team_id = club
                        .teams
                        .teams
                        .first()
                        .map(|t| t.id)
                        .unwrap_or(0);

                    let asking_price = if candidate.loan_fee > 0.0 {
                        CurrencyValue {
                            amount: candidate.loan_fee,
                            currency: Currency::Usd,
                        }
                    } else {
                        // Loan fee is ~10% of player value, not full value
                        let full_value = PlayerValuationCalculator::calculate_value_with_price_level(
                            player,
                            date,
                            price_level,
                            0, 0,
                        );
                        CurrencyValue {
                            amount: FormattingUtils::round_fee(full_value.amount * 0.10),
                            currency: full_value.currency,
                        }
                    };

                    let listing = TransferListing::new(
                        candidate.player_id,
                        club.id,
                        team_id,
                        asking_price,
                        date,
                        TransferListingType::Loan,
                    );

                    listings_to_add.push((club.id, listing));
                }
            }
        }

        for (club_id, listing) in listings_to_add {
            let player_id = listing.player_id;
            country.transfer_market.add_listing(listing);

            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) {
                if let Some(candidate) = club
                    .transfer_plan
                    .loan_out_candidates
                    .iter_mut()
                    .find(|c| c.player_id == player_id)
                {
                    candidate.status = LoanOutStatus::Listed;
                }

                for team in &mut club.teams.teams {
                    if let Some(player) = team
                        .players
                        .players
                        .iter_mut()
                        .find(|p| p.id == player_id)
                    {
                        if !player.statuses.get().contains(&PlayerStatusType::Loa) {
                            player.statuses.add(date, PlayerStatusType::Loa);
                        }
                    }
                }
            }
        }
    }
}
