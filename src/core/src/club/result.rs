use crate::club::academy::result::ClubAcademyResult;
use crate::club::player::calculators::{
    AutomaticReleaseEligibility, ContractValuation, FreeAgentReleaseReason,
    ReleaseEligibilityContext, ValuationContext,
};
use crate::club::player::contract::{
    AffordabilityInput, ContractStalemate, RENEWAL_OFFERED_LABEL, RENEWAL_REJECTED_LABEL,
};
use crate::club::staff::perception::PotentialEstimator;
use crate::club::team::squad::SquadAssetContext;
use crate::club::{BoardResult, ClubFinanceResult};
use crate::league::result::LeagueProcessAccess;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::{CompletedTransfer, TransferListing, TransferListingType};
use crate::utils::{DateUtils, FormattingUtils};
use crate::{
    Player, PlayerContractProposal, PlayerMessage, PlayerMessageType, PlayerResult,
    PlayerSquadStatus, PlayerStatusType, SimulationResult, StaffStatus, TeamResult, TransferItem,
};
use chrono::NaiveDate;
use log::debug;

enum UnresolvedSalaryDecision {
    TransferList,
    FreeTransfer,
}

/// Why the salary fallback was invoked. Drives decision_history reason
/// and listing pricing so the player page surfaces "contract talks
/// stalled" instead of an opaque "transfer listed".
#[derive(Debug, Clone, Copy)]
enum UnresolvedSalaryTrigger {
    /// Player rejected a renewal offer enough times that the stalemate
    /// assessment now permits listing.
    FailedRenewal,
    /// Player wants a raise but is `NotNeeded` in the squad plan — club
    /// won't sink wages into someone they want to move on anyway.
    NotNeededWantsRaise,
}

impl UnresolvedSalaryTrigger {
    fn list_reason(self) -> &'static str {
        match self {
            UnresolvedSalaryTrigger::FailedRenewal => "dec_reason_contract_stalemate",
            UnresolvedSalaryTrigger::NotNeededWantsRaise => "dec_reason_surplus_squad",
        }
    }

    /// Explicit free-agent exit reason for the release branch. A failed
    /// renewal reads as its own narrative; a `NotNeeded` player who wanted
    /// a raise the club won't fund is a squad-surplus free release, NOT a
    /// negotiated mutual termination — keep the two distinct in history.
    fn release_reason(self) -> FreeAgentReleaseReason {
        match self {
            UnresolvedSalaryTrigger::FailedRenewal => {
                FreeAgentReleaseReason::FailedRenewalRelease
            }
            UnresolvedSalaryTrigger::NotNeededWantsRaise => {
                FreeAgentReleaseReason::SurplusFreeRelease
            }
        }
    }
}

pub struct ClubResult {
    pub club_id: u32,
    pub finance: ClubFinanceResult,
    pub teams: Vec<TeamResult>,
    pub board: BoardResult,
    pub academy: ClubAcademyResult,
    pub academy_transfers: Vec<CompletedTransfer>,
    /// Academy prospects who aged out (turned 18 without being graduated
    /// to the U18 pathway). They have their contract cleared and an
    /// `Frt` status stamped before being surfaced here; the country
    /// drain routes them into the global `data.free_agents` pool so
    /// they remain discoverable players instead of vanishing.
    pub academy_released_players: Vec<Player>,
}

impl ClubResult {
    pub fn new(
        club_id: u32,
        finance: ClubFinanceResult,
        teams: Vec<TeamResult>,
        board: BoardResult,
        academy: ClubAcademyResult,
    ) -> Self {
        ClubResult {
            club_id,
            finance,
            teams,
            board,
            academy,
            academy_transfers: Vec::new(),
            academy_released_players: Vec::new(),
        }
    }

    pub fn process<D: LeagueProcessAccess>(self, data: &mut D, _result: &mut SimulationResult) {
        self.finance.process(data);
        self.process_teams(data);
        self.board.process(data);
        self.academy.process(data);
    }

    fn process_teams<D: LeagueProcessAccess>(&self, data: &mut D) {
        for team_result in &self.teams {
            for player_result in &team_result.players.players {
                if player_result.has_contract_actions() {
                    Self::process_player_contract_interaction(player_result, data, self.club_id);
                }
            }

            team_result.process(data);
        }
    }

