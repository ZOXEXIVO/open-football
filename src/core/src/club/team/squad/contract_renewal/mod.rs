use crate::club::player::mailbox::handlers::contract_proposal::{ProcessContractHandler, RENEWAL_REJECTED_LABEL};
use crate::club::player::player::Player;
use crate::{
    PlayerContractProposal, PlayerMessage, PlayerMessageType, PlayerSquadStatus,
    PlayerStatusType, Team,
};
use crate::utils::DateUtils;
use chrono::NaiveDate;

/// Minimum gap between proactive offers when the previous one hasn't
/// actually been turned down yet.
const RENEWAL_COOLDOWN_DAYS: i64 = 30;
/// Longer cooldown after the player rejects. Real clubs don't hammer the
/// same player every month with the same deal — they regroup, raise, and
/// try again after a while.
const RENEWAL_COOLDOWN_AFTER_REJECT_DAYS: i64 = 120;
/// Hard cap on how many times a proactive offer is made within a year.
/// After three flat refusals the club normally gives up — unless a
/// final-year or market-pressure override fires.
const MAX_RENEWAL_ATTEMPTS_PER_YEAR: usize = 3;
/// Days-to-expiry at which Bosman rules start biting: any longer and a
/// rival club can agree a pre-contract. Below this threshold the club
/// abandons the attempts cap and pushes harder.
const BOSMAN_PRESSURE_DAYS: i64 = 180;
/// Final-month panic. No cap, maximum sweeteners.
const FINAL_PANIC_DAYS: i64 = 30;
const DECISION_LABEL: &str = "dec_contract_renewal_offered";

pub struct ContractRenewalManager;

impl ContractRenewalManager {
    /// Walk the main team, deliver renewal proposals to valuable players
    /// whose contracts are approaching expiry. Deterministic — no AI call.
    ///
    /// Runs before the monthly TransferListManager so valuable players have
    /// already been offered a renewal by the time the listing AI evaluates
    /// them. This prevents the listing AI from inventing "contract expiring"
    /// as a reason to sell a player the club actually wants to keep.
    pub fn run(teams: &mut [Team], main_idx: usize, date: NaiveDate) {
        let (coach_name, negotiation_skill, judging_ability) =
            Self::resolve_staff(&teams[main_idx]);
        let team_rep_factor = teams[main_idx].reputation.overall_score();
        let wage_budget = 0u32; // No board-level wage cap wired into Team yet

        let candidates = Self::collect_candidates(&teams[main_idx], date);

        for candidate in candidates {
            let attempts = teams[main_idx]
                .players
                .players
                .iter()
                .find(|p| p.id == candidate.player_id)
                .map(|p| {
                    p.decision_history
                        .items
                        .iter()
                        .filter(|d| {
                            d.decision == DECISION_LABEL
                                && (date - d.date).num_days() < 365
                        })
                        .count()
                })
                .unwrap_or(0);

            let over_cap = attempts >= MAX_RENEWAL_ATTEMPTS_PER_YEAR;
            if over_cap && !candidate.override_attempts_cap {
                continue;
            }

            let built = Self::build_offer(
                &teams[main_idx],
                candidate.player_id,
                negotiation_skill,
                judging_ability,
                date,
                attempts,
                team_rep_factor,
                wage_budget,
                &candidate,
            );
            let proposal = match built {
                Some(p) => p,
                None => continue,
            };

            if let Some(player) = teams[main_idx]
                .players
                .players
                .iter_mut()
                .find(|p| p.id == candidate.player_id)
            {
                let movement = format!(
                    "{}y · ${}/y",
                    proposal.years,
                    crate::utils::FormattingUtils::format_money(proposal.salary as f64)
                );
                player.decision_history.add(
                    date,
                    movement,
                    DECISION_LABEL.to_string(),
                    coach_name.clone(),
                );
                player.mailbox.push(PlayerMessage {
                    message_type: PlayerMessageType::ContractProposal(proposal),
                });
            }
        }
    }

