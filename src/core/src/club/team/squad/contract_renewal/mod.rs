use crate::club::player::player::Player;
use crate::club::player::mailbox::handlers::contract_proposal::RENEWAL_REJECTED_LABEL;
use crate::{
    PlayerContractProposal, PlayerMessage, PlayerMessageType, PlayerSquadStatus,
    PlayerStatusType, Team,
};
use chrono::NaiveDate;

/// Minimum gap between proactive offers when the previous one hasn't
/// actually been turned down yet.
const RENEWAL_COOLDOWN_DAYS: i64 = 30;
/// Longer cooldown after the player rejects. Real clubs don't hammer the
/// same player every month with the same deal — they regroup, raise, and
/// try again after a while.
const RENEWAL_COOLDOWN_AFTER_REJECT_DAYS: i64 = 120;
/// Hard cap on how many times a proactive offer is made within a year.
/// After three flat refusals the club gives up and the player runs down
/// their deal (or gets transfer-listed via the usual flows).
const MAX_RENEWAL_ATTEMPTS_PER_YEAR: usize = 3;
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

        let candidates = Self::collect_candidates(&teams[main_idx], date);

        for candidate in candidates {
            // Count how many previous attempts we've already made in the
            // last year. After MAX_RENEWAL_ATTEMPTS_PER_YEAR the club
            // stops pursuing — the player will run the deal down or the
            // normal transfer-listing pipeline will handle them.
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
            if attempts >= MAX_RENEWAL_ATTEMPTS_PER_YEAR {
                continue;
            }

            let (salary, years) = match Self::build_offer(
                &teams[main_idx],
                candidate.player_id,
                negotiation_skill,
                judging_ability,
                date,
                attempts,
            ) {
                Some(pair) => pair,
                None => continue,
            };

            if let Some(player) = teams[main_idx]
                .players
                .players
                .iter_mut()
                .find(|p| p.id == candidate.player_id)
            {
                player.mailbox.push(PlayerMessage {
                    message_type: PlayerMessageType::ContractProposal(PlayerContractProposal {
                        salary,
                        years,
                        negotiation_skill,
                    }),
                });

                let movement = format!("{}y · ${}/y", years, salary);
                player.decision_history.add(
                    date,
                    movement,
                    DECISION_LABEL.to_string(),
                    coach_name.clone(),
                );
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
        // Loaned-in players belong to another club — parent club handles renewal
        if player.is_on_loan() {
            return None;
        }

        let contract = player.contract.as_ref()?;
        let days_remaining = (contract.expiration - date).num_days();
        if days_remaining <= 0 {
            return None;
        }

        // Already negotiating / being sold / rejected recently: skip
        let statuses = player.statuses.get();
        if statuses.contains(&PlayerStatusType::Req)
            || statuses.contains(&PlayerStatusType::Lst)
            || statuses.contains(&PlayerStatusType::Frt)
        {
            return None;
        }

        let threshold = Self::renewal_threshold_days(&contract.squad_status);
        if days_remaining > threshold {
            return None;
        }

        // Cooldown: skip if we've already offered a renewal recently.
        // The existing contract_proposal handler would re-trigger the same
        // evaluation and nothing will have changed in 30 days.
        if Self::recently_offered(player, date) {
            return None;
        }

        Some(RenewalCandidate {
            player_id: player.id,
        })
    }

    fn renewal_threshold_days(squad_status: &PlayerSquadStatus) -> i64 {
        match squad_status {
            // Protect core squad 18 months out
            PlayerSquadStatus::KeyPlayer
            | PlayerSquadStatus::FirstTeamRegular => 540,
            // Rotation and prospects get 12 months notice
            PlayerSquadStatus::FirstTeamSquadRotation
            | PlayerSquadStatus::HotProspectForTheFuture => 365,
            // Backups and youngsters: short notice only
            PlayerSquadStatus::MainBackupPlayer
            | PlayerSquadStatus::DecentYoungster => 180,
            // Not needed / unset: don't proactively renew —
            // let transfer listing or the existing reactive flow decide
            _ => 0,
        }
    }

    /// Two-tier cooldown. A proactive offer uses the short window; a
    /// rejected offer gets a much longer one before we come back with a
    /// revised deal.
    fn recently_offered(player: &Player, date: NaiveDate) -> bool {
        // Look at the most recent renewal-related decision.
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
    ) -> Option<(u32, u8)> {
        let player = team.players.find(player_id)?;
        let contract = player.contract.as_ref()?;

        let ability = player.player_attributes.current_ability;
        let age = crate::utils::DateUtils::age(player.birth_date, date);

        let base_salary = ability_based_salary(ability);
        // Accuracy bonus for valuable players: judging_ability 0-20 →
        // offer 85-110% of fair value. Proactive renewals are more generous
        // than reactive ones so the player is more likely to accept.
        let accuracy = 0.85 + (judging_ability as f32 / 20.0) * 0.25;
        let adjusted_base = (base_salary as f32 * accuracy) as u32;

        let current_salary = contract.salary;
        // Each prior rejection pushes the offer up a meaningful step —
        // +10% per failed attempt. By the 3rd attempt the offer is 20%
        // above the opening bid, which is a credible final position.
        let escalation = 1.0 + previous_attempts as f32 * 0.10;
        let escalated_base = (adjusted_base as f32 * escalation) as u32;

        // Always offer at least a small raise — this is a proactive renewal,
        // not a haircut. Player happiness logic rewards salary increases.
        let offered = escalated_base
            .max(current_salary + current_salary / 20) // +5% floor
            .max(current_salary + 1);

        let years = proactive_contract_years(age, ability, &contract.squad_status, negotiation_skill);
        Some((offered, years))
    }
}

struct RenewalCandidate {
    player_id: u32,
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
fn proactive_contract_years(
    age: u8,
    ability: u8,
    squad_status: &PlayerSquadStatus,
    negotiation_skill: u8,
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

    // Better negotiator gets marginally shorter club-favourable deals
    if negotiation_skill >= 15 {
        years -= 0.5;
    }

    (years.round() as u8).clamp(1, 5)
}
