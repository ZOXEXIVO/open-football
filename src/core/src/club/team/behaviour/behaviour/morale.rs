//! Morale-shifting events: contract jealousy from a teammate's fresh
//! signing, monthly loan playing-time audits, controversy incidents, and
//! the periodic peer-wage envy sweep.

use super::TeamBehaviour;
use crate::context::GlobalContext;
use crate::utils::IntegerUtils;
use crate::{HappinessEventType, PlayerCollection, PlayerFieldPositionGroup, PlayerSquadStatus};
use chrono::Datelike;
use std::collections::HashMap;

impl TeamBehaviour {
    /// When a teammate signs a notably bigger deal and this player earns
    /// meaningfully less, morale takes a hit — unless they're close friends.
    /// Fires at most once per player per signing window (the signer's
    /// `last_salary_negotiation` timestamp gates it). Gap threshold ≥25%.
    pub(super) fn process_contract_jealousy(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        // Cutoff: teammate's raise within the last 14 days counts as fresh news.
        let freshness_days = 14;

        // Collect fresh signers first (id, salary, last_negotiation) so we
        // don't clash borrows while mutating other players below.
        // Loaned-in players are excluded as signers — their parent club's
        // renewal isn't borrower-squad news, and the borrower's wage
        // hierarchy doesn't include them anyway.
        let signers: Vec<(u32, u32)> = players
            .players
            .iter()
            .filter(|p| !p.is_on_loan())
            .filter_map(|p| {
                let last = p.happiness.last_salary_negotiation?;
                let age_days = (today - last).num_days();
                if age_days >= 0 && age_days <= freshness_days {
                    p.contract.as_ref().map(|c| (p.id, c.salary))
                } else {
                    None
                }
            })
            .collect();

        if signers.is_empty() {
            return;
        }

        for (signer_id, signer_salary) in signers {
            if signer_salary == 0 {
                continue;
            }
            for player in players.players.iter_mut() {
                if player.id == signer_id {
                    continue;
                }
                // Loanees see star wages every day at a top club — they
                // know they're a temporary visitor on a different
                // contract structure (the loan deal), so a star
                // teammate's renewal isn't a personal slight.
                if player.is_on_loan() {
                    continue;
                }
                let own_salary = match player.contract.as_ref() {
                    Some(c) if c.salary > 0 => c.salary,
                    _ => continue,
                };
                // Only established players notice salary gaps. A reserve
                // or recent academy graduate at a top club isn't unsettled
                // when the star striker re-signs for ten times their wage —
                // they're grateful to be in the changing room. Without this
                // gate, a CA-60 squad filler at Real Madrid produces an
                // "unsettled by teammate's salary" event every renewal.
                if player.player_attributes.current_ability < 100
                    && player.player_attributes.world_reputation < 3000
                {
                    continue;
                }
                // Only noticed when the gap is ≥25%.
                let ratio = own_salary as f32 / signer_salary as f32;
                if ratio >= 0.75 {
                    continue;
                }

                // Close friends shrug it off.
                let friendship = player
                    .relations
                    .get_player(signer_id)
                    .map(|r| r.friendship)
                    .unwrap_or(30.0);
                if friendship >= 40.0 {
                    continue;
                }

                // Magnitude scales with the gap: 25% gap → -1.5, 50% gap → -3.5, cap at -5.
                // Cooldown prevents a fresh raise refiring inside the
                // 14-day jealousy window from the same signer.
                let gap = (1.0 - ratio).clamp(0.25, 0.9);
                let magnitude = -((gap - 0.25) * 6.0 + 1.5).min(5.0);
                player.happiness.add_event_with_cooldown(
                    HappinessEventType::SalaryGapNoticed,
                    magnitude,
                    freshness_days as u16,
                );
            }
        }
    }