    fn resolve_staff(team: &Team) -> (String, u8, u8) {
        let coach_name = team.staffs.head_coach().full_name.to_string();

        let resolver = team
            .staffs
            .responsibility
            .contract_renewal
            .handle_first_team_contracts
            .and_then(|id| team.staffs.find(id));

        let (negotiation, judging) = match resolver {
            Some(staff) => (
                staff.staff_attributes.mental.man_management,
                staff.staff_attributes.knowledge.judging_player_ability,
            ),
            None => {
                let hc = team.staffs.head_coach();
                (
                    hc.staff_attributes.mental.man_management,
                    hc.staff_attributes.knowledge.judging_player_ability,
                )
            }
        };

        (coach_name, negotiation, judging)
    }

    fn collect_candidates(team: &Team, date: NaiveDate) -> Vec<RenewalCandidate> {
        team.players
            .players
            .iter()
            .filter_map(|player| Self::evaluate(player, date))
            .collect()
    }

    fn evaluate(player: &Player, date: NaiveDate) -> Option<RenewalCandidate> {
        if player.is_on_loan() {
            return None;
        }

        let contract = player.contract.as_ref()?;
        let days_remaining = (contract.expiration - date).num_days();
        if days_remaining <= 0 {
            return None;
        }

        let statuses = player.statuses.get();
        if statuses.contains(&PlayerStatusType::Req)
            || statuses.contains(&PlayerStatusType::Lst)
            || statuses.contains(&PlayerStatusType::Frt)
        {
            return None;
        }

        let has_market_interest = statuses.iter().any(|s| {
            matches!(s, PlayerStatusType::Wnt | PlayerStatusType::Enq | PlayerStatusType::Bid)
        });

        let effective_status = Self::effective_squad_status(player, &contract.squad_status);
        let threshold = Self::renewal_threshold_days(&effective_status);
        let final_panic = days_remaining <= FINAL_PANIC_DAYS;
        let bosman_pressure = days_remaining <= BOSMAN_PRESSURE_DAYS && has_market_interest;

        if days_remaining > threshold && !final_panic && !bosman_pressure {
            return None;
        }

        let override_cap = final_panic || bosman_pressure;

        if !override_cap && Self::recently_offered(player, date) {
            return None;
        }

        Some(RenewalCandidate {
            player_id: player.id,
            effective_status,
            has_market_interest,
            final_panic,
            bosman_pressure,
            override_attempts_cap: override_cap,
        })
    }

    /// Effective squad status used for renewal decisions. A player on a
    /// purple-patch run gets treated one tier up — real clubs rush to tie
    /// down in-form players regardless of their formal role. Same engine
    /// for every player; the boost just needs a strong, universal signal.
    fn effective_squad_status(player: &Player, base: &PlayerSquadStatus) -> PlayerSquadStatus {
        let form = player.load.form_rating;
        if form < 7.5 {
            return base.clone();
        }
        match base {
            PlayerSquadStatus::FirstTeamRegular => PlayerSquadStatus::KeyPlayer,
            PlayerSquadStatus::FirstTeamSquadRotation => PlayerSquadStatus::FirstTeamRegular,
            PlayerSquadStatus::MainBackupPlayer => PlayerSquadStatus::FirstTeamSquadRotation,
            PlayerSquadStatus::DecentYoungster => PlayerSquadStatus::HotProspectForTheFuture,
            PlayerSquadStatus::NotNeeded | PlayerSquadStatus::NotYetSet => {
                PlayerSquadStatus::MainBackupPlayer
            }
            _ => base.clone(),
        }
    }

    fn renewal_threshold_days(squad_status: &PlayerSquadStatus) -> i64 {
        match squad_status {
            PlayerSquadStatus::KeyPlayer
            | PlayerSquadStatus::FirstTeamRegular => 540,
            PlayerSquadStatus::FirstTeamSquadRotation
            | PlayerSquadStatus::HotProspectForTheFuture => 365,
            PlayerSquadStatus::MainBackupPlayer
            | PlayerSquadStatus::DecentYoungster => 180,
            // NotNeeded / unset still get a 90-day window. Real clubs at
            // least consider every expiring contract before letting a
            // player walk on a free — listing is a separate decision.
            _ => 90,
        }
    }

