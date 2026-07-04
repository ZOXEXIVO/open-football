use crate::club::staff::{CoachMatchSnapshot, CoachProfile, CoachStrategy};
use crate::club::team::MatchdayLeadership;
use crate::r#match::squad::{CoachStrategyForSelection, PlayerSelectionResult};
use crate::r#match::{MatchPlayer, MatchSquad, SelectionContext, SquadSelector};
use crate::{Player, Staff, Tactics, TacticsSelector, Team};
use chrono::NaiveDate;
use std::cmp::Ordering;
use std::collections::HashMap;

impl Team {
    /// Get match squad using rotation — prioritizes players who haven't played recently.
    /// Used for friendly/development leagues where all players need game time.
    /// `date` drives the matchday-armband age read (a 20-year-old shouldn't
    /// captain over a 28-year-old even on a rotated rotation/development XI).
    pub fn get_rotation_match_squad_at(&self, date: NaiveDate) -> MatchSquad {
        let head_coach = self.staffs.head_coach();

        let squad_result = SquadSelector::select_for_rotation(self, head_coach);

        let final_tactics = self
            .tactics
            .as_ref()
            .cloned()
            .unwrap_or_else(|| TacticsSelector::select(self, head_coach));

        self.validate_squad_selection(&squad_result, &final_tactics);

        let (captain_id, vice_captain_id) = MatchdayLeadership::from_match_squad_at(
            self.captain_id,
            self.vice_captain_id,
            &squad_result.main_squad,
            date,
        );

        let coach_snapshot = MatchCoachSnapshot::for_rotation(head_coach);
        let penalty_taker_id = self.select_penalty_taker(&squad_result.main_squad);
        let free_kick_taker_id = self.select_free_kick_taker(&squad_result.main_squad);

        MatchSquad {
            team_id: self.id,
            team_name: self.name.clone(),
            tactics: final_tactics,
            main_squad: squad_result.main_squad,
            substitutes: squad_result.substitutes,
            captain_id,
            vice_captain_id,
            penalty_taker_id,
            free_kick_taker_id,
            selection_omissions: squad_result.omissions,
            coach_snapshot,
        }
    }

    /// Get match squad using rotation with supplementary players from other club teams.
    /// Ensures non-main teams always have enough players for a full squad.
    pub fn get_rotation_match_squad_with_reserves(
        &self,
        reserve_players: &[&Player],
        ctx: &SelectionContext,
    ) -> MatchSquad {
        let head_coach = self.staffs.head_coach();

        let squad_result =
            SquadSelector::select_for_rotation_with_context(self, head_coach, reserve_players, ctx);

        let final_tactics = self
            .tactics
            .as_ref()
            .cloned()
            .unwrap_or_else(|| TacticsSelector::select(self, head_coach));

        self.validate_squad_selection(&squad_result, &final_tactics);

        let (captain_id, vice_captain_id) = MatchdayLeadership::from_match_squad_at(
            self.captain_id,
            self.vice_captain_id,
            &squad_result.main_squad,
            ctx.date,
        );

        let coach_snapshot = MatchCoachSnapshot::for_selection_context(head_coach, ctx);
        let penalty_taker_id = self.select_penalty_taker(&squad_result.main_squad);
        let free_kick_taker_id = self.select_free_kick_taker(&squad_result.main_squad);

        MatchSquad {
            team_id: self.id,
            team_name: self.name.clone(),
            tactics: final_tactics,
            main_squad: squad_result.main_squad,
            substitutes: squad_result.substitutes,
            captain_id,
            vice_captain_id,
            penalty_taker_id,
            free_kick_taker_id,
            selection_omissions: squad_result.omissions,
            coach_snapshot,
        }
    }

    /// Enhanced get_match_squad that uses improved tactical analysis
    /// Accepts optional reserve players that can be selected for the match squad
    pub fn get_enhanced_match_squad(
        &self,
        reserve_players: &[&Player],
        ctx: &SelectionContext,
    ) -> MatchSquad {
        let head_coach = self.staffs.head_coach();

        // Pick the match tactic before selecting players. Otherwise the XI
        // can be built for the default shape and then validated against a
        // late opponent-aware counter shape.
        let head_coach_tac = head_coach.staff_attributes.knowledge.tactical_knowledge;
        let final_tactics = if let (Some(opp), true) = (ctx.opponent_tactic, head_coach_tac >= 14) {
            let roster: Vec<&Player> = self.players.players();
            TacticsSelector::select_counter_tactic(&opp, &roster)
        } else {
            self.tactics
                .as_ref()
                .cloned()
                .unwrap_or_else(|| TacticsSelector::select(self, head_coach))
        };

        // Use squad selection with reserve pool for the final match tactic.
        let squad_result = SquadSelector::select_with_tactics_context(
            self,
            head_coach,
            reserve_players,
            &final_tactics,
            ctx,
        );

        // Step 5: Validate squad selection
        self.validate_squad_selection(&squad_result, &final_tactics);

        let (captain_id, vice_captain_id) = MatchdayLeadership::from_match_squad_at(
            self.captain_id,
            self.vice_captain_id,
            &squad_result.main_squad,
            ctx.date,
        );

        let coach_snapshot = MatchCoachSnapshot::for_selection_context(head_coach, ctx);
        let penalty_taker_id = self.select_penalty_taker(&squad_result.main_squad);
        let free_kick_taker_id = self.select_free_kick_taker(&squad_result.main_squad);

        MatchSquad {
            team_id: self.id,
            team_name: self.name.clone(),
            tactics: final_tactics,
            main_squad: squad_result.main_squad,
            substitutes: squad_result.substitutes,
            captain_id,
            vice_captain_id,
            penalty_taker_id,
            free_kick_taker_id,
            selection_omissions: squad_result.omissions,
            coach_snapshot,
        }
    }

