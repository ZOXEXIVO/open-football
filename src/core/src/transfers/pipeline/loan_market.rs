use chrono::{Datelike, NaiveDate, Weekday};
use log::debug;

use crate::club::team::squad::{SquadAssetClass, SquadAssetContext};
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::ScoutingRegion;
use crate::transfers::market::{
    TransferListing, TransferListingOrigin, TransferListingStatus, TransferListingType,
};
use crate::transfers::offer::{TransferClause, TransferOffer};
use crate::transfers::pipeline::plausibility::{
    BuyerPlausibilityContext, TransferPlausibilityBuilder, TransferPlausibilityVerdict,
};
use crate::transfers::pipeline::processor::{PipelineProcessor, PlayerSummary};
use crate::transfers::pipeline::{LoanBroadcast, LoanOutStatus, TransferRequestStatus};
use std::collections::HashSet;
use crate::transfers::window::PlayerValuationCalculator;
use crate::utils::FormattingUtils;
use crate::{
    ClubPhilosophy, Country, Person, Player, PlayerFieldPositionGroup, PlayerStatusType,
    ReputationLevel, Team,
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
                // Treat as a "development" move (stricter minutes gate so he
                // actually plays, relaxed reputation/level floors so he can
                // drop a level or two to do so) either when the loan is
                // game-time-driven (development pathway, blocked prospect,
                // needs-minutes) OR simply when the player is young: a <=23
                // on the loan market is there for match practice. Pure
                // older-surplus / financial loans keep the looser cover bar.
                let is_development = player.age(date) <= UnsolicitedLoanTarget::DEVELOPMENT_AGE
                    || parent_club
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

        // ── Unsolicited loan targets (no `Loa` required) ─────────────
        //
        // A lower- or other-league club can approach a bigger club to take a
        // player on loan even when that player has NOT been loan-listed — the
        // `Loa` badge is not a precondition for loan demand. Built once per
        // pass on a weekly (Monday) cadence so the squad-wide scan stays
        // cheap; the listed-market scan above stays daily. Eligibility is the
        // central squad-asset classifier's job (see `UnsolicitedLoanTarget`):
        // young prospects and rotation players go as development loans and
        // genuine surplus goes at any age up to the loan cap — but a
        // first-team contributor is never cold-approached.
        let scan_unsolicited = date.weekday() == Weekday::Mon;
        let mut unsolicited_targets: Vec<LoanListing> = Vec::new();
        if scan_unsolicited {
            for club in &country.clubs {
                let parent_team = match club.teams.main().or_else(|| club.teams.teams.first()) {
                    Some(t) => t,
                    None => continue,
                };
                let parent_rep = parent_team.reputation.world;
                let asset_ctx = SquadAssetContext::build(club, date);
                let (seller_league_rep, seller_club_rep) =
                    PlayerValuationCalculator::seller_context(country, club);

                for team in &club.teams.teams {
                    for player in team.players.iter() {
                        let age = player.age(date);
                        let asset_class = asset_ctx.classify(player, date);
                        let is_development = match UnsolicitedLoanTarget::classify(
                            player,
                            age,
                            MAX_LOAN_TARGET_AGE,
                            asset_class,
                        ) {
                            Some(dev) => dev,
                            None => continue,
                        };

                        let group = player.position().position_group();
                        let parent_best_in_group = parent_team
                            .players
                            .iter()
                            .filter(|p| p.position().position_group() == group)
                            .map(|p| p.player_attributes.current_ability)
                            .max()
                            .unwrap_or(0);
                        // Development loans go out FREE: the parent wants the
                        // player developed, and a 10%-of-value fee would price
                        // a valuable prospect out of exactly the smaller,
                        // poorer clubs that actually have minutes for him (a
                        // low `max_loan_fee` was silently filtering them). This
                        // matches the main-squad board loan listing, which also
                        // asks zero. Older cover loans keep the nominal fee.
                        let asking_price = if is_development {
                            0.0
                        } else {
                            let value = player.value(date, seller_league_rep, seller_club_rep);
                            FormattingUtils::round_fee(value * 0.10)
                        };

                        unsolicited_targets.push(LoanListing {
                            player_id: player.id,
                            club_id: club.id,
                            asking_price,
                            ability: player.player_attributes.current_ability,
                            age,
                            position_group: group,
                            parent_rep,
                            parent_best_in_group,
                            is_development,
                        });
                    }
                }
            }
        }

        if loan_listings.is_empty() && unsolicited_targets.is_empty() {
            return;
        }

        // Collect club scanning actions (Pass 1 read)
        struct LoanScanAction {
            club_id: u32,
            player_id: u32,
            selling_club_id: u32,
            offer_amount: f64,
            reason: String,
            /// Cold approach for a player his club never loan-listed: Pass 2
            /// fabricates a synthetic listing and tags the negotiation
            /// unsolicited so the resolver withholds the "advertised" bonus.
            is_unsolicited: bool,
            /// Seller-side asking that backs that synthetic listing. Equal to
            /// the real listing's asking for listed targets (then unused).
            seller_asking: f64,
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
                                l.is_development,
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
                        is_unsolicited: false,
                        seller_asking: best.asking_price,
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
                            // Squad-average floor is for cover loans; a youth
                            // match-practice loan leans on the minutes gate.
                            && (l.is_development || l.ability >= avg_ability.saturating_sub(5))
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
                                l.is_development,
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
                        is_unsolicited: false,
                        seller_asking: opp.asking_price,
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
                            // Squad-average floor is for cover loans; a youth
                            // match-practice loan leans on the minutes gate.
                            && (l.is_development || l.ability >= avg_ability.saturating_sub(8))
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
                                l.is_development,
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
                        is_unsolicited: false,
                        seller_asking: opp.asking_price,
                    });
                }
            }

            // ── Unsolicited loan approach (no `Loa` required) ─────────
            //
            // A lower-/mid-tier club asks a bigger club to take a player who
            // is NOT loan-listed. The badge is not required; the move only
            // has to be a credible loan — the parent is bigger (approach
            // "up"), the player would actually play, and the reputation drop
            // is plausible. First-team contributors were already excluded
            // when the target pool was built, so this never strips a club of
            // a key player.
            if scan_unsolicited
                && scans_this_club < max_scans
                && matches!(
                    rep_level,
                    ReputationLevel::National
                        | ReputationLevel::Regional
                        | ReputationLevel::Local
                        | ReputationLevel::Amateur
                )
            {
                let mut cold: Vec<&LoanListing> = unsolicited_targets
                    .iter()
                    .filter(|l| {
                        l.club_id != club.id
                            && !club.is_rival(l.club_id)
                            // Approach "up": only a club below the parent
                            // borrows the player for minutes.
                            && l.parent_rep > borrower_world_rep
                            && l.age <= MAX_LOAN_TARGET_AGE
                            && l.asking_price * 0.8 <= max_loan_fee
                            && !country
                                .transfer_market
                                .has_active_negotiation_for(l.player_id, club.id)
                            && !actions.iter().any(|a| a.player_id == l.player_id)
                            && !scanned_position_groups.contains(&l.position_group)
                            // Will he actually play here? Position-aware, and
                            // for keepers the strict plausible-#1 rule — this
                            // is the realism check for a development loan.
                            && !should_skip_loan(l.position_group, l.ability)
                            && borrower_depth.would_get_loan_minutes(
                                l.position_group,
                                l.ability,
                                l.is_development,
                            )
                            // Squad-average / reputation-drop floors apply to
                            // cover loans only; development loans lean on the
                            // minutes gate above so a young keeper can drop to
                            // a club where he STARTS (see `clears_level_gate`).
                            && UnsolicitedLoanTarget::clears_level_gate(
                                l.is_development,
                                l.ability,
                                avg_ability,
                                borrower_world_rep,
                                l.parent_rep,
                                l.parent_best_in_group,
                            )
                    })
                    .collect();
                cold.sort_by(|a, b| b.ability.cmp(&a.ability));

                // Terminal branch for this club: `take` already caps the
                // approaches at the per-club scan budget, and nothing after
                // this reads the counter or the scanned-groups set, so no
                // further bookkeeping is needed.
                for tgt in cold.iter().take(max_scans - scans_this_club) {
                    actions.push(LoanScanAction {
                        club_id: club.id,
                        player_id: tgt.player_id,
                        selling_club_id: tgt.club_id,
                        offer_amount: FormattingUtils::round_fee(tgt.asking_price * 0.8),
                        reason: "Loan signing — unsolicited development approach".to_string(),
                        is_unsolicited: true,
                        seller_asking: tgt.asking_price,
                    });
                }
            }
        }

        // Pass 2: Start loan negotiations
        for action in actions {
            // Unsolicited approaches target players their club never loan-
            // listed, so the market has no listing to negotiate against.
            // Mirror the permanent-pipeline pattern: fabricate a synthetic
            // loan listing (tagged so the resolver withholds the
            // "seller-advertised" acceptance bonus), priced at the seller's
            // asking. `start_negotiation` requires a listing, so this must
            // come first.
            if action.is_unsolicited
                && country
                    .transfer_market
                    .get_listing_by_player(action.player_id)
                    .is_none()
            {
                let selling_team_id = country
                    .clubs
                    .iter()
                    .find(|c| c.id == action.selling_club_id)
                    .and_then(|c| c.teams.teams.first())
                    .map(|t| t.id)
                    .unwrap_or(0);
                country
                    .transfer_market
                    .add_listing(TransferListing::new_with_origin(
                        action.player_id,
                        action.selling_club_id,
                        selling_team_id,
                        CurrencyValue {
                            amount: FormattingUtils::round_fee(action.seller_asking.max(0.0)),
                            currency: Currency::Usd,
                        },
                        date,
                        TransferListingType::Loan,
                        TransferListingOrigin::SyntheticUnsolicited,
                    ));
            }

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
                    negotiation.is_unsolicited = action.is_unsolicited;
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
    // Step 7a-bis: Staged Loan-Availability Broadcast (seller-side push)
    // ============================================================

    /// Days a broadcast sits at one reputation tier before, unanswered, it
    /// widens to the next tier down.
    const BROADCAST_RESPONSE_DAYS: i64 = 14;

    /// Seller-side loan placement. A resource-rich parent club (National
    /// reputation or above) actively offers each loan-listed player to
    /// other clubs instead of only waiting to be scanned: it broadcasts
    /// the player to the highest realistic reputation tier first (its own
    /// level), and an interested club responds by opening a loan
    /// negotiation — the existing seller-acceptance path then resolves it.
    /// If no club at the current tier responds within
    /// [`Self::BROADCAST_RESPONSE_DAYS`], the net widens one tier down and
    /// re-offers, cascading high → low until the player is placed.
    ///
    /// Complements the borrower-driven [`Self::scan_loan_market`] (pull): a
    /// prized prospect at a giant whom no small club happens to scan still
    /// gets placed, because the parent goes looking. Weekly cadence;
    /// domestic only (a foreign push compounds with the cross-border gates
    /// in [`Self::scan_foreign_loan_market`]). Reuses the same realism
    /// gates the borrower side applies — would-get-minutes depth and the
    /// reputation-drop floor — so a push never lands a player on a bench or
    /// somewhere that makes no sporting sense.
    pub fn broadcast_listed_loans(country: &mut Country, date: NaiveDate) {
        // Weekly cadence — the squad-wide push is heavier than the daily
        // listed-market scan, and a placement decision needn't be revisited
        // every day.
        if date.weekday() != Weekday::Mon {
            return;
        }

        // Players with an in-flight negotiation already have a pending
        // response: don't widen their net or open a second approach. Their
        // broadcast entry is preserved (not pruned) so a failed negotiation
        // resumes the cascade where it left off.
        let in_negotiation: HashSet<u32> = country
            .transfer_market
            .negotiations
            .values()
            .map(|n| n.player_id)
            .collect();

        // ── Pass 1 (read): broadcastable players ────────────────────────
        // A player is broadcastable when he carries an Available loan
        // listing (the same source the borrower scan reads — which also
        // guarantees `start_negotiation` has a listing to anchor on) AND
        // his parent club is resource-rich enough to run a push (National+).
        struct Broadcastable {
            player_id: u32,
            parent_club_id: u32,
            parent_tier: ReputationLevel,
            parent_rep: u16,
            parent_best_in_group: u8,
            group: PlayerFieldPositionGroup,
            ability: u8,
            is_development: bool,
            asking: f64,
        }

        let mut broadcastable: Vec<Broadcastable> = Vec::new();
        for listing in &country.transfer_market.listings {
            if listing.listing_type != TransferListingType::Loan
                || listing.status != TransferListingStatus::Available
            {
                continue;
            }
            let Some(parent_club) = country.clubs.iter().find(|c| c.id == listing.club_id) else {
                continue;
            };
            let Some(parent_team) = parent_club
                .teams
                .main()
                .or_else(|| parent_club.teams.teams.first())
            else {
                continue;
            };
            let parent_tier = parent_team.reputation.level();
            // Resource gate: only National-and-above clubs run a push;
            // smaller clubs fall back to passive listing.
            if !parent_tier.runs_loan_broadcast() {
                continue;
            }
            let Some(player) = Self::find_player_in_country(country, listing.player_id) else {
                continue;
            };
            if player.is_on_loan() {
                continue;
            }
            let group = player.position().position_group();
            let parent_best_in_group = parent_team
                .players
                .iter()
                .filter(|p| p.position().position_group() == group)
                .map(|p| p.player_attributes.current_ability)
                .max()
                .unwrap_or(0);
            let is_development = player.age(date) <= UnsolicitedLoanTarget::DEVELOPMENT_AGE
                || parent_club.transfer_plan.loan_out_candidates.iter().any(|cand| {
                    cand.player_id == listing.player_id
                        && cand.reason.expects_guaranteed_minutes()
                });
            broadcastable.push(Broadcastable {
                player_id: listing.player_id,
                parent_club_id: listing.club_id,
                parent_tier,
                parent_rep: parent_team.reputation.world,
                parent_best_in_group,
                group,
                ability: player.player_attributes.current_ability,
                is_development,
                asking: listing.asking_price.amount,
            });
        }

        // Prune broadcasts whose player is no longer broadcastable (sold,
        // recalled, loan agreed, parent fell below the resource tier). Runs
        // even when the list is empty so the map never accumulates.
        let live_ids: Vec<u32> = broadcastable.iter().map(|b| b.player_id).collect();
        Self::prune_loan_broadcasts(country, &live_ids);
        if broadcastable.is_empty() {
            return;
        }

        // ── Pass 2 (mut clubs): advance each broadcast's tier ───────────
        // Open a broadcast at the parent's own tier; widen one tier down
        // once the current tier has gone unanswered past the response
        // window. In-negotiation players are left frozen.
        for b in &broadcastable {
            if in_negotiation.contains(&b.player_id) {
                continue;
            }
            let Some(club) = country.clubs.iter_mut().find(|c| c.id == b.parent_club_id) else {
                continue;
            };
            let next = match club.transfer_plan.loan_broadcasts.get(&b.player_id) {
                None => LoanBroadcast {
                    tier: b.parent_tier,
                    since: date,
                },
                Some(prev) => {
                    if (date - prev.since).num_days() >= Self::BROADCAST_RESPONSE_DAYS {
                        LoanBroadcast {
                            tier: prev.tier.next_lower(),
                            since: date,
                        }
                    } else {
                        prev.clone()
                    }
                }
            };
            club.transfer_plan.loan_broadcasts.insert(b.player_id, next);
        }

        // ── Pass 3 (read): pick a borrower at each broadcast's tier ─────
        struct PushAction {
            borrower_id: u32,
            player_id: u32,
            selling_club_id: u32,
            offer_amount: f64,
        }
        let mut actions: Vec<PushAction> = Vec::new();
        for b in &broadcastable {
            if in_negotiation.contains(&b.player_id) {
                continue;
            }
            // A development loanee is shopped to the WHOLE market at once: the
            // parent evaluates every club that would actually play him and sends
            // him to the best (highest-reputation) one — the strongest
            // environment where he still STARTS — instead of cascading down to
            // the first taker. The `would_get_loan_minutes` gate keeps him from
            // being placed too high (a keeper must be the undisputed #1), so the
            // "best passer" naturally falls to a lower club only when no higher
            // one has room. Cover / surplus loans keep the staged high → low
            // cascade and are placed at the current broadcast tier.
            let restrict_tier = if b.is_development {
                None
            } else {
                match country
                    .clubs
                    .iter()
                    .find(|c| c.id == b.parent_club_id)
                    .and_then(|c| c.transfer_plan.loan_broadcasts.get(&b.player_id))
                    .map(|br| br.tier)
                {
                    Some(t) => Some(t),
                    None => continue,
                }
            };

            // Highest world reputation among clubs that would actually play him
            // wins — the strongest environment that still guarantees minutes.
            let mut best: Option<(u32, u16)> = None;
            for club in &country.clubs {
                if club.id == b.parent_club_id || club.is_rival(b.parent_club_id) {
                    continue;
                }
                let Some(team) = club.teams.main().or_else(|| club.teams.teams.first()) else {
                    continue;
                };
                if let Some(tier) = restrict_tier {
                    if team.reputation.level() != tier {
                        continue;
                    }
                }
                if country
                    .transfer_market
                    .has_active_negotiation_for(b.player_id, club.id)
                {
                    continue;
                }
                let borrower_rep = team.reputation.world;
                let depth = BorrowerPositionDepth::snapshot(team);
                if !depth.has_room_for(b.group, b.ability)
                    || !depth.would_get_loan_minutes(b.group, b.ability, b.is_development)
                    || !Self::loan_reputation_drop_ok(
                        borrower_rep,
                        b.parent_rep,
                        b.ability,
                        b.parent_best_in_group,
                        b.is_development,
                    )
                {
                    continue;
                }
                match best {
                    Some((_, best_rep)) if borrower_rep <= best_rep => {}
                    _ => best = Some((club.id, borrower_rep)),
                }
            }
            if let Some((borrower_id, _)) = best {
                actions.push(PushAction {
                    borrower_id,
                    player_id: b.player_id,
                    selling_club_id: b.parent_club_id,
                    offer_amount: FormattingUtils::round_fee(b.asking * 0.8),
                });
            }
        }

        // ── Pass 4 (mut market): open the loan negotiations ─────────────
        // The interested club's "response". The player already carries an
        // Available loan listing, so `start_negotiation` has its anchor.
        for action in actions {
            let selling_rep = Self::get_club_reputation(country, action.selling_club_id);
            let buying_rep = Self::get_club_reputation(country, action.borrower_id);
            let (p_age, p_ambition) =
                Self::get_player_negotiation_data(country, action.player_id, date);

            let offer = TransferOffer {
                base_fee: CurrencyValue {
                    amount: action.offer_amount,
                    currency: Currency::Usd,
                },
                clauses: Vec::new(),
                salary_contribution: None,
                contract_length_years: None,
                loan_duration_months: Some(10),
                personal_terms: None,
                offering_club_id: action.borrower_id,
                offered_date: date,
            };

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                action.player_id,
                action.borrower_id,
                offer,
                date,
                selling_rep,
                buying_rep,
                p_age,
                p_ambition,
            ) {
                let (p_name, sc_name) = Self::resolve_player_and_club_name(
                    country,
                    action.player_id,
                    action.selling_club_id,
                );
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_loan = true;
                    negotiation.reason = "Loan placement — parent broadcast to scouts".to_string();
                    negotiation.player_name = p_name;
                    negotiation.selling_club_name = sc_name;
                }
                debug!(
                    "Loan broadcast: parent {} placed listed player {} at borrower {}",
                    action.selling_club_id, action.player_id, action.borrower_id
                );
            }
        }
    }

    /// Drop broadcast entries whose player is no longer broadcastable.
    /// `live_ids` is the set still in play this pass; everything else is
    /// removed so the per-club map stays bounded.
    fn prune_loan_broadcasts(country: &mut Country, live_ids: &[u32]) {
        for club in &mut country.clubs {
            if !club.transfer_plan.loan_broadcasts.is_empty() {
                club.transfer_plan
                    .loan_broadcasts
                    .retain(|pid, _| live_ids.contains(pid));
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
                // Either the parent advertised the loan, or the player is a
                // credible cold target his club isn't building around — the
                // `Loa` badge is not required for a loan approach. Importance
                // is gated below by the staged (unsolicited) plausibility.
                let approachable = p.is_loan_listed
                    || (p.age <= MAX_LOAN_TARGET_AGE
                        && ForeignUnsolicitedLoanTarget::looks_loanable(
                            p.age,
                            p.skill_ability,
                            p.club_best_in_group,
                        ));
                if !approachable {
                    return false;
                }
                // Country-reputation step-down. A player from a more
                // prestigious footballing nation isn't a realistic loan-in
                // for a smaller country — EXCEPT development-profile
                // youngsters, who routinely drop a national tier for
                // guaranteed senior minutes (Russia → Belarus, an Argentine
                // prospect → a smaller league). The region-prestige gate
                // below and the club-rep reality band downstream still bound
                // how far the move can fall.
                if !Self::foreign_loan_country_rep_ok(
                    p.country_reputation,
                    country_rep,
                    ForeignUnsolicitedLoanTarget::is_development(p.age),
                ) {
                    return false;
                }
                let player_region = ScoutingRegion::from_country(p.continent_id, &p.country_code);
                // Cross-region prestige step-down. A settled player won't loan
                // down into a clearly smaller ecosystem for a bit-part role, but
                // a development youngster drops abroad for guaranteed minutes
                // (an Italian U18 → Romania). See `foreign_loan_region_ok`.
                Self::foreign_loan_region_ok(
                    player_region.league_prestige(),
                    club_region_prestige,
                    ForeignUnsolicitedLoanTarget::is_development(p.age),
                )
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
                            // Squad-average floor is for cover loans; a youth
                            // match-practice loan leans on the minutes gate.
                            && (ForeignUnsolicitedLoanTarget::is_development(p.age)
                                || p.skill_ability >= avg_ability.saturating_sub(10))
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
                                ForeignUnsolicitedLoanTarget::is_development(p.age),
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
                                ForeignUnsolicitedLoanTarget::is_development(p.age),
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

            // Unsolicited foreign approaches target players their club never
            // loan-listed: tag the backing listing synthetic so the resolver
            // withholds the "seller-advertised" acceptance bonus. A genuinely
            // loan-listed foreign target keeps the seller-advertised origin.
            let is_unsolicited = !action.player.is_loan_listed;
            let listing = TransferListing::new_with_origin(
                action.player.player_id,
                action.player.club_id,
                0, // Foreign team — no local team_id
                asking_price.clone(),
                date,
                TransferListingType::Loan,
                if is_unsolicited {
                    TransferListingOrigin::SyntheticUnsolicited
                } else {
                    TransferListingOrigin::LoanOutListed
                },
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
                    negotiation.is_unsolicited = is_unsolicited;
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
    ///
    /// `is_development` lifts the floor entirely: a youth / match-practice
    /// loan (a young player going out to PLAY) is supposed to drop a level
    /// or two for senior minutes, and the stricter `would_get_loan_minutes`
    /// gate already guarantees he actually plays wherever he lands — so that,
    /// not club reputation, is the realism check. Without this a near-ready
    /// 20-23 (who isn't "very raw") hit the 0.25 floor and could never drop
    /// far enough to get games.
    fn loan_reputation_drop_ok(
        borrower_rep: u16,
        parent_rep: u16,
        player_ability: u8,
        parent_best_in_group: u8,
        is_development: bool,
    ) -> bool {
        if is_development || parent_rep == 0 {
            return true;
        }
        let very_raw = player_ability.saturating_add(25) <= parent_best_in_group;
        let floor = if very_raw { 0.12 } else { 0.25 };
        borrower_rep as f32 >= parent_rep as f32 * floor
    }

    /// Cross-border country-reputation gate for the foreign loan market.
    /// A player from a more prestigious footballing nation isn't a
    /// realistic loan-in for a smaller country: an established fringe
    /// player would rather stay or move sideways than drop a national tier
    /// for a bit-part role abroad, so the borrower's country must be at
    /// least as reputable as the player's.
    ///
    /// `is_development` lifts the gate entirely — the deliberate exception.
    /// A development-profile youngster (≤23) routinely loans DOWN a country
    /// tier for guaranteed senior minutes (Russia → Belarus, an Argentine
    /// prospect → a smaller league); vetoing that on country reputation
    /// alone is exactly what blocked the realistic "go abroad to play"
    /// move. The drop is still bounded downstream by the region-prestige
    /// gate and the club-rep reality band, so this can't launder a
    /// wonderkid into a clearly smaller ecosystem. Mirrors the
    /// `is_development` lift in [`Self::loan_reputation_drop_ok`].
    fn foreign_loan_country_rep_ok(
        player_country_rep: u16,
        borrower_country_rep: u16,
        is_development: bool,
    ) -> bool {
        if is_development {
            return true;
        }
        player_country_rep <= borrower_country_rep
    }

    /// Cross-region prestige step-down for a foreign loan. A settled player from
    /// a more prestigious football region won't loan down into a clearly smaller
    /// ecosystem for a bit-part role — but a development youngster (≤23) accepts
    /// a much larger drop to go abroad for guaranteed minutes (an Italian U18 →
    /// Romania / Russia). Without this lift a Western-European prospect was
    /// region-LOCKED: only borrowers within +0.20 of his own 1.0 prestige
    /// qualify, i.e. only Western Europe, so a giant's youngster never moved
    /// abroad at all. The wider development allowance reaches the mid regions
    /// (Eastern Europe / Scandinavia / South America, ~0.45-0.50) for senior
    /// football but still falls short of the bottom regions, and the downstream
    /// club-rep reality band bounds the actual destination. Mirrors the
    /// development lift in [`Self::foreign_loan_country_rep_ok`].
    fn foreign_loan_region_ok(
        player_region_prestige: f32,
        club_region_prestige: f32,
        is_development: bool,
    ) -> bool {
        // A settled player tolerates only a small step down in region prestige;
        // a development youngster goes much further for minutes.
        let allowance = if is_development { 0.55 } else { 0.20 };
        player_region_prestige <= club_region_prestige + allowance
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

/// Eligibility policy for an *unsolicited* domestic loan — a smaller club
/// approaching a bigger one for a player his club has NOT loan-listed. The
/// `Loa` badge is deliberately not required; loan demand should not depend
/// on the parent advertising the player. The realism is in WHO is
/// approachable, decided by the central [`SquadAssetClass`] classifier that
/// the audit and listing paths already share.
struct UnsolicitedLoanTarget;

impl UnsolicitedLoanTarget {
    /// Upper age for a *development* unsolicited loan — a young player a
    /// smaller club takes to get him minutes. Above it, only a genuinely
    /// surplus player is a credible cold loan target (generic cover, not
    /// development).
    const DEVELOPMENT_AGE: u8 = 23;

    /// Classify a potential unsolicited loan target. Returns `Some(is_dev)`
    /// when the player may be cold-approached — `is_dev` selecting the
    /// stricter development-minutes gate — or `None` when he is not a
    /// credible target at all.
    ///
    /// Never a first-team contributor (`CorePlayer` / `FirstTeamUseful`),
    /// and never a player the club hasn't even evaluated yet
    /// (`UnknownNeedsEvaluation`). Prospects and young rotation players go
    /// as development loans; genuine surplus goes at any age up to `max_age`.
    fn classify(
        player: &Player,
        age: u8,
        max_age: u8,
        asset_class: SquadAssetClass,
    ) -> Option<bool> {
        if player.contract.is_none() || player.is_on_loan() {
            return None;
        }
        if age > max_age {
            return None;
        }
        // Manager-pinned players are off the table entirely.
        if player.is_force_match_selection {
            return None;
        }
        // Already on a list / heading out under his own steam: those flow
        // through the normal (listed) paths, not a cold loan approach.
        let statuses = player.statuses.get();
        if statuses.contains(&PlayerStatusType::Lst)
            || statuses.contains(&PlayerStatusType::Loa)
            || statuses.contains(&PlayerStatusType::Frt)
        {
            return None;
        }
        match asset_class {
            SquadAssetClass::CorePlayer
            | SquadAssetClass::FirstTeamUseful
            | SquadAssetClass::UnknownNeedsEvaluation => None,
            SquadAssetClass::ProspectDevelopment => Some(true),
            SquadAssetClass::RotationUseful => (age <= Self::DEVELOPMENT_AGE).then_some(true),
            SquadAssetClass::TrueSurplus => Some(age <= Self::DEVELOPMENT_AGE),
        }
    }

    /// Does the target clear the "right level for this borrower" gate?
    ///
    /// For a development loan the realism check is "will he actually play
    /// here", which the caller enforces with the position-aware minutes /
    /// room gates (and, for keepers, the strict plausible-#1 rule). When
    /// that holds, the squad-average floor and the reputation-drop floor are
    /// not merely redundant but actively harmful — they block the signature
    /// development move: a big-club youngster dropping to a smaller club to
    /// START. Young keepers are the sharpest case — they develop late, so a
    /// teenage keeper's CA sits far below an outfield-heavy squad average and
    /// far below a giant parent's reputation, which is exactly why none ever
    /// moved. Cover (non-development) loans keep both floors.
    fn clears_level_gate(
        is_development: bool,
        ability: u8,
        borrower_avg_ability: u8,
        borrower_rep: u16,
        parent_rep: u16,
        parent_best_in_group: u8,
    ) -> bool {
        if is_development {
            return true;
        }
        ability >= borrower_avg_ability.saturating_sub(5)
            && PipelineProcessor::loan_reputation_drop_ok(
                borrower_rep,
                parent_rep,
                ability,
                parent_best_in_group,
                false,
            )
    }
}

/// Eligibility approximation for an *unsolicited* foreign loan. A cross-
/// country [`PlayerSummary`] doesn't carry the squad-asset classification
/// (it is built without the owning club's full squad context), so the "is
/// he a first-team contributor?" question is approximated from how far the
/// player sits below his club's best at his position: a key man is at/near
/// the top, a prospect or fringe player clearly below it. The staged
/// plausibility gate (run as unsolicited) still applies on top.
struct ForeignUnsolicitedLoanTarget;

impl ForeignUnsolicitedLoanTarget {
    /// Young players up to this age qualify as development targets on a
    /// small gap; older players need a clear surplus gap.
    const DEVELOPMENT_AGE: u8 = 23;
    /// CA below his club's best at the position that marks a young player as
    /// a development prospect (not the first-choice).
    const PROSPECT_GAP: u8 = 5;
    /// Larger gap an older player must sit below his club's best to read as
    /// clearly-surplus fringe rather than a contributor.
    const FRINGE_GAP: u8 = 15;

    /// Does this foreign player look like one his club would entertain a
    /// loan out for — i.e. clearly not their first-choice at the position?
    fn looks_loanable(age: u8, skill_ability: u8, club_best_in_group: u8) -> bool {
        let gap = if age <= Self::DEVELOPMENT_AGE {
            Self::PROSPECT_GAP
        } else {
            Self::FRINGE_GAP
        };
        club_best_in_group >= skill_ability.saturating_add(gap)
    }

    /// Whether a development-grade (stricter) minutes gate should apply.
    fn is_development(age: u8) -> bool {
        age <= Self::DEVELOPMENT_AGE
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
        let groups = [
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ];
        let rows = groups
            .iter()
            .map(|&group| {
                let max = group.ideal_squad_depth();
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
            2000, 9000, 120, 130, false
        ));
        // Same borrower is fine for a very raw player — any senior
        // football is the point of the loan.
        assert!(PipelineProcessor::loan_reputation_drop_ok(
            2000, 9000, 90, 130, false
        ));
        // A mid-table borrower clears the floor for the established
        // player too.
        assert!(PipelineProcessor::loan_reputation_drop_ok(
            3000, 9000, 120, 130, false
        ));
        // Unknown parent reputation never blocks.
        assert!(PipelineProcessor::loan_reputation_drop_ok(
            2000, 0, 120, 130, false
        ));
        // Youth / match-practice loan: the reputation floor is lifted
        // entirely so a 20-23 can drop a level or two for senior minutes —
        // even the giant-to-minnow case the first assertion blocks for a
        // cover loan. The minutes gate is the realism check instead.
        assert!(PipelineProcessor::loan_reputation_drop_ok(
            2000, 9000, 120, 130, true
        ));
    }

    #[test]
    fn foreign_loan_country_rep_gate_lifts_for_development_step_down() {
        // Higher-reputation nation → lower (e.g. Russia → Belarus): an
        // established fringe player can't drop a national tier on loan.
        assert!(!PipelineProcessor::foreign_loan_country_rep_ok(
            7000, 5000, false
        ));
        // ...but a development-profile youngster going abroad for senior
        // minutes is exactly the move the gate is meant to permit. The
        // region-prestige and club-rep gates still bound how far he falls.
        assert!(PipelineProcessor::foreign_loan_country_rep_ok(
            7000, 5000, true
        ));
        // Equal-or-lower-reputation source never trips the gate regardless
        // of profile — there's no step-down to guard against.
        assert!(PipelineProcessor::foreign_loan_country_rep_ok(
            5000, 7000, false
        ));
        assert!(PipelineProcessor::foreign_loan_country_rep_ok(
            5000, 5000, false
        ));
    }

    #[test]
    fn foreign_loan_region_gate_lifts_for_development_step_down() {
        // Italy (Western Europe, 1.0) → Romania/Russia (Eastern Europe, 0.50).
        // A settled player won't loan down two prestige bands for a bit-part
        // role — the 0.50 gap exceeds the 0.20 cover allowance.
        assert!(!PipelineProcessor::foreign_loan_region_ok(1.0, 0.50, false));
        // ...but a development youngster going abroad for senior minutes is
        // exactly the "go abroad to play" move the region gate used to block —
        // the wider development allowance clears the gap. The downstream
        // club-rep band still bounds how far he actually falls.
        assert!(PipelineProcessor::foreign_loan_region_ok(1.0, 0.50, true));
        // The development lift stays bounded: a top-region prospect still can't
        // reach the very bottom regions (e.g. South Asia, 0.10) from 1.0.
        assert!(!PipelineProcessor::foreign_loan_region_ok(1.0, 0.10, true));
        // Moving to an equal-or-more-prestigious region is never blocked.
        assert!(PipelineProcessor::foreign_loan_region_ok(0.50, 1.0, false));
    }
}

#[cfg(test)]
mod unsolicited_loan_target_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, Player, PlayerAttributes, PlayerClubContract, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills,
    };
    use chrono::NaiveDate;

    /// Fixtures for the unsolicited-target eligibility policy. Wrapped in a
    /// unit struct per the project's no-free-helpers convention.
    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 7, 6).unwrap()
        }

        /// A contracted central midfielder. `with_contract = false` leaves
        /// him contract-less (a returning loanee / free agent on the books).
        fn player(with_contract: bool) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = 95;
            attrs.potential_ability = 150;
            let mut builder = PlayerBuilder::new()
                .id(1)
                .full_name(FullName::new("Y".into(), "P".into()))
                .birth_date(NaiveDate::from_ymd_opt(2007, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 16,
                    }],
                })
                .player_attributes(attrs);
            if with_contract {
                builder = builder.contract(Some(PlayerClubContract::new(
                    20_000,
                    NaiveDate::from_ymd_opt(2030, 6, 30).unwrap(),
                )));
            }
            builder.build().unwrap()
        }

        const MAX: u8 = MAX_LOAN_TARGET_AGE;
    }

    #[test]
    fn young_unlisted_prospect_is_a_development_target() {
        let p = Fx::player(true);
        assert_eq!(
            UnsolicitedLoanTarget::classify(&p, 18, Fx::MAX, SquadAssetClass::ProspectDevelopment),
            Some(true),
            "an unlisted young prospect is approachable as a development loan"
        );
    }

    #[test]
    fn first_team_contributors_are_never_cold_approached() {
        let p = Fx::player(true);
        for class in [SquadAssetClass::CorePlayer, SquadAssetClass::FirstTeamUseful] {
            assert_eq!(
                UnsolicitedLoanTarget::classify(&p, 18, Fx::MAX, class),
                None,
                "a first-team contributor must never be cold-approached"
            );
        }
    }

    #[test]
    fn unevaluated_player_is_not_a_target() {
        let p = Fx::player(true);
        assert_eq!(
            UnsolicitedLoanTarget::classify(
                &p,
                18,
                Fx::MAX,
                SquadAssetClass::UnknownNeedsEvaluation
            ),
            None,
            "a player the club hasn't evaluated yet is left alone"
        );
    }

    #[test]
    fn older_surplus_is_a_generic_cover_target() {
        let p = Fx::player(true);
        assert_eq!(
            UnsolicitedLoanTarget::classify(&p, 30, Fx::MAX, SquadAssetClass::TrueSurplus),
            Some(false),
            "older genuine surplus is loanable, but as generic cover (not development)"
        );
    }

    #[test]
    fn young_rotation_develops_but_older_rotation_does_not() {
        let p = Fx::player(true);
        assert_eq!(
            UnsolicitedLoanTarget::classify(&p, 20, Fx::MAX, SquadAssetClass::RotationUseful),
            Some(true),
            "a young rotation player can go on a development loan"
        );
        assert_eq!(
            UnsolicitedLoanTarget::classify(&p, 30, Fx::MAX, SquadAssetClass::RotationUseful),
            None,
            "an older rotation player is squad depth, not a cold loan target"
        );
    }

    #[test]
    fn over_age_cap_is_not_a_target() {
        let p = Fx::player(true);
        assert_eq!(
            UnsolicitedLoanTarget::classify(&p, Fx::MAX + 1, Fx::MAX, SquadAssetClass::TrueSurplus),
            None,
            "past the loan age cap nobody is a target"
        );
    }

    #[test]
    fn already_listed_or_pinned_players_use_other_paths() {
        let mut listed = Fx::player(true);
        listed.statuses.add(Fx::date(), PlayerStatusType::Loa);
        assert_eq!(
            UnsolicitedLoanTarget::classify(
                &listed,
                18,
                Fx::MAX,
                SquadAssetClass::ProspectDevelopment
            ),
            None,
            "a loan-listed player flows through the normal listed path"
        );

        let mut pinned = Fx::player(true);
        pinned.is_force_match_selection = true;
        assert_eq!(
            UnsolicitedLoanTarget::classify(
                &pinned,
                18,
                Fx::MAX,
                SquadAssetClass::ProspectDevelopment
            ),
            None,
            "a manager-pinned player is never cold-approached"
        );
    }

    #[test]
    fn contract_less_player_is_not_a_target() {
        let p = Fx::player(false);
        assert_eq!(
            UnsolicitedLoanTarget::classify(&p, 18, Fx::MAX, SquadAssetClass::ProspectDevelopment),
            None,
            "a contract-less player (free agent / returning loanee) is not loaned out"
        );
    }

    #[test]
    fn foreign_target_must_sit_clearly_below_the_clubs_best() {
        // Young prospect: a small gap below the club's best is enough.
        assert!(ForeignUnsolicitedLoanTarget::looks_loanable(18, 90, 100));
        assert!(!ForeignUnsolicitedLoanTarget::looks_loanable(18, 98, 100));
        // Older player: needs a clear surplus gap to read as fringe.
        assert!(!ForeignUnsolicitedLoanTarget::looks_loanable(30, 90, 100));
        assert!(ForeignUnsolicitedLoanTarget::looks_loanable(30, 80, 100));
    }

    #[test]
    fn foreign_development_band_tracks_age() {
        assert!(ForeignUnsolicitedLoanTarget::is_development(22));
        assert!(ForeignUnsolicitedLoanTarget::is_development(23));
        assert!(!ForeignUnsolicitedLoanTarget::is_development(24));
    }

    #[test]
    fn development_loan_bypasses_level_floors() {
        // A young keeper (low CA) dropping from a giant parent (rep 8000,
        // best keeper 145) to a tiny club (avg 90, rep 400) would fail both
        // the squad-average floor and the reputation-drop floor — but as a
        // development loan he clears the level gate, because the caller's
        // minutes gate is the real "will he play here" check. This is the
        // case that left U18/U20 keepers stranded.
        assert!(UnsolicitedLoanTarget::clears_level_gate(
            true, 60, 90, 400, 8000, 145
        ));
    }

    #[test]
    fn cover_loan_keeps_level_floors() {
        // Non-development cover: both floors still apply.
        // Far below the borrower's squad average → blocked by the floor.
        assert!(!UnsolicitedLoanTarget::clears_level_gate(
            false, 60, 90, 3000, 8000, 145
        ));
        // Near the borrower's level AND a plausible (raw-player) rep drop
        // from a giant → allowed.
        assert!(UnsolicitedLoanTarget::clears_level_gate(
            false, 86, 90, 3000, 8000, 130
        ));
        // Near level, but a non-raw player dropping from a giant to a
        // minnow is implausible → blocked by the reputation gate.
        assert!(!UnsolicitedLoanTarget::clears_level_gate(
            false, 120, 118, 500, 8000, 125
        ));
    }
}