    /// Two-tier cooldown. A proactive offer uses the short window; a
    /// rejected offer gets a much longer one before we come back with a
    /// revised deal.
    fn recently_offered(player: &Player, date: NaiveDate) -> bool {
        let last = player
            .decision_history
            .items
            .iter()
            .rev()
            .find(|d| {
                d.decision == DECISION_LABEL || d.decision == RENEWAL_REJECTED_LABEL
            });
        match last {
            Some(d) if d.decision == RENEWAL_REJECTED_LABEL => {
                (date - d.date).num_days() < RENEWAL_COOLDOWN_AFTER_REJECT_DAYS
            }
            Some(d) => (date - d.date).num_days() < RENEWAL_COOLDOWN_DAYS,
            None => false,
        }
    }

    fn build_offer(
        team: &Team,
        player_id: u32,
        negotiation_skill: u8,
        judging_ability: u8,
        date: NaiveDate,
        previous_attempts: usize,
        team_rep_factor: f32,
        wage_budget: u32,
        candidate: &RenewalCandidate,
    ) -> Option<PlayerContractProposal> {
        let player = team.players.find(player_id)?;
        let contract = player.contract.as_ref()?;

        let ability = player.player_attributes.current_ability;
        let age = DateUtils::age(player.birth_date, date);

        // Wage inflation: clubs with higher reputation pay higher wages at
        // every tier. A Key Player at an elite club earns materially more
        // than a Key Player at a relegation-zone club.
        let band = ability_based_salary(ability);
        let rep_multiplier = 0.8 + team_rep_factor * 0.9;
        let inflated_band = (band as f32 * rep_multiplier) as u32;

        let accuracy = 0.85 + (judging_ability as f32 / 20.0) * 0.25;
        let adjusted_base = (inflated_band as f32 * accuracy) as u32;

        let current_salary = contract.salary;

        // Anchor the escalation to whichever is higher — the player's
        // current wage or our band-based valuation. Escalating only on the
        // band meant a player already above their band saw attempt 1-3
        // stuck at current×1.05, which greedy agents rejected every time.
        let anchor = adjusted_base.max(current_salary);

        // Urgency escalation: +10% per prior rejection, plus a jolt when
        // Bosman / final-year kicks in.
        let mut escalation = 1.0 + previous_attempts as f32 * 0.10;
        if candidate.bosman_pressure {
            escalation += 0.10;
        }
        if candidate.final_panic {
            escalation += 0.15;
        }
        if candidate.has_market_interest {
            escalation += 0.05;
        }

        let mut offered = (anchor as f32 * escalation) as u32;
        offered = offered
            .max(current_salary + current_salary / 20)
            .max(current_salary + 1);

        // Converge toward the player's own ask when we have it. This is the
        // signal the player left after the previous rejection. Never
        // capitulate fully — split the gap so negotiation still has teeth.
        if let Some(ask) = &player.pending_contract_ask {
            if ask.desired_salary > offered {
                offered = (offered + ask.desired_salary) / 2;
            }
        }

        // FFP gate: if the club has a wage budget set, never bust it.
        if wage_budget > 0 {
            let current_wage_bill: u32 = team.get_annual_salary();
            let salary_delta = offered.saturating_sub(current_salary);
            if current_wage_bill + salary_delta > wage_budget {
                let remaining = wage_budget.saturating_sub(current_wage_bill);
                offered = (current_salary + remaining).max(current_salary);
            }
        }

        let years = proactive_contract_years(
            age,
            ability,
            &candidate.effective_status,
            negotiation_skill,
            player,
            date,
        );

        // Sweeteners come out once the base deal isn't enough. Scale with
        // urgency — a first offer is clean, a third-attempt/final-panic
        // offer pulls the full kit: signing bonus, loyalty bonus, release
        // clause sized to the market.
        let (signing_bonus, loyalty_bonus, release_clause) = build_sweeteners(
            offered,
            ability,
            age,
            previous_attempts,
            candidate,
            player,
        );

        Some(PlayerContractProposal {
            salary: offered,
            years,
            negotiation_skill,
            signing_bonus,
            loyalty_bonus,
            release_clause,
        })
    }
}