    pub(crate) fn process_player_contract_interaction<D: LeagueProcessAccess>(
        result: &PlayerResult,
        data: &mut D,
        club_id: u32,
    ) {
        let is_on_loan = data
            .player(result.player_id)
            .map(|p| p.is_on_loan())
            .unwrap_or(false);

        // Contract rejected — escalation is gated on the rolling
        // contract-stalemate assessment, NOT on a single rejection.
        // Loaned players can't be transfer-listed remotely by the parent
        // club; the rejection still records in history so the parent
        // sees the failed talk next assessment.
        if result.contract.contract_rejected {
            if !is_on_loan && Self::stalemate_permits_listing(result.player_id, data, club_id) {
                Self::handle_unresolved_salary(
                    result.player_id,
                    data,
                    club_id,
                    UnresolvedSalaryTrigger::FailedRenewal,
                );
            }
            // First / mid-stalemate rejections: the renewal manager will
            // come back next cycle with an improved offer informed by
            // `pending_contract_ask`. No listing yet.
            return;
        }

        // Reactive renewal cooldown. process_contract sets
        // want_extend_contract every tick the contract is < 1 year from
        // expiry, and process_happiness sets want_improve_contract every
        // week the player is salary-unhappy. Without a gate here, the
        // club would send a fresh proposal each tick and pile up
        // rejection entries. Reuse the same retry rules as the proactive
        // ContractRenewalManager: 60-day cooldown after an offer, 120-day
        // cooldown after a rejection, max 3 attempts per rolling year.
        const OFFER_COOLDOWN_DAYS: i64 = 60;
        const REJECT_COOLDOWN_DAYS: i64 = 120;
        const MAX_ATTEMPTS_PER_YEAR: usize = 3;
        let today = data.date().date();
        if let Some(player) = data.player(result.player_id) {
            let last = player.decision_history.items.iter().rev().find(|d| {
                d.decision == RENEWAL_OFFERED_LABEL || d.decision == RENEWAL_REJECTED_LABEL
            });
            if let Some(d) = last {
                let days = (today - d.date).num_days();
                let cooldown = if d.decision == RENEWAL_REJECTED_LABEL {
                    REJECT_COOLDOWN_DAYS
                } else {
                    OFFER_COOLDOWN_DAYS
                };
                if days < cooldown {
                    return;
                }
            }
            let attempts = player
                .decision_history
                .items
                .iter()
                .filter(|d| {
                    d.decision == RENEWAL_OFFERED_LABEL && (today - d.date).num_days() < 365
                })
                .count();
            if attempts >= MAX_ATTEMPTS_PER_YEAR {
                return;
            }
        }

        if result.contract.no_contract
            || result.contract.want_improve_contract
            || result.contract.want_extend_contract
        {
            // For loaned players, only handle parent contract extensions
            // Salary improvements are between the player and borrowing club — not relevant here
            if is_on_loan && !result.contract.want_extend_contract {
                return;
            }

            // Resolve which club handles the contract: parent club for loaned players
            let contract_club_id = if is_on_loan {
                match data
                    .player(result.player_id)
                    .and_then(|p| p.contract_loan.as_ref())
                    .and_then(|c| c.loan_from_club_id)
                {
                    Some(id) => id,
                    None => return,
                }
            } else {
                club_id
            };

            // Step 1: Resolve contract renewal staff, wage budget, and current wage bill
            // Uses the parent club's context for loaned players. Also pull
            // club + league reputation so the reactive offer can run through
            // the same ContractValuation as the proactive renewal pass.
            let (
                negotiation_skill,
                judging_ability,
                wage_budget,
                current_wage_bill,
                club_rep_score,
                league_reputation,
            ) = data
                .club(contract_club_id)
                .map(|club| {
                    let main_team = club.teams.teams.first();
                    let (neg, judge) = main_team
                        .map(|team| {
                            let staff = team
                                .staffs
                                .responsibility
                                .contract_renewal
                                .handle_first_team_contracts
                                .and_then(|id| team.staffs.find(id));
                            match staff {
                                Some(s) => {
                                    let is_active = s
                                        .contract
                                        .as_ref()
                                        .map(|c| matches!(c.status, StaffStatus::Active))
                                        .unwrap_or(false);
                                    if is_active {
                                        (
                                            s.staff_attributes.mental.man_management,
                                            s.staff_attributes.knowledge.judging_player_ability,
                                        )
                                    } else {
                                        (3u8, 3u8)
                                    }
                                }
                                None => (5u8, 5u8),
                            }
                        })
                        .unwrap_or((5, 5));

                    let wb = club
                        .board
                        .season_targets
                        .as_ref()
                        .map(|t| t.wage_budget as u32)
                        .unwrap_or(0);

                    let total_wages: u32 = club.teams.iter().map(|t| t.get_annual_salary()).sum();

                    let club_rep = main_team
                        .map(|t| t.reputation.overall_score())
                        .unwrap_or(0.5);
                    let league_id = main_team.and_then(|t| t.league_id);
                    let league_rep = league_id
                        .and_then(|lid| data.league(lid))
                        .map(|l| l.reputation)
                        .unwrap_or(5_000);

                    (neg, judge, wb, total_wages, club_rep, league_rep)
                })
                .unwrap_or((5, 5, 0, 0, 0.5, 5_000));

            // Step 2: Read player info (immutable)
            let player = match data.player(result.player_id) {
                Some(p) => p,
                None => return,
            };

            let current_salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
            let ability = player.player_attributes.current_ability;
            let age = DateUtils::age(player.birth_date, data.date().date());

            // Reactive renewal salary now flows through ContractValuation,
            // the same model used by the proactive renewal pass and salary
            // happiness — otherwise the three systems disagree about what a
            // fair wage looks like and chase each other in circles.
            let squad_status = player
                .contract
                .as_ref()
                .map(|c| c.squad_status.clone())
                .unwrap_or(PlayerSquadStatus::FirstTeamRegular);
            let is_not_needed = matches!(squad_status, PlayerSquadStatus::NotNeeded);

            // Not needed players don't get raises — transfer list or
            // release instead. This path doesn't require a prior
            // rejection: the squad plan already says "we don't want
            // this player", and they're asking for more money. Going
            // through the stalemate gate would be a no-op (NotNeeded
            // permits listing at Emerging, which a fresh "wants raise"
            // signal satisfies).
            if !is_on_loan && result.contract.want_improve_contract && is_not_needed {
                Self::handle_unresolved_salary(
                    result.player_id,
                    data,
                    club_id,
                    UnresolvedSalaryTrigger::NotNeededWantsRaise,
                );
                return;
            }

            let months_remaining = player
                .contract
                .as_ref()
                .map(|c| ((c.expiration - data.date().date()).num_days() / 30).max(0) as i32)
                .unwrap_or(0);
            let has_market_interest = player.statuses.get().iter().any(|s| {
                matches!(
                    s,
                    PlayerStatusType::Wnt | PlayerStatusType::Enq | PlayerStatusType::Bid
                )
            });

            let valuation_ctx = ValuationContext {
                age,
                club_reputation_score: club_rep_score,
                league_reputation,
                squad_status: squad_status.clone(),
                current_salary,
                months_remaining,
                has_market_interest,
            };
            let valuation = ContractValuation::evaluate(player, &valuation_ctx);

            // Staff judging_ability narrows the offer around the unified
            // target: poor staff under-offer (~85%), elite ones land within
            // ±15% of the right number.
            let accuracy = 0.85 + (judging_ability as f32 / 20.0) * 0.30;
            let target = (valuation.expected_wage as f32 * accuracy) as u32;

            // Anchor never below current salary plus a token bump.
            let mut offered_salary = target
                .max(current_salary + current_salary / 20)
                .max(current_salary + 1);

            // Salary-unhappy reactive offer: lean toward the player's own
            // valuation rather than asking them to settle for a band-based
            // average. Use max(target, expected) here so we don't underpay
            // when the staff filter clipped it back.
            if !is_on_loan && result.contract.want_improve_contract {
                offered_salary = offered_salary.max(valuation.expected_wage);
            }
            let _ = ability; // silence unused-warning if future logic drops it

            // Converge toward the player's own ask when we have it. When
            // affordability evidence says the ask fits wage headroom, match
            // it outright — repeated split-the-gap underbids let the
            // rejection cap fire long before either side compromises, and
            // the stalemate escalates to a listing for a deal the club
            // could have closed.
            if let Some(ask) = &player.pending_contract_ask {
                if ask.desired_salary > offered_salary {
                    let headroom = if wage_budget > 0 {
                        Some(wage_budget.saturating_sub(current_wage_bill))
                    } else {
                        None
                    };
                    let stalemate = ContractStalemate::assess(
                        player,
                        data.date().date(),
                        AffordabilityInput {
                            wage_budget_headroom: headroom,
                            current_salary,
                        },
                    );
                    if stalemate.should_improve_offer() {
                        offered_salary = ask.desired_salary;
                    } else {
                        offered_salary = (offered_salary + ask.desired_salary) / 2;
                    }
                }
            }

            // Wage budget enforcement: don't offer salary that would bust the budget
            // The new salary replaces the current one, so check the net increase
            let salary_increase = offered_salary.saturating_sub(current_salary);
            let final_salary =
                if wage_budget > 0 && current_wage_bill + salary_increase > wage_budget {
                    // Over budget: cap the offer to what the budget allows
                    let remaining = wage_budget.saturating_sub(current_wage_bill);
                    let capped_salary = current_salary + remaining;
                    // If we can't even match current salary the renewal
                    // is structurally unaffordable. We don't auto-list
                    // here — that would skip the rejection ladder. Skip
                    // the offer this tick instead; the player stays
                    // salary-unhappy, may eventually go UNH/REQ, and the
                    // country pipeline picks that up explicitly. Listing
                    // from this branch would jump straight to "club
                    // forced him out" without the negotiation history
                    // the stalemate model is built to track.
                    if capped_salary <= current_salary
                        && result.contract.want_improve_contract
                        && !is_on_loan
                    {
                        return;
                    }
                    capped_salary.max(current_salary)
                } else {
                    offered_salary
                };

            let years =
                negotiate_contract_years(player, age, negotiation_skill, data.date().date());

            // Reactive path stays lean on sweeteners — the player has
            // asked for the renewal themselves, so greed is usually not
            // the blocker. Attach loyalty bonus for veterans to cover the
            // same case where a 30+ player hesitates on length.
            let loyalty_bonus = if age >= 30 {
                (final_salary as f32 * 0.10) as u32
            } else {
                0
            };

            let mut reactive_proposal = PlayerContractProposal::basic(
                final_salary,
                years,
                negotiation_skill,
                0,
                loyalty_bonus,
                None,
            );
            // Pass the valuation context through so player acceptance
            // evaluates against the same club/league expectations this
            // reactive offer was built from — matches the proactive path.
            reactive_proposal.valuation_club_reputation = Some(club_rep_score);
            reactive_proposal.valuation_league_reputation = Some(league_reputation);
            reactive_proposal.valuation_expected_wage = Some(valuation.expected_wage);
            reactive_proposal.valuation_min_acceptable = Some(valuation.min_acceptable);

            Self::deliver_message(
                data,
                club_id,
                result.player_id,
                PlayerMessage {
                    message_type: PlayerMessageType::ContractProposal(reactive_proposal),
                },
            );

            // Record the offer so the cooldown + attempt cap at the top
            // of this function can see it next tick. Without this the
            // reactive path would chain proposals every cycle while the
            // player kept signalling want_improve/extend_contract.
            let movement = format!(
                "{}y · ${}/y",
                years,
                FormattingUtils::format_money(final_salary as f64)
            );
            let offer_date = data.date().date();
            if let Some(player_mut) = data.player_mut(result.player_id) {
                player_mut.decision_history.add(
                    offer_date,
                    movement,
                    RENEWAL_OFFERED_LABEL.to_string(),
                    String::new(),
                );
            }
        }

        /// Contract duration negotiation.
        ///
        /// Player wants: long contract (job security, commitment signal)
        /// Club wants: shorter contract (flexibility if player declines)
        ///
        /// Factors:
        /// - Age: clubs offer shorter deals to older players; young stars get longer
        /// - Ability/reputation: high-profile players demand and get longer deals
        /// - Loyalty: loyal players accept shorter deals (trust the club)
        /// - Ambition: ambitious players push for longer deals (higher commitment)
        /// - Other club interest (Wnt/Enq/Bid statuses): gives player leverage for longer deals
        /// - Staff negotiation skill: better negotiator → result closer to club's preference
        fn negotiate_contract_years(
            player: &Player,
            age: u8,
            negotiation_skill: u8,
            date: NaiveDate,
        ) -> u8 {
            let ability = player.player_attributes.current_ability;
            let reputation = player.player_attributes.current_reputation;
            let loyalty = player.attributes.loyalty;
            let ambition = player.attributes.ambition;

            // --- Player desired years (what the agent demands) ---
            let mut player_years: f32 = 3.0;

            // High reputation players demand longer contracts (security)
            if reputation > 7000 {
                player_years += 2.0;
            } else if reputation > 4000 {
                player_years += 1.0;
            }

            // High ability players want commitment
            if ability > 150 {
                player_years += 1.0;
            } else if ability > 120 {
                player_years += 0.5;
            }

            // Young players whose VISIBLE trajectory points up want
            // long-term deals — agent and club negotiate on the
            // observable ceiling, not the hidden biological PA.
            if age < 24
                && PotentialEstimator::observable_ceiling(player, date) > ability.saturating_add(20)
            {
                player_years += 1.0;
            }

            // Ambitious players push for longer contracts
            // ambition is 0-20
            if ambition > 15.0 {
                player_years += 1.0;
            } else if ambition > 10.0 {
                player_years += 0.5;
            }

            // Low loyalty = wants flexibility to move, shorter preferred
            if loyalty < 5.0 {
                player_years -= 1.0;
            } else if loyalty < 10.0 {
                player_years -= 0.5;
            }

            // Other club interest gives player leverage → pushes for longer commitment
            let has_interest = player.statuses.get().iter().any(|s| {
                matches!(
                    s,
                    PlayerStatusType::Wnt | PlayerStatusType::Enq | PlayerStatusType::Bid
                )
            });
            if has_interest {
                player_years += 1.0;
            }

            // Older players know they can't demand as much
            if age >= 34 {
                player_years -= 2.0;
            } else if age >= 32 {
                player_years -= 1.0;
            } else if age >= 30 {
                player_years -= 0.5;
            }

            // --- Club desired years (what the club wants to offer) ---
            let mut club_years: f32 = 3.0;

            // Club wants shorter deals for older players (decline risk)
            if age >= 34 {
                club_years = 1.0;
            } else if age >= 32 {
                club_years = 1.5;
            } else if age >= 30 {
                club_years = 2.0;
            }

            // Club wants to lock in young prospects (protect investment)
            if age < 22 && ability > 80 {
                club_years += 2.0;
            } else if age < 24 {
                club_years += 1.0;
            }

            // Club wants to lock in star players
            if ability > 150 {
                club_years += 1.5;
            } else if ability > 120 {
                club_years += 1.0;
            }

            // Low ability/rotation players: club wants short deals
            if ability < 70 {
                club_years -= 1.0;
            }

            // --- Negotiation: compromise between player and club ---
            // Staff negotiation skill (0-20) determines how much the club gets its way
            // 0 skill → 50/50 split, 20 skill → 80% club's preference
            let staff_weight = 0.5 + (negotiation_skill as f32 / 20.0) * 0.3; // 0.5 to 0.8
            let negotiated = club_years * staff_weight + player_years * (1.0 - staff_weight);

            // Clamp to realistic range: 1-5 years
            (negotiated.round() as u8).clamp(1, 5)
        }
    }