    /// Monthly audit of inbound loanees — did the borrowing club actually
    /// give them the minutes the loan contract required? If pace falls
    /// behind, open the recall window (parent may yank them back) and fire
    /// `LackOfPlayingTime` on the player. Runs on day 1 only.
    pub(super) fn process_loan_playing_time_audit(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        if today.day() != 1 {
            return;
        }

        for player in players.players.iter_mut() {
            let Some(loan) = player.contract_loan.as_mut() else {
                continue;
            };
            let Some(min_apps) = loan.loan_min_appearances else {
                continue;
            };
            let Some(loan_start) = loan.started else {
                continue;
            };
            let loan_end = loan.expiration;

            let total_days = (loan_end - loan_start).num_days().max(1) as f32;
            let elapsed_days = (today - loan_start).num_days().max(0) as f32;
            if elapsed_days < 30.0 {
                continue; // Too early to judge pace
            }
            let progress = (elapsed_days / total_days).clamp(0.0, 1.0);
            let expected_by_now = (min_apps as f32 * progress).floor() as u16;
            let actual = player.statistics.played + player.statistics.played_subs;

            if actual >= expected_by_now {
                continue;
            }

            let deficit = expected_by_now.saturating_sub(actual);
            // Open the recall window for any meaningful shortfall.
            if loan.loan_recall_available_after.is_none() {
                loan.loan_recall_available_after = Some(today);
            }
            // Morale hit scales with how badly we're trailing.
            let magnitude = -((deficit as f32 * 0.8).min(6.0) + 1.0);
            player
                .happiness
                .add_event(HappinessEventType::LackOfPlayingTime, magnitude);
        }
    }

    /// Monthly controversy roll — high-controversy players with poor
    /// temperament occasionally find themselves in incidents: a dressing-
    /// room row, a media storm, a training-ground scrap. Fires a morale
    /// hit on the player + a relationship drag against a random teammate.
    /// Scaled so a calm, sportsmanlike star ~never triggers, while a hot-
    /// head with controversy >15 and temperament <8 fires frequently.
    pub(super) fn process_controversy_incidents(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        if today.day() != 1 {
            return; // Monthly cadence
        }

        // Collect potential troublemakers (immutable pass).
        let candidates: Vec<(u32, u32, f32)> = players
            .players
            .iter()
            .filter_map(|p| {
                let controversy = p.attributes.controversy;
                let temperament = p.attributes.temperament;
                let sportsmanship = p.attributes.sportsmanship;
                if controversy < 12.0 {
                    return None;
                }
                // Risk score: big when controversial + hot-tempered + unsporting
                let risk = controversy + (20.0 - temperament) * 0.6 + (20.0 - sportsmanship) * 0.4;
                if risk < 35.0 {
                    return None;
                }
                // Convert to 0-100 trigger chance this month.
                let chance = ((risk - 35.0) * 1.8).clamp(0.0, 60.0);
                let roll = IntegerUtils::random(0, 100) as f32;
                if roll > chance {
                    return None;
                }
                Some((p.id, 0u32, controversy))
            })
            .collect();

        if candidates.is_empty() {
            return;
        }

        // Pick a nearby teammate (low-friendship, different age bracket) to
        // be involved in the spat. Only one per incident.
        let all_ids: Vec<u32> = players.players.iter().map(|p| p.id).collect();

        for (offender_id, _, controversy) in candidates {
            // Find a candidate teammate — scan for low-friendship relation.
            let victim_id = {
                let offender = match players.find(offender_id) {
                    Some(p) => p,
                    None => continue,
                };
                let mut picked: Option<u32> = None;
                for tid in &all_ids {
                    if *tid == offender_id {
                        continue;
                    }
                    let friendship = offender
                        .relations
                        .get_player(*tid)
                        .map(|r| r.friendship)
                        .unwrap_or(30.0);
                    if friendship < 35.0 {
                        picked = Some(*tid);
                        break;
                    }
                }
                picked
            };

            // Fire the incident event on the offender.
            if let Some(offender) = players.players.iter_mut().find(|p| p.id == offender_id) {
                let magnitude = -(3.0 + ((controversy - 12.0) * 0.3).clamp(0.0, 4.0));
                offender
                    .happiness
                    .add_event(HappinessEventType::ControversyIncident, magnitude);
            }
            // And a smaller ripple on the teammate (if one was found).
            // ConflictWithTeammate must carry the partner id — the events
            // UI filters out partner-required events that can't name the
            // teammate (otherwise "Argued with a teammate" reads as ghost
            // text). The partner here is the offender, not the victim.
            if let Some(vid) = victim_id {
                if let Some(victim) = players.players.iter_mut().find(|p| p.id == vid) {
                    victim.happiness.add_event_with_partner(
                        HappinessEventType::ConflictWithTeammate,
                        -2.0,
                        Some(offender_id),
                    );
                }
            }
        }
    }

