use crate::club::team::behaviour::{ContractTermination, ManagerTalkResult, ManagerTalkType, PlayerRelationshipChangeResult, TeamBehaviourResult};
use crate::context::GlobalContext;
use crate::utils::{DateUtils, FloatUtils};
use crate::utils::IntegerUtils;
use crate::{
    ChangeType, ContractType, HappinessEventType, Person, PersonBehaviourState, Player,
    PlayerCollection, PlayerFieldPositionGroup, PlayerPositionType, PlayerRelation,
    PlayerSquadStatus, PlayerStatusType, Staff, StaffCollection, StaffPosition,
};
use chrono::{Datelike, NaiveDate};
use log::debug;

#[derive(Debug, Clone)]
pub struct TeamBehaviour {
    last_full_update: Option<chrono::NaiveDateTime>,
    last_minor_update: Option<chrono::NaiveDateTime>,
}

impl Default for TeamBehaviour {
    fn default() -> Self {
        Self::new()
    }
}

impl TeamBehaviour {
    pub fn new() -> Self {
        TeamBehaviour {
            last_full_update: None,
            last_minor_update: None,
        }
    }

    /// Main simulate function that decides what type of update to run
    pub fn simulate(
        &mut self,
        players: &mut PlayerCollection,
        staffs: &mut StaffCollection,
        ctx: GlobalContext<'_>,
    ) -> TeamBehaviourResult {
        let current_time = ctx.simulation.date;

        let should_run_full = self.should_run_full_update(current_time);
        let should_run_minor = self.should_run_minor_update(current_time);

        if should_run_full {
            debug!("Running FULL team behaviour update at {}", current_time);
            self.last_full_update = Some(current_time);
            self.run_full_behaviour_simulation(players, staffs, ctx)
        } else if should_run_minor {
            debug!("Running minor team behaviour update at {}", current_time);
            self.last_minor_update = Some(current_time);
            self.run_minor_behaviour_simulation(players, staffs, ctx)
        } else {
            TeamBehaviourResult::new()
        }
    }

    fn should_run_full_update(&self, current_time: chrono::NaiveDateTime) -> bool {
        match self.last_full_update {
            None => true,
            Some(last) => {
                let days_since = current_time.signed_duration_since(last).num_days();
                days_since >= 7
                    || (days_since >= 1
                        && (current_time.weekday() == chrono::Weekday::Sat
                            || current_time.weekday() == chrono::Weekday::Sun
                            || current_time.day() == 1))
            }
        }
    }

    fn should_run_minor_update(&self, current_time: chrono::NaiveDateTime) -> bool {
        match self.last_minor_update {
            None => true,
            Some(last) => {
                let days_since = current_time.signed_duration_since(last).num_days();
                days_since >= 2
                    || (days_since >= 1
                        && matches!(
                            current_time.weekday(),
                            chrono::Weekday::Tue | chrono::Weekday::Wed | chrono::Weekday::Thu
                        ))
            }
        }
    }

    /// Full comprehensive behaviour simulation
    fn run_full_behaviour_simulation(
        &self,
        players: &mut PlayerCollection,
        staffs: &mut StaffCollection,
        ctx: GlobalContext<'_>,
    ) -> TeamBehaviourResult {
        let mut result = TeamBehaviourResult::new();

        Self::log_team_state(players, "BEFORE full update");

        // Core interaction types
        Self::process_position_group_dynamics(players, &mut result);
        Self::process_age_group_dynamics(players, &mut result, &ctx);
        Self::process_performance_based_relationships(players, &mut result);
        Self::process_personality_conflicts(players, &mut result);
        Self::process_leadership_influence(players, &mut result);
        Self::process_playing_time_jealousy(players, &mut result);

        // Reputation-driven dynamics
        Self::process_reputation_dynamics(players, &mut result);
        Self::process_mentorship_dynamics(players, &mut result, &ctx);

        // Additional full-update processes
        Self::process_contract_satisfaction(players, &mut result, &ctx);
        Self::process_injury_sympathy(players, &mut result, &ctx);
        Self::process_international_duty_bonds(players, &mut result, &ctx);

        // Squad integration events — settled in or feeling isolated
        Self::process_squad_integration(players, &ctx);

        // Captain's mood propagates: happy captain lifts the squad, a
        // demoralised captain drags it. Runs before manager talks so the
        // manager-talk picker sees the updated morale distribution.
        Self::process_captain_morale_propagation(players);

        // Contract jealousy — a teammate's new big deal unsettles the
        // lower-paid players around them, especially ones who weren't
        // already on good terms with the signer.
        Self::process_contract_jealousy(players, &ctx);

        // Monthly peer-wage audit: a starter earning <60% of the top
        // earner at their position gets a structural envy hit even when
        // no one has just signed.
        Self::process_periodic_wage_envy(players, &ctx);

        // Monthly controversy check — hot-headed players occasionally light
        // fires that drag their own morale and unsettle nearby teammates.
        Self::process_controversy_incidents(players, &ctx);

        // Monthly loan-playing-time audit: if a loanee isn't tracking to
        // hit their contractual minimum apps, the parent club's recall
        // window opens and the player feels the frustration.
        Self::process_loan_playing_time_audit(players, &ctx);

        // Manager-player talks (weekly during full update)
        Self::process_manager_player_talks(players, staffs, &mut result);

        // Playing time complaints (player-initiated requests)
        Self::process_playing_time_complaints(players, staffs, &mut result, &ctx);

        // Head-coach-driven squad cleanup: terminate contracts of players
        // the manager has given up on, provided the payout is acceptable.
        Self::process_coach_contract_terminations(players, staffs, &mut result, &ctx);

        debug!(
            "Full team behaviour update complete - {} relationship changes, {} manager talks",
            result.players.relationship_result.len(),
            result.manager_talks.len()
        );

        result
    }

    /// Lighter, more frequent behaviour updates
    fn run_minor_behaviour_simulation(
        &self,
        players: &mut PlayerCollection,
        staffs: &mut StaffCollection,
        ctx: GlobalContext<'_>,
    ) -> TeamBehaviourResult {
        let _ = staffs; // Not used in minor updates
        let mut result = TeamBehaviourResult::new();

        Self::process_daily_interactions(players, &mut result, &ctx);
        Self::process_mood_changes(players, &mut result, &ctx);
        Self::process_recent_performance_reactions(players, &mut result);

        result
    }

    fn log_team_state(players: &PlayerCollection, context: &str) {
        debug!("Team State {}: {} players", context, players.players.len());

        let mut happy_players = 0;
        let mut unhappy_players = 0;
        let mut neutral_players = 0;

        for player in &players.players {
            let happiness = Self::calculate_player_happiness(player);
            if happiness > 0.2 {
                happy_players += 1;
            } else if happiness < -0.2 {
                unhappy_players += 1;
            } else {
                neutral_players += 1;
            }
        }

        debug!(
            "Happy: {} | Neutral: {} | Unhappy: {}",
            happy_players, neutral_players, unhappy_players
        );
    }

    // ========== MINOR UPDATE PROCESSES ==========

    fn process_daily_interactions(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        for i in 0..players.players.len().min(10) {
            for j in i + 1..players.players.len().min(10) {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                if let Some(existing_relationship) = player_i.relations.get_player(player_j.id) {
                    let daily_change = Self::calculate_daily_interaction_change(
                        player_i,
                        player_j,
                        existing_relationship,
                        ctx,
                    );

                    if daily_change.abs() > 0.005 {
                        result
                            .players
                            .relationship_result
                            .push(PlayerRelationshipChangeResult {
                                from_player_id: player_i.id,
                                to_player_id: player_j.id,
                                relationship_change: daily_change,
                                change_type: ChangeType::NaturalProgression,
                            });
                    }
                }
            }
        }
    }