    /// Wage budget headroom for the club's main team. Used to evaluate
    /// affordability of the player's `pending_contract_ask`; missing
    /// when the board hasn't set season targets (test fixtures, edge
    /// cases) — in which case the stalemate falls back to its
    /// rejection-count based rules without an affordability signal.
    fn wage_budget_headroom<D: LeagueProcessAccess>(data: &D, club_id: u32) -> Option<u32> {
        let club = data.club(club_id)?;
        let budget = club
            .board
            .season_targets
            .as_ref()
            .map(|t| t.wage_budget as u32)?;
        let total_wages: u32 = club.teams.iter().map(|t| t.get_annual_salary()).sum();
        Some(budget.saturating_sub(total_wages))
    }

    /// Run the stalemate gate against a player so the club only acts on
    /// a renewal rejection when the negotiation has genuinely failed.
    fn stalemate_permits_listing<D: LeagueProcessAccess>(
        player_id: u32,
        data: &D,
        club_id: u32,
    ) -> bool {
        let today = data.date().date();
        let headroom = Self::wage_budget_headroom(data, club_id);
        let Some(player) = data.player(player_id) else {
            return false;
        };
        let current_salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
        let stalemate = ContractStalemate::assess(
            player,
            today,
            AffordabilityInput {
                wage_budget_headroom: headroom,
                current_salary,
            },
        );
        stalemate.permits_listing()
    }