#[cfg(test)]
mod scan_loan_market_tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::shared::{Currency, CurrencyValue};
    use crate::transfers::market::{TransferListing, TransferListingOrigin, TransferListingType};
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PersonAttributes,
        Player, PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills, PlayerSquadStatus, StaffCollection, Team,
        TeamBuilder, TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::{Datelike, Duration, NaiveDate, NaiveTime, Weekday};

    /// End-to-end fixture for the loan-market scan: a parent club, a
    /// borrowing club, and one country wrapping both. Wrapped in a unit
    /// struct per the project's no-free-helpers convention.
    struct Fx;

    impl Fx {
        /// A Monday inside a window-agnostic part of the calendar — the
        /// unsolicited pool only builds on Mondays.
        fn monday() -> NaiveDate {
            let d = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
            assert_eq!(d.weekday(), Weekday::Mon, "fixture date must be a Monday");
            d
        }

        fn schedule() -> TrainingSchedule {
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            )
        }

        fn keeper(id: u32, ca: u8, pa: u8, age: u8, youth: bool) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ca;
            attrs.potential_ability = pa;
            attrs.condition = 10_000;
            let expiration = NaiveDate::from_ymd_opt(2030, 6, 30).unwrap();
            let mut contract = if youth {
                PlayerClubContract::new_youth(10_000, expiration)
            } else {
                PlayerClubContract::new(20_000, expiration)
            };
            contract.squad_status = PlayerSquadStatus::NotYetSet;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("K".into(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(2026 - age as i32, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::Goalkeeper,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(contract))
                .build()
                .unwrap()
        }

        fn team(id: u32, club_id: u32, tt: TeamType, world: u16, players: Vec<Player>) -> Team {
            TeamBuilder::new()
                .id(id)
                .league_id(Some(1))
                .club_id(club_id)
                .name(format!("t{id}"))
                .slug(format!("t{id}"))
                .team_type(tt)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(world, world, world))
                .training_schedule(Self::schedule())
                .build()
                .unwrap()
        }

        fn club(id: u32, teams: Vec<Team>, balance: i64) -> Club {
            Club::new(
                id,
                format!("Club{id}"),
                Location::new(1),
                ClubFinances::new(balance, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(teams),
                ClubFacilities::default(),
            )
        }

        fn country(clubs: Vec<Club>) -> Country {
            let league = League::new(
                1,
                "L".into(),
                "l".into(),
                1,
                500,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                },
                false,
            );
            Country::builder()
                .id(1)
                .code("EN".into())
                .slug("en".into())
                .name("England".into())
                .continent_id(1)
                .leagues(LeagueCollection::new(vec![league]))
                .clubs(clubs)
                .build()
                .unwrap()
        }

        /// Put an Available loan listing on the market for `player_id`,
        /// advertised free (a development loan). This is what the seller-
        /// side broadcast reads — the equivalent of the parent club having
        /// loan-listed the player.
        fn loan_list(country: &mut Country, player_id: u32, club_id: u32, team_id: u32) {
            country
                .transfer_market
                .add_listing(TransferListing::new_with_origin(
                    player_id,
                    club_id,
                    team_id,
                    CurrencyValue {
                        amount: 0.0,
                        currency: Currency::Usd,
                    },
                    Self::monday(),
                    TransferListingType::Loan,
                    TransferListingOrigin::SellerListed,
                ));
        }
    }

    /// The headline case: an Elite club's young, unlisted reserve keeper is
    /// approached on loan by a Regional club that has a keeper vacancy. This
    /// is exactly "no club has interest in my U18/U20 keeper" — and the only
    /// way interest registers is an actual negotiation, so the scan must
    /// create one.
    #[test]
    fn regional_club_makes_unsolicited_loan_approach_for_elite_youth_keeper() {
        let date = Fx::monday();

        // Elite parent (world 9000): three senior keepers on the main roster
        // plus one young, unlisted keeper in the reserves.
        let parent_main = Fx::team(
            10,
            1,
            TeamType::Main,
            9000,
            vec![
                Fx::keeper(101, 120, 120, 28, false),
                Fx::keeper(102, 118, 118, 26, false),
                Fx::keeper(103, 115, 115, 30, false),
            ],
        );
        let parent_reserve =
            Fx::team(11, 1, TeamType::Reserve, 6000, vec![Fx::keeper(200, 70, 150, 18, true)]);
        let parent = Fx::club(1, vec![parent_main, parent_reserve], 50_000_000);

        // Regional borrower (world 4000) with two weak keepers and budget.
        let borrower_main = Fx::team(
            20,
            2,
            TeamType::Main,
            4000,
            vec![
                Fx::keeper(301, 55, 55, 27, false),
                Fx::keeper(302, 50, 50, 29, false),
            ],
        );
        let mut borrower = Fx::club(2, vec![borrower_main], 5_000_000);
        borrower.transfer_plan.initialized = true;

        let mut country = Fx::country(vec![parent, borrower]);

        PipelineProcessor::scan_loan_market(&mut country, date);

        assert!(
            country.transfer_market.has_active_negotiation_for(200, 2),
            "a Regional club should make an unsolicited loan approach for the Elite club's \
             young reserve keeper — this is the interest that was never registering"
        );
    }

    /// The production-realistic case: the borrowing club is broke. The old
    /// `value * 0.10` asking made the loan fee exceed a poor club's tiny
    /// `max_loan_fee`, silently filtering the prospect out. A development
    /// loan now goes out free, so a cash-strapped club can still take him.
    #[test]
    fn cash_strapped_borrower_still_approaches_on_a_free_development_loan() {
        let date = Fx::monday();

        let parent_main = Fx::team(
            10,
            1,
            TeamType::Main,
            9000,
            vec![
                Fx::keeper(101, 120, 120, 28, false),
                Fx::keeper(102, 118, 118, 26, false),
                Fx::keeper(103, 115, 115, 30, false),
            ],
        );
        let parent_reserve =
            Fx::team(11, 1, TeamType::Reserve, 6000, vec![Fx::keeper(200, 70, 150, 18, true)]);
        let parent = Fx::club(1, vec![parent_main, parent_reserve], 50_000_000);

        // Negative balance → `max_loan_fee` is just 50k. A value-based fee
        // would have blocked the prospect; a free development loan must not.
        let borrower_main = Fx::team(
            20,
            2,
            TeamType::Main,
            4000,
            vec![
                Fx::keeper(301, 55, 55, 27, false),
                Fx::keeper(302, 50, 50, 29, false),
            ],
        );
        let mut borrower = Fx::club(2, vec![borrower_main], -2_000_000);
        borrower.transfer_plan.initialized = true;

        let mut country = Fx::country(vec![parent, borrower]);

        PipelineProcessor::scan_loan_market(&mut country, date);

        assert!(
            country.transfer_market.has_active_negotiation_for(200, 2),
            "a cash-strapped club must still take a youngster on a free development loan"
        );
        let listing = country
            .transfer_market
            .listings
            .iter()
            .find(|l| l.player_id == 200)
            .expect("the approach must back itself with a synthetic loan listing");
        assert_eq!(
            listing.asking_price.amount, 0.0,
            "a development loan must be advertised free so a poor club's loan-fee cap can't filter it"
        );
    }

    /// Seller-side push: a National+ club broadcasts its loan-listed
    /// youngster and a same-tier club with a keeper vacancy responds on the
    /// first cycle — no waiting for that club to happen to scan.
    #[test]
    fn broadcast_places_listed_youth_at_a_same_tier_taker() {
        let date = Fx::monday();

        // National parent (world 5500): a blocked young keeper in the
        // reserves, loan-listed.
        let parent_main = Fx::team(
            10,
            1,
            TeamType::Main,
            5500,
            vec![
                Fx::keeper(101, 120, 120, 28, false),
                Fx::keeper(102, 118, 118, 26, false),
                Fx::keeper(103, 115, 115, 30, false),
            ],
        );
        let parent_reserve =
            Fx::team(11, 1, TeamType::Reserve, 4000, vec![Fx::keeper(200, 70, 150, 18, true)]);
        let parent = Fx::club(1, vec![parent_main, parent_reserve], 50_000_000);

        // National taker at the same tier (world 5500) with a keeper vacancy.
        let borrower_main = Fx::team(
            20,
            2,
            TeamType::Main,
            5500,
            vec![
                Fx::keeper(301, 55, 55, 27, false),
                Fx::keeper(302, 50, 50, 29, false),
            ],
        );
        let borrower = Fx::club(2, vec![borrower_main], 5_000_000);

        let mut country = Fx::country(vec![parent, borrower]);
        Fx::loan_list(&mut country, 200, 1, 11);

        PipelineProcessor::broadcast_listed_loans(&mut country, date);

        assert!(
            country.transfer_market.has_active_negotiation_for(200, 2),
            "a National+ club should broadcast its loan-listed youngster and a same-tier club \
             with a vacancy responds"
        );
    }

    /// Non-development (surplus) loan: an Elite parent, the only realistic taker
    /// a Regional club. A surplus player still cascades — the broadcast opens at
    /// the parent's own (Elite) tier and widens one rung per unanswered window,
    /// Elite → Continental → National → Regional, before the Regional club is
    /// offered him. High reputation first, cascading down. (Development loanees
    /// instead skip the cascade and are placed at the best taker immediately —
    /// see `broadcast_places_development_loanee_at_best_taker_immediately`.)
    #[test]
    fn broadcast_cascades_non_development_loan_high_to_low() {
        let d0 = Fx::monday(); // 2026-01-05, a Monday

        let parent_main = Fx::team(
            10,
            1,
            TeamType::Main,
            9000,
            vec![
                Fx::keeper(101, 120, 120, 28, false),
                Fx::keeper(102, 118, 118, 26, false),
                Fx::keeper(103, 115, 115, 30, false),
            ],
        );
        // A 30-year-old surplus keeper (not a development loanee), so the staged
        // high → low cascade applies rather than immediate best-taker placement.
        let parent_reserve =
            Fx::team(11, 1, TeamType::Reserve, 6000, vec![Fx::keeper(200, 70, 150, 30, false)]);
        let parent = Fx::club(1, vec![parent_main, parent_reserve], 50_000_000);

        let borrower_main = Fx::team(
            20,
            2,
            TeamType::Main,
            4000,
            vec![
                Fx::keeper(301, 55, 55, 27, false),
                Fx::keeper(302, 50, 50, 29, false),
            ],
        );
        let borrower = Fx::club(2, vec![borrower_main], 5_000_000);

        let mut country = Fx::country(vec![parent, borrower]);
        Fx::loan_list(&mut country, 200, 1, 11);

        // Cycle 1: opens at Elite — no Elite taker exists, nobody responds.
        PipelineProcessor::broadcast_listed_loans(&mut country, d0);
        assert!(
            !country.transfer_market.has_active_negotiation_for(200, 2),
            "no Elite club exists to take him on the first broadcast"
        );

        // Widen one tier per 14-day window: Continental, then National.
        PipelineProcessor::broadcast_listed_loans(&mut country, d0 + Duration::days(14));
        PipelineProcessor::broadcast_listed_loans(&mut country, d0 + Duration::days(28));
        assert!(
            !country.transfer_market.has_active_negotiation_for(200, 2),
            "still being offered above the Regional taker's tier"
        );

        // Fourth window reaches Regional — the club with a vacancy responds.
        PipelineProcessor::broadcast_listed_loans(&mut country, d0 + Duration::days(42));
        assert!(
            country.transfer_market.has_active_negotiation_for(200, 2),
            "once the net widens to Regional, the club with a vacancy responds"
        );
    }

    /// A DEVELOPMENT loanee is shopped to the whole market at once: the parent
    /// evaluates every club that would actually play him and places him at the
    /// best — here the only realistic — taker on the FIRST broadcast, with no
    /// slow tier cascade. Same Elite-parent / Regional-taker shape as the
    /// non-development cascade above, but the youngster lands immediately
    /// instead of after four unanswered windows.
    #[test]
    fn broadcast_places_development_loanee_at_best_taker_immediately() {
        let d0 = Fx::monday();

        let parent_main = Fx::team(
            10,
            1,
            TeamType::Main,
            9000,
            vec![
                Fx::keeper(101, 120, 120, 28, false),
                Fx::keeper(102, 118, 118, 26, false),
                Fx::keeper(103, 115, 115, 30, false),
            ],
        );
        // An 18-year-old development keeper: a prospect who needs minutes.
        let parent_reserve =
            Fx::team(11, 1, TeamType::Reserve, 6000, vec![Fx::keeper(200, 70, 150, 18, true)]);
        let parent = Fx::club(1, vec![parent_main, parent_reserve], 50_000_000);

        let borrower_main = Fx::team(
            20,
            2,
            TeamType::Main,
            4000,
            vec![
                Fx::keeper(301, 55, 55, 27, false),
                Fx::keeper(302, 50, 50, 29, false),
            ],
        );
        let borrower = Fx::club(2, vec![borrower_main], 5_000_000);

        let mut country = Fx::country(vec![parent, borrower]);
        Fx::loan_list(&mut country, 200, 1, 11);

        PipelineProcessor::broadcast_listed_loans(&mut country, d0);
        assert!(
            country.transfer_market.has_active_negotiation_for(200, 2),
            "a development loanee is placed at the best (only) taker on the first \
             broadcast, not after a tier cascade"
        );
    }

    /// Resource gate: a below-National parent doesn't have the loan-
    /// management reach to run a push, so it never broadcasts — it falls
    /// back to passive listing no matter what takers exist.
    #[test]
    fn broadcast_skipped_for_a_below_national_parent() {
        let date = Fx::monday();

        // Regional parent (world 4000) — below the resource threshold.
        let parent_main = Fx::team(
            10,
            1,
            TeamType::Main,
            4000,
            vec![
                Fx::keeper(101, 90, 90, 28, false),
                Fx::keeper(102, 88, 88, 26, false),
            ],
        );
        let parent_reserve =
            Fx::team(11, 1, TeamType::Reserve, 3000, vec![Fx::keeper(200, 60, 120, 18, true)]);
        let parent = Fx::club(1, vec![parent_main, parent_reserve], 5_000_000);

        let borrower_main =
            Fx::team(20, 2, TeamType::Main, 3500, vec![Fx::keeper(301, 40, 40, 27, false)]);
        let borrower = Fx::club(2, vec![borrower_main], 1_000_000);

        let mut country = Fx::country(vec![parent, borrower]);
        Fx::loan_list(&mut country, 200, 1, 11);

        PipelineProcessor::broadcast_listed_loans(&mut country, date);

        assert!(
            !country.transfer_market.has_active_negotiation_for(200, 2),
            "a Regional parent lacks the loan-management resource to run a push"
        );
    }
}
