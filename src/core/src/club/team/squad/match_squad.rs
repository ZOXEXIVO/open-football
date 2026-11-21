use crate::club::staff::{
    CoachMatchSnapshot, CoachPlayerBond, CoachProfile, CoachStrategy, PlayerMatchIntent,
};
use crate::club::team::MatchdayLeadership;
use crate::r#match::squad::{CoachStrategyForSelection, PlayerSelectionResult};
use crate::r#match::{CoachCard, MatchPlayer, MatchSquad, SelectionContext, SquadSelector};
use crate::{FormationFamiliarity, MatchTacticType, Player, Staff, Tactics, TacticsSelector, Team};
use chrono::NaiveDate;
use rustc_hash::FxHashMap;
use std::cmp::Ordering;

impl Team {
    /// Get match squad using rotation — prioritizes players who haven't played recently.
    /// Used for friendly/development leagues where all players need game time.
    /// `date` drives the matchday-armband age read (a 20-year-old shouldn't
    /// captain over a 28-year-old even on a rotated rotation/development XI).
    pub fn get_rotation_match_squad_at(&self, date: NaiveDate) -> MatchSquad {
        let head_coach = self.staffs.head_coach();

        let mut squad_result = SquadSelector::select_for_rotation(self, head_coach);

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

        let coach_snapshot = MatchCoachSnapshot::for_rotation(head_coach, date);
        self.stamp_buy_in(&mut squad_result, &[], head_coach, date);
        self.stamp_partnerships(&mut squad_result);
        let familiarity = self.familiarity(final_tactics.tactic_type, &squad_result);
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
            overlooked: squad_result.overlooked,
            selection: Default::default(),
            coach_snapshot,
            coach_card: CoachCard::of_head_coach(head_coach),
            familiarity: Some(familiarity),
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

        let mut squad_result =
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

        let coach_snapshot = MatchCoachSnapshot::for_selection_context(
            head_coach,
            ctx,
            self.players.iter().chain(reserve_players.iter().copied()),
        );
        self.stamp_buy_in(&mut squad_result, reserve_players, head_coach, ctx.date);
        self.stamp_partnerships(&mut squad_result);
        let familiarity = self.familiarity(final_tactics.tactic_type, &squad_result);
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
            overlooked: squad_result.overlooked,
            selection: ctx.reading(),
            coach_snapshot,
            coach_card: CoachCard::of_head_coach(head_coach),
            familiarity: Some(familiarity),
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

        // The match is played in the side's own system; the plan answers
        // the opponent.
        let final_tactics = self
            .tactics
            .as_ref()
            .cloned()
            .unwrap_or_else(|| TacticsSelector::select(self, head_coach));

        // Use squad selection with reserve pool for the final match tactic.
        let mut squad_result = SquadSelector::select_with_tactics_context(
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

        let coach_snapshot = MatchCoachSnapshot::for_selection_context(
            head_coach,
            ctx,
            self.players.iter().chain(reserve_players.iter().copied()),
        );
        self.stamp_buy_in(&mut squad_result, reserve_players, head_coach, ctx.date);
        self.stamp_partnerships(&mut squad_result);
        let familiarity = self.familiarity(final_tactics.tactic_type, &squad_result);
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
            overlooked: squad_result.overlooked,
            selection: ctx.reading(),
            coach_snapshot,
            coach_card: CoachCard::of_head_coach(head_coach),
            familiarity: Some(familiarity),
        }
    }

    /// How much each man the selector named believes in `coach`'s plan.
    fn stamp_buy_in(
        &self,
        squad: &mut PlayerSelectionResult,
        reserves: &[&Player],
        coach: &Staff,
        date: NaiveDate,
    ) {
        for picked in squad
            .main_squad
            .iter_mut()
            .chain(squad.substitutes.iter_mut())
        {
            let player = self
                .players
                .find(picked.id)
                .or_else(|| reserves.iter().copied().find(|p| p.id == picked.id));
            if let Some(player) = player {
                picked.buy_in = CoachPlayerBond::build(player, coach, date).tactical_buy_in;
            }
        }
    }