    fn validate_squad_selection(&self, squad_result: &PlayerSelectionResult, tactics: &Tactics) {
        let formation_positions = tactics.positions();

        // A short XI is a real problem — the match engine will field an
        // incomplete side — so log it loudly with enough context to trace the
        // offending team. Never panic, though: a degraded squad is still better
        // than aborting the whole simulation tick, and the selector has already
        // exhausted its emergency fallbacks by this point.
        if squad_result.main_squad.len() < formation_positions.len() {
            let selected_ids: Vec<u32> = squad_result.main_squad.iter().map(|p| p.id).collect();
            log::debug!(
                "Squad too small for team {} ({}): selected {} players for {} formation positions; selected ids {:?}",
                self.id,
                self.name,
                squad_result.main_squad.len(),
                formation_positions.len(),
                selected_ids
            );
        } else if squad_result.main_squad.len() != formation_positions.len() {
            log::debug!(
                "Squad size mismatch for team {} ({}): got {} players for {} positions",
                self.id,
                self.name,
                squad_result.main_squad.len(),
                formation_positions.len()
            );
        }

        let mut position_coverage = HashMap::new();
        for match_player in &squad_result.main_squad {
            let pos = match_player.tactical_position.current_position;
            *position_coverage.entry(pos).or_insert(0) += 1;
        }

        for &required_pos in formation_positions {
            if !position_coverage.contains_key(&required_pos) {
                log::debug!(
                    "No player selected for required position: {}",
                    required_pos.get_short_name()
                );
            }
        }
    }

    // Matchday captaincy now lives in `MatchdayLeadership::from_match_squad_at`,
    // which resolves the armband over the *selected* XI (honouring the
    // persistent club hierarchy on `Team.captain_id` / `vice_captain_id`)
    // rather than scanning the whole roster. See `squad_life/matchday_leadership.rs`.

    /// Select penalty taker from the starting XI — the designated taker
    /// has to actually be on the pitch at kickoff, not at home in the
    /// stands. Ranked by penalty taking + composure.
    fn select_penalty_taker(&self, main_squad: &[MatchPlayer]) -> Option<MatchPlayer> {
        main_squad
            .iter()
            .max_by(|a, b| {
                let penalty_skill_a = a.skills.technical.penalty_taking + a.skills.mental.composure;
                let penalty_skill_b = b.skills.technical.penalty_taking + b.skills.mental.composure;

                penalty_skill_a
                    .partial_cmp(&penalty_skill_b)
                    .unwrap_or(Ordering::Equal)
            })
            .cloned()
    }

    /// Select free-kick taker from the starting XI, ranked by free
    /// kicks + technique.
    fn select_free_kick_taker(&self, main_squad: &[MatchPlayer]) -> Option<MatchPlayer> {
        main_squad
            .iter()
            .max_by(|a, b| {
                let fk_skill_a = a.skills.technical.free_kicks + a.skills.technical.technique;
                let fk_skill_b = b.skills.technical.free_kicks + b.skills.technical.technique;

                fk_skill_a
                    .partial_cmp(&fk_skill_b)
                    .unwrap_or(Ordering::Equal)
            })
            .cloned()
    }
}

/// Stateless namespace owning the [`CoachMatchSnapshot`] construction
/// used by the three [`MatchSquad`] builders above. Bundles the
/// memory-clone + profile-derivation + strategy-derivation in one
/// place so the build sites read declaratively and the strategy rule
/// stays consistent with the selection layer's read.
struct MatchCoachSnapshot;

impl MatchCoachSnapshot {
    /// Build a snapshot for a rotation / dev-league fixture — match
    /// importance is implicitly low, philosophy is irrelevant. The
    /// strategy collapses to [`CoachStrategy::DevelopYouth`].
    fn for_rotation(head_coach: &Staff) -> Option<CoachMatchSnapshot> {
        if head_coach.id == 0 {
            return None;
        }
        let profile = CoachProfile::from_staff(head_coach);
        let strategy = CoachStrategy::DevelopYouth;
        Some(CoachMatchSnapshot::new(
            head_coach.coach_memory.clone(),
            profile,
            strategy,
        ))
    }

    /// Build a snapshot for a competitive fixture. The strategy is
    /// derived through [`CoachStrategyForSelection`] — the same
    /// context-to-inputs map the selection layer reads — so a UI
    /// showing "manager strategy" and the match engine's in-flight
    /// reads agree on the same call, including the fixture game
    /// model's derby / opponent-strength / squad-depth signals.
    fn for_selection_context(
        head_coach: &Staff,
        ctx: &SelectionContext,
    ) -> Option<CoachMatchSnapshot> {
        if head_coach.id == 0 {
            return None;
        }
        let profile = CoachProfile::from_staff(head_coach);
        let strategy = CoachStrategyForSelection::derive(&profile, ctx);
        Some(CoachMatchSnapshot::new(
            head_coach.coach_memory.clone(),
            profile,
            strategy,
        ))
    }
}