    fn process_mood_changes(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        _ctx: &GlobalContext<'_>,
    ) {
        for player in &players.players {
            let current_happiness = Self::calculate_player_happiness(player);

            if current_happiness < -0.5 {
                for other_player in &players.players {
                    if player.id != other_player.id {
                        let mood_impact =
                            Self::calculate_mood_spread(player, other_player, current_happiness);

                        if mood_impact.abs() > 0.01 {
                            result.players.relationship_result.push(
                                PlayerRelationshipChangeResult {
                                    from_player_id: other_player.id,
                                    to_player_id: player.id,
                                    relationship_change: mood_impact,
                                    change_type: ChangeType::PersonalConflict,
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    /// Generate squad integration events: "settled into squad" or "feeling isolated".
    /// Runs weekly. Also generates "dressing room speech" from team leaders.
    fn process_squad_integration(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let sim_date = ctx.simulation.date.date();

        // Collect teammate IDs for relationship lookups
        let teammate_ids: Vec<u32> = players.iter().map(|p| p.id).collect();

        for player in players.iter_mut() {
            // Integration events for recent transfers (first 90 days)
            let is_recent = player.last_transfer_date
                .map(|d| (sim_date - d).num_days() < 90)
                .unwrap_or(false);

            if is_recent {
                // Count positive relationships with current teammates
                let positive_count = teammate_ids.iter()
                    .filter(|&&tid| tid != player.id)
                    .filter(|&&tid| player.relations.get_player(tid)
                        .map(|r| r.level > 20.0).unwrap_or(false))
                    .count();

                let has_any_relation = teammate_ids.iter()
                    .any(|&tid| tid != player.id && player.relations.get_player(tid).is_some());

                // ~10% weekly chance, biased by relationship count.
                // Cooldowns prevent the same player firing back-to-back
                // weekly — settling-in is a slow process; isolation
                // shouldn't tick every fortnight while the player is
                // already adjusting elsewhere.
                if positive_count >= 3 && rand::random::<f32>() < 0.10 {
                    // Long cooldown so a single transfer spell yields at
                    // most one settling-in event — the previous 21-day
                    // window let it fire ~4× per 90-day adaptation period
                    // and read as event-feed spam. Happiness is cleared
                    // on every transfer, so the cooldown effectively
                    // resets at the next club.
                    player.happiness.add_event_with_cooldown(
                        HappinessEventType::SettledIntoSquad,
                        2.0,
                        365,
                    );
                } else if !has_any_relation && rand::random::<f32>() < 0.08 {
                    player.happiness.add_event_with_cooldown(
                        HappinessEventType::FeelingIsolated,
                        -1.5,
                        14,
                    );
                }
            }
        }
    }

    fn process_recent_performance_reactions(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
        for player in &players.players {
            // Require actual appearances and a notable goal ratio, not just goals > 0.
            // This prevents a single early-season goal from generating boosts all year.
            if player.statistics.played == 0 || !player.position().is_forward() {
                continue;
            }

            let goals_per_game =
                player.statistics.goals as f32 / player.statistics.played as f32;

            if goals_per_game > 0.25 {
                let rep_factor = (player.player_attributes.current_reputation as f32 / 10000.0)
                    .clamp(0.1, 1.0);
                // Scale down for minor-update frequency (~every 2 days vs weekly full update)
                let popularity_boost = (0.03 + 0.04 * rep_factor) * 0.25;

                for other_player in &players.players {
                    if player.id != other_player.id {
                        result
                            .players
                            .relationship_result
                            .push(PlayerRelationshipChangeResult {
                                from_player_id: other_player.id,
                                to_player_id: player.id,
                                relationship_change: popularity_boost,
                                change_type: ChangeType::MatchCooperation,
                            });
                    }
                }
            }
        }
    }

    // ========== FULL UPDATE PROCESSES ==========

    /// Players in similar positions compete; complementary positions bond
    fn process_position_group_dynamics(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let position_i = player_i.position();
                let position_j = player_j.position();

                if position_i == position_j {
                    let competition_factor = Self::calculate_competition_factor(player_i, player_j);

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: -competition_factor,
                            change_type: ChangeType::CompetitionRivalry,
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: -competition_factor,
                            change_type: ChangeType::CompetitionRivalry,
                        });
                } else if Self::are_complementary_positions(&position_i, &position_j) {
                    let synergy_factor = Self::calculate_synergy_factor(player_i, player_j);

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: synergy_factor,
                            change_type: ChangeType::TrainingBonding,
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: synergy_factor,
                            change_type: ChangeType::TrainingBonding,
                        });
                }
            }
        }
    }

    /// Age groups form bonds - young players stick together, veterans mentor youth
    fn process_age_group_dynamics(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        let current_date = ctx.simulation.date.date();

        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let age_i = player_i.age(current_date);
                let age_j = player_j.age(current_date);

                let age_diff = (age_i as i32 - age_j as i32).abs();
                let relationship_change =
                    Self::calculate_age_relationship_factor(age_i, age_j, age_diff);

                if relationship_change.abs() > 0.01 {
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change,
                            change_type: ChangeType::NaturalProgression,
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change,
                            change_type: ChangeType::NaturalProgression,
                        });
                }
            }
        }
    }

    /// Performance-based relationships using stats and reputation
    fn process_performance_based_relationships(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let performance_i = Self::calculate_player_performance_rating(player_i);
                let performance_j = Self::calculate_player_performance_rating(player_j);

                let performance_diff = (performance_i - performance_j).abs();
                let relationship_change = Self::calculate_performance_relationship_factor(
                    performance_i,
                    performance_j,
                    performance_diff,
                    player_i,
                    player_j,
                );

                if relationship_change.abs() > 0.01 {
                    let change_type = if relationship_change > 0.0 {
                        ChangeType::MatchCooperation
                    } else {
                        ChangeType::CompetitionRivalry
                    };

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change,
                            change_type: change_type.clone(),
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change,
                            change_type,
                        });
                }
            }
        }
    }

    /// Personality conflicts
    fn process_personality_conflicts(players: &PlayerCollection, result: &mut TeamBehaviourResult) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let conflict_factor = Self::calculate_personality_conflict(player_i, player_j);

                if conflict_factor.abs() > 0.02 {
                    let change_type = if conflict_factor > 0.0 {
                        ChangeType::PersonalSupport
                    } else {
                        ChangeType::PersonalConflict
                    };

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: conflict_factor,
                            change_type: change_type.clone(),
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: conflict_factor,
                            change_type,
                        });
                }
            }
        }
    }

    /// When a teammate signs a notably bigger deal and this player earns
    /// meaningfully less, morale takes a hit — unless they're close friends.
    /// Fires at most once per player per signing window (the signer's
    /// `last_salary_negotiation` timestamp gates it). Gap threshold ≥25%.
    fn process_contract_jealousy(players: &mut PlayerCollection, ctx: &GlobalContext<'_>) {
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
    fn process_loan_playing_time_audit(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        if today.day() != 1 {
            return;
        }

        for player in players.players.iter_mut() {
            let Some(loan) = player.contract_loan.as_mut() else { continue };
            let Some(min_apps) = loan.loan_min_appearances else { continue };
            let Some(loan_start) = loan.started else { continue };
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
    fn process_controversy_incidents(players: &mut PlayerCollection, ctx: &GlobalContext<'_>) {
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
                let risk = controversy + (20.0 - temperament) * 0.6
                    + (20.0 - sportsmanship) * 0.4;
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
    fn process_periodic_wage_envy(players: &mut PlayerCollection, ctx: &GlobalContext<'_>) {
        let today = ctx.simulation.date.date();
        if today.day() != 1 {
            return; // Monthly only
        }

        use std::collections::HashMap;

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
            let Some(contract) = p.contract.as_ref() else { continue };
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
            let Some(contract) = player.contract.as_ref() else { continue };
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

    /// Captain = highest `leadership + influence` on the squad. Their
    /// mood leaks out to teammates: ~±2 morale points/week based on how
    /// happy the captain is relative to neutral 50. Sits on top of the
    /// existing `process_leadership_influence` pass (which only moves
    /// relationship numbers, not morale).
    fn process_captain_morale_propagation(players: &mut PlayerCollection) {
        // Pick the captain by compound score. Don't fall back to anyone
        // with <10 leadership — a weak captain shouldn't propagate.
        let captain_id_opt = players
            .players
            .iter()
            .filter(|p| p.skills.mental.leadership >= 10.0)
            .max_by(|a, b| {
                let sa = a.skills.mental.leadership * 1.0
                    + a.attributes.loyalty * 0.5
                    + a.player_attributes.current_reputation as f32 / 2000.0;
                let sb = b.skills.mental.leadership * 1.0
                    + b.attributes.loyalty * 0.5
                    + b.player_attributes.current_reputation as f32 / 2000.0;
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| p.id);

        let captain_id = match captain_id_opt {
            Some(id) => id,
            None => return,
        };

        let captain_morale = match players.find(captain_id) {
            Some(c) => c.happiness.morale,
            None => return,
        };

        // Delta: captain at 50 morale → 0 effect. At 80 → +0.6, at 20 → -0.6.
        // Leadership scales the magnitude (a 20-leadership captain hits 2x
        // a 10-leadership captain).
        let captain_leadership = players
            .find(captain_id)
            .map(|c| c.skills.mental.leadership)
            .unwrap_or(10.0);
        let leadership_scale = (captain_leadership / 20.0).clamp(0.0, 1.0);
        let base_delta = (captain_morale - 50.0) * 0.02;  // -1..1
        let delta = base_delta * leadership_scale;        // -1..1 scaled

        if delta.abs() < 0.05 {
            return;
        }

        for player in players.players.iter_mut() {
            if player.id == captain_id {
                continue;
            }
            // Per-teammate sway — a captain someone respects lands harder,
            // someone the squad mistrusts lands weaker (or slightly
            // negative on a good-mood captain). Maps level [-100,100] →
            // multiplier [0.3, 1.5].
            let relation_mult = player
                .relations
                .get_player(captain_id)
                .map(|r| 0.9 + (r.level / 100.0).clamp(-0.6, 0.6))
                .unwrap_or(1.0);
            player.happiness.adjust_morale(delta * relation_mult);
        }
    }

    /// High leadership players influence team morale and relationships
    fn process_leadership_influence(players: &PlayerCollection, result: &mut TeamBehaviourResult) {
        let leaders: Vec<&Player> = players
            .players
            .iter()
            .filter(|p| p.skills.mental.leadership > 15.0)
            .collect();

        for leader in leaders {
            for player in &players.players {
                if leader.id == player.id {
                    continue;
                }

                let influence = Self::calculate_leadership_influence(leader, player);

                if influence.abs() > 0.01 {
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player.id,
                            to_player_id: leader.id,
                            relationship_change: influence,
                            change_type: ChangeType::MentorshipBond,
                        });
                }
            }
        }
    }

    /// Playing time jealousy
    fn process_playing_time_jealousy(players: &PlayerCollection, result: &mut TeamBehaviourResult) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let playing_time_i = player_i.statistics.played;
                let playing_time_j = player_j.statistics.played;

                let jealousy_factor = Self::calculate_playing_time_jealousy(
                    playing_time_i,
                    playing_time_j,
                    player_i,
                    player_j,
                );

                if jealousy_factor.abs() > 0.01 {
                    let change_type = if jealousy_factor > 0.0 {
                        ChangeType::TrainingBonding
                    } else {
                        ChangeType::CompetitionRivalry
                    };

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: jealousy_factor,
                            change_type: change_type.clone(),
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: jealousy_factor,
                            change_type,
                        });
                }
            }
        }
    }

    // ========== REPUTATION-DRIVEN PROCESSES ==========

    /// Reputation dynamics: star players command respect or create tension
    /// High-reputation players are admired by professional teammates but
    /// resented by ambitious players who feel overshadowed
    fn process_reputation_dynamics(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let rep_i = player_i.player_attributes.current_reputation as f32;
                let rep_j = player_j.player_attributes.current_reputation as f32;
                let rep_diff = (rep_i - rep_j).abs();

                if rep_diff < 500.0 {
                    // Similar reputation: mutual professional respect
                    let max_rep = rep_i.max(rep_j);
                    let respect_bonus = 0.03 * (max_rep / 10000.0).clamp(0.1, 1.0);

                    if respect_bonus > 0.005 {
                        result.players.relationship_result.push(
                            PlayerRelationshipChangeResult {
                                from_player_id: player_i.id,
                                to_player_id: player_j.id,
                                relationship_change: respect_bonus,
                                change_type: ChangeType::ReputationAdmiration,
                            },
                        );
                        result.players.relationship_result.push(
                            PlayerRelationshipChangeResult {
                                from_player_id: player_j.id,
                                to_player_id: player_i.id,
                                relationship_change: respect_bonus,
                                change_type: ChangeType::ReputationAdmiration,
                            },
                        );
                    }
                } else if rep_diff > 3000.0 {
                    // Large reputation gap: admiration or resentment
                    let (star, lesser) = if rep_i > rep_j {
                        (player_i, player_j)
                    } else {
                        (player_j, player_i)
                    };
                    let (star_id, lesser_id) = (star.id, lesser.id);

                    // Lesser player's reaction depends on personality
                    let admiration = (lesser.attributes.sportsmanship / 20.0) * 0.1
                        + (lesser.attributes.professionalism / 20.0) * 0.05;

                    let resentment = if lesser.attributes.ambition > 14.0 {
                        (lesser.attributes.ambition - 14.0) / 6.0 * -0.08
                    } else {
                        0.0
                    };

                    let lesser_to_star = admiration + resentment;

                    if lesser_to_star.abs() > 0.01 {
                        let change_type = if lesser_to_star > 0.0 {
                            ChangeType::ReputationAdmiration
                        } else {
                            ChangeType::ReputationTension
                        };

                        result.players.relationship_result.push(
                            PlayerRelationshipChangeResult {
                                from_player_id: lesser_id,
                                to_player_id: star_id,
                                relationship_change: lesser_to_star,
                                change_type,
                            },
                        );
                    }

                    // Star player's reaction: professional stars are approachable,
                    // controversial stars create tension
                    let star_to_lesser = if star.attributes.professionalism > 14.0 {
                        0.04 * (star.attributes.professionalism / 20.0)
                    } else if star.attributes.controversy > 14.0 {
                        -0.06 * (star.attributes.controversy / 20.0)
                    } else {
                        0.0
                    };

                    if star_to_lesser.abs() > 0.01 {
                        let change_type = if star_to_lesser > 0.0 {
                            ChangeType::PersonalSupport
                        } else {
                            ChangeType::ReputationTension
                        };

                        result.players.relationship_result.push(
                            PlayerRelationshipChangeResult {
                                from_player_id: star_id,
                                to_player_id: lesser_id,
                                relationship_change: star_to_lesser,
                                change_type,
                            },
                        );
                    }
                } else if rep_diff > 1000.0 {
                    // Moderate reputation gap: small professional respect toward higher-rep player
                    let (higher_id, lower_id) = if rep_i > rep_j {
                        (player_i.id, player_j.id)
                    } else {
                        (player_j.id, player_i.id)
                    };

                    let respect = 0.02 * (rep_i.max(rep_j) / 10000.0).clamp(0.1, 1.0);

                    result.players.relationship_result.push(
                        PlayerRelationshipChangeResult {
                            from_player_id: lower_id,
                            to_player_id: higher_id,
                            relationship_change: respect,
                            change_type: ChangeType::ReputationAdmiration,
                        },
                    );
                }
            }
        }
    }

    /// Mentorship dynamics: experienced veterans mentor young players
    /// based on reputation, leadership, age gap, and position compatibility
    fn process_mentorship_dynamics(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        let current_date = ctx.simulation.date.date();

        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let age_i = player_i.age(current_date);
                let age_j = player_j.age(current_date);

                let age_diff = (age_i as i32 - age_j as i32).abs();
                if age_diff < 5 {
                    continue;
                }

                let (veteran, youth) = if age_i > age_j {
                    (player_i, player_j)
                } else {
                    (player_j, player_i)
                };
                let (vet_age, youth_age) = if age_i > age_j {
                    (age_i, age_j)
                } else {
                    (age_j, age_i)
                };

                // Veteran must be 28+ and youth must be under 24
                if vet_age < 28 || youth_age > 23 {
                    continue;
                }

                // Mentorship potential factors:
                // - Veteran's leadership and experience
                // - Veteran's reputation (experienced, respected players mentor better)
                // - Youth's adaptability (how well they receive mentorship)
                // - Position compatibility (same position = more relevant)
                let leadership_factor =
                    (veteran.skills.mental.leadership / 20.0).clamp(0.0, 1.0);
                let rep_factor =
                    (veteran.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
                let adaptability_factor =
                    (youth.attributes.adaptability / 20.0).clamp(0.0, 1.0);

                let same_position = veteran.position() == youth.position();
                let position_bonus = if same_position { 1.5 } else { 1.0 };

                // Professionalism of the veteran matters
                let professionalism_factor =
                    (veteran.attributes.professionalism / 20.0).clamp(0.0, 1.0);

                let mentorship_strength = leadership_factor
                    * (0.3 + rep_factor * 0.4 + professionalism_factor * 0.3)
                    * adaptability_factor
                    * position_bonus
                    * 0.12;

                if mentorship_strength > 0.015 {
                    // Youth admires and learns from veteran
                    result.players.relationship_result.push(
                        PlayerRelationshipChangeResult {
                            from_player_id: youth.id,
                            to_player_id: veteran.id,
                            relationship_change: mentorship_strength,
                            change_type: ChangeType::MentorshipBond,
                        },
                    );

                    // Veteran gains satisfaction from mentoring (slightly less)
                    result.players.relationship_result.push(
                        PlayerRelationshipChangeResult {
                            from_player_id: veteran.id,
                            to_player_id: youth.id,
                            relationship_change: mentorship_strength * 0.6,
                            change_type: ChangeType::MentorshipBond,
                        },
                    );
                }
            }
        }
    }

    // ========== ADDITIONAL FULL UPDATE PROCESSES ==========

    fn process_contract_satisfaction(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        _ctx: &GlobalContext<'_>,
    ) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let (jealousy_i_to_j, jealousy_j_to_i) =
                    Self::calculate_contract_jealousy(player_i, player_j);

                if jealousy_i_to_j.abs() > 0.02 {
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: jealousy_i_to_j,
                            change_type: ChangeType::PersonalConflict,
                        });
                }

                if jealousy_j_to_i.abs() > 0.02 {
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: jealousy_j_to_i,
                            change_type: ChangeType::PersonalConflict,
                        });
                }
            }
        }
    }

    fn process_injury_sympathy(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        _ctx: &GlobalContext<'_>,
    ) {
        for injured_player in players
            .players
            .iter()
            .filter(|p| p.player_attributes.is_injured)
        {
            for other_player in &players.players {
                if injured_player.id != other_player.id {
                    let sympathy = Self::calculate_injury_sympathy(injured_player, other_player);

                    if sympathy > 0.01 {
                        result
                            .players
                            .relationship_result
                            .push(PlayerRelationshipChangeResult {
                                from_player_id: other_player.id,
                                to_player_id: injured_player.id,
                                relationship_change: sympathy,
                                change_type: ChangeType::PersonalSupport,
                            });
                    }
                }
            }
        }
    }

    fn process_international_duty_bonds(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        _ctx: &GlobalContext<'_>,
    ) {
        use std::collections::HashMap;
        let mut country_groups: HashMap<u32, Vec<&Player>> = HashMap::new();

        for player in &players.players {
            country_groups
                .entry(player.country_id)
                .or_default()
                .push(player);
        }

        for (_, country_players) in country_groups {
            if country_players.len() > 1 {
                for i in 0..country_players.len() {
                    for j in i + 1..country_players.len() {
                        let bond_strength = Self::calculate_national_team_bond(
                            country_players[i],
                            country_players[j],
                        );

                        if bond_strength > 0.01 {
                            result.players.relationship_result.push(
                                PlayerRelationshipChangeResult {
                                    from_player_id: country_players[i].id,
                                    to_player_id: country_players[j].id,
                                    relationship_change: bond_strength,
                                    change_type: ChangeType::TrainingBonding,
                                },
                            );

                            result.players.relationship_result.push(
                                PlayerRelationshipChangeResult {
                                    from_player_id: country_players[j].id,
                                    to_player_id: country_players[i].id,
                                    relationship_change: bond_strength,
                                    change_type: ChangeType::TrainingBonding,
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    // ========== MANAGER-PLAYER TALKS ==========

    fn process_manager_player_talks(
        players: &PlayerCollection,
        staffs: &StaffCollection,
        result: &mut TeamBehaviourResult,
    ) {
        // Find the manager
        let manager = match staffs.find_by_position(StaffPosition::Manager) {
            Some(m) => m,
            None => return,
        };

        // Identify players who need talks, sorted by priority
        let mut talk_candidates: Vec<(u32, ManagerTalkType, u8)> = Vec::new(); // (player_id, type, priority)

        for player in &players.players {
            let statuses = player.statuses.get();

            // Highest priority: transfer request
            if statuses.contains(&PlayerStatusType::Req) {
                talk_candidates.push((player.id, ManagerTalkType::TransferDiscussion, 100));
            }

            // High priority: unhappy players
            if statuses.contains(&PlayerStatusType::Unh) {
                // Decide between playing time talk and morale talk
                let talk_type = if player.happiness.factors.playing_time < -5.0 {
                    ManagerTalkType::PlayingTimeTalk
                } else {
                    ManagerTalkType::MoraleTalk
                };
                talk_candidates.push((player.id, talk_type, 90));
            }

            // Proactive: coach talks to high-ability players showing early playing time
            // frustration BEFORE they become fully unhappy — persuade them to stay patient.
            // Only for players with developed skills (CA >= 80).
            let ability = player.player_attributes.current_ability;
            if ability >= 80
                && player.happiness.factors.playing_time < -3.0
                && !statuses.contains(&PlayerStatusType::Unh)
                && !statuses.contains(&PlayerStatusType::Req)
            {
                // Higher ability = higher priority for proactive talk
                let priority = 75 + (ability.saturating_sub(80) / 10).min(15);
                talk_candidates.push((player.id, ManagerTalkType::PlayingTimeTalk, priority));
            }

            // Medium priority: very low morale
            if player.happiness.morale < 30.0
                && !statuses.contains(&PlayerStatusType::Unh)
                && !statuses.contains(&PlayerStatusType::Req)
            {
                talk_candidates.push((player.id, ManagerTalkType::Motivational, 70));
            }

            // Lower priority: praise good performers
            if player.behaviour.is_good() && player.happiness.morale < 80.0 {
                talk_candidates.push((player.id, ManagerTalkType::Praise, 30));
            }

            // Discipline for poor behaviour + high ability
            if player.behaviour.is_poor() && player.player_attributes.current_ability > 100 {
                talk_candidates.push((player.id, ManagerTalkType::Discipline, 60));
            }

            // Form-driven automatic talks — gate on the manager's personality.
            // A strong motivator spots hot streaks; a strong disciplinarian
            // spots slumps. Managers weak in both skip form-based talks.
            let mgr_motivating = manager.staff_attributes.mental.motivating;
            let mgr_discipline = manager.staff_attributes.mental.discipline;
            let form = player.statistics.average_rating;
            let apps = player.statistics.played + player.statistics.played_subs;
            if apps >= 3 {
                if mgr_motivating >= 14 && form >= 7.5 && player.happiness.morale < 85.0 {
                    talk_candidates.push((player.id, ManagerTalkType::Praise, 55));
                }
                if mgr_discipline >= 14
                    && form > 0.0
                    && form < 5.5
                    && player.player_attributes.current_ability >= 70
                {
                    talk_candidates.push((player.id, ManagerTalkType::Discipline, 55));
                }
            }
        }

        // Sort by priority (highest first)
        talk_candidates.sort_by(|a, b| b.2.cmp(&a.2));

        // Max 4 talks per week
        let max_talks = 4.min(talk_candidates.len());

        for i in 0..max_talks {
            let (player_id, talk_type, _) = &talk_candidates[i];

            if let Some(player) = players.find(*player_id) {
                let talk_result = Self::conduct_manager_talk(manager, player, talk_type.clone());
                result.manager_talks.push(talk_result);
            }
        }
    }

    fn conduct_manager_talk(
        manager: &Staff,
        player: &Player,
        talk_type: ManagerTalkType,
    ) -> ManagerTalkResult {
        // Success chance formula
        let man_management = manager.staff_attributes.mental.man_management as f32;
        let motivating = manager.staff_attributes.mental.motivating as f32;
        let temperament = player.attributes.temperament;
        let professionalism = player.attributes.professionalism;
        let loyalty = player.attributes.loyalty;

        // Relationship bonus from existing relationship
        let relationship_bonus = player.relations.get_staff(manager.id)
            .map(|r| (r.level / 100.0) * 0.2)
            .unwrap_or(0.0);

        let success_chance = (0.5
            + man_management / 40.0
            + motivating / 60.0
            - temperament / 60.0
            + professionalism / 80.0
            + loyalty / 80.0
            + relationship_bonus)
            .clamp(0.1, 0.95);

        let success = rand::random::<f32>() < success_chance;

        // For transfer discussion, the talk succeeding doesn't guarantee the player
        // withdraws the request — there's only a 30% chance of that happening.
        let actual_success = if talk_type == ManagerTalkType::TransferDiscussion && success {
            rand::random::<f32>() < 0.3
        } else {
            success
        };

        // Outcomes are determined by actual_success so that the morale/relationship
        // effects are consistent with what the result processing sees.
        let (morale_change, relationship_change) = match (&talk_type, actual_success) {
            (ManagerTalkType::PlayingTimeTalk, true) => (10.0, 0.3),
            (ManagerTalkType::PlayingTimeTalk, false) => (-5.0, -0.1),
            (ManagerTalkType::MoraleTalk, true) => (8.0, 0.3),
            (ManagerTalkType::MoraleTalk, false) => (-3.0, -0.2),
            (ManagerTalkType::TransferDiscussion, true) => (5.0, 0.2),
            (ManagerTalkType::TransferDiscussion, false) => (0.0, 0.0),
            (ManagerTalkType::Praise, true) => (5.0, 0.5),
            (ManagerTalkType::Praise, false) => (1.0, 0.1),
            (ManagerTalkType::Discipline, true) => (-3.0, 0.1),
            (ManagerTalkType::Discipline, false) => (-8.0, -0.5),
            (ManagerTalkType::Motivational, true) => (6.0, 0.2),
            (ManagerTalkType::Motivational, false) => (-2.0, -0.1),
            (ManagerTalkType::PlayingTimeRequest, true) => (8.0, 0.3),
            (ManagerTalkType::PlayingTimeRequest, false) => (-5.0, -0.2),
            (ManagerTalkType::LoanRequest, true) => (5.0, 0.2),
            (ManagerTalkType::LoanRequest, false) => (-3.0, -0.1),
        };

        debug!(
            "Manager talk: {} with player {} - type {:?}, success: {}",
            manager.full_name, player.full_name, talk_type, actual_success
        );

        ManagerTalkResult {
            player_id: player.id,
            staff_id: manager.id,
            talk_type,
            success: actual_success,
            morale_change,
            relationship_change,
        }
    }

    // ========== PLAYING TIME COMPLAINTS ==========

    fn process_playing_time_complaints(
        players: &PlayerCollection,
        staffs: &StaffCollection,
        result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        let manager = match staffs.find_by_position(StaffPosition::Manager) {
            Some(m) => m,
            None => return,
        };

        let current_date = ctx.simulation.date.date();

        // Collect complaint candidates with priority score for sorting
        let mut candidates: Vec<(u32, ManagerTalkType, u32)> = Vec::new();

        for player in &players.players {
            if player.player_attributes.is_injured {
                continue;
            }

            let age = DateUtils::age(player.birth_date, current_date);
            if age < 16 {
                continue;
            }

            // Already has a transfer request or loan status
            let statuses = player.statuses.get();
            if statuses.contains(&PlayerStatusType::Req)
                || statuses.contains(&PlayerStatusType::Loa)
            {
                continue;
            }

            // Skip players already on loan from another club
            if player.is_on_loan() {
                continue;
            }

            let ability = player.player_attributes.current_ability;
            let ambition = player.attributes.ambition;
            let determination = player.skills.mental.determination;
            let days = player.player_attributes.days_since_last_match;

            // Skip players marked as NotNeeded (they accept their fate)
            let squad_status = player.contract.as_ref().map(|c| &c.squad_status);
            if matches!(squad_status, Some(PlayerSquadStatus::NotNeeded)) {
                continue;
            }

            // ── Check 1: Youth prospect wants real football (loan request) ──
            // Young players with prospect status who aren't getting meaningful
            // first-team football should request loans for development.
            let is_prospect = matches!(
                squad_status,
                Some(PlayerSquadStatus::HotProspectForTheFuture)
                    | Some(PlayerSquadStatus::DecentYoungster)
            );

            if is_prospect && age >= 19 && age <= 23 {
                // Priority increases with age — a 22yo prospect is more urgent than a 19yo
                let age_urgency = (age as f32 - 18.0) / 5.0; // 0.2 at 19, 0.8 at 22
                let ambition_factor = ambition / 20.0; // 0-1
                let determination_factor = determination / 20.0;

                // Ambitious, determined prospects request loans sooner
                let desire = age_urgency * 0.4 + ambition_factor * 0.35 + determination_factor * 0.25;

                // At age 21+ with decent ambition (>10), almost always request
                // At age 19-20, need high ambition (>14) or long wait
                let threshold = if age >= 21 {
                    0.35 // Lower bar — most 21+ prospects want real football
                } else {
                    0.55 // Higher bar — 19-20 year olds need more drive
                };

                if desire > threshold || (age >= 21 && days > 14) {
                    let priority = (desire * 100.0) as u32 + age as u32 * 10;
                    candidates.push((player.id, ManagerTalkType::LoanRequest, priority));
                    continue;
                }
            }

            // ── Check 2: Playing time complaints (existing logic, enhanced) ──
            // Only skilled players complain
            if ability < 60 {
                continue;
            }

            let ability_modifier = (ability as f32 - 60.0) / 140.0;
            let ambition_modifier = 1.0 - ambition / 30.0;
            let combined_modifier = (ambition_modifier * 0.5 + (1.0 - ability_modifier) * 0.5).max(0.4);
            let threshold = (21.0 * combined_modifier) as u16;

            let playing_time_factor =
                Self::calculate_playing_time_factor_for_complaint(player);

            if days > threshold || playing_time_factor < -10.0 {
                let talk_type = if age < 23 {
                    // Young players request loans, not just playing time
                    ManagerTalkType::LoanRequest
                } else {
                    ManagerTalkType::PlayingTimeRequest
                };

                let priority = days as u32 + if playing_time_factor < -10.0 { 50 } else { 0 };
                candidates.push((player.id, talk_type, priority));
            }
        }

        // Sort by priority descending (most urgent first)
        candidates.sort_by(|a, b| b.2.cmp(&a.2));

        // Max 2 complaints per week
        let max_complaints = 2.min(candidates.len());

        for i in 0..max_complaints {
            let (player_id, talk_type, _) = &candidates[i];

            if let Some(player) = players.find(*player_id) {
                let talk_result =
                    Self::conduct_loan_or_playing_time_talk(manager, player, talk_type.clone());
                result.manager_talks.push(talk_result);
            }
        }
    }

    // ========== COACH-DRIVEN CONTRACT TERMINATION ==========

    /// Head coach reviews the squad for unwanted players whose contracts
    /// can be torn up cheaply. Fires when the three FM-style criteria line
    /// up: the player is structurally surplus (NotNeeded or a deadwood
    /// youth), they're not a developing prospect, and the payout is small
    /// enough that the club would rather eat it than keep paying wages.
    fn process_coach_contract_terminations(
        players: &PlayerCollection,
        staffs: &StaffCollection,
        result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        if staffs.find_by_position(StaffPosition::Manager).is_none() {
            return;
        }

        let date = ctx.simulation.date.date();

        // Use one month of the squad's total wage bill as the cap for a
        // cheap termination — clubs tolerate a payout of that size to free
        // a squad slot. Scales naturally with club size/wealth.
        let monthly_wage_bill: u64 = players
            .players
            .iter()
            .filter(|p| !p.is_on_loan())
            .filter_map(|p| p.contract.as_ref().map(|c| c.salary as u64 / 12))
            .sum();
        let payout_cap = (monthly_wage_bill / 2).max(5_000) as u32;

        const MAX_TERMINATIONS_PER_WEEK: usize = 2;
        let mut emitted = 0;

        for player in &players.players {
            if emitted >= MAX_TERMINATIONS_PER_WEEK {
                break;
            }
            if let Some((payout, reason)) = Self::evaluate_termination(player, date, payout_cap) {
                result.contract_terminations.push(ContractTermination {
                    player_id: player.id,
                    payout,
                    reason,
                });
                emitted += 1;
            }
        }
    }

    /// Decide whether this player's contract should be terminated today.
    /// Returns the payout and a short reason code; None means keep.
    fn evaluate_termination(
        player: &Player,
        date: NaiveDate,
        payout_cap: u32,
    ) -> Option<(u32, &'static str)> {
        if player.is_on_loan() {
            return None;
        }
        let contract = player.contract.as_ref()?;

        // Don't tear up an existing sale — they're leaving anyway.
        if contract.is_transfer_listed {
            // Listed but still sitting around after a while? Let the market
            // finish its job; termination is a last resort, not the first.
        }

        // Promising youngsters stay even when squad-status says NotNeeded.
        let age = DateUtils::age(player.birth_date, date);
        let ca = player.player_attributes.current_ability;
        let pa = player.player_attributes.potential_ability;
        let is_prospect = age <= 23 && pa > ca + 15;
        if is_prospect {
            return None;
        }

        // Any player the squad really needs stays.
        let unneeded = matches!(
            contract.squad_status,
            PlayerSquadStatus::NotNeeded | PlayerSquadStatus::NotYetSet
        );
        if !unneeded {
            return None;
        }

        let payout = contract.termination_cost(date);

        // Free release (youth / amateur / non-contract / expiring): always
        // fine. Paid buyouts only if below the club's comfort threshold.
        let reason = match contract.contract_type {
            ContractType::Youth => "term_reason_youth_surplus",
            ContractType::Amateur | ContractType::NonContract => "term_reason_free_release",
            _ => "term_reason_surplus_squad",
        };

        if payout == 0 || payout <= payout_cap {
            Some((payout, reason))
        } else {
            None
        }
    }

    /// Loan/playing-time talk with enhanced success logic.
    /// For LoanRequest: success depends heavily on player ambition, determination,
    /// and manager's man_management. Ambitious players are harder to convince to stay.
    fn conduct_loan_or_playing_time_talk(
        manager: &Staff,
        player: &Player,
        talk_type: ManagerTalkType,
    ) -> ManagerTalkResult {
        let man_management = manager.staff_attributes.mental.man_management as f32;
        let motivating = manager.staff_attributes.mental.motivating as f32;
        let professionalism = player.attributes.professionalism;
        let loyalty = player.attributes.loyalty;
        let ambition = player.attributes.ambition;
        let determination = player.skills.mental.determination;

        let relationship_bonus = player.relations.get_staff(manager.id)
            .map(|r| (r.level / 100.0) * 0.2)
            .unwrap_or(0.0);

        if talk_type == ManagerTalkType::LoanRequest {
            // For loan requests, "success" means the manager AGREES to loan the player.
            // High ambition/determination players are MORE convincing (harder to deny).
            // Good man_management coaches are more likely to agree to a sensible loan.
            let player_conviction = ambition / 20.0 * 0.4 + determination / 20.0 * 0.3
                + professionalism / 20.0 * 0.2 + 0.1;
            let coach_willingness = man_management / 20.0 * 0.5 + motivating / 20.0 * 0.3;

            // Base: 50% chance. Player conviction pushes it up, loyalty pulls it down.
            let success_chance = (0.50
                + player_conviction * 0.25
                + coach_willingness * 0.15
                - loyalty / 40.0  // loyal players are less insistent
                + relationship_bonus)
                .clamp(0.20, 0.90);

            let success = rand::random::<f32>() < success_chance;

            let (morale_change, rel_change) = if success {
                (5.0, 0.2)   // Player happy — loan agreed
            } else {
                // Denied loan — ambitious players take it harder
                let morale_hit = -3.0 - (ambition / 20.0) * 4.0; // -3 to -7
                (morale_hit, -0.15)
            };

            ManagerTalkResult {
                player_id: player.id,
                staff_id: manager.id,
                talk_type,
                success,
                morale_change,
                relationship_change: rel_change,
            }
        } else {
            // Standard playing time talk — use existing logic
            Self::conduct_manager_talk(manager, player, talk_type)
        }
    }

    fn calculate_playing_time_factor_for_complaint(player: &Player) -> f32 {
        let total = player.statistics.played + player.statistics.played_subs;
        if total < 5 {
            return 0.0;
        }

        let play_ratio = player.statistics.played as f32 / total as f32;

        let expected_ratio = if let Some(ref contract) = player.contract {
            match contract.squad_status {
                PlayerSquadStatus::KeyPlayer => 0.70,
                PlayerSquadStatus::FirstTeamRegular => 0.50,
                PlayerSquadStatus::FirstTeamSquadRotation => 0.25,
                PlayerSquadStatus::MainBackupPlayer => 0.20,
                PlayerSquadStatus::HotProspectForTheFuture => 0.10,
                PlayerSquadStatus::DecentYoungster => 0.10,
                PlayerSquadStatus::NotNeeded => 0.05,
                _ => 0.30,
            }
        } else {
            0.30
        };

        if play_ratio >= expected_ratio {
            0.0
        } else {
            let deficit = (expected_ratio - play_ratio) / expected_ratio.max(0.01);
            -deficit * 20.0
        }
    }

    // ========== CALCULATION HELPERS ==========

    fn calculate_daily_interaction_change(
        player_a: &Player,
        player_b: &Player,
        existing_relationship: &PlayerRelation,
        _ctx: &GlobalContext<'_>,
    ) -> f32 {
        let relationship_level = existing_relationship.level;

        let temperament_factor =
            (player_a.attributes.temperament + player_b.attributes.temperament) / 40.0;
        let base_change = FloatUtils::random(-0.02, 0.02) * temperament_factor;

        let trust_factor = existing_relationship.trust / 100.0;
        let friendship_factor = existing_relationship.friendship / 100.0;

        if relationship_level > 50.0 {
            let stability_bonus = (trust_factor * 0.3 + friendship_factor * 0.2) * base_change;
            base_change * 0.5 + stability_bonus
        } else if relationship_level < -50.0 {
            base_change + 0.01 * (1.0 - trust_factor)
        } else {
            let professional_factor = existing_relationship.professional_respect / 100.0;
            base_change * (1.0 - professional_factor * 0.3)
        }
    }

    fn calculate_mood_spread(
        unhappy_player: &Player,
        other_player: &Player,
        happiness: f32,
    ) -> f32 {
        // Unhappy players with high leadership or reputation spread negativity more
        let leadership_influence = unhappy_player.skills.mental.leadership / 20.0;
        let rep_influence =
            (unhappy_player.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let influence = ((leadership_influence + rep_influence) / 2.0) * happiness.abs() * 0.1;

        // Players with high professionalism resist negative influence
        let resistance = other_player.attributes.professionalism / 20.0;

        // Return negative: mood spread from unhappy players damages relationships
        -(influence * (1.0 - resistance).max(0.0))
    }

    /// Returns (a_toward_b, b_toward_a) jealousy values.
    /// The lower-paid player feels jealousy (negative); the higher-paid player is unaffected.
    fn calculate_contract_jealousy(player_a: &Player, player_b: &Player) -> (f32, f32) {
        let salary_a = player_a.contract.as_ref().map(|c| c.salary).unwrap_or(0);
        let salary_b = player_b.contract.as_ref().map(|c| c.salary).unwrap_or(0);

        if salary_a == 0 || salary_b == 0 {
            return (0.0, 0.0);
        }

        let salary_ratio = salary_a as f32 / salary_b as f32;

        if salary_ratio > 2.0 || salary_ratio < 0.5 {
            let rep_a = player_a.player_attributes.current_reputation as f32;
            let rep_b = player_b.player_attributes.current_reputation as f32;

            if salary_a > salary_b {
                // A earns more — B feels jealousy toward A
                // Jealousy is reduced if A also has higher reputation (justified pay)
                let rep_alignment = if rep_a > rep_b { 0.5 } else { 1.5 };
                let jealousy = -0.08 * (player_b.attributes.ambition / 20.0) * rep_alignment;
                (0.0, jealousy)
            } else {
                // B earns more — A feels jealousy toward B
                let rep_alignment = if rep_b > rep_a { 0.5 } else { 1.5 };
                let jealousy = -0.08 * (player_a.attributes.ambition / 20.0) * rep_alignment;
                (jealousy, 0.0)
            }
        } else {
            (0.0, 0.0)
        }
    }

    fn calculate_injury_sympathy(_injured_player: &Player, other_player: &Player) -> f32 {
        let empathy = other_player.attributes.sportsmanship / 20.0;
        let team_spirit = other_player.skills.mental.teamwork / 20.0;

        (empathy + team_spirit) * 0.08
    }

    fn calculate_national_team_bond(player_a: &Player, player_b: &Player) -> f32 {
        let int_experience_a =
            (player_a.player_attributes.international_apps as f32 / 50.0).min(1.0);
        let int_experience_b =
            (player_b.player_attributes.international_apps as f32 / 50.0).min(1.0);

        // Reputation similarity among compatriots strengthens bonds
        let rep_a = player_a.player_attributes.current_reputation as f32;
        let rep_b = player_b.player_attributes.current_reputation as f32;
        let rep_similarity = 1.0 - ((rep_a - rep_b).abs() / 10000.0).clamp(0.0, 1.0);

        (int_experience_a + int_experience_b) * 0.04 * (0.7 + 0.3 * rep_similarity)
    }

    fn calculate_player_happiness(player: &Player) -> f32 {
        let mut happiness = 0.0;

        // Contract satisfaction - high reputation players have higher expectations
        let rep_expectation =
            (player.player_attributes.current_reputation as f32 / 5000.0).clamp(0.5, 2.0);

        happiness += player
            .contract
            .as_ref()
            .map(|c| (c.salary as f32 / (10000.0 * rep_expectation)).min(1.0))
            .unwrap_or(-0.5);

        // Playing time satisfaction - star players expect to start
        if player.statistics.played > 20 {
            happiness += 0.3;
        } else if player.statistics.played > 10 {
            happiness += 0.1;
        } else {
            // High rep players get more upset about not playing
            happiness -= 0.2 * (1.0 + (rep_expectation - 1.0) * 0.5);
        }

        // Performance satisfaction
        let goals_ratio =
            player.statistics.goals as f32 / player.statistics.played.max(1) as f32;
        if player.position().is_forward() && goals_ratio > 0.5 {
            happiness += 0.2;
        } else if !player.position().is_forward() && goals_ratio > 0.3 {
            happiness += 0.15;
        }

        // Personality factors
        happiness += (player.attributes.professionalism - 10.0) / 100.0;
        happiness -= (player.attributes.controversy - 10.0) / 50.0;

        // Behavior state
        match player.behaviour.state {
            PersonBehaviourState::Good => happiness += 0.2,
            PersonBehaviourState::Poor => happiness -= 0.3,
            PersonBehaviourState::Normal => {}
        }

        happiness.clamp(-1.0, 1.0)
    }

    fn calculate_competition_factor(player_a: &Player, player_b: &Player) -> f32 {
        let ability_diff = (player_a.player_attributes.current_ability as f32
            - player_b.player_attributes.current_ability as f32)
            .abs();

        // Similar abilities = more competition
        let competition_base = 0.3 - (ability_diff / 100.0);

        // Ambition increases competition
        let ambition_factor =
            (player_a.attributes.ambition + player_b.attributes.ambition) / 40.0;

        // Reputation amplifies competition: both high-rep players fight harder for spots
        let rep_a =
            (player_a.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let rep_b =
            (player_b.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let rep_factor = 1.0 + (rep_a + rep_b) * 0.25;

        (competition_base * ambition_factor * rep_factor).clamp(0.0, 0.5)
    }

    fn calculate_synergy_factor(player_a: &Player, player_b: &Player) -> f32 {
        let teamwork_factor =
            (player_a.skills.mental.teamwork + player_b.skills.mental.teamwork) / 40.0;
        let professionalism_factor =
            (player_a.attributes.professionalism + player_b.attributes.professionalism) / 40.0;

        // Higher combined reputation means higher-quality partnership
        let rep_a =
            (player_a.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let rep_b =
            (player_b.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let rep_bonus = 1.0 + (rep_a + rep_b) * 0.15;

        (teamwork_factor * professionalism_factor * 0.2 * rep_bonus).min(0.3)
    }

    fn are_complementary_positions(pos_a: &PlayerPositionType, pos_b: &PlayerPositionType) -> bool {
        use PlayerPositionType::*;

        match (pos_a, pos_b) {
            (
                DefenderCenter | DefenderLeft | DefenderRight,
                MidfielderCenter | MidfielderLeft | MidfielderRight | DefensiveMidfielder,
            ) => true,
            (
                MidfielderCenter | MidfielderLeft | MidfielderRight | AttackingMidfielderCenter,
                Striker | ForwardLeft | ForwardRight | ForwardCenter,
            ) => true,
            (
                MidfielderCenter | MidfielderLeft | MidfielderRight | DefensiveMidfielder,
                DefenderCenter | DefenderLeft | DefenderRight,
            ) => true,
            (
                Striker | ForwardLeft | ForwardRight | ForwardCenter,
                MidfielderCenter | MidfielderLeft | MidfielderRight | AttackingMidfielderCenter,
            ) => true,
            _ => false,
        }
    }

    fn calculate_age_relationship_factor(age_a: u8, age_b: u8, age_diff: i32) -> f32 {
        match (age_a, age_b) {
            // Both young (16-22) - natural bonding
            (16..=22, 16..=22) if age_diff <= 3 => FloatUtils::random(0.1, 0.25),

            // Young and experienced (30+) - mentorship potential
            (16..=22, 30..) | (30.., 16..=22) => FloatUtils::random(-0.05, 0.2),

            // Prime age players (23-29) - competitive tension
            (23..=29, 23..=29) if age_diff <= 2 => FloatUtils::random(-0.1, 0.1),

            // Large age gaps - respect or indifference
            _ if age_diff > 8 => FloatUtils::random(-0.1, 0.1),

            // Similar ages in general - slight positive
            _ if age_diff <= 2 => FloatUtils::random(0.0, 0.1),

            _ => 0.0,
        }
    }

    fn calculate_player_performance_rating(player: &Player) -> f32 {
        let goals_factor =
            (player.statistics.goals as f32 / (player.statistics.played.max(1) as f32)) * 10.0;
        let assists_factor =
            (player.statistics.assists as f32 / (player.statistics.played.max(1) as f32)) * 5.0;
        let appearance_factor = (player.statistics.played as f32 / 30.0).min(1.0) * 5.0;
        let rating_factor = player.statistics.average_rating;

        // Factor in reputation: a high-reputation player who performs poorly stands out
        let rep_factor =
            (player.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let rep_adjustment = rep_factor * 2.0;

        (goals_factor + assists_factor + appearance_factor + rating_factor + rep_adjustment) / 5.0
    }

    fn calculate_performance_relationship_factor(
        perf_a: f32,
        perf_b: f32,
        diff: f32,
        player_a: &Player,
        player_b: &Player,
    ) -> f32 {
        if diff < 1.0 {
            // Similar performance - mutual respect
            FloatUtils::random(0.05, 0.15)
        } else if diff > 3.0 {
            // Large performance gap
            let higher_rep = player_a
                .player_attributes
                .current_reputation
                .max(player_b.player_attributes.current_reputation)
                as f32;
            let rep_scale = (higher_rep / 10000.0).clamp(0.1, 1.0);

            if perf_a > perf_b {
                // Higher performer: professional players give credit, ambitious ones resent
                let sportsmanship_a =
                    (player_a.attributes.sportsmanship / 20.0).clamp(0.0, 1.0);
                FloatUtils::random(-0.1, 0.05) * (1.0 + sportsmanship_a * 0.3) * rep_scale
            } else {
                FloatUtils::random(-0.12, 0.08) * rep_scale
            }
        } else {
            0.0
        }
    }

    fn calculate_personality_conflict(player_a: &Player, player_b: &Player) -> f32 {
        // High controversy players clash with professional players
        let controversy_clash = if player_a.attributes.controversy > 15.0
            && player_b.attributes.professionalism > 15.0
            || player_b.attributes.controversy > 15.0
                && player_a.attributes.professionalism > 15.0
        {
            -0.25
        } else {
            0.0
        };

        // High temperament players clash
        let temperament_clash =
            if player_a.attributes.temperament > 18.0 && player_b.attributes.temperament > 18.0 {
                FloatUtils::random(-0.15, -0.03)
            } else {
                0.0
            };

        // Different behavioral states cause friction
        let behavior_clash = match (&player_a.behaviour.state, &player_b.behaviour.state) {
            (PersonBehaviourState::Poor, PersonBehaviourState::Good)
            | (PersonBehaviourState::Good, PersonBehaviourState::Poor) => -0.12,
            _ => 0.0,
        };

        // Mutual loyalty and professionalism create bonds
        let positive_traits =
            if player_a.attributes.loyalty > 15.0 && player_b.attributes.loyalty > 15.0 {
                0.08
            } else {
                0.0
            };

        // Mutual sportsmanship creates bonds
        let sportsmanship_bond = if player_a.attributes.sportsmanship > 14.0
            && player_b.attributes.sportsmanship > 14.0
        {
            0.05
        } else {
            0.0
        };

        controversy_clash + temperament_clash + behavior_clash + positive_traits + sportsmanship_bond
    }

    fn calculate_leadership_influence(leader: &Player, player: &Player) -> f32 {
        let leadership_strength = leader.skills.mental.leadership / 20.0;

        // Reputation amplifies leadership: respected players are listened to more
        let rep_boost =
            (leader.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let effective_leadership = leadership_strength * (1.0 + rep_boost * 0.5);

        let influence = match player.behaviour.state {
            PersonBehaviourState::Good => effective_leadership * 0.15,
            PersonBehaviourState::Normal => effective_leadership * 0.08,
            PersonBehaviourState::Poor => {
                if player.attributes.professionalism > 10.0 {
                    effective_leadership * 0.12
                } else {
                    -effective_leadership * 0.08
                }
            }
        };

        influence
    }

    fn calculate_playing_time_jealousy(
        time_a: u16,
        time_b: u16,
        player_a: &Player,
        player_b: &Player,
    ) -> f32 {
        let time_diff = (time_a as i32 - time_b as i32).abs();

        if time_diff < 3 {
            return FloatUtils::random(0.03, 0.1);
        }

        if time_diff > 10 {
            let ambition_factor =
                (player_a.attributes.ambition + player_b.attributes.ambition) / 40.0;

            // High reputation players who don't play feel it more acutely
            let rep_a =
                (player_a.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
            let rep_b =
                (player_b.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);

            if time_a < time_b && player_a.attributes.ambition > 15.0 {
                return -0.15 * ambition_factor * (1.0 + rep_a * 0.3);
            } else if time_b < time_a && player_b.attributes.ambition > 15.0 {
                return -0.15 * ambition_factor * (1.0 + rep_b * 0.3);
            }
        }

        0.0
    }
}
