//! Team-behaviour simulation, split by concern. The `TeamBehaviour`
//! struct lives here along with the cadence gate (`should_run_*`) and
//! the two top-level orchestration passes; every individual `process_*`
//! step has been moved to a sibling module so each concern stays small
//! and self-explanatory:
//!
//! | Submodule        | Concern                                                    |
//! |------------------|------------------------------------------------------------|
//! | [`interactions`] | Daily ticks: micro-relationship changes, mood spread, squad integration, recent-form reactions |
//! | [`dynamics`]     | Position / age / performance / personality cross-pair effects |
//! | [`morale`]       | Contract jealousy, loan playing-time audit, controversy, periodic wage envy |
//! | [`leadership`]   | Captain mediation, captain morale propagation, leadership influence, playing-time jealousy |
//! | [`relationships`]| Reputation, mentorship, contract satisfaction, injury sympathy, international-duty bonds |
//! | [`manager_talks`]| Weekly manager talks, playing-time complaints, coach-driven contract terminations + tone picker |
//! | [`calculations`] | Shared `calculate_*` helpers consumed across the steps     |

use crate::club::team::behaviour::TeamBehaviourResult;
use crate::context::GlobalContext;
use crate::{PlayerCollection, StaffCollection};
use chrono::Datelike;
use log::debug;

mod calculations;
mod dynamics;
mod interactions;
mod leadership;
mod manager_talks;
mod morale;
mod relationships;

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

        // Captain & senior leaders mediate dressing-room conflicts.
        // Where two teammates have a strongly negative relationship and
        // a high-leadership / high-professionalism captain is present,
        // the friction softens. The opposite case — a controversial
        // captain — is handled by the general mood-spread already.
        Self::process_captain_mediation(players, &mut result);

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

        // Manager-player talks (weekly during full update). Pass the
        // simulation date so the per-(player, topic) cooldown gate
        // actually fires; without a date the gate is a permissive no-op.
        Self::process_manager_player_talks_dated(
            players,
            staffs,
            &mut result,
            Some(ctx.simulation.date.date()),
        );

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
}
