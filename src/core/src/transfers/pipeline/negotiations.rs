use chrono::NaiveDate;
use log::debug;

use crate::SimulatorData;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::TransferWindowManager;
use crate::transfers::market::{TransferListing, TransferListingType};
use crate::transfers::offer::{TransferClause, TransferOffer};
use crate::transfers::pipeline::processor::PipelineProcessor;
use crate::transfers::pipeline::{
    ShortlistCandidateStatus, TransferApproach, TransferNeedPriority, TransferNeedReason,
    TransferRequest, TransferRequestStatus,
};
use crate::utils::FormattingUtils;
use crate::{
    ClubPhilosophy, ClubTransferStrategy, Country, Person, PlayerStatusType, ReputationLevel,
    StaffPosition, WageCalculator,
};

/// Continuous buying aggressiveness from reputation ratio.
/// Replaces the old bucketed ReputationLevel cliff. A club's willingness to
/// offer close to asking scales smoothly with how established it is, and
/// how big it is relative to the seller — a small club overreaching for a
/// giant's player stays disciplined; a giant dealing with a small club can
/// push hard because they can wear the premium.
fn buying_aggressiveness_from_rep(buying_score: f32, selling_score: f32) -> f32 {
    let base = 0.30 + 0.55 * buying_score.clamp(0.0, 1.0);
    let ratio = if selling_score > 0.01 {
        (buying_score / selling_score).clamp(0.4, 2.0)
    } else {
        1.2
    };
    let ratio_adj = (ratio - 1.0) * 0.06;
    (base + ratio_adj).clamp(0.25, 0.90)
}

struct NegotiationAction {
    club_id: u32,
    player_id: u32,
    selling_club_id: u32,
    offer: TransferOffer,
    is_loan: bool,
    has_option_to_buy: bool,
    shortlist_request_id: u32,
    negotiator_staff_id: Option<u32>,
    reason: String,
    player_name: String,
    selling_club_name: String,
    player_sold_from: Option<(u32, f64)>,
    offered_annual_wage: u32,
    buying_league_reputation: u16,
    is_rival: bool,
}

