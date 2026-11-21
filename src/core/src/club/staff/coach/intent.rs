//! What the plan's ledger asks of one fixture. The same read feeds the XI,
//! the bench, explanations and live cameo decisions.
use super::plan::PlannedRole;
use crate::club::player::PlayerUsage;
use crate::{Player, Staff};

#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerMatchIntent {
    /// Football the plan owes him, in matches. Negative when he is ahead of
    /// his share.
    pub owed: f32,
    /// How far football owed may move a shirt in this fixture, 0..1.
    pub room: f32,
    pub development: bool,
    /// Earliest sensible minute for the planned substitute appearance.
    pub cameo_from: u32,
    /// Protected debuts require a two-goal cushion; experienced players do not.
    pub minimum_lead: i32,
    pub succeeds: Option<u32>,
}

impl PlayerMatchIntent {
    /// What one match of football owed is worth to a shirt, in the points
    /// fit is scored in, in a fixture that leaves all of its room.
    pub const OWED: f32 = 1.5;
    /// Room under which a fixture is too tight to blood anyone without a
    /// cushion.
    const TIGHT_ROOM: f32 = 0.18;

    pub fn assess(player: &Player, staff: &Staff, room: f32) -> Self {
        let room = room.clamp(0.0, 1.0);
        let mut intent = Self {
            room,
            cameo_from: 60,
            minimum_lead: if room <= Self::TIGHT_ROOM { 2 } else { 0 },
            ..Self::default()
        };
        let Some(entry) = staff.squad_plan.entry(player.id) else {
            return intent;
        };
        intent.owed = entry.owed(player);
        intent.succeeds = entry.succeeds;
        intent.development = matches!(
            entry.role,
            PlannedRole::SuccessionHeir | PlannedRole::DevelopmentPathway
        );
        if intent.development {
            // Real senior minutes advance the introduction. Five short
            // cameos do not establish the player as five starts would.
            let minutes = PlayerUsage::of(player).minutes;
            intent.cameo_from = if minutes < 45 { 70 } else { 60 };
            intent.minimum_lead = if minutes < 180 { 2 } else { 1 };
        }
        intent
    }

    /// What the ledger adds to his score for a shirt in the XI: football
    /// owed pulls him in, football already had pushes him out, and neither
    /// reaches further than the fixture's room.
    pub fn start_adjustment(self) -> f32 {
        self.room * Self::OWED * self.owed
    }

    /// The bench is where a man owed football is brought along; being
    /// ahead of his share does not cost anyone a seat on it.
    pub fn bench_adjustment(self) -> f32 {
        self.room * Self::OWED * self.owed.max(0.0)
    }