    /// The team-mates each man named has started alongside, from the
    /// side's record.
    fn stamp_partnerships(&self, squad: &mut PlayerSelectionResult) {
        for picked in squad
            .main_squad
            .iter_mut()
            .chain(squad.substitutes.iter_mut())
        {
            picked.started_with = self.match_history.started_with(picked.id);
        }
    }

    /// The side's familiarity with each formation, for the eleven named.
    fn familiarity(
        &self,
        shape: MatchTacticType,
        squad: &PlayerSelectionResult,
    ) -> FormationFamiliarity {
        let eleven: Vec<u32> = squad.main_squad.iter().map(|p| p.id).collect();
        self.match_history.formation_familiarity(shape, &eleven)
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

        let mut position_coverage = FxHashMap::default();
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
    fn select_penalty_taker(&self, main_squad: &[MatchPlayer]) -> Option<u32> {
        main_squad
            .iter()
            .max_by(|a, b| {
                let penalty_skill_a = a.skills.technical.penalty_taking + a.skills.mental.composure;
                let penalty_skill_b = b.skills.technical.penalty_taking + b.skills.mental.composure;

                penalty_skill_a
                    .partial_cmp(&penalty_skill_b)
                    .unwrap_or(Ordering::Equal)
            })
            .map(|p| p.id)
    }

    /// Select free-kick taker from the starting XI, ranked by free
    /// kicks + technique.
    fn select_free_kick_taker(&self, main_squad: &[MatchPlayer]) -> Option<u32> {
        main_squad
            .iter()
            .max_by(|a, b| {
                let fk_skill_a = a.skills.technical.free_kicks + a.skills.technical.technique;
                let fk_skill_b = b.skills.technical.free_kicks + b.skills.technical.technique;

                fk_skill_a
                    .partial_cmp(&fk_skill_b)
                    .unwrap_or(Ordering::Equal)
            })
            .map(|p| p.id)
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
    fn for_rotation(head_coach: &Staff, date: NaiveDate) -> Option<CoachMatchSnapshot> {
        if head_coach.id == 0 {
            return None;
        }
        let profile = CoachProfile::from_staff(head_coach);
        let strategy = CoachStrategy::DevelopYouth;
        Some(CoachMatchSnapshot::new(
            head_coach.coach_memory.clone(),
            profile,
            strategy,
            date,
        ))
    }

    /// Build a snapshot for a competitive fixture. The strategy is
    /// derived through [`CoachStrategyForSelection`] — the same
    /// context-to-inputs map the selection layer reads — so a UI
    /// showing "manager strategy" and the match engine's in-flight
    /// reads agree on the same call, including the fixture game
    /// model's derby / opponent-strength / squad-depth signals.
    fn for_selection_context<'a>(
        head_coach: &Staff,
        ctx: &SelectionContext,
        players: impl Iterator<Item = &'a Player>,
    ) -> Option<CoachMatchSnapshot> {
        if head_coach.id == 0 {
            return None;
        }
        let profile = CoachProfile::from_staff(head_coach);
        let strategy = CoachStrategyForSelection::derive(&profile, ctx);
        let mut snapshot =
            CoachMatchSnapshot::new(head_coach.coach_memory.clone(), profile, strategy, ctx.date);
        snapshot.player_intents = players
            .map(|p| {
                (
                    p.id,
                    PlayerMatchIntent::assess(p, head_coach, ctx.room()),
                )
            })
            .collect();
        Some(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::team::TeamType;
    use crate::r#match::{CoachCard, SelectionContext};
    use crate::shared::fullname::FullName;
    use crate::{MatchTacticType, Tactics, TacticsSelector};
    use crate::{
        PersonAttributes, Player, PlayerAttributes, PlayerCollection, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills, StaffClubContract, StaffCollection,
        StaffPosition, StaffStatus, StaffStub, Team, TeamBuilder, TeamReputation, TrainingSchedule,
    };
    use chrono::{NaiveDate, NaiveTime};

    fn player(id: u32, position: PlayerPositionType) -> Player {
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("T".into(), id.to_string()))
            .birth_date(NaiveDate::from_ymd_opt(1998, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 18,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn team(staff: Vec<crate::Staff>) -> Team {
        use PlayerPositionType::*;
        let positions = [
            Goalkeeper,
            Goalkeeper,
            DefenderLeft,
            DefenderCenterLeft,
            DefenderCenterRight,
            DefenderRight,
            DefenderCenter,
            MidfielderLeft,
            MidfielderCenterLeft,
            MidfielderCenterRight,
            MidfielderRight,
            MidfielderCenter,
            ForwardLeft,
            ForwardRight,
            Striker,
            Striker,
        ];
        let players = positions
            .iter()
            .enumerate()
            .map(|(i, &pos)| player(i as u32 + 1, pos))
            .collect();
        TeamBuilder::new()
            .id(1)
            .league_id(Some(1))
            .club_id(1)
            .name("Main".to_string())
            .slug("main".to_string())
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(staff))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }

    #[test]
    fn a_club_squad_carries_its_head_coachs_card() {
        let mut manager = StaffStub::build();
        manager.id = 77;
        manager.staff_attributes.mental.adaptability = 17;
        manager.contract = Some(StaffClubContract::new(
            50_000,
            NaiveDate::from_ymd_opt(2030, 6, 30).unwrap(),
            StaffPosition::Manager,
            StaffStatus::Active,
        ));
        let expected = CoachCard::of_head_coach(&manager);
        assert!(expected.is_some());
        let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let squad = team(vec![manager]).get_rotation_match_squad_at(date);
        assert_eq!(squad.coach_card, expected);

        let squad = team(Vec::new()).get_rotation_match_squad_at(date);
        assert_eq!(
            squad.coach_card, None,
            "an empty dugout leaves the match to draw"
        );
    }

    fn manager(tactical_knowledge: u8) -> crate::Staff {
        let mut manager = StaffStub::build();
        manager.id = 77;
        manager.staff_attributes.knowledge.tactical_knowledge = tactical_knowledge;
        manager.contract = Some(StaffClubContract::new(
            50_000,
            NaiveDate::from_ymd_opt(2030, 6, 30).unwrap(),
            StaffPosition::Manager,
            StaffStatus::Active,
        ));
        manager
    }

    #[test]
    fn a_new_coach_and_a_closed_window_choose_the_system_again() {
        let mut side = team(vec![manager(12)]);
        let chosen = TacticsSelector::select(&side, side.staffs.head_coach()).tactic_type;
        let other = MatchTacticType::all()
            .into_iter()
            .find(|&t| t != chosen)
            .unwrap();

        side.tactics = Some(Tactics::new(other));
        side.on_head_coach_appointed();
        assert_eq!(side.tactics.as_ref().map(|t| t.tactic_type), Some(chosen));

        side.tactics = Some(Tactics::new(other));
        side.on_window_closed();
        assert_eq!(side.tactics.as_ref().map(|t| t.tactic_type), Some(chosen));
    }

    #[test]
    fn a_side_starts_in_its_own_system_whatever_its_coach_knows_and_keeps_it() {
        for knowledge in [13, 14] {
            let mut side = team(vec![manager(knowledge)]);
            side.tactics = Some(Tactics::new(MatchTacticType::T442));
            for _ in 0..2 {
                let squad = side.get_enhanced_match_squad(&[], &SelectionContext::default());
                assert_eq!(
                    squad.tactics.tactic_type,
                    MatchTacticType::T442,
                    "knowledge {knowledge}"
                );
            }
            assert_eq!(
                side.tactics.as_ref().map(|t| t.tactic_type),
                Some(MatchTacticType::T442)
            );
        }
    }
}
