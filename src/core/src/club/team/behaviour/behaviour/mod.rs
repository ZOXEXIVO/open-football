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
use chrono::NaiveDateTime;
use chrono::Weekday;
use log::debug;

mod calculations;
pub(crate) mod conflict_escalation;
mod discipline;
mod dynamics;
mod hierarchy;
mod interactions;
mod leadership;
mod manager_credibility;
mod manager_talks;
mod morale;
mod partnerships;
mod relationships;
mod training_direction;

#[derive(Debug, Clone)]
pub struct TeamBehaviour {
    last_full_update: Option<NaiveDateTime>,
    last_minor_update: Option<NaiveDateTime>,
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

    fn should_run_full_update(&self, current_time: NaiveDateTime) -> bool {
        match self.last_full_update {
            None => true,
            Some(last) => {
                let days_since = current_time.signed_duration_since(last).num_days();
                days_since >= 7
                    || (days_since >= 1
                        && (current_time.weekday() == Weekday::Sat
                            || current_time.weekday() == Weekday::Sun
                            || current_time.day() == 1))
            }
        }
    }

    fn should_run_minor_update(&self, current_time: NaiveDateTime) -> bool {
        match self.last_minor_update {
            None => true,
            Some(last) => {
                let days_since = current_time.signed_duration_since(last).num_days();
                days_since >= 2
                    || (days_since >= 1
                        && matches!(
                            current_time.weekday(),
                            Weekday::Tue | Weekday::Wed | Weekday::Thu
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

        // Unit-level partnership chemistry (CB pairs, FB+W flank,
        // DM-CM, CM-AM, W-ST, GK-CB) plus same-position rivalry and
        // language/nationality bonds. Runs after the broader passes
        // so its emit can tilt totals without being swamped.
        Self::process_unit_partnerships(players, &mut result, &ctx);

        // Senior-leader influence on youth + controversial-star clique
        // tension. Captain mediation already runs below; these are the
        // slower drifts that build between teammates around the leadership
        // group rather than acts the captain directly performs.
        Self::process_dressing_room_hierarchy(players, &mut result, &ctx);

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

        // Monthly loan-development audit: a separate concern from raw
        // minutes — a young loanee can be benched, misused, at the wrong
        // level, or in a weak training setup. Aggregates those signals
        // into a development warning distinct from the minutes concern.
        Self::process_loan_development_audit(players, &ctx);

        // Monthly squad-ambition audit: ambitious stars far above their
        // squad's level (or whose key teammates were sold) push the board
        // to strengthen before committing their future.
        Self::process_squad_ambition_audit(players, &ctx);

        // Monthly title-ambition audit: elite stars at a club off the
        // title pace want a genuine challenger. Reads league-table context
        // from `ctx.club`.
        Self::process_title_ambition_audit(players, &ctx);

        // Monthly reserve-ambition audit: senior players parked in a
        // B / Reserve / Second squad dream of genuine first-team football.
        // The weekly complaint pass below escalates the lingering mood
        // into a loan / transfer request.
        Self::process_reserve_ambition_audit(players, &ctx);

        // Monthly perennial-backup audit: the main squad's mirror case —
        // a settled career backup (or serial loanee) whose ambition and
        // closing career window outweigh the comforts of the bench dreams
        // of being a regular somewhere else, possibly a weaker club.
        Self::process_perennial_backup_audit(players, &ctx);

        // Monthly loanee-permanence audit: a loanee thriving at the
        // borrowing club wants the move made permanent instead of a
        // return to the parent's fringe.
        Self::process_loanee_permanence_audit(players, &ctx);

        // Monthly contract-horizon audit: final-year seniors with no
        // renewal talks on record — anxiety for most, a shop-window
        // drive for the in-form.
        Self::process_contract_horizon_audit(players, &ctx);

        // Weekly disciplinary pass: fresh misconduct draws the formal
        // club response — warning first, wage fines for repeat
        // offenders — instead of ending at the mood event.
        Self::process_disciplinary_actions(players, staffs, &mut result, &ctx);

        // Monthly training-direction pass: the coach progresses and
        // assigns personal development plans — retraining toward thin
        // groups, fitness blocks for the injury-prone.
        Self::process_training_direction(players, staffs, &ctx);

        // Monthly veteran career-stage audit: older players whose role has
        // faded weigh up retirement; veteran leaders signal coaching
        // interest. Informational late-career colour.
        Self::process_veteran_career_stage_audit(players, &ctx);

        // Monthly contract-stalemate audit: surface the player-facing
        // "talks have stalled" signal once a renewal has genuinely broken
        // down. Mirrors the listing pipeline's stalemate assessment.
        Self::process_contract_stalemate_audit(players, &ctx);

        // Background manager-trust drift — runs before the talk
        // picker so per-talk modifiers (rapport, credibility) read the
        // post-drift values rather than last week's snapshot.
        Self::process_manager_relationship_context(players, staffs, &mut result, &ctx);

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

        // Living conflict escalation: persistent high-bond friction
        // crosses from "the coach noticed" into player-driven incidents
        // — private complaints, Unhappy status, transfer-request risk,
        // public criticism. Runs after the talks pass so the streak
        // reads the post-talk bond reflecting any successful chat.
        Self::process_conflict_escalation(players, staffs, &mut result, &ctx);

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