    /// When club can't resolve salary unhappiness (rejected proposal, over budget, not needed):
    /// decide whether to transfer list, release on free transfer, or do nothing.
    fn handle_unresolved_salary<D: LeagueProcessAccess>(
        player_id: u32,
        data: &mut D,
        club_id: u32,
        trigger: UnresolvedSalaryTrigger,
    ) {
        let date = data.date().date();

        // Gather decision info from immutable access
        let (decision, asking_price, team_id) = {
            let club = match data.club(club_id) {
                Some(c) => c,
                None => return,
            };
            let main_team = match club.teams.teams.first() {
                Some(t) => t,
                None => return,
            };
            let squad_avg_ability = main_team.players.current_ability_avg();
            let squad_avg = squad_avg_ability as i16;
            let annual_wage_bill: u32 = club.teams.iter().map(|t| t.get_annual_salary()).sum();
            let team_id = main_team.id;
            let club_rep = main_team.reputation.market_value_score();
            let league_rep = main_team
                .league_id
                .and_then(|lid| data.league(lid))
                .map(|l| l.reputation)
                .unwrap_or(5_000);

            let player = match data.player(player_id) {
                Some(p) => p,
                None => return,
            };

            // Already listed — don't re-process
            let statuses = player.statuses.get();
            if statuses.contains(&PlayerStatusType::Lst)
                || statuses.contains(&PlayerStatusType::Frt)
            {
                return;
            }

            // Manager-pinned players: skip the salary-dispute fallback
            // entirely. Neither transfer-list nor free-release applies.
            if player.is_force_match_selection {
                return;
            }

            // Defence in depth: loaned players are handled by their
            // parent club, never the borrower. The rejection path above
            // already filters this out, but `handle_unresolved_salary`
            // is also reachable from the NotNeeded branch and tests.
            if player.is_on_loan() {
                return;
            }

            let ability = player.player_attributes.current_ability as i16;
            let age = DateUtils::age(player.birth_date, date);
            let loyalty = player.attributes.loyalty;

            // Central squad-asset classification — computed once and reused
            // for both the keep guard and the release gate below. Inferring
            // the role here is what protects a still-`NotYetSet` but useful
            // senior from being walked for free over a salary dispute.
            let asset_class = SquadAssetContext::build(club, date).classify(player, date);
            let formal_key = player
                .contract
                .as_ref()
                .map(|c| {
                    matches!(
                        c.squad_status,
                        PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
                    )
                })
                .unwrap_or(false);
            // A formally-designated OR inferred-core/first-team player is
            // treated as key for the "keep trying" guard.
            let is_key = formal_key || asset_class.is_first_team_protected();

            // Key players and first-team regulars: club keeps trying — don't list them
            // unless they're well below squad average
            if is_key && ability >= squad_avg - 10 {
                return;
            }

            // Loyal players get more patience
            if loyalty > 14.0 && ability >= squad_avg - 10 {
                return;
            }

            // Competitive players (within 15 CA of squad avg): keep trying regardless of age
            // A 32-year-old with ability near the squad average is still valuable
            if ability >= squad_avg - 15 && age < 35 {
                return;
            }

            // Free release only when the central eligibility gate agrees:
            // clearly below team level, negligible market value, and a
            // severance the club can shrug off. Anything it blocks —
            // still useful, sellable, or expensive to pay off — goes to
            // the transfer list instead of walking for free.
            let market_value = player.value(date, league_rep, club_rep);
            let release_ctx = ReleaseEligibilityContext {
                date,
                squad_avg_ability,
                market_value,
                annual_wage_bill,
                asset_class,
            };
            let decision = match AutomaticReleaseEligibility::assess(player, &release_ctx) {
                None => UnresolvedSalaryDecision::FreeTransfer,
                Some(block) => {
                    debug!(
                        "salary-fallback free release blocked for {} (id={}): {:?} — CA {} vs \
                         squad avg {}, value={:.0}, severance={} → transfer-listing instead",
                        player.full_name,
                        player_id,
                        block,
                        ability,
                        squad_avg,
                        market_value,
                        player
                            .contract
                            .as_ref()
                            .map(|c| c.termination_cost(date))
                            .unwrap_or(0),
                    );
                    UnresolvedSalaryDecision::TransferList
                }
            };

            let asking_price = CurrencyValue {
                amount: FormattingUtils::round_fee(market_value),
                currency: Currency::Usd,
            };
            (decision, asking_price, team_id)
        };

        let listed_now = matches!(decision, UnresolvedSalaryDecision::TransferList);

        // Apply player-side state under club_mut. The mirror into
        // `team.transfer_list` keeps the web team-transfers page in sync
        // with the country-level market: the buying pipeline reads the
        // country market, the team UI reads its own list, and both must
        // see the same listing or the player effectively disappears from
        // half the surface. Both writes are idempotent (`Transfers::add`
        // and `TransferMarket::add_listing` both skip duplicates), so
        // repeated cycles don't create stutter.
        {
            let club = match data.club_mut(club_id) {
                Some(c) => c,
                None => return,
            };
            // Coach name is resolved from the main team so the decision
            // history attributes the listing to the manager, not the
            // stub. Falls back to the board key when the seat is empty.
            let mut coach_name = String::new();
            for team in &mut club.teams.teams {
                if team.id == team_id {
                    coach_name = team.staffs.head_coach_name();
                }
                if let Some(player) = team.players.players.iter_mut().find(|p| p.id == player_id) {
                    let decided_by = if coach_name.is_empty() {
                        "dec_decided_board".to_string()
                    } else {
                        coach_name.clone()
                    };
                    match decision {
                        UnresolvedSalaryDecision::FreeTransfer => {
                            let release_reason = trigger.release_reason();
                            player.statuses.add(date, PlayerStatusType::Frt);
                            player.contract = None;
                            // Stamp the explicit exit so the free-agent
                            // sweep records this specific reason rather
                            // than collapsing it to "mutual agreement".
                            player.set_release_reason(release_reason);
                            player.decision_history.add(
                                date,
                                "dec_free_transfer_listed".to_string(),
                                release_reason.history_reason().to_string(),
                                decided_by,
                            );
                        }
                        UnresolvedSalaryDecision::TransferList => {
                            player.statuses.add(date, PlayerStatusType::Lst);
                            if let Some(ref mut contract) = player.contract {
                                contract.is_transfer_listed = true;
                            }
                            player.decision_history.add(
                                date,
                                "dec_transfer_listed".to_string(),
                                trigger.list_reason().to_string(),
                                decided_by,
                            );
                        }
                    }
                    break;
                }
            }

            // Mirror the listing onto the team's own transfer_list so the
            // web team-transfers page surfaces stalemate listings. Only
            // the actual selling team carries the item; loaned players
            // were filtered out earlier, so the parent team is always the
            // current team.
            if listed_now {
                if let Some(team) = club.teams.teams.iter_mut().find(|t| t.id == team_id) {
                    team.transfer_list
                        .add(TransferItem::new(player_id, asking_price.clone()));
                }
            }
        }

        // Push the listing into the country transfer market so the
        // buying pipeline can actually discover it. Without this step
        // the country pipeline's "already listed" guard short-circuits
        // and the listing never reaches the market.
        if listed_now {
            if let Some(country_id) = data.country_by_club(club_id).map(|c| c.id) {
                if let Some(country) = data.country_mut(country_id) {
                    country.transfer_market.add_listing(TransferListing::new(
                        player_id,
                        club_id,
                        team_id,
                        asking_price,
                        date,
                        TransferListingType::Transfer,
                    ));
                }
            }
        }
    }