    /// A conditional intention, not a forced substitution. Injuries, role
    /// fit and the team's tactical needs are still checked by the live scorer.
    pub fn cameo_adjustment(
        self,
        minute: u32,
        goal_diff: i32,
        position_fit: f32,
        outgoing: u32,
    ) -> f32 {
        if minute < self.cameo_from
            || minute > 85
            || goal_diff < self.minimum_lead
            || position_fit < 0.7
        {
            return 0.0;
        }
        if self.development && self.succeeds.is_some_and(|id| id != outgoing) {
            return 0.0;
        }
        (self.bench_adjustment() * 0.18).min(0.25)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::staff::{CoachSquadPlan, PlanReading};
    use crate::shared::fullname::FullName;
    use crate::{MatchTacticType, Tactics};
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerBuilder, PlayerClubContract, PlayerCollection,
        PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills, StaffStub,
    };
    use chrono::{Duration, NaiveDate};

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()
    }
    fn player(id: u32, ability: u8, age: i32) -> Player {
        at(id, PlayerPositionType::DefenderLeft, ability, age)
    }
    fn at(id: u32, position: PlayerPositionType, ability: u8, age: i32) -> Player {
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Test".into(), format!("{id}")))
            .birth_date(NaiveDate::from_ymd_opt(2026 - age, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::flat_for_ability(ability))
            .player_attributes(PlayerAttributes {
                current_ability: ability,
                potential_ability: ability,
                ..Default::default()
            })
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 18,
                }],
            })
            .contract(Some(PlayerClubContract::new(
                50_000,
                date() + Duration::days(1000),
            )))
            .build()
            .unwrap()
    }

    /// A coach whose plan has a full side and two left-backs: the first
    /// choice and a close second.
    fn coach_with_plan(first: &Player, second: &Player) -> Staff {
        let mut c = StaffStub::build();
        c.id = 7;
        let shape = Tactics::new(MatchTacticType::T442);
        let mut squad: Vec<Player> = shape
            .positions()
            .iter()
            .enumerate()
            .filter(|(_, slot)| **slot != PlayerPositionType::DefenderLeft)
            .map(|(i, slot)| at(100 + i as u32, *slot, 140, 27))
            .collect();
        squad.push(first.clone());
        squad.push(second.clone());
        let squad = PlayerCollection::new(squad);
        let mut plan = CoachSquadPlan::new();
        plan.revise(&PlanReading::of(&squad, &shape, date()));
        c.squad_plan = plan;
        c
    }

    fn left_out(mut p: Player, fixtures: u8) -> Player {
        for _ in 0..fixtures {
            p.on_match_overlooked(false);
        }
        p
    }
    fn played(mut p: Player, matches: u8) -> Player {
        for _ in 0..matches {
            p.happiness.note_official_appearance(true);
            p.happiness.official_minutes_since_join += 90;
        }
        p
    }

    #[test]
    fn a_player_nobody_has_planned_for_is_owed_nothing() {
        let p = left_out(player(1, 140, 26), 6);
        let intent = PlayerMatchIntent::assess(&p, &StaffStub::build(), 0.8);
        assert_eq!(intent.owed, 0.0);
        assert_eq!(intent.start_adjustment(), 0.0);
        assert_eq!(intent.bench_adjustment(), 0.0);
    }

    #[test]
    fn football_owed_moves_a_shirt_only_as_far_as_the_fixture_leaves_room() {
        let first = player(1, 140, 27);
        let second = player(2, 138, 26);
        let coach = coach_with_plan(&first, &second);
        let waiting = left_out(second, 5);

        let must_win = PlayerMatchIntent::assess(&waiting, &coach, 0.0);
        assert!(must_win.owed > 1.0, "owed {}", must_win.owed);
        assert_eq!(must_win.start_adjustment(), 0.0);
        assert_eq!(must_win.bench_adjustment(), 0.0);

        let routine = PlayerMatchIntent::assess(&waiting, &coach, 0.4);
        let dead_rubber = PlayerMatchIntent::assess(&waiting, &coach, 0.9);
        assert!(routine.start_adjustment() > 0.0);
        assert!(
            (dead_rubber.start_adjustment() / routine.start_adjustment() - 0.9 / 0.4).abs() < 1e-4,
            "the pull is the room times what he is owed"
        );
    }

    #[test]
    fn a_man_ahead_of_his_share_gives_way_in_the_xi_and_keeps_his_seat_on_the_bench() {
        let first = player(1, 140, 27);
        let second = player(2, 138, 26);
        let coach = coach_with_plan(&first, &second);
        let ever_present = played(first, 8);

        let intent = PlayerMatchIntent::assess(&ever_present, &coach, 0.6);
        assert!(intent.owed < 0.0, "owed {}", intent.owed);
        assert!(intent.start_adjustment() < 0.0);
        assert_eq!(intent.bench_adjustment(), 0.0);
    }

    #[test]
    fn a_tight_fixture_asks_for_a_cushion_before_anyone_is_brought_on() {
        let p = player(1, 120, 24);
        let coach = StaffStub::build();
        assert_eq!(PlayerMatchIntent::assess(&p, &coach, 0.1).minimum_lead, 2);
        assert_eq!(PlayerMatchIntent::assess(&p, &coach, 0.5).minimum_lead, 0);
    }

    #[test]
    fn a_prospect_is_brought_on_late_with_a_cushion_until_he_has_real_minutes() {
        let mut coach = StaffStub::build();
        coach.id = 7;
        let p = player(1, 100, 18);
        coach
            .squad_plan
            .force_role(p.id, PlannedRole::DevelopmentPathway, date());
        let debut = PlayerMatchIntent::assess(&p, &coach, 0.6);
        assert!(debut.development);
        assert_eq!((debut.cameo_from, debut.minimum_lead), (70, 2));

        let p = played(p, 3);
        let settled = PlayerMatchIntent::assess(&p, &coach, 0.6);
        assert_eq!((settled.cameo_from, settled.minimum_lead), (60, 1));
    }

    #[test]
    fn a_planned_cameo_waits_for_its_minute_its_cushion_and_its_shirt() {
        let owed = PlayerMatchIntent {
            owed: 1.5,
            room: 0.6,
            development: true,
            cameo_from: 70,
            minimum_lead: 2,
            succeeds: Some(9),
        };
        assert!(owed.cameo_adjustment(75, 2, 1.0, 9) > 0.0);
        assert_eq!(owed.cameo_adjustment(65, 2, 1.0, 9), 0.0, "too early");
        assert_eq!(owed.cameo_adjustment(75, 0, 1.0, 9), 0.0, "no cushion");
        assert_eq!(owed.cameo_adjustment(75, 2, 0.5, 9), 0.0, "not his position");
        assert_eq!(owed.cameo_adjustment(75, 2, 1.0, 4), 0.0, "not the man he succeeds");
        assert!(owed.cameo_adjustment(75, 2, 1.0, 9) <= 0.25);

        let ahead = PlayerMatchIntent { owed: -2.0, ..owed };
        assert_eq!(ahead.cameo_adjustment(75, 2, 1.0, 9), 0.0);
    }
}
