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

        // Cap club-decided listings so no position group on a main team
        // drops below a minimum. Player-initiated (REQ/UNH) listings are
        // honoured even when this leaves the group short — the player
        // wants out and the club must replace him.
        let listings_to_add = Self::enforce_position_group_minimums(country, listings_to_add);

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

    /// Drop club-decided listings that would push a position group on the
    /// main team below a minimum. Player-initiated listings (REQ/UNH) and
    /// free-transfer releases for under-16s bypass the cap — those must
    /// be honoured regardless of depth.
    ///
    /// Without this, the pipeline's below-average / surplus / aging /
    /// contract-expiring paths can independently flag every goalkeeper
    /// in a club whose squad-wide CA average sits above the keepers', and
    /// the result is a team with zero recognised goalkeepers.
    fn enforce_position_group_minimums(
        country: &Country,
        listings: Vec<PendingListing>,
    ) -> Vec<PendingListing> {
        use std::collections::HashMap;

        const EXEMPT_REASONS: &[&str] = &[
            "dec_reason_player_requested",
            "dec_reason_player_unhappy",
            "dec_reason_under16_release",
        ];

        let (exempt, capped): (Vec<PendingListing>, Vec<PendingListing>) = listings
            .into_iter()
            .partition(|l| EXEMPT_REASONS.contains(&l.reason.as_str()));

        let find_main = |club_id: u32| {
            country
                .clubs
                .iter()
                .find(|c| c.id == club_id)
                .and_then(|c| c.teams.main())
        };

        let player_group = |club_id: u32, player_id: u32| {
            find_main(club_id).and_then(|t| {
                t.players
                    .players
                    .iter()
                    .find(|p| p.id == player_id)
                    .map(|p| p.position().position_group())
            })
        };

        let player_ca = |club_id: u32, player_id: u32| {
            find_main(club_id)
                .and_then(|t| t.players.players.iter().find(|p| p.id == player_id))
                .map(|p| p.player_attributes.current_ability)
                .unwrap_or(0)
        };

        let mut groups: HashMap<(u32, PlayerFieldPositionGroup), Vec<PendingListing>> =
            HashMap::new();
        for listing in capped {
            if let Some(group) = player_group(listing.club_id, listing.player_id) {
                groups.entry((listing.club_id, group)).or_default().push(listing);
            }
        }

        let mut result = exempt;

        for ((club_id, group), mut group_listings) in groups {
            let current_count = find_main(club_id)
                .map(|t| {
                    t.players
                        .iter()
                        .filter(|p| !p.is_on_loan())
                        .filter(|p| p.position().position_group() == group)
                        .count()
                })
                .unwrap_or(0);

            let exempt_in_group = result
                .iter()
                .filter(|l| l.club_id == club_id)
                .filter(|l| player_group(l.club_id, l.player_id) == Some(group))
                .count();

            // State-derived throttle: count players in this group that are
            // ALREADY on a transfer / loan / free-transfer list from
            // earlier passes. Each one occupies a "selling slot" until it
            // moves on, so the cap emerges naturally from squad state
            // instead of a hard-coded per-pass maximum. A club that has
            // already put two backups on the market can't list a third
            // this month; once one clears (either sells or gets delisted),
            // a new slot opens next cycle. Exempt listings (REQ / UNH)
            // aren't subject to this throttle — when the player wants out,
            // he goes regardless of how full the selling queue is.
            let already_listed_in_group = find_main(club_id)
                .map(|t| {
                    t.players
                        .iter()
                        .filter(|p| p.position().position_group() == group)
                        .filter(|p| {
                            let s = p.statuses.get();
                            s.contains(&PlayerStatusType::Lst)
                                || s.contains(&PlayerStatusType::Loa)
                                || s.contains(&PlayerStatusType::Frt)
                        })
                        .count()
                })
                .unwrap_or(0);

            let min_to_keep = min_squad_for_group(group);
            let slots_after_min = current_count.saturating_sub(min_to_keep);
            let max_can_list = slots_after_min
                .saturating_sub(exempt_in_group)
                .saturating_sub(already_listed_in_group);

            // Worst-CA players get listed first
            group_listings.sort_by_key(|l| player_ca(l.club_id, l.player_id));

            result.extend(group_listings.into_iter().take(max_can_list));
        }

        result
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

        // Manager has pinned this player to the squad — never auto-list.
        if player.is_force_match_selection {
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

        // Squad members the club wouldn't move on pure maths. Runs after
        // explicit decisions (NotNeeded / club-listed / REQ / UNH) so those
        // still dictate, but before numeric triggers so a club captain with
        // a few rating points below the squad mean isn't auto-sold.
        if Self::is_squad_protected(player, club, date) {
            return ListingDecision::Keep;
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

        // Aging players past their prime — only top clubs cycle aging
        // squad-average players out. Smaller clubs keep them to the end of
        // their careers: loyalty, shorter shopping lists, a 35-year-old
        // stalwart at a regional club is a feature, not a problem.
        if rep_level.cycles_aging_squad() {
            let aging_threshold = aging_listing_threshold(player.position().position_group());
            if age >= aging_threshold && ca_i < avg + 5 {
                return ListingDecision::Transfer {
                    reason: "dec_reason_aging_declining".to_string(),
                };
            }
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

        // Contract expiring within 6 months. ContractRenewalManager runs
        // monthly and targets players 12-18 months out, so only pull this
        // trigger after that system has had a chance — and failed — to
        // lock the player down. Earlier than 6 months and we pre-empt the
        // renewal flow on players the club actually wants to keep.
        if let Some(ref contract) = player.contract {
            let days_remaining = (contract.expiration - date).num_days();
            if days_remaining < 180 && days_remaining > 0 {
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

        // Aging AND peaked → transfer. "Aging" scales with position group
        // so a 30-year-old GK isn't treated the same as a 30-year-old
        // winger. Requires both conditions — the previous OR labelled any
        // 27-year-old who'd reached his potential as "peaked or declining",
        // which is simply a mature player, not a selling point.
        let peaked_age = aging_listing_threshold(player.position().position_group()).saturating_sub(2);
        if age >= peaked_age && pa <= ca {
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

    /// Is this a player the club would keep on non-numeric grounds?
    ///
    /// Real-world squad management keeps players whose value isn't
    /// captured by a CA/PA spreadsheet: formal squad-core designation,
    /// dressing-room leadership, and long-serving pros still contributing
    /// on the pitch. Player-initiated departures (REQ/UNH) and explicit
    /// club decisions (NotNeeded, club-listed) are evaluated earlier and
    /// bypass this — the club can still sell, the player can still ask
    /// out, but routine below-average/surplus/aging sweeps don't touch
    /// this tier.
    fn is_squad_protected(player: &Player, club: &Club, date: NaiveDate) -> bool {
        // Club has formally labelled the player as core to the project.
        if let Some(ref c) = player.contract {
            if matches!(
                c.squad_status,
                PlayerSquadStatus::KeyPlayer
                    | PlayerSquadStatus::FirstTeamRegular
                    | PlayerSquadStatus::HotProspectForTheFuture
            ) {
                return true;
            }
        }

        // Highest-CA player in his position group on the main team — i.e.
        // the de facto starter. squad_status is updated monthly, so at
        // simulation start (or before the first-of-month tick on a fresh
        // save) every player still has `NotYetSet` and can't be protected
        // via the formal-designation branch above. Without this fallback,
        // the starting goalkeeper at every club was fair game for the
        // numeric listing paths on day one.
        if let Some(main_team) = club.teams.teams.first() {
            let group = player.position().position_group();
            let player_ca = player.player_attributes.current_ability;
            let group_top_ca = main_team
                .players
                .iter()
                .filter(|p| p.position().position_group() == group)
                .filter(|p| !p.is_on_loan())
                .map(|p| p.player_attributes.current_ability)
                .max()
                .unwrap_or(0);
            if player_ca == group_top_ca && group_top_ca > 0 {
                return true;
            }
        }

        let age = player.age(date);

        // Dressing-room leader — strong leadership attribute + seasoned.
        // Skills are on the 1-20 scale; >=15 is genuine locker-room
        // authority, not just any veteran.
        if age >= 26 && player.skills.mental.leadership >= 15.0 {
            return true;
        }

        // Long-serving pro still delivering: tenure AND last-season form.
        let tenure_years = player
            .contract
            .as_ref()
            .and_then(|c| c.started)
            .map(|start| (date - start).num_days() / 365)
            .unwrap_or(0);

        let last_rating = player
            .statistics_history
            .items
            .last()
            .map(|h| h.statistics.average_rating)
            .unwrap_or(0.0);

        if tenure_years >= 4 && last_rating >= 6.9 {
            return true;
        }

        // Club stalwart — 6+ years regardless of recent form. Deep-backup
        // roles naturally produce thin playing records (and thus no form
        // data or low ratings from few appearances); the tenure+form
        // branch above punishes them unfairly. Six-year loyalty earned
        // patience from the dressing room and, typically, the boardroom.
        if tenure_years >= 6 {
            return true;
        }

        // Experienced goalkeeper — keepers have the longest careers of
        // any position and #2/#3 veterans are kept on specifically to
        // mentor the starter, cover injuries, and anchor the dressing
        // room. Pure CA-vs-squad-average maths lists them every season;
        // real clubs do the opposite. Antonio Chimenti spent eight years
        // as Juventus backup without being listed. Equivalent carve-outs
        // for outfield positions aren't warranted — those roles turn
        // over much faster.
        let group = player.position().position_group();
        if group == PlayerFieldPositionGroup::Goalkeeper && age >= 30 {
            return true;
        }

        false
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

/// Age at which a mid-tier player at or below squad average is considered
/// "past his prime" for transfer-listing purposes. Mirrors real-world
/// career lengths: keepers last longest, forwards (speed-dependent)
/// decline first, defenders and holding midfielders sit in between.
fn aging_listing_threshold(group: PlayerFieldPositionGroup) -> u8 {
    match group {
        PlayerFieldPositionGroup::Goalkeeper => 37,
        PlayerFieldPositionGroup::Defender => 34,
        PlayerFieldPositionGroup::Midfielder => 33,
        PlayerFieldPositionGroup::Forward => 32,
    }
}

/// Minimum number of main-team players a club must retain per position
/// group after any club-decided transfer/loan listings in a single pass.
/// Player-initiated listings (REQ/UNH) bypass this cap.
fn min_squad_for_group(group: PlayerFieldPositionGroup) -> usize {
    match group {
        PlayerFieldPositionGroup::Goalkeeper => 2,
        PlayerFieldPositionGroup::Defender => 6,
        PlayerFieldPositionGroup::Midfielder => 6,
        PlayerFieldPositionGroup::Forward => 3,
    }
}