impl PipelineProcessor {
    pub fn initiate_negotiations(country: &mut Country, date: NaiveDate) {
        let mut actions: Vec<NegotiationAction> = Vec::new();
        let price_level = country.settings.pricing.price_level;
        let window_mgr = TransferWindowManager::new();
        let current_window = window_mgr.current_window_dates(country.id, date);

        for club in &country.clubs {
            let plan = &club.transfer_plan;

            if !plan.initialized || !plan.can_start_negotiation() {
                continue;
            }

            // Skip clubs that have reached their squad cap (main team only)
            let max_squad = club
                .board
                .season_targets
                .as_ref()
                .map(|t| t.max_squad_size as usize)
                .unwrap_or(50);
            let main_squad = club
                .teams
                .teams
                .first()
                .map(|t| t.players.players.len())
                .unwrap_or(0);
            if main_squad >= max_squad {
                continue;
            }

            let actual_active = country
                .transfer_market
                .active_negotiation_count_for_club(club.id);
            if actual_active >= plan.max_concurrent_negotiations {
                continue;
            }

            let budget = club
                .finance
                .transfer_budget
                .as_ref()
                .map(|b| b.amount)
                .unwrap_or_else(|| (club.finance.balance.balance.max(0) as f64) * 0.3);

            if club.teams.teams.is_empty() {
                continue;
            }

            let team = &club.teams.teams[0];
            let rep_level = team.reputation.level();
            let buying_rep_score = team.reputation.overall_score();
            let buying_league_reputation = team
                .league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);

            let avg_ability = {
                let avg = team.players.current_ability_avg();
                if avg == 0 { 50 } else { avg }
            };

            let slots_available = plan
                .max_concurrent_negotiations
                .saturating_sub(actual_active) as usize;
            let mut negotiations_this_club = 0usize;

            for shortlist in &plan.shortlists {
                if negotiations_this_club >= slots_available {
                    break;
                }

                if shortlist.has_pursuing_candidate() {
                    continue;
                }

                if shortlist.all_exhausted() {
                    continue;
                }

                let candidate = match shortlist.current_candidate() {
                    Some(c) if c.status == ShortlistCandidateStatus::Available => c,
                    _ => continue,
                };

                let player_id = candidate.player_id;

                if country
                    .transfer_market
                    .has_active_negotiation_for(player_id, club.id)
                {
                    continue;
                }

                // Skip players on loan contracts — they belong to another club
                // Skip recently signed players — their club has a plan for them
                let (is_on_loan, is_protected) = Self::find_player_in_country(country, player_id)
                    .map(|p| {
                        (
                            p.is_on_loan(),
                            p.is_transfer_protected(date, current_window),
                        )
                    })
                    .unwrap_or((false, false));
                if is_on_loan || is_protected {
                    continue;
                }

                let selling_club_id = country
                    .clubs
                    .iter()
                    .find(|c| c.teams.contains_player(player_id))
                    .map(|c| c.id);

                let selling_club_id = match selling_club_id {
                    Some(id) if id != club.id => id,
                    _ => continue, // Foreign players handled by initiate_foreign_negotiations
                };

                // Rivalry is a deal friction, not an absolute block. A weaker
                // rival approaching a giant has essentially no chance; a club
                // at parity or above can still force the move through by
                // paying a premium or on a reputation-gap flinch. The penalty
                // is applied during resolve_initial_approach via is_rival flag.
                let is_rival = club.is_rival(selling_club_id);

                // ──────────────────────────────────────────────────
                // SMART BUY/LOAN DECISION
                // The DoF decides the approach based on context:
                // - Club reputation tier
                // - Budget vs player value
                // - Transfer request reason
                // - Whether the player is loan-listed
                // - Player age and potential
                // ──────────────────────────────────────────────────

                let request = plan
                    .transfer_requests
                    .iter()
                    .find(|r| r.id == shortlist.transfer_request_id);

                let approach = Self::determine_transfer_approach(
                    &rep_level,
                    budget,
                    candidate.estimated_fee,
                    request,
                    country,
                    player_id,
                    date,
                    club.finance.balance.balance,
                    &club.philosophy,
                );

                let is_loan = !matches!(approach, TransferApproach::PermanentTransfer);
                let has_option_to_buy = matches!(approach, TransferApproach::LoanWithOption);

                if let Some(player) = Self::find_player_in_country(country, player_id) {
                    let selling_club = country
                        .clubs
                        .iter()
                        .find(|c| c.id == selling_club_id)
                        .unwrap();
                    let selling_rep_score = selling_club
                        .teams
                        .teams
                        .first()
                        .map(|t| t.reputation.overall_score())
                        .unwrap_or(0.3);

                    let buying_aggressiveness =
                        buying_aggressiveness_from_rep(buying_rep_score, selling_rep_score);

                    let strategy = ClubTransferStrategy {
                        club_id: club.id,
                        budget: Some(CurrencyValue {
                            amount: shortlist.allocated_budget.min(budget),
                            currency: Currency::Usd,
                        }),
                        selling_willingness: 0.5,
                        buying_aggressiveness,
                        target_positions: vec![player.position()],
                        reputation_level: avg_ability as u16,
                    };

                    let asking_price = Self::calculate_asking_price(
                        player,
                        country,
                        selling_club,
                        date,
                        price_level,
                    );

                    let actual_asking = if is_loan {
                        let salary_proxy = player
                            .contract
                            .as_ref()
                            .map(|c| c.salary as f64 * 0.35)
                            .unwrap_or(0.0);
                        let loan_fee_rate = if has_option_to_buy { 0.04 } else { 0.07 };
                        CurrencyValue {
                            amount: FormattingUtils::round_fee(
                                (asking_price.amount * loan_fee_rate).max(salary_proxy),
                            ),
                            currency: asking_price.currency.clone(),
                        }
                    } else {
                        asking_price.clone()
                    };

                    let mut offer = strategy.calculate_initial_offer(player, &actual_asking, date);

                    // Add appearance fee clause for loans from high-reputation sellers
                    if is_loan {
                        let selling_rep_level =
                            Self::get_club_reputation_level(country, selling_club_id);
                        match selling_rep_level {
                            ReputationLevel::Elite => {
                                offer.clauses.push(TransferClause::AppearanceFee(
                                    CurrencyValue {
                                        amount: FormattingUtils::round_fee(
                                            offer.base_fee.amount * 0.30,
                                        ),
                                        currency: Currency::Usd,
                                    },
                                    10,
                                ));
                            }
                            ReputationLevel::Continental => {
                                offer.clauses.push(TransferClause::AppearanceFee(
                                    CurrencyValue {
                                        amount: FormattingUtils::round_fee(
                                            offer.base_fee.amount * 0.20,
                                        ),
                                        currency: Currency::Usd,
                                    },
                                    15,
                                ));
                            }
                            _ => {}
                        }
                    }

                    if has_option_to_buy {
                        let option_price = FormattingUtils::round_fee(asking_price.amount * 0.7);
                        offer
                            .clauses
                            .push(TransferClause::LoanOptionToBuy(CurrencyValue {
                                amount: option_price,
                                currency: Currency::Usd,
                            }));
                    }

                    let offered_annual_wage = WageCalculator::expected_annual_wage(
                        player,
                        player.age(date),
                        buying_rep_score,
                        buying_league_reputation,
                    );

                    // Resolve negotiator staff and build reason
                    let negotiator_staff_id = team.staffs.find_negotiator().map(|s| s.id);

                    let scout_report = plan
                        .scouting_reports
                        .iter()
                        .find(|r| r.player_id == player_id);

                    let reason = Self::build_transfer_reason(request, scout_report);

                    actions.push(NegotiationAction {
                        club_id: club.id,
                        player_id,
                        selling_club_id,
                        offer,
                        is_loan,
                        has_option_to_buy,
                        shortlist_request_id: shortlist.transfer_request_id,
                        negotiator_staff_id,
                        reason,
                        player_name: player.full_name.to_string(),
                        selling_club_name: selling_club.name.clone(),
                        player_sold_from: player.sold_from.clone(),
                        offered_annual_wage,
                        buying_league_reputation,
                        is_rival,
                    });

                    negotiations_this_club += 1;
                }
            }

            // Loan-out candidates are handled by process_loan_out_listings()
        }

        // Pass 2: Start negotiations
        for action in actions {
            let selling_rep = Self::get_club_reputation(country, action.selling_club_id);
            let buying_rep = Self::get_club_reputation(country, action.club_id);
            let (p_age, p_ambition) =
                Self::get_player_negotiation_data(country, action.player_id, date);

            let has_listing = country
                .transfer_market
                .get_listing_by_player(action.player_id)
                .is_some();

            if !has_listing {
                let listing_type = if action.is_loan {
                    TransferListingType::Loan
                } else {
                    TransferListingType::Transfer
                };

                let selling_team_id = country
                    .clubs
                    .iter()
                    .find(|c| c.id == action.selling_club_id)
                    .and_then(|c| c.teams.teams.first())
                    .map(|t| t.id)
                    .unwrap_or(0);

                let asking = CurrencyValue {
                    amount: FormattingUtils::round_fee(action.offer.base_fee.amount * 1.2),
                    currency: Currency::Usd,
                };

                let listing = TransferListing::new(
                    action.player_id,
                    action.selling_club_id,
                    selling_team_id,
                    asking,
                    date,
                    listing_type,
                );
                country.transfer_market.add_listing(listing);
            }

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                action.player_id,
                action.club_id,
                action.offer,
                date,
                selling_rep,
                buying_rep,
                p_age,
                p_ambition,
            ) {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_loan = action.is_loan;
                    negotiation.has_option_to_buy = action.has_option_to_buy;
                    negotiation.is_unsolicited = !has_listing;
                    negotiation.negotiator_staff_id = action.negotiator_staff_id;
                    negotiation.reason = action.reason.clone();
                    negotiation.player_name = action.player_name.clone();
                    negotiation.selling_club_name = action.selling_club_name.clone();
                    negotiation.player_sold_from = action.player_sold_from.clone();
                    negotiation.offered_salary = Some(action.offered_annual_wage);
                    negotiation.buying_league_reputation = action.buying_league_reputation;
                    if action.is_rival {
                        negotiation.reason = format!("{} (rival)", negotiation.reason.trim());
                    }
                }