    /// Monthly squad-wide wage audit: compare every player's salary to the
    /// top earner at their position group. If they're a starter earning
    /// <60% of the top salary in their slot, fire a gentle recurring
    /// `SalaryGapNoticed` event. Complements `process_contract_jealousy`,
    /// which only fires on fresh raises.
    pub(super) fn process_periodic_wage_envy(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        if today.day() != 1 {
            return; // Monthly only
        }

        // Build the top-earner-by-position map from permanent squad
        // contracts only. Loanees' parent contracts may be huge (a Real
        // Madrid loanee carrying a Madrid wage) or tiny (a youth loanee
        // from a lower-league parent) and neither belongs in the
        // borrower's wage structure.
        let mut top_by_group: HashMap<PlayerFieldPositionGroup, u32> = HashMap::new();
        for p in &players.players {
            if p.is_on_loan() {
                continue;
            }
            let Some(contract) = p.contract.as_ref() else {
                continue;
            };
            if contract.salary == 0 {
                continue;
            }
            let group = p.position().position_group();
            let entry = top_by_group.entry(group).or_insert(0);
            if contract.salary > *entry {
                *entry = contract.salary;
            }
        }

        for player in players.players.iter_mut() {
            // Loanees know their wage at the borrower is the loan deal —
            // not the parent contract — and that their stay is temporary.
            // Comparing the parent salary to the borrower's stars is
            // doubly nonsensical and produces the "low-CA loanee
            // unsettled by stars" bug.
            if player.is_on_loan() {
                continue;
            }
            let Some(contract) = player.contract.as_ref() else {
                continue;
            };
            if contract.salary == 0 {
                continue;
            }
            // Only players who play a meaningful role care about the gap —
            // the third-choice keeper being underpaid vs the #1 is the way
            // the world works.
            if !matches!(
                contract.squad_status,
                PlayerSquadStatus::KeyPlayer
                    | PlayerSquadStatus::FirstTeamRegular
                    | PlayerSquadStatus::FirstTeamSquadRotation
            ) {
                continue;
            }
            // Reputation gate (mirror of `process_contract_jealousy`).
            // Squad-status alone isn't enough — a top club may slot a
            // CA-60 youth into rotation as cover, and that player has no
            // business being unsettled by the star earner's wages.
            if player.player_attributes.current_ability < 100
                && player.player_attributes.world_reputation < 3000
            {
                continue;
            }
            let group = player.position().position_group();
            let top = match top_by_group.get(&group) {
                Some(t) if *t > 0 => *t,
                _ => continue,
            };
            if player.id == 0 || contract.salary >= top {
                continue;
            }
            let ratio = contract.salary as f32 / top as f32;
            if ratio >= 0.6 {
                continue;
            }
            // Magnitude: 60% ratio → -1.5, 30% ratio → -4.5, cap at -5.
            // 28-day cooldown so the monthly audit doesn't re-fire the
            // same player while last month's wage-envy event is still
            // visible in the history.
            let magnitude = -(((0.6 - ratio) * 10.0) + 1.5).min(5.0);
            player.happiness.add_event_with_cooldown(
                HappinessEventType::SalaryGapNoticed,
                magnitude,
                28,
            );
        }
    }
}
