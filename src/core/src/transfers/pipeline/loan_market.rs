use chrono::NaiveDate;
use log::debug;

use crate::shared::{Currency, CurrencyValue};
use crate::transfers::ScoutingRegion;
use crate::transfers::market::{TransferListing, TransferListingStatus, TransferListingType};
use crate::transfers::offer::{TransferClause, TransferOffer};
use crate::transfers::pipeline::plausibility::{
    BuyerPlausibilityContext, TransferPlausibilityBuilder, TransferPlausibilityVerdict,
};
use crate::transfers::pipeline::processor::{PipelineProcessor, PlayerSummary};
use crate::transfers::pipeline::{LoanOutStatus, TransferRequestStatus};
use crate::transfers::window::PlayerValuationCalculator;
use crate::utils::FormattingUtils;
use crate::{
    ClubPhilosophy, Country, Person, PlayerFieldPositionGroup, PlayerStatusType, ReputationLevel,
    Team,
};

// Loans fund short-term development or rotation minutes. Players older
// than this are signed cheap permanent (or as free agents) rather than
// loaned, so loan targeting above it is noise regardless of whether the
// move is request-driven or opportunistic.
const MAX_LOAN_TARGET_AGE: u8 = 34;

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
            /// Parent club's main-team world reputation — drives the
            /// reputation-drop realism gate on the borrower side.
            parent_rep: u16,
            /// Best CA at the player's position group on the parent's
            /// main roster. A player far below it is "very raw" and
            /// tolerates a much bigger reputation drop.
            parent_best_in_group: u8,
            /// Parent listed this player via the development pathway —
            /// the loan exists to buy minutes, so the borrower-side
            /// expected-minutes gate runs at its stricter bar.
            is_development: bool,
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
                let group = player.position().position_group();
                let parent_club = country.clubs.iter().find(|c| c.id == listing.club_id);
                let parent_team =
                    parent_club.and_then(|c| c.teams.main().or(c.teams.teams.first()));
                let parent_rep = parent_team.map(|t| t.reputation.world).unwrap_or(0);
                let parent_best_in_group = parent_team
                    .map(|t| {
                        t.players
                            .iter()
                            .filter(|p| p.position().position_group() == group)
                            .map(|p| p.player_attributes.current_ability)
                            .max()
                            .unwrap_or(0)
                    })
                    .unwrap_or(0);
                // Treat every game-time-driven loan (development pathway,
                // blocked prospect, needs-minutes) as a "development" move
                // for the borrower-side minutes gate: the player must land
                // somewhere he will actually play, not behind a wall of
                // better names. Pure surplus / financial loans keep the
                // looser cover bar.
                let is_development = parent_club
                    .map(|c| {
                        c.transfer_plan.loan_out_candidates.iter().any(|cand| {
                            cand.player_id == listing.player_id
                                && cand.reason.expects_guaranteed_minutes()
                        })
                    })
                    .unwrap_or(false);
                loan_listings.push(LoanListing {
                    player_id: listing.player_id,
                    club_id: listing.club_id,
                    asking_price: listing.asking_price.amount,
                    ability: player.player_attributes.current_ability,
                    age: player.age(date),
                    position_group: group,
                    parent_rep,
                    parent_best_in_group,
                    is_development,
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

            // Critical-need override: a club whose squad is genuinely
            // short at any position MUST scan the loan market, even
            // outside its usual scanning window. Without this, a
            // National-rep club that loses both senior GKs in October
            // would otherwise wait until January to cover — leaving
            // them fielding youth keepers for two months.
            //
            // Threshold: < 2 at GK, < 4 at DEF/MID, < 2 at FWD. Below
            // these and the team genuinely cannot field a balanced
            // matchday squad.
            let has_critical_shortage = {
                use crate::PlayerFieldPositionGroup as G;
                let mut counts = [0usize; 4]; // GK, DEF, MID, FWD
                for p in team.players.iter() {
                    let idx = match p.position().position_group() {
                        G::Goalkeeper => 0,
                        G::Defender => 1,
                        G::Midfielder => 2,
                        G::Forward => 3,
                    };
                    counts[idx] += 1;
                }
                counts[0] < 2 || counts[1] < 4 || counts[2] < 4 || counts[3] < 2
            };

            // Determine if club should scan — philosophy overrides reputation defaults.
            // LoanFocused clubs always scan; SignToCompete clubs almost never loan.
            let should_scan = has_critical_shortage
                || match &club.philosophy {
                    ClubPhilosophy::LoanFocused => true,
                    ClubPhilosophy::SignToCompete => {
                        // Only loan as emergency cover in January
                        is_january && club.finance.balance.balance < 0
                    }
                    _ => match rep_level {
                        ReputationLevel::Regional
                        | ReputationLevel::Local
                        | ReputationLevel::Amateur => true,
                        ReputationLevel::National => is_january || club.finance.balance.balance < 0,
                        ReputationLevel::Continental => {
                            is_january && club.finance.balance.balance < 0
                        }
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

            // Borrower-side depth snapshot — shared with the foreign
            // scan. A club with 3 mediocre GKs should still loan a
            // world-class GK, but not a 4th mediocre one.
            let borrower_depth = BorrowerPositionDepth::snapshot(team);
            let borrower_world_rep = team.reputation.world;

            let should_skip_loan = |group: PlayerFieldPositionGroup, loan_ability: u8| -> bool {
                !borrower_depth.has_room_for(group, loan_ability)
            };

            // Check unfulfilled transfer requests first. Emergency
            // free-agent depth requests are excluded — they're
            // serviced by the free-agent matcher only, not by loans.
            let unfulfilled = plan.transfer_requests.iter().filter(|r| {
                r.status != TransferRequestStatus::Fulfilled
                    && r.status != TransferRequestStatus::Abandoned
                    && !r.is_emergency_free_agent_depth()
            });

            for request in unfulfilled {
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
                let relaxed_age_max = request
                    .preferred_age_max
                    .saturating_add(3)
                    .min(MAX_LOAN_TARGET_AGE);

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
                            // Development realism: the move must buy
                            // minutes, and the reputation drop from the
                            // parent must stay plausible.
                            && borrower_depth.would_get_loan_minutes(
                                l.position_group,
                                l.ability,
                                l.is_development,
                            )
                            && Self::loan_reputation_drop_ok(
                                borrower_world_rep,
                                l.parent_rep,
                                l.ability,
                                l.parent_best_in_group,
                            )
                    })
                    .max_by_key(|l| l.ability)
                {
                    let reason = format!(
                        "Loan signing — {}",
                        Self::transfer_need_reason_text(&request.reason)
                    );
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

            // Opportunistic scan: small clubs always look for deals,
            // not just in January. National clubs join in too when
            // their squad has a genuine shortage — the critical-need
            // override above already let them in past `should_scan`,
            // but the opportunistic branch was small-club-only and
            // would otherwise leave them empty-handed.
            let is_small_club = matches!(
                rep_level,
                ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur
            );

            // Small clubs (always) and National clubs in critical
            // shortage scan for available loan players.
            if (is_small_club || has_critical_shortage) && scans_this_club < max_scans {
                let mut opps: Vec<&LoanListing> = loan_listings
                    .iter()
                    .filter(|l| {
                        l.club_id != club.id
                            && !club.is_rival(l.club_id)
                            && l.age <= MAX_LOAN_TARGET_AGE
                            && l.ability >= avg_ability.saturating_sub(5)
                            && l.asking_price * 0.8 <= max_loan_fee
                            && !country
                                .transfer_market
                                .has_active_negotiation_for(l.player_id, club.id)
                            && !actions.iter().any(|a| a.player_id == l.player_id)
                            && !scanned_position_groups.contains(&l.position_group)
                            && !should_skip_loan(l.position_group, l.ability)
                            && borrower_depth.would_get_loan_minutes(
                                l.position_group,
                                l.ability,
                                l.is_development,
                            )
                            && Self::loan_reputation_drop_ok(
                                borrower_world_rep,
                                l.parent_rep,
                                l.ability,
                                l.parent_best_in_group,
                            )
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
                            && l.age <= MAX_LOAN_TARGET_AGE
                            && l.ability >= avg_ability.saturating_sub(8)
                            && l.asking_price * 0.8 <= max_loan_fee
                            && !country
                                .transfer_market
                                .has_active_negotiation_for(l.player_id, club.id)
                            && !actions.iter().any(|a| a.player_id == l.player_id)
                            && !scanned_position_groups.contains(&l.position_group)
                            && !should_skip_loan(l.position_group, l.ability)
                            && borrower_depth.would_get_loan_minutes(
                                l.position_group,
                                l.ability,
                                l.is_development,
                            )
                            && Self::loan_reputation_drop_ok(
                                borrower_world_rep,
                                l.parent_rep,
                                l.ability,
                                l.parent_best_in_group,
                            )
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
            let selling_rep_level =
                Self::get_club_reputation_level(country, action.selling_club_id);
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

            // Loans run for a season — set the explicit duration field
            // (months) rather than the permanent-contract years field so
            // the market history doesn't double-encode "1" as both 1 year
            // and 1 month.
            let offer = TransferOffer {
                base_fee: CurrencyValue {
                    amount: action.offer_amount,
                    currency: Currency::Usd,
                },
                clauses,
                salary_contribution: None,
                contract_length_years: None,
                loan_duration_months: Some(10),
                personal_terms: None,
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
                let (p_name, sc_name) = Self::resolve_player_and_club_name(
                    country,
                    action.player_id,
                    action.selling_club_id,
                );

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
        let club_region = ScoutingRegion::from_country(country.continent_id, &country.code);
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
                let player_region = ScoutingRegion::from_country(p.continent_id, &p.country_code);
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
                ReputationLevel::Continental => is_january && club.finance.balance.balance < 0,
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

            // Check unfulfilled transfer requests. Emergency
            // free-agent depth requests are excluded — they're
            // serviced by the free-agent matcher only, not by loans.
            let unfulfilled = plan.transfer_requests.iter().filter(|r| {
                r.status != TransferRequestStatus::Fulfilled
                    && r.status != TransferRequestStatus::Abandoned
                    && !r.is_emergency_free_agent_depth()
            });

            let mut scans = 0usize;
            let max_scans: usize = match rep_level {
                ReputationLevel::Elite => 3,
                ReputationLevel::Continental => 2,
                _ => 1,
            };

            // Track position groups already targeted to avoid multiple negotiations
            // for the same position (e.g. FormationGap + DepthCover for GK)
            let mut scanned_position_groups: Vec<PlayerFieldPositionGroup> = Vec::new();

            // Borrower-side depth snapshot — the same gate the
            // domestic scan uses to avoid loaning into an already-full
            // position. Building it once here keeps the filter inside
            // `foreign_loans.iter()` cheap.
            let borrower_position_depth = BorrowerPositionDepth::snapshot(team);

            // Staged-plausibility buyer context, built once per club so the
            // foreign-loan filter can run the same cross-border veto the
            // scouting / permanent paths use. The candidate `PlayerSummary`
            // carries the seller-side context, so no selling-country ref is
            // needed here.
            let buyer_loan_ctx = BuyerPlausibilityContext::build(country, club);

            for request in unfulfilled {
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
                            && p.age <= request
                                .preferred_age_max
                                .saturating_add(3)
                                .min(MAX_LOAN_TARGET_AGE)
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
                            // Borrower-depth gate: if the borrower's
                            // squad is already full at this position
                            // group, accept only when the incoming
                            // player is clearly better than what's
                            // already there. Mirrors the domestic
                            // `should_skip_loan` shape so we don't
                            // import a 4th mid-tier GK on top of three.
                            && borrower_position_depth.has_room_for(p.position_group, p.skill_ability)
                            // The loan must buy minutes — not a bench
                            // seat behind a wall of better players.
                            // Young loanees (the development profile)
                            // need the stricter expected-minutes bar.
                            && borrower_position_depth.would_get_loan_minutes(
                                p.position_group,
                                p.skill_ability,
                                p.age <= 22,
                            )
                            // Parent-club reputation drop — same realism
                            // gate as the domestic scan, anchored on the
                            // player's own club rather than his personal
                            // reputation.
                            && Self::loan_reputation_drop_ok(
                                team_rep,
                                p.club_world_reputation.max(0) as u16,
                                p.skill_ability,
                                p.club_best_in_group,
                            )
                            // Staged cross-border veto: an important player at
                            // a much stronger club abroad isn't a credible
                            // loan target even when loan-listed, unless his
                            // availability genuinely opens the move. Mirrors
                            // the permanent foreign gate.
                            && !matches!(
                                TransferPlausibilityBuilder::evaluate_summary(
                                    &buyer_loan_ctx,
                                    p,
                                    true,
                                    true,
                                    date,
                                ),
                                Some(TransferPlausibilityVerdict::HardReject(_))
                            )
                    })
                    .max_by_key(|p| {
                        // Prefer culturally closer destinations: a club in
                        // the borrower's own region outranks a marginally
                        // better-rated alternative on another continent.
                        let same_region =
                            ScoutingRegion::from_country(p.continent_id, &p.country_code)
                                == club_region;
                        (same_region as u8, p.skill_ability)
                    })
                {
                    let loan_fee = FormattingUtils::round_fee(best.estimated_value * 0.1 * 0.8);
                    let reason = format!("Loan signing",);
                    actions.push(ForeignLoanAction {
                        club_id: club.id,
                        player: (*best).clone(),
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

            // Same as the domestic loan path — explicit months-side
            // duration so the market history record matches the actual
            // loan length.
            let offer = TransferOffer {
                base_fee: asking_price,
                clauses: Vec::new(),
                salary_contribution: None,
                contract_length_years: None,
                loan_duration_months: Some(10),
                personal_terms: None,
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

    /// Realism gate for the borrower side of a domestic loan: players
    /// don't drop from a giant to a minnow for a bit-part role.
    /// `borrower_rep` / `parent_rep` are main-team world reputations
    /// (0..10000). Very raw players — far below the parent's best at
    /// their position — tolerate a much bigger drop, because for them
    /// ANY senior football is the point of the loan.
    fn loan_reputation_drop_ok(
        borrower_rep: u16,
        parent_rep: u16,
        player_ability: u8,
        parent_best_in_group: u8,
    ) -> bool {
        if parent_rep == 0 {
            return true;
        }
        let very_raw = player_ability.saturating_add(25) <= parent_best_in_group;
        let floor = if very_raw { 0.12 } else { 0.25 };
        borrower_rep as f32 >= parent_rep as f32 * floor
    }

    /// List loan-out candidates on the transfer market.
    pub(crate) fn process_loan_out_listings(country: &mut Country, date: NaiveDate) {
        let pending: Vec<(u32, u32)> = country
            .clubs
            .iter()
            .flat_map(|club| {
                club.transfer_plan
                    .loan_out_candidates
                    .iter()
                    .filter(|c| c.status == LoanOutStatus::Identified)
                    .map(move |c| (club.id, c.player_id))
            })
            .collect();

        for (club_id, player_id) in pending {
            Self::list_loan_out_candidate(country, club_id, player_id, date);
        }
    }

    /// List ONE identified loan-out candidate on the market. Returns
    /// true when a new listing was created. Deduped against existing
    /// listings, so the daily pipeline pass and the Phase-C development-
    /// pathway staging can both call it without double-listing.
    pub(crate) fn list_loan_out_candidate(
        country: &mut Country,
        club_id: u32,
        player_id: u32,
        date: NaiveDate,
    ) -> bool {
        if country
            .transfer_market
            .get_listing_by_player(player_id)
            .is_some()
        {
            return false;
        }

        let price_level = country.settings.pricing.price_level;
        let listing = {
            let Some(club) = country.clubs.iter().find(|c| c.id == club_id) else {
                return false;
            };
            let Some(candidate) = club
                .transfer_plan
                .loan_out_candidates
                .iter()
                .find(|c| c.player_id == player_id && c.status == LoanOutStatus::Identified)
            else {
                return false;
            };
            let Some(player) = Self::find_player_in_club(club, player_id) else {
                return false;
            };
            if player.is_on_loan() {
                return false;
            }

            let team_id = club.teams.teams.first().map(|t| t.id).unwrap_or(0);

            let asking_price = if candidate.loan_fee > 0.0 {
                CurrencyValue {
                    amount: candidate.loan_fee,
                    currency: Currency::Usd,
                }
            } else {
                // Loan fee is ~10% of player value, not full value
                let (seller_league_rep, seller_club_rep) =
                    PlayerValuationCalculator::seller_context(country, club);
                let full_value = PlayerValuationCalculator::calculate_value_with_price_level(
                    player,
                    date,
                    price_level,
                    seller_league_rep,
                    seller_club_rep,
                );
                CurrencyValue {
                    amount: FormattingUtils::round_fee(full_value.amount * 0.10),
                    currency: full_value.currency,
                }
            };

            TransferListing::new(
                player_id,
                club.id,
                team_id,
                asking_price,
                date,
                TransferListingType::Loan,
            )
        };

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
                if let Some(player) = team.players.players.iter_mut().find(|p| p.id == player_id) {
                    if !player.statuses.get().contains(&PlayerStatusType::Loa) {
                        player.statuses.add(date, PlayerStatusType::Loa);
                    }
                }
            }
        }

        true
    }
}

/// Snapshot of the borrowing club's position-group depth — per-group
/// ability lists plus squad caps. Used by every loan scan (domestic and
/// foreign) for two realism gates: `has_room_for` stops the borrower
/// piling a fourth mid-tier player onto a position that's already three
/// deep, and `would_get_minutes` rejects destinations where the player
/// would sit behind a wall of clearly better names — a development loan
/// must buy pitch time, not a bench seat.
struct BorrowerPositionDepth {
    rows: Vec<(PlayerFieldPositionGroup, usize, Vec<u8>)>,
}

impl BorrowerPositionDepth {
    fn snapshot(team: &Team) -> Self {
        let caps = [
            (PlayerFieldPositionGroup::Goalkeeper, 3usize),
            (PlayerFieldPositionGroup::Defender, 8),
            (PlayerFieldPositionGroup::Midfielder, 8),
            (PlayerFieldPositionGroup::Forward, 6),
        ];
        let rows = caps
            .iter()
            .map(|&(group, max)| {
                let abilities: Vec<u8> = team
                    .players
                    .iter()
                    .filter(|p| p.position().position_group() == group)
                    .map(|p| p.player_attributes.current_ability)
                    .collect();
                (group, max, abilities)
            })
            .collect();
        BorrowerPositionDepth { rows }
    }

    fn row(
        &self,
        group: PlayerFieldPositionGroup,
    ) -> Option<&(PlayerFieldPositionGroup, usize, Vec<u8>)> {
        self.rows.iter().find(|(g, _, _)| *g == group)
    }

    /// True when adding a loan player at `group` makes sense — either
    /// there's room (count < max) or the incoming player is clearly
    /// stronger than the existing best in that group.
    fn has_room_for(&self, group: PlayerFieldPositionGroup, candidate_ability: u8) -> bool {
        match self.row(group) {
            Some((_, max, abilities)) => {
                if abilities.len() < *max {
                    return true;
                }
                // Group is full — only accept if the incoming player
                // would clearly upgrade the position (≥10 CA over the
                // current best).
                let best = abilities.iter().copied().max().unwrap_or(0);
                candidate_ability >= best.saturating_add(10)
            }
            None => true,
        }
    }

    /// Minutes gate with a development-strictness switch. Development
    /// loans exist to buy PLAYING time, so a young development loanee
    /// tolerates at most ONE clearly better outfielder ahead of him;
    /// generic squad/emergency cover keeps the looser bar of two. GK
    /// loans must always arrive as plausible first choice.
    fn would_get_loan_minutes(
        &self,
        group: PlayerFieldPositionGroup,
        candidate_ability: u8,
        development: bool,
    ) -> bool {
        match self.row(group) {
            Some((_, _, abilities)) => {
                let clearly_better = abilities
                    .iter()
                    .filter(|&&a| a >= candidate_ability.saturating_add(8))
                    .count();
                match group {
                    PlayerFieldPositionGroup::Goalkeeper => clearly_better == 0,
                    _ => clearly_better < if development { 2 } else { 3 },
                }
            }
            None => true,
        }
    }
}

#[cfg(test)]
mod borrower_gate_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, Player, PlayerAttributes, PlayerCollection, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills, StaffCollection, TeamReputation,
        TeamType, TrainingSchedule,
    };
    use chrono::{NaiveDate, NaiveTime};

    /// Borrower-side fixtures. Wrapped in a unit struct per the
    /// project's no-free-helpers convention.
    struct BorrowerFixtures;

    impl BorrowerFixtures {
        fn player(id: u32, position: PlayerPositionType, ca: u8) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ca;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Loan".to_string(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 16,
                    }],
                })
                .player_attributes(attrs)
                .build()
                .unwrap()
        }

        fn team(players: Vec<Player>) -> Team {
            Team::builder()
                .id(1)
                .league_id(Some(1))
                .club_id(1)
                .name("Borrower".to_string())
                .slug("borrower".to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(2000, 2000, 2000))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }
    }

    #[test]
    fn full_group_rejects_comparable_loan_but_accepts_clear_upgrade() {
        // Three keepers fill the GK cap; a comparable 4th is bloat, a
        // clear upgrade still gets through.
        let team = BorrowerFixtures::team(vec![
            BorrowerFixtures::player(1, PlayerPositionType::Goalkeeper, 80),
            BorrowerFixtures::player(2, PlayerPositionType::Goalkeeper, 70),
            BorrowerFixtures::player(3, PlayerPositionType::Goalkeeper, 60),
        ]);
        let depth = BorrowerPositionDepth::snapshot(&team);
        assert!(
            !depth.has_room_for(PlayerFieldPositionGroup::Goalkeeper, 85),
            "comparable keeper into a full group is squad bloat"
        );
        assert!(
            depth.has_room_for(PlayerFieldPositionGroup::Goalkeeper, 95),
            "a clear upgrade (≥10 over the best) is still allowed"
        );
    }

    #[test]
    fn gk_loan_blocked_when_clearly_better_keeper_present() {
        let team = BorrowerFixtures::team(vec![BorrowerFixtures::player(
            1,
            PlayerPositionType::Goalkeeper,
            80,
        )]);
        let depth = BorrowerPositionDepth::snapshot(&team);
        assert!(
            !depth.would_get_loan_minutes(PlayerFieldPositionGroup::Goalkeeper, 70, false),
            "a dev keeper behind a clearly better #1 plays zero minutes"
        );
        assert!(
            depth.would_get_loan_minutes(PlayerFieldPositionGroup::Goalkeeper, 75, false),
            "a keeper close to the incumbent can compete for the shirt"
        );
    }

    #[test]
    fn outfield_loan_blocked_behind_three_clearly_better_players() {
        let blocked_team = BorrowerFixtures::team(vec![
            BorrowerFixtures::player(1, PlayerPositionType::MidfielderCenter, 90),
            BorrowerFixtures::player(2, PlayerPositionType::MidfielderCenter, 90),
            BorrowerFixtures::player(3, PlayerPositionType::MidfielderCenter, 90),
        ]);
        let depth = BorrowerPositionDepth::snapshot(&blocked_team);
        assert!(
            !depth.would_get_loan_minutes(PlayerFieldPositionGroup::Midfielder, 70, false),
            "three clearly better midfielders leave no realistic minutes"
        );

        let open_team = BorrowerFixtures::team(vec![
            BorrowerFixtures::player(1, PlayerPositionType::MidfielderCenter, 90),
            BorrowerFixtures::player(2, PlayerPositionType::MidfielderCenter, 90),
            BorrowerFixtures::player(3, PlayerPositionType::MidfielderCenter, 72),
        ]);
        let depth = BorrowerPositionDepth::snapshot(&open_team);
        assert!(
            depth.would_get_loan_minutes(PlayerFieldPositionGroup::Midfielder, 70, false),
            "with only two clearly better names the loanee can rotate in"
        );
    }

    #[test]
    fn development_loans_demand_stricter_minutes_than_generic_cover() {
        // Two clearly better midfielders: fine for emergency cover,
        // not for a development loan that exists to buy starts.
        let team = BorrowerFixtures::team(vec![
            BorrowerFixtures::player(1, PlayerPositionType::MidfielderCenter, 90),
            BorrowerFixtures::player(2, PlayerPositionType::MidfielderCenter, 90),
        ]);
        let depth = BorrowerPositionDepth::snapshot(&team);
        assert!(
            depth.would_get_loan_minutes(PlayerFieldPositionGroup::Midfielder, 70, false),
            "generic cover tolerates two better names"
        );
        assert!(
            !depth.would_get_loan_minutes(PlayerFieldPositionGroup::Midfielder, 70, true),
            "a development loanee behind two starters won't get his minutes"
        );
    }

    #[test]
    fn reputation_drop_gate_blocks_giant_to_minnow_unless_player_is_raw() {
        // Established player (close to the parent's best): a 2000-rep
        // borrower is too far below a 9000-rep parent.
        assert!(!PipelineProcessor::loan_reputation_drop_ok(
            2000, 9000, 120, 130
        ));
        // Same borrower is fine for a very raw player — any senior
        // football is the point of the loan.
        assert!(PipelineProcessor::loan_reputation_drop_ok(
            2000, 9000, 90, 130
        ));
        // A mid-table borrower clears the floor for the established
        // player too.
        assert!(PipelineProcessor::loan_reputation_drop_ok(
            3000, 9000, 120, 130
        ));
        // Unknown parent reputation never blocks.
        assert!(PipelineProcessor::loan_reputation_drop_ok(
            2000, 0, 120, 130
        ));
    }
}