                if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                    let plan = &mut club.transfer_plan;

                    if let Some(shortlist) = plan
                        .shortlists
                        .iter_mut()
                        .find(|s| s.transfer_request_id == action.shortlist_request_id)
                    {
                        if let Some(candidate) = shortlist.current_candidate_mut() {
                            if candidate.player_id == action.player_id {
                                candidate.status = ShortlistCandidateStatus::CurrentlyPursuing;
                            }
                        }
                    }

                    if let Some(req) = plan
                        .transfer_requests
                        .iter_mut()
                        .find(|r| r.id == action.shortlist_request_id)
                    {
                        req.status = TransferRequestStatus::Negotiating;
                    }

                    plan.active_negotiation_count += 1;
                }

                debug!(
                    "Pipeline: Club {} started negotiation for player {} ({})",
                    action.club_id,
                    action.player_id,
                    if action.is_loan { "loan" } else { "transfer" }
                );
            }
        }

        Self::process_loan_out_listings(country, date);
    }

    /// Determine whether to buy or loan a player.
    /// This is the "DoF decision" - mirrors real-world logic:
    ///
    /// - Elite clubs: Buy starters, loan promising youngsters with options
    /// - Continental clubs: Buy key targets, loan when budget is tight
    /// - National clubs: Buy affordable targets, loan expensive ones
    /// - Regional/Local: Loan most players, only buy cheap or free agents
    /// - If player is loan-listed by their club: always loan
    /// - Development signings: always loan
    /// - January window and negative balance rules bias toward loans
    fn determine_transfer_approach(
        rep_level: &ReputationLevel,
        budget: f64,
        estimated_fee: f64,
        request: Option<&TransferRequest>,
        country: &Country,
        player_id: u32,
        date: NaiveDate,
        buying_club_balance: i64,
        philosophy: &ClubPhilosophy,
    ) -> TransferApproach {
        let is_january = Self::is_january_window(date);

        let age = Self::find_player_in_country(country, player_id)
            .map(|p| p.age(date))
            .unwrap_or(25);

        // Philosophy-based overrides
        match philosophy {
            ClubPhilosophy::DevelopAndSell => {
                // Develop-and-sell clubs buy young assets and avoid expensive
                // older purchases. Loans are fallback cover, not the default
                // strategy for prospects.
                if age > 28 {
                    return TransferApproach::Loan;
                }
            }
            ClubPhilosophy::SignToCompete => {
                // Prefer permanent transfers even at lower affordability
                // (handled below in affordability section with relaxed thresholds)
            }
            ClubPhilosophy::LoanFocused => {
                // Always prefer loan unless fee < 50k
                let affordability = if estimated_fee > 0.0 {
                    budget / estimated_fee
                } else {
                    10.0
                };
                if estimated_fee >= 50_000.0 || affordability < 0.8 {
                    return TransferApproach::Loan;
                }
            }
            ClubPhilosophy::Balanced => {
                // No override — use existing logic
            }
        }

        // Reasons that always result in loan approach
        if let Some(req) = request {
            match req.reason {
                TransferNeedReason::DevelopmentSigning
                | TransferNeedReason::LoanToFillSquad
                | TransferNeedReason::InjuryCoverLoan
                | TransferNeedReason::OpportunisticLoanUpgrade
                | TransferNeedReason::SquadPadding => {
                    return TransferApproach::Loan;
                }
                TransferNeedReason::ExperiencedHead | TransferNeedReason::CheapReinforcement => {
                    // Prefer loan, but allow cheap buy if very affordable
                    if estimated_fee > 50_000.0 || buying_club_balance < 0 {
                        return TransferApproach::Loan;
                    }
                }
                _ => {}
            }
        }

        let is_critical = request
            .map(|r| r.priority == TransferNeedPriority::Critical)
            .unwrap_or(false);

        // January + Regional/Local/Amateur → always Loan
        if is_january
            && matches!(
                rep_level,
                ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur
            )
        {
            return TransferApproach::Loan;
        }

        // January + National + non-Critical request → Loan
        if is_january && *rep_level == ReputationLevel::National && !is_critical {
            return TransferApproach::Loan;
        }

        // Negative balance + non-Elite → Loan
        if buying_club_balance < 0 && *rep_level != ReputationLevel::Elite {
            return TransferApproach::Loan;
        }

        // Can we even afford to buy?
        let affordability = if estimated_fee > 0.0 {
            budget / estimated_fee
        } else {
            10.0 // Free agent, always affordable
        };

        // SignToCompete: accept higher fees, lower affordability thresholds
        if *philosophy == ClubPhilosophy::SignToCompete {
            return if affordability >= 0.75 || (is_critical && affordability >= 0.55) {
                TransferApproach::PermanentTransfer
            } else {
                TransferApproach::LoanWithOption
            };
        }

        match rep_level {
            ReputationLevel::Elite => {
                if affordability >= 0.3 {
                    TransferApproach::PermanentTransfer
                } else {
                    TransferApproach::LoanWithOption
                }
            }
            ReputationLevel::Continental => {
                if affordability >= 0.4 {
                    TransferApproach::PermanentTransfer
                } else if affordability >= 0.15 {
                    TransferApproach::LoanWithOption
                } else {
                    TransferApproach::Loan
                }
            }
            ReputationLevel::National => {
                if affordability >= 0.6 {
                    TransferApproach::PermanentTransfer
                } else if affordability >= 0.25 {
                    TransferApproach::LoanWithOption
                } else {
                    TransferApproach::Loan
                }
            }
            ReputationLevel::Regional => {
                if affordability >= 0.7 {
                    TransferApproach::PermanentTransfer
                } else if affordability >= 0.3 {
                    TransferApproach::LoanWithOption
                } else {
                    TransferApproach::Loan
                }
            }
            _ => {
                if affordability >= 1.5 && estimated_fee < 100_000.0 {
                    TransferApproach::PermanentTransfer
                } else {
                    TransferApproach::Loan
                }
            }
        }
    }

    pub fn on_negotiation_resolved(
        country: &mut Country,
        buying_club_id: u32,
        player_id: u32,
        accepted: bool,
    ) {
        let mut manager_satisfaction_hit: f32 = 0.0;
        if let Some(club) = country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
            let plan = &mut club.transfer_plan;

            // Monitoring lifecycle: mirror the negotiation outcome
            // onto every active monitoring row for this player. Signed
            // = scouts got their man; Lost = the pursuit collapsed.
            if accepted {
                plan.set_monitoring_status_for_player(
                    player_id,
                    crate::transfers::pipeline::ScoutMonitoringStatus::Signed,
                );
            } else {
                plan.set_monitoring_status_for_player(
                    player_id,
                    crate::transfers::pipeline::ScoutMonitoringStatus::Lost,
                );
            }

            for shortlist in &mut plan.shortlists {
                if let Some(candidate) = shortlist
                    .candidates
                    .iter_mut()
                    .find(|c| c.player_id == player_id)
                {
                    if accepted {
                        candidate.status = ShortlistCandidateStatus::Signed;

                        if let Some(req) = plan
                            .transfer_requests
                            .iter_mut()
                            .find(|r| r.id == shortlist.transfer_request_id)
                        {
                            req.status = TransferRequestStatus::Fulfilled;
                            // Signing a Critical target is a real morale lift.
                            manager_satisfaction_hit += match req.priority {
                                TransferNeedPriority::Critical => 3.0,
                                TransferNeedPriority::Important => 1.5,
                                TransferNeedPriority::Optional => 0.5,
                            };
                        }
                    } else {
                        candidate.status = ShortlistCandidateStatus::NegotiationFailed;
                        shortlist.advance_to_next();

                        if shortlist.all_exhausted() {
                            if let Some(req) = plan
                                .transfer_requests
                                .iter_mut()
                                .find(|r| r.id == shortlist.transfer_request_id)
                            {
                                if req.priority == TransferNeedPriority::Critical {
                                    // Critical targets re-open — but the
                                    // repeated failure still stings.
                                    req.status = TransferRequestStatus::Pending;
                                    manager_satisfaction_hit -= 2.0;
                                } else {
                                    req.status = TransferRequestStatus::Abandoned;
                                    // Abandoned target = identified need we
                                    // couldn't address. Hits manager morale.
                                    manager_satisfaction_hit -= match req.priority {
                                        TransferNeedPriority::Critical => 4.0,
                                        TransferNeedPriority::Important => 2.5,
                                        TransferNeedPriority::Optional => 0.75,
                                    };
                                }
                            }
                        } else {
                            if let Some(req) = plan
                                .transfer_requests
                                .iter_mut()
                                .find(|r| r.id == shortlist.transfer_request_id)
                            {
                                req.status = TransferRequestStatus::Shortlisted;
                            }
                        }
                    }

                    break;
                }
            }

            plan.active_negotiation_count = plan.active_negotiation_count.saturating_sub(1);

            // Push the aggregated delta into the manager's job_satisfaction
            // so a run of failed bids visibly erodes morale. Scoped inside
            // the same `if let Some(club)` so the borrow is still alive.
            if manager_satisfaction_hit.abs() > 0.01 {
                if let Some(main_team) = club.teams.main_mut() {
                    if let Some(mgr) = main_team
                        .staffs
                        .find_mut_by_position(StaffPosition::Manager)
                    {
                        mgr.job_satisfaction =
                            (mgr.job_satisfaction + manager_satisfaction_hit).clamp(0.0, 100.0);
                    }
                }
            }
        }
    }

    /// After a player moves club (transfer, loan, or free agent), remove all
    /// interest data for that player from every club in the country so that
    /// stale scouting/shortlist entries don't linger.
    pub fn clear_player_interest(country: &mut Country, player_id: u32) {
        for club in &mut country.clubs {
            let plan = &mut club.transfer_plan;

            // Scouting assignments: drop observations for this player
            for assignment in &mut plan.scouting_assignments {
                assignment.observations.retain(|o| o.player_id != player_id);
            }

            // Scouting reports
            plan.scouting_reports.retain(|r| r.player_id != player_id);

            // Shortlists: remove the candidate entry
            for shortlist in &mut plan.shortlists {
                shortlist.candidates.retain(|c| c.player_id != player_id);
            }

            // Staff recommendations
            plan.staff_recommendations
                .retain(|r| r.player_id != player_id);

            // Loan-out candidates: a free-agent / moved player is no longer
            // at this club's disposal to be loaned out.
            plan.loan_out_candidates
                .retain(|c| c.player_id != player_id);

            // Drop active monitoring rows so the player no longer
            // appears as "watched" by clubs that didn't sign them.
            plan.scout_monitoring.retain(|m| m.player_id != player_id);
        }
    }

    /// Global post-success cleanup invoked after a successful transfer,
    /// loan, or free-agent signing. Walks every country and:
    ///   - clears scouting / shortlist / monitoring / known-player rows
    ///     for the moved player (`clear_player_interest`),
    ///   - completes any open listings for the player anywhere,
    ///   - rejects active (Pending / Countered) negotiations for the
    ///     player anywhere,
    ///   - syncs the `Wnt` status so the player is no longer flagged
    ///     "wanted" once no real interest remains.
    ///
    /// Completed transfer history is intentionally left untouched so the
    /// player's career page still shows the move on record.
    ///
    /// `clear_player_interest(country)` is per-country and was already
    /// being called at negotiation acceptance time, but only on the
    /// negotiation's owning country — clubs in other countries that had
    /// scout monitoring or shortlist rows kept their stale interest. This
    /// helper closes that gap by sweeping the whole world after the move
    /// actually completes.
    pub fn cleanup_player_transfer_interest(data: &mut SimulatorData, player_id: u32) {
        use crate::transfers::market::TransferListingStatus;
        use crate::transfers::negotiation::NegotiationStatus;

        for continent in data.continents.iter_mut() {
            for country in continent.countries.iter_mut() {
                Self::clear_player_interest(country, player_id);

                for listing in country.transfer_market.listings.iter_mut() {
                    if listing.player_id == player_id
                        && listing.status != TransferListingStatus::Completed
                        && listing.status != TransferListingStatus::Cancelled
                    {
                        listing.status = TransferListingStatus::Completed;
                    }
                }

                for negotiation in country.transfer_market.negotiations.values_mut() {
                    if negotiation.player_id == player_id
                        && (negotiation.status == NegotiationStatus::Pending
                            || negotiation.status == NegotiationStatus::Countered)
                    {
                        negotiation.status = NegotiationStatus::Rejected;
                    }
                }

                Self::sync_wanted_status(country);
            }
        }
    }

    /// Reconcile `Wnt` statuses with actual interest. `Wnt` is added during
    /// scouting but has no intrinsic expiry — when window resets wipe all
    /// interest tracking, the status lingers and players appear "Wanted"
    /// with no interested clubs behind it. This walks the country once per
    /// invocation, collects the set of still-tracked player ids, and strips
    /// `Wnt` from anyone who is no longer on any club's radar.
    pub fn sync_wanted_status(country: &mut Country) {
        use std::collections::HashSet;

        let mut tracked: HashSet<u32> = HashSet::new();
        for club in &country.clubs {
            let plan = &club.transfer_plan;
            for assignment in &plan.scouting_assignments {
                for obs in &assignment.observations {
                    tracked.insert(obs.player_id);
                }
            }
            for r in &plan.scouting_reports {
                tracked.insert(r.player_id);
            }
            for s in &plan.shortlists {
                for c in &s.candidates {
                    tracked.insert(c.player_id);
                }
            }
            for r in &plan.staff_recommendations {
                tracked.insert(r.player_id);
            }
            // Active monitoring rows count as live interest — the
            // recruitment department is still watching the player even
            // if no scouting assignment row exists yet.
            for m in &plan.scout_monitoring {
                if m.is_active_interest() {
                    tracked.insert(m.player_id);
                }
            }
        }

        for club in &mut country.clubs {
            for team in &mut club.teams.teams {
                for player in team.players.players.iter_mut() {
                    if player.statuses.get().contains(&PlayerStatusType::Wnt)
                        && !tracked.contains(&player.id)
                    {
                        player.statuses.remove(PlayerStatusType::Wnt);
                    }
                }
            }
        }
    }

    pub fn initiate_foreign_negotiations(
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) {
        // Pass 1: Read — collect foreign candidates from shortlists
        struct ForeignCandidate {
            buying_club_id: u32,
            player_id: u32,
            shortlist_request_id: u32,
        }

        let mut candidates: Vec<ForeignCandidate> = Vec::new();

        if let Some(country) = data.country(country_id) {
            for club in &country.clubs {
                let plan = &club.transfer_plan;
                if !plan.initialized || !plan.can_start_negotiation() {
                    continue;
                }

                let actual_active = country
                    .transfer_market
                    .active_negotiation_count_for_club(club.id);
                if actual_active >= plan.max_concurrent_negotiations {
                    continue;
                }

                for shortlist in &plan.shortlists {
                    if shortlist.has_pursuing_candidate() || shortlist.all_exhausted() {
                        continue;
                    }

                    let candidate = match shortlist.current_candidate() {
                        Some(c) if c.status == ShortlistCandidateStatus::Available => c,
                        _ => continue,
                    };

                    // Only process if player is NOT in the local country
                    let is_local =
                        Self::find_player_in_country(country, candidate.player_id).is_some();
                    if is_local {
                        continue;
                    }

                    if country
                        .transfer_market
                        .has_active_negotiation_for(candidate.player_id, club.id)
                    {
                        continue;
                    }

                    candidates.push(ForeignCandidate {
                        buying_club_id: club.id,
                        player_id: candidate.player_id,
                        shortlist_request_id: shortlist.transfer_request_id,
                    });
                }
            }
        }

        if candidates.is_empty() {
            return;
        }

        // Pass 2: Resolve — find each player globally, compute offers
        struct ResolvedNeg {
            buying_club_id: u32,
            selling_country_id: u32,
            selling_continent_id: u32,
            selling_country_code: String,
            selling_club_id: u32,
            player_id: u32,
            is_loan: bool,
            has_option_to_buy: bool,
            offer: TransferOffer,
            reason: String,
            shortlist_request_id: u32,
            selling_rep: f32,
            buying_rep: f32,
            player_age: u8,
            player_ambition: f32,
            asking_price: CurrencyValue,
            player_name: String,
            selling_club_name: String,
            player_sold_from: Option<(u32, f64)>,
            offered_annual_wage: u32,
            buying_league_reputation: u16,
        }

        let mut resolved: Vec<ResolvedNeg> = Vec::new();

        for cand in candidates {
            // Find player globally
            let mut found = None;
            for continent in &data.continents {
                for country in &continent.countries {
                    if country.id == country_id {
                        continue;
                    }
                    for club in &country.clubs {
                        if club.teams.contains_player(cand.player_id) {
                            found = Some((
                                country.id,
                                club.id,
                                country.settings.pricing.price_level,
                                country.continent_id,
                                country.code.clone(),
                            ));
                            break;
                        }
                    }
                    if found.is_some() {
                        break;
                    }
                }
                if found.is_some() {
                    break;
                }
            }

            let (
                sell_country_id,
                sell_club_id,
                sell_price_level,
                sell_continent_id,
                sell_country_code,
            ) = match found {
                Some(v) => v,
                None => continue,
            };

            let sell_country = match data.country(sell_country_id) {
                Some(c) => c,
                None => continue,
            };
            let player = match Self::find_player_in_country(sell_country, cand.player_id) {
                Some(p) => p,
                None => continue,
            };
            if player.is_on_loan() {
                continue;
            }
            let sell_window =
                TransferWindowManager::new().current_window_dates(sell_country_id, date);
            if player.is_transfer_protected(date, sell_window) {
                continue;
            }

            let sell_club = match sell_country.clubs.iter().find(|c| c.id == sell_club_id) {
                Some(c) => c,
                None => continue,
            };
            let asking_price = Self::calculate_asking_price(
                player,
                sell_country,
                sell_club,
                date,
                sell_price_level,
            );
            let player_age = player.age(date);
            let player_ambition = player.skills.mental.determination;
            let player_name = player.full_name.to_string();
            let selling_club_name = sell_club.name.clone();

            let selling_rep = sell_club
                .teams
                .teams
                .first()
                .map(|t| t.reputation.world as f32 / 10000.0)
                .unwrap_or(0.3);

            let buy_country = match data.country(country_id) {
                Some(c) => c,
                None => continue,
            };
            let buy_club = match buy_country
                .clubs
                .iter()
                .find(|c| c.id == cand.buying_club_id)
            {
                Some(c) => c,
                None => continue,
            };

            let buying_rep = buy_club
                .teams
                .teams
                .first()
                .map(|t| t.reputation.world as f32 / 10000.0)
                .unwrap_or(0.3);
            let rep_level = buy_club
                .teams
                .teams
                .first()
                .map(|t| t.reputation.level())
                .unwrap_or(ReputationLevel::Amateur);
            let budget = buy_club
                .finance
                .transfer_budget
                .as_ref()
                .map(|b| b.amount)
                .unwrap_or_else(|| (buy_club.finance.balance.balance.max(0) as f64) * 0.3);

            let request = buy_club
                .transfer_plan
                .transfer_requests
                .iter()
                .find(|r| r.id == cand.shortlist_request_id);

            let approach = Self::determine_transfer_approach(
                &rep_level,
                budget,
                asking_price.amount,
                request,
                sell_country,
                cand.player_id,
                date,
                buy_club.finance.balance.balance,
                &buy_club.philosophy,
            );

            let is_loan = !matches!(approach, TransferApproach::PermanentTransfer);
            let has_option_to_buy = matches!(approach, TransferApproach::LoanWithOption);

            let actual_asking = if is_loan {
                let salary_proxy = player
                    .contract
                    .as_ref()
                    .map(|c| c.salary as f64 * 0.35)
                    .unwrap_or(0.0);
                let loan_fee_rate = if has_option_to_buy { 0.04 } else { 0.07 };
                CurrencyValue {
                    amount: FormattingUtils::round_fee(
                        (asking_price.amount * loan_fee_rate).max(salary_proxy),
                    ),
                    currency: asking_price.currency.clone(),
                }
            } else {
                asking_price.clone()
            };

            let avg_ability: u8 = buy_club
                .teams
                .teams
                .first()
                .map(|t| {
                    let avg = t.players.current_ability_avg();
                    if avg == 0 { 50 } else { avg }
                })
                .unwrap_or(50);

            let buying_league_reputation = buy_club
                .teams
                .teams
                .first()
                .and_then(|t| t.league_id)
                .and_then(|lid| buy_country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);

            let strategy = ClubTransferStrategy {
                club_id: cand.buying_club_id,
                budget: Some(CurrencyValue {
                    amount: budget,
                    currency: Currency::Usd,
                }),
                selling_willingness: 0.5,
                buying_aggressiveness: buying_aggressiveness_from_rep(buying_rep, selling_rep),
                target_positions: vec![player.position()],
                reputation_level: avg_ability as u16,
            };

            let mut offer = strategy.calculate_initial_offer(player, &actual_asking, date);

            if has_option_to_buy {
                let option_price = FormattingUtils::round_fee(asking_price.amount * 0.7);
                offer
                    .clauses
                    .push(TransferClause::LoanOptionToBuy(CurrencyValue {
                        amount: option_price,
                        currency: Currency::Usd,
                    }));
            }

            let offered_annual_wage = WageCalculator::expected_annual_wage(
                player,
                player_age,
                buying_rep,
                buying_league_reputation,
            );

            let reason = if is_loan {
                "Loan signing".to_string()
            } else {
                "Transfer signing".to_string()
            };

            resolved.push(ResolvedNeg {
                buying_club_id: cand.buying_club_id,
                selling_country_id: sell_country_id,
                selling_continent_id: sell_continent_id,
                selling_country_code: sell_country_code,
                selling_club_id: sell_club_id,
                player_id: cand.player_id,
                is_loan,
                has_option_to_buy,
                offer,
                reason,
                shortlist_request_id: cand.shortlist_request_id,
                selling_rep,
                buying_rep,
                player_age,
                player_ambition,
                asking_price,
                player_name,
                selling_club_name,
                player_sold_from: player.sold_from.clone(),
                offered_annual_wage,
                buying_league_reputation,
            });
        }

        // Pass 3: Write — create listings and negotiations
        for action in resolved {
            let country = match data.country_mut(country_id) {
                Some(c) => c,
                None => continue,
            };

            let listing = TransferListing::new(
                action.player_id,
                action.selling_club_id,
                0,
                action.asking_price,
                date,
                if action.is_loan {
                    TransferListingType::Loan
                } else {
                    TransferListingType::Transfer
                },
            );
            country.transfer_market.add_listing(listing);

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                action.player_id,
                action.buying_club_id,
                action.offer,
                date,
                action.selling_rep,
                action.buying_rep,
                action.player_age,
                action.player_ambition,
            ) {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_loan = action.is_loan;
                    negotiation.has_option_to_buy = action.has_option_to_buy;
                    negotiation.reason = action.reason;
                    negotiation.selling_country_id = Some(action.selling_country_id);
                    negotiation.selling_continent_id = Some(action.selling_continent_id);
                    negotiation.selling_country_code = action.selling_country_code;
                    negotiation.player_sold_from = action.player_sold_from;
                    negotiation.player_name = action.player_name;
                    negotiation.selling_club_name = action.selling_club_name;
                    negotiation.offered_salary = Some(action.offered_annual_wage);
                    negotiation.buying_league_reputation = action.buying_league_reputation;
                }

                if let Some(club) = country
                    .clubs
                    .iter_mut()
                    .find(|c| c.id == action.buying_club_id)
                {
                    let plan = &mut club.transfer_plan;
                    if let Some(shortlist) = plan
                        .shortlists
                        .iter_mut()
                        .find(|s| s.transfer_request_id == action.shortlist_request_id)
                    {
                        if let Some(candidate) = shortlist.current_candidate_mut() {
                            if candidate.player_id == action.player_id {
                                candidate.status = ShortlistCandidateStatus::CurrentlyPursuing;
                            }
                        }
                    }
                    if let Some(req) = plan
                        .transfer_requests
                        .iter_mut()
                        .find(|r| r.id == action.shortlist_request_id)
                    {
                        req.status = TransferRequestStatus::Negotiating;
                    }
                    plan.active_negotiation_count += 1;
                }

                debug!(
                    "Foreign negotiation: Club {} started negotiation for player {} from country {}",
                    action.buying_club_id, action.player_id, action.selling_country_id
                );
            }
        }
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    use crate::competitions::global::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::{Currency, CurrencyValue, Location};
    use crate::transfers::market::{
        TransferListing, TransferListingStatus, TransferListingType,
    };
    use crate::transfers::negotiation::{NegotiationStatus, TransferNegotiation};
    use crate::transfers::offer::TransferOffer;
    use crate::transfers::pipeline::recruitment::{
        ScoutMonitoringSource, ScoutMonitoringStatus, ScoutPlayerMonitoring,
    };
    use crate::transfers::pipeline::{ShortlistCandidate, ShortlistCandidateStatus, TransferShortlist};
    use crate::club::academy::ClubAcademy;
    use crate::transfers::{CompletedTransfer, TransferType};
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, TeamCollection,
    };
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn make_club(id: u32, name: &str) -> Club {
        Club::new(
            id,
            name.to_string(),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(Vec::new()),
            ClubFacilities::default(),
        )
    }

    fn make_league(id: u32, slug: &str) -> League {
        League::new(
            id,
            "L".to_string(),
            slug.to_string(),
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
        )
    }

    fn make_country(id: u32, code: &str, slug: &str, clubs: Vec<Club>) -> Country {
        Country::builder()
            .id(id)
            .code(code.to_string())
            .slug(slug.to_string())
            .name(slug.to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(vec![make_league(id, slug)]))
            .clubs(clubs)
            .build()
            .unwrap()
    }

    fn make_simulator(date: NaiveDate, countries: Vec<Country>) -> SimulatorData {
        let continent = Continent::new(1, "Europe".to_string(), countries, Vec::new());
        SimulatorData::new(
            date.and_hms_opt(12, 0, 0).unwrap(),
            vec![continent],
            GlobalCompetitions::new(Vec::new()),
        )
    }

    fn put_monitoring(club: &mut Club, scout_id: u32, player_id: u32, status: ScoutMonitoringStatus) {
        let id = club.transfer_plan.next_monitoring_id();
        let mut row = ScoutPlayerMonitoring::new(
            id,
            scout_id,
            player_id,
            ScoutMonitoringSource::TransferRequest,
            d(2026, 6, 1),
        );
        row.status = status;
        club.transfer_plan.scout_monitoring.push(row);
    }

    fn put_shortlist_candidate(club: &mut Club, request_id: u32, player_id: u32) {
        let mut sl = TransferShortlist::new(request_id, 0.0);
        sl.candidates.push(ShortlistCandidate {
            player_id,
            score: 0.5,
            estimated_fee: 0.0,
            status: ShortlistCandidateStatus::Available,
        });
        club.transfer_plan.shortlists.push(sl);
    }

    fn put_listing(country: &mut Country, player_id: u32, club_id: u32, status: TransferListingStatus) {
        let mut listing = TransferListing::new(
            player_id,
            club_id,
            0,
            CurrencyValue::new(1_000_000.0, Currency::Usd),
            d(2026, 6, 1),
            TransferListingType::Transfer,
        );
        listing.status = status;
        country.transfer_market.listings.push(listing);
    }

    fn put_negotiation(
        country: &mut Country,
        neg_id: u32,
        player_id: u32,
        buying_club_id: u32,
        selling_club_id: u32,
        status: NegotiationStatus,
    ) {
        let offer = TransferOffer::new(
            CurrencyValue::new(1_000_000.0, Currency::Usd),
            buying_club_id,
            d(2026, 6, 1),
        );
        let mut neg = TransferNegotiation::new(
            neg_id,
            player_id,
            0,
            selling_club_id,
            buying_club_id,
            offer,
            d(2026, 6, 1),
            500.0,
            500.0,
            25,
            10.0,
        );
        neg.status = status;
        country.transfer_market.negotiations.insert(neg_id, neg);
    }

    fn put_history(country: &mut Country, player_id: u32, from_club_id: u32, to_club_id: u32) {
        country.transfer_market.transfer_history.push(CompletedTransfer::new(
            player_id,
            "Player".to_string(),
            from_club_id,
            0,
            "From".to_string(),
            to_club_id,
            "To".to_string(),
            d(2026, 6, 5),
            CurrencyValue::new(2_000_000.0, Currency::Usd),
            TransferType::Permanent,
        ));
    }

    #[test]
    fn cleanup_clears_active_monitoring_and_shortlist() {
        // Buying club has a Negotiating-state monitoring row plus a
        // shortlist entry for the player. After the centralized cleanup
        // both should be gone — the UI should not surface the player as
        // actively monitored.
        let player_id: u32 = 100;
        let buyer_club_id: u32 = 1;
        let other_club_id: u32 = 2;
        let mut buyer = make_club(buyer_club_id, "Buyer");
        put_monitoring(&mut buyer, 11, player_id, ScoutMonitoringStatus::Negotiating);
        put_shortlist_candidate(&mut buyer, 1, player_id);

        // A second domestic club had the player on its scouting radar.
        let mut other = make_club(other_club_id, "Other");
        put_monitoring(&mut other, 12, player_id, ScoutMonitoringStatus::Active);

        let country = make_country(1, "UR", "uruguay", vec![buyer, other]);
        let mut data = make_simulator(d(2026, 6, 5), vec![country]);

        PipelineProcessor::cleanup_player_transfer_interest(&mut data, player_id);

        let country = data.country(1).unwrap();
        for club in &country.clubs {
            assert!(
                club.transfer_plan
                    .scout_monitoring
                    .iter()
                    .all(|m| m.player_id != player_id),
                "club {} still has stale monitoring rows for player {}",
                club.id,
                player_id
            );
            for sl in &club.transfer_plan.shortlists {
                assert!(
                    sl.candidates.iter().all(|c| c.player_id != player_id),
                    "club {} still has shortlist candidate for player {}",
                    club.id,
                    player_id
                );
            }
        }
    }

    #[test]
    fn cleanup_completes_listings_and_rejects_active_negotiations() {
        let player_id: u32 = 200;
        let selling_club_id: u32 = 3;
        let buying_club_id: u32 = 4;
        let losing_club_id: u32 = 5;

        let buyer = make_club(buying_club_id, "Buyer");
        let seller = make_club(selling_club_id, "Seller");
        let losing_bidder = make_club(losing_club_id, "Loser");
        let mut country = make_country(1, "EN", "england", vec![buyer, seller, losing_bidder]);

        // An open listing — must be marked Completed.
        put_listing(&mut country, player_id, selling_club_id, TransferListingStatus::Available);
        // The buyer's negotiation got accepted — leave Accepted alone but
        // a parallel bid from the losing club is still Pending and must
        // be rejected.
        put_negotiation(
            &mut country,
            10,
            player_id,
            buying_club_id,
            selling_club_id,
            NegotiationStatus::Accepted,
        );
        put_negotiation(
            &mut country,
            11,
            player_id,
            losing_club_id,
            selling_club_id,
            NegotiationStatus::Pending,
        );
        put_history(&mut country, player_id, selling_club_id, buying_club_id);

        let mut data = make_simulator(d(2026, 6, 5), vec![country]);

        PipelineProcessor::cleanup_player_transfer_interest(&mut data, player_id);

        let country = data.country(1).unwrap();
        // All listings for the player are Completed.
        for listing in &country.transfer_market.listings {
            if listing.player_id == player_id {
                assert_eq!(
                    listing.status,
                    TransferListingStatus::Completed,
                    "listing for player {} not completed",
                    player_id
                );
            }
        }
        // Pending negotiation is now Rejected; Accepted stays Accepted.
        let losing = &country.transfer_market.negotiations[&11];
        assert_eq!(losing.status, NegotiationStatus::Rejected);
        let winning = &country.transfer_market.negotiations[&10];
        assert_eq!(winning.status, NegotiationStatus::Accepted);
        // Transfer history must NOT be deleted.
        assert!(
            country
                .transfer_market
                .transfer_history
                .iter()
                .any(|t| t.player_id == player_id),
            "completed transfer history for player {} was deleted",
            player_id
        );
    }

    #[test]
    fn cross_country_cleanup_clears_both_sides() {
        let player_id: u32 = 300;
        let selling_club_id: u32 = 6;
        let buying_club_id: u32 = 7;
        let third_party_club_id: u32 = 8;

        let mut seller = make_club(selling_club_id, "Seller");
        // Seller's home country had an open listing for the player.
        let mut buyer = make_club(buying_club_id, "Buyer");
        put_monitoring(
            &mut buyer,
            21,
            player_id,
            ScoutMonitoringStatus::Negotiating,
        );
        put_shortlist_candidate(&mut buyer, 1, player_id);

        // A club in a third country scouted the same player — its
        // monitoring row must be cleared too.
        let mut third_party = make_club(third_party_club_id, "ThirdParty");
        put_monitoring(&mut third_party, 31, player_id, ScoutMonitoringStatus::Active);
        // Selling-side staff also had an internal monitoring (e.g. their
        // own academy DoF flagged the player on the way out).
        put_monitoring(&mut seller, 41, player_id, ScoutMonitoringStatus::Active);

        let mut selling_country = make_country(1, "AR", "argentina", vec![seller]);
        // Selling country listing pre-completion.
        put_listing(
            &mut selling_country,
            player_id,
            selling_club_id,
            TransferListingStatus::InNegotiation,
        );
        let buying_country = make_country(2, "ES", "spain", vec![buyer]);
        let third_country = make_country(3, "PT", "portugal", vec![third_party]);

        let mut data = make_simulator(
            d(2026, 6, 5),
            vec![selling_country, buying_country, third_country],
        );

        PipelineProcessor::cleanup_player_transfer_interest(&mut data, player_id);

        // Verify each country has been swept clean.
        for cont in &data.continents {
            for country in &cont.countries {
                for club in &country.clubs {
                    assert!(
                        club.transfer_plan
                            .scout_monitoring
                            .iter()
                            .all(|m| m.player_id != player_id),
                        "country {} club {} still has monitoring for player {}",
                        country.id,
                        club.id,
                        player_id
                    );
                    for sl in &club.transfer_plan.shortlists {
                        assert!(
                            sl.candidates.iter().all(|c| c.player_id != player_id),
                            "country {} club {} still has shortlist candidate",
                            country.id,
                            club.id
                        );
                    }
                }
                for listing in &country.transfer_market.listings {
                    if listing.player_id == player_id {
                        assert_eq!(
                            listing.status,
                            TransferListingStatus::Completed,
                            "country {} listing for player {} not completed",
                            country.id,
                            player_id
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn cleanup_preserves_unrelated_player_interest() {
        // Two players: 400 has been signed; 500 is unrelated. The sweep
        // for 400 must NOT touch 500's monitoring / shortlist entries.
        let signed_id: u32 = 400;
        let other_id: u32 = 500;

        let mut buyer = make_club(1, "Buyer");
        put_monitoring(&mut buyer, 11, signed_id, ScoutMonitoringStatus::Negotiating);
        put_monitoring(&mut buyer, 12, other_id, ScoutMonitoringStatus::Active);
        put_shortlist_candidate(&mut buyer, 1, signed_id);
        put_shortlist_candidate(&mut buyer, 2, other_id);

        let country = make_country(1, "FR", "france", vec![buyer]);
        let mut data = make_simulator(d(2026, 6, 5), vec![country]);

        PipelineProcessor::cleanup_player_transfer_interest(&mut data, signed_id);

        let buyer = &data.country(1).unwrap().clubs[0];
        // Signed player interest is gone.
        assert!(
            buyer
                .transfer_plan
                .scout_monitoring
                .iter()
                .all(|m| m.player_id != signed_id)
        );
        // Other player interest survives.
        assert!(
            buyer
                .transfer_plan
                .scout_monitoring
                .iter()
                .any(|m| m.player_id == other_id),
            "monitoring for unrelated player wiped"
        );
        let still_shortlisted = buyer
            .transfer_plan
            .shortlists
            .iter()
            .any(|s| s.candidates.iter().any(|c| c.player_id == other_id));
        assert!(still_shortlisted, "unrelated player removed from shortlist");
    }
}