struct RenewalCandidate {
    player_id: u32,
    effective_status: PlayerSquadStatus,
    has_market_interest: bool,
    final_panic: bool,
    bosman_pressure: bool,
    override_attempts_cap: bool,
}

fn ability_based_salary(ability: u8) -> u32 {
    match ability {
        0..=50 => 5_000,
        51..=70 => 20_000,
        71..=90 => 50_000,
        91..=110 => 100_000,
        111..=130 => 200_000,
        131..=150 => 350_000,
        _ => 500_000,
    }
}

/// Proactive renewals favour longer deals — the club is locking in value.
/// Veterans with high reputation now get at least their minimum acceptable
/// term via the natural max(), rather than a club-cap that forces auto-
/// rejection on length.
fn proactive_contract_years(
    age: u8,
    ability: u8,
    squad_status: &PlayerSquadStatus,
    negotiation_skill: u8,
    player: &Player,
    date: NaiveDate,
) -> u8 {
    let mut years: f32 = match squad_status {
        PlayerSquadStatus::KeyPlayer => 4.0,
        PlayerSquadStatus::FirstTeamRegular => 3.5,
        PlayerSquadStatus::HotProspectForTheFuture => 4.0,
        PlayerSquadStatus::FirstTeamSquadRotation => 3.0,
        _ => 2.0,
    };

    if age >= 34 {
        years = years.min(1.5);
    } else if age >= 32 {
        years = years.min(2.0);
    } else if age >= 30 {
        years = years.min(3.0);
    }

    if age < 22 && ability > 80 {
        years += 1.0;
    }

    if negotiation_skill >= 15 {
        years -= 0.5;
    }

    let min_accept = ProcessContractHandler::player_minimum_years(player, date) as f32;
    // Respect the player's minimum — otherwise the offer auto-rejects on
    // length and the club wastes attempts. Capped at 5 so elite-rep
    // veterans still get a 3-4 year deal without breaching the upper limit.
    let raised = years.max(min_accept);

    (raised.round() as u8).clamp(1, 5)
}

fn build_sweeteners(
    offered_salary: u32,
    ability: u8,
    age: u8,
    previous_attempts: usize,
    candidate: &RenewalCandidate,
    player: &Player,
) -> (u32, u32, Option<u32>) {
    let mut signing_bonus = 0u32;
    let mut loyalty_bonus = 0u32;
    let mut release_clause: Option<u32> = None;

    let urgency = previous_attempts
        + if candidate.bosman_pressure { 1 } else { 0 }
        + if candidate.final_panic { 2 } else { 0 }
        + if candidate.has_market_interest { 1 } else { 0 };

    let greedy_agent = player.attributes.ambition + player.attributes.controversy > 24.0;
    let loyal_agent = player.attributes.loyalty > 14.0;

    if urgency >= 2 || greedy_agent {
        // Signing bonus scales with salary — 20-60% of annual wage.
        signing_bonus = (offered_salary as f32 * (0.20 + (urgency.min(5) as f32) * 0.08)) as u32;
    }

    if urgency >= 2 || loyal_agent || age >= 30 {
        // Loyalty bonus: yearly, ~10-25% of salary.
        loyalty_bonus = (offered_salary as f32 * (0.10 + (urgency.min(4) as f32) * 0.04)) as u32;
    }

    // Release clauses get introduced once the player has leverage (market
    // interest) or after two failed attempts. Sized from player ability
    // and reputation — big enough to deter tyre-kickers but reachable.
    if candidate.has_market_interest || previous_attempts >= 2 {
        let rep = player.player_attributes.current_reputation as u32;
        let base = (ability as u32) * (ability as u32) * 4_000; // ability²-shaped
        let rep_boost = rep * 8_000;
        release_clause = Some(base + rep_boost);
    }

    (signing_bonus, loyalty_bonus, release_clause)
}