    fn deliver_message<D: LeagueProcessAccess>(
        data: &mut D,
        club_id: u32,
        player_id: u32,
        message: PlayerMessage,
    ) {
        let club = match data.club_mut(club_id) {
            Some(c) => c,
            None => return,
        };

        for team in &mut club.teams.teams {
            if let Some(player) = team.players.players.iter_mut().find(|p| p.id == player_id) {
                player.mailbox.push(message);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::board::SeasonTargets;
    use crate::club::player::contract::{RENEWAL_OFFERED_LABEL, RENEWAL_REJECTED_LABEL};
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::club::player::mailbox::{PlayerContractAsk, RejectionReason};
    use crate::club::player::personality::PlayerDecisionHistory;
    use crate::competitions::global::GlobalCompetitions;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::simulator::SimulatorData;
    use crate::{
        Club, ClubColors, ClubFinances, ClubStatus, Country, PersonAttributes, Player,
        PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, StaffCollection, TeamBuilder, TeamCollection,
        TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn make_decision_history(items: &[(NaiveDate, &str)]) -> PlayerDecisionHistory {
        let mut h = PlayerDecisionHistory::new();
        for (date, decision) in items {
            h.add(
                *date,
                "test".to_string(),
                decision.to_string(),
                "tester".to_string(),
            );
        }
        h
    }

    fn make_contract(
        salary: u32,
        squad_status: PlayerSquadStatus,
        expiration: NaiveDate,
    ) -> PlayerClubContract {
        let mut c = PlayerClubContract::new(salary, expiration);
        c.squad_status = squad_status;
        c
    }

    fn make_player(
        id: u32,
        ability: u8,
        birth_date: NaiveDate,
        decisions: PlayerDecisionHistory,
        contract: Option<PlayerClubContract>,
    ) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = ability;
        attrs.potential_ability = ability;
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Test".into(), format!("Player{}", id)))
            .birth_date(birth_date)
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 20,
                }],
            })
            .player_attributes(attrs)
            .decision_history(decisions)
            .contract(contract)
            .build()
            .unwrap()
    }

    fn training_schedule() -> TrainingSchedule {
        use chrono::NaiveTime;
        TrainingSchedule::new(
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
        )
    }

    fn make_team(id: u32, club_id: u32, players: Vec<Player>) -> crate::Team {
        TeamBuilder::new()
            .id(id)
            .league_id(Some(1))
            .club_id(club_id)
            .name("Main".to_string())
            .slug("main".to_string())
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(500, 500, 500))
            .training_schedule(training_schedule())
            .build()
            .unwrap()
    }

    fn make_club(id: u32, team: crate::Team, wage_budget: Option<i32>) -> Club {
        let mut club = Club::new(
            id,
            "Club".to_string(),
            Location::new(1),
            ClubFinances::new(10_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![team]),
            crate::ClubFacilities::default(),
        );
        if let Some(budget) = wage_budget {
            club.board.season_targets = Some(SeasonTargets {
                transfer_budget: 0,
                wage_budget: budget,
                max_squad_size: 30,
                min_squad_size: 18,
                expected_position: 5,
                min_acceptable_position: 10,
            });
        }
        club
    }

    fn make_country(club: Club) -> Country {
        let league = League::new(
            1,
            "L".to_string(),
            "l".to_string(),
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
            .code("EN".to_string())
            .slug("en".to_string())
            .name("England".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(vec![league]))
            .clubs(vec![club])
            .build()
            .unwrap()
    }

    fn make_sim(country: Country, today: NaiveDate) -> SimulatorData {
        let continent =
            crate::continent::Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
        SimulatorData::new(
            today.and_hms_opt(12, 0, 0).unwrap(),
            vec![continent],
            GlobalCompetitions::new(Vec::new()),
        )
    }

    fn rejected_result(player_id: u32) -> PlayerResult {
        let mut r = PlayerResult::new(player_id);
        r.contract.contract_rejected = true;
        r
    }

    // ── Tests ───────────────────────────────────────────────────────

    #[test]
    fn first_rejection_does_not_list_useful_player() {
        let today = d(2026, 5, 1);
        // One rejection, FirstTeamRegular — stalemate is Emerging, must
        // not list.
        let history = make_decision_history(&[
            (d(2026, 4, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 4, 15), RENEWAL_REJECTED_LABEL),
        ]);
        let contract = make_contract(50_000, PlayerSquadStatus::FirstTeamRegular, d(2027, 7, 1));
        let player = make_player(101, 120, d(1995, 1, 1), history, Some(contract));
        let team = make_team(10, 100, vec![player]);
        let club = make_club(100, team, Some(5_000_000));
        let country = make_country(club);
        let mut sim = make_sim(country, today);

        ClubResult::process_player_contract_interaction(&rejected_result(101), &mut sim, 100);

        let p = sim.player(101).expect("player still present");
        assert!(
            !p.statuses.get().contains(&PlayerStatusType::Lst),
            "first rejection must not transfer-list"
        );
        let country = sim.country(1).unwrap();
        assert!(country.transfer_market.listings.is_empty());
        assert!(
            country.clubs[0].teams.teams[0]
                .transfer_list
                .listed_player_ids()
                .is_empty()
        );
    }

    #[test]
    fn three_unaffordable_rejections_list_into_market_and_team() {
        let today = d(2026, 5, 1);
        let history = make_decision_history(&[
            (d(2026, 1, 10), RENEWAL_OFFERED_LABEL),
            (d(2026, 1, 25), RENEWAL_REJECTED_LABEL),
            (d(2026, 3, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 3, 18), RENEWAL_REJECTED_LABEL),
            (d(2026, 4, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 4, 25), RENEWAL_REJECTED_LABEL),
        ]);
        let contract = make_contract(50_000, PlayerSquadStatus::MainBackupPlayer, d(2027, 7, 1));
        // Player with low ability so the TransferList routing (rather
        // than FreeTransfer) is chosen and squad-protection gates don't
        // fire — squad_avg is the only-other player's ability.
        let player = make_player(101, 75, d(1995, 1, 1), history, Some(contract));
        let teammate = make_player(
            102,
            125,
            d(1993, 1, 1),
            PlayerDecisionHistory::new(),
            Some(make_contract(
                60_000,
                PlayerSquadStatus::FirstTeamRegular,
                d(2027, 7, 1),
            )),
        );
        let team = make_team(10, 100, vec![player, teammate]);
        // Tight budget so the affordability signal is "ask unaffordable"
        // — but the rejection-count alone already pushes to Exhausted
        // when there's no pending ask, which is the case here.
        let club = make_club(100, team, Some(120_000));
        let country = make_country(club);
        let mut sim = make_sim(country, today);

        ClubResult::process_player_contract_interaction(&rejected_result(101), &mut sim, 100);

        let p = sim.player(101).expect("player still present");
        assert!(
            p.statuses.get().contains(&PlayerStatusType::Lst),
            "exhausted stalemate must transfer-list"
        );
        let listing_reason = p
            .decision_history
            .items
            .iter()
            .find(|d| d.movement == "dec_transfer_listed")
            .expect("decision_history must record dec_transfer_listed");
        assert_eq!(listing_reason.decision, "dec_reason_contract_stalemate");

        let country = sim.country(1).unwrap();
        assert_eq!(
            country.transfer_market.listings.len(),
            1,
            "country market must contain the listing"
        );
        assert_eq!(country.transfer_market.listings[0].player_id, 101);
        let team_list_ids = country.clubs[0].teams.teams[0]
            .transfer_list
            .listed_player_ids();
        assert_eq!(
            team_list_ids,
            vec![101],
            "team transfer_list must mirror the market listing"
        );
    }

    #[test]
    fn affordable_pending_ask_does_not_list_protected_player() {
        let today = d(2026, 5, 1);
        let history = make_decision_history(&[
            (d(2026, 1, 10), RENEWAL_OFFERED_LABEL),
            (d(2026, 1, 25), RENEWAL_REJECTED_LABEL),
            (d(2026, 3, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 3, 18), RENEWAL_REJECTED_LABEL),
            (d(2026, 4, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 4, 25), RENEWAL_REJECTED_LABEL),
        ]);
        let contract = make_contract(50_000, PlayerSquadStatus::FirstTeamRegular, d(2027, 7, 1));
        let mut player = make_player(101, 120, d(1995, 1, 1), history, Some(contract));
        // Affordable ask: 70k vs current 50k, headroom 200k → fits.
        player.pending_contract_ask = Some(PlayerContractAsk {
            desired_salary: 70_000,
            desired_years: 3,
            recorded_on: d(2026, 4, 25),
            demanded_status: None,
            demanded_release_clause: None,
            demanded_signing_bonus: None,
            rejection_reason: Some(RejectionReason::LowSalary),
        });
        let team = make_team(10, 100, vec![player]);
        // wage_budget = 250k, current_wage_bill = 50k, headroom = 200k.
        let club = make_club(100, team, Some(250_000));
        let country = make_country(club);
        let mut sim = make_sim(country, today);

        ClubResult::process_player_contract_interaction(&rejected_result(101), &mut sim, 100);

        let p = sim.player(101).unwrap();
        assert!(
            !p.statuses.get().contains(&PlayerStatusType::Lst),
            "affordable ask must NOT escalate FirstTeamRegular to a listing"
        );
        let country = sim.country(1).unwrap();
        assert!(country.transfer_market.listings.is_empty());
    }

    #[test]
    fn force_selected_player_is_not_listed() {
        let today = d(2026, 5, 1);
        let history = make_decision_history(&[
            (d(2026, 1, 10), RENEWAL_OFFERED_LABEL),
            (d(2026, 1, 25), RENEWAL_REJECTED_LABEL),
            (d(2026, 3, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 3, 18), RENEWAL_REJECTED_LABEL),
            (d(2026, 4, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 4, 25), RENEWAL_REJECTED_LABEL),
        ]);
        let contract = make_contract(50_000, PlayerSquadStatus::MainBackupPlayer, d(2027, 7, 1));
        let mut player = make_player(101, 75, d(1995, 1, 1), history, Some(contract));
        player.is_force_match_selection = true;
        let teammate = make_player(
            102,
            125,
            d(1993, 1, 1),
            PlayerDecisionHistory::new(),
            Some(make_contract(
                60_000,
                PlayerSquadStatus::FirstTeamRegular,
                d(2027, 7, 1),
            )),
        );
        let team = make_team(10, 100, vec![player, teammate]);
        let club = make_club(100, team, Some(120_000));
        let country = make_country(club);
        let mut sim = make_sim(country, today);

        ClubResult::process_player_contract_interaction(&rejected_result(101), &mut sim, 100);

        let p = sim.player(101).unwrap();
        assert!(
            !p.statuses.get().contains(&PlayerStatusType::Lst),
            "force-selected player must never be auto-listed"
        );
    }

    #[test]
    fn loaned_player_is_not_listed_by_borrower() {
        let today = d(2026, 5, 1);
        let history = make_decision_history(&[
            (d(2026, 1, 10), RENEWAL_OFFERED_LABEL),
            (d(2026, 1, 25), RENEWAL_REJECTED_LABEL),
            (d(2026, 3, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 3, 18), RENEWAL_REJECTED_LABEL),
            (d(2026, 4, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 4, 25), RENEWAL_REJECTED_LABEL),
        ]);
        // Contract with the borrowing club (the team the player is in).
        let contract = make_contract(50_000, PlayerSquadStatus::MainBackupPlayer, d(2027, 7, 1));
        let mut player = make_player(101, 75, d(1995, 1, 1), history, Some(contract));
        // Loan flag from parent club id = 999 (does not exist in sim,
        // which is fine — we're testing the borrower side).
        let mut loan_contract = PlayerClubContract::new(50_000, d(2027, 7, 1));
        loan_contract.loan_from_club_id = Some(999);
        player.contract_loan = Some(loan_contract);
        let teammate = make_player(
            102,
            125,
            d(1993, 1, 1),
            PlayerDecisionHistory::new(),
            Some(make_contract(
                60_000,
                PlayerSquadStatus::FirstTeamRegular,
                d(2027, 7, 1),
            )),
        );
        let team = make_team(10, 100, vec![player, teammate]);
        let club = make_club(100, team, Some(120_000));
        let country = make_country(club);
        let mut sim = make_sim(country, today);

        ClubResult::process_player_contract_interaction(&rejected_result(101), &mut sim, 100);

        let p = sim.player(101).unwrap();
        assert!(
            !p.statuses.get().contains(&PlayerStatusType::Lst),
            "loaned player cannot be transfer-listed by the borrowing club"
        );
        let country = sim.country(1).unwrap();
        assert!(country.transfer_market.listings.is_empty());
    }

    #[test]
    fn unresolved_salary_releases_cheap_fringe_player() {
        let today = d(2026, 5, 1);
        let history = make_decision_history(&[
            (d(2026, 1, 10), RENEWAL_OFFERED_LABEL),
            (d(2026, 1, 25), RENEWAL_REJECTED_LABEL),
            (d(2026, 3, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 3, 18), RENEWAL_REJECTED_LABEL),
            (d(2026, 4, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 4, 25), RENEWAL_REJECTED_LABEL),
        ]);
        // CA 40 vs squad avg 82, age 33, tiny salary, 3 months left:
        // every eligibility gate passes → genuine mutual release.
        let contract = make_contract(30_000, PlayerSquadStatus::MainBackupPlayer, d(2026, 8, 1));
        let player = make_player(101, 40, d(1993, 1, 1), history, Some(contract));
        let teammate = make_player(
            102,
            125,
            d(1993, 1, 1),
            PlayerDecisionHistory::new(),
            Some(make_contract(
                60_000,
                PlayerSquadStatus::FirstTeamRegular,
                d(2027, 7, 1),
            )),
        );
        let team = make_team(10, 100, vec![player, teammate]);
        let club = make_club(100, team, Some(120_000));
        let country = make_country(club);
        let mut sim = make_sim(country, today);

        ClubResult::process_player_contract_interaction(&rejected_result(101), &mut sim, 100);

        let p = sim.player(101).expect("player still present");
        assert!(
            p.statuses.get().contains(&PlayerStatusType::Frt),
            "cheap fringe player must be released on a free"
        );
        assert!(
            p.contract.is_none(),
            "release must clear the contract for the free-agent sweep"
        );
        assert!(
            p.decision_history
                .items
                .iter()
                .any(|d| d.movement == "dec_free_transfer_listed"),
            "release must be recorded in decision history"
        );
        let country = sim.country(1).unwrap();
        assert!(
            country.transfer_market.listings.is_empty(),
            "a free release must not create a market listing"
        );
    }

    #[test]
    fn unresolved_salary_lists_when_compensation_too_high() {
        let today = d(2026, 5, 1);
        let history = make_decision_history(&[
            (d(2026, 1, 10), RENEWAL_OFFERED_LABEL),
            (d(2026, 1, 25), RENEWAL_REJECTED_LABEL),
            (d(2026, 3, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 3, 18), RENEWAL_REJECTED_LABEL),
            (d(2026, 4, 1), RENEWAL_OFFERED_LABEL),
            (d(2026, 4, 25), RENEWAL_REJECTED_LABEL),
        ]);
        // Same fringe quality (CA 40 vs avg 82) — the legacy rule would
        // have torn the deal up — but two years of a 2M salary remain, so
        // the severance gate forces a transfer listing instead.
        let contract = make_contract(
            2_000_000,
            PlayerSquadStatus::MainBackupPlayer,
            d(2028, 7, 1),
        );
        let player = make_player(101, 40, d(1995, 1, 1), history, Some(contract));
        let teammate = make_player(
            102,
            125,
            d(1993, 1, 1),
            PlayerDecisionHistory::new(),
            Some(make_contract(
                60_000,
                PlayerSquadStatus::FirstTeamRegular,
                d(2027, 7, 1),
            )),
        );
        let team = make_team(10, 100, vec![player, teammate]);
        let club = make_club(100, team, Some(120_000));
        let country = make_country(club);
        let mut sim = make_sim(country, today);

        ClubResult::process_player_contract_interaction(&rejected_result(101), &mut sim, 100);

        let p = sim.player(101).expect("player still present");
        assert!(
            !p.statuses.get().contains(&PlayerStatusType::Frt),
            "expensive contract must not be released for free"
        );
        assert!(
            p.statuses.get().contains(&PlayerStatusType::Lst),
            "blocked release must fall back to a transfer listing"
        );
        let contract = p
            .contract
            .as_ref()
            .expect("listed player must keep his contract");
        assert!(contract.is_transfer_listed);
        let country = sim.country(1).unwrap();
        assert_eq!(
            country.transfer_market.listings.len(),
            1,
            "the fallback listing must reach the country market"
        );
        assert_eq!(country.transfer_market.listings[0].player_id, 101);
    }
}
