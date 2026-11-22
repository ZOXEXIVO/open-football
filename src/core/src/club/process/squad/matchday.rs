//! The day before a fixture: which of the club's players go with which of
//! its sides tomorrow.
//!
//! The matchday itself only reads. Squads for a whole country are picked
//! side by side, so who is lent where has to be settled before the day
//! starts, and the club is the one owner that sees every roster.

use chrono::{Duration, NaiveDate};
use rustc_hash::FxHashSet;

use crate::club::person::Person;
use crate::club::player::events::discipline::CompetitionFamily;
use crate::club::staff::AbilityEstimator;
use crate::club::staff::CoachSquadPlan;
use crate::continent::ContinentalRegulations;
use crate::league::LeagueRegulations;
use crate::r#match::squad::selection::helpers::DEFAULT_SQUAD_SIZE;
use crate::{Club, Player, Team, TeamType};

impl Club {
    pub(in crate::club::process) fn assign_matchday_sides(&mut self, today: NaiveDate) {
        self.settle_matchday_sides(today + Duration::days(1));
    }

    /// One of the club's sides has had a fixture moved. When either day is
    /// today, last night's decisions were made for a day that is no longer
    /// the one being played, and the club makes them again.
    pub fn on_fixture_rearranged(
        &mut self,
        team_id: u32,
        from: NaiveDate,
        to: NaiveDate,
        today: NaiveDate,
    ) {
        if let Some(team) = self.teams.teams.iter_mut().find(|t| t.id == team_id) {
            team.fixture_window.on_fixture_moved(from, to);
        }
        if from == today || to == today {
            self.settle_matchday_sides(today);
        }
    }

    /// Decides who is with which side on `day`, in place of anything
    /// decided for it before.
    fn settle_matchday_sides(&mut self, day: NaiveDate) {
        let assignments = MatchdayAssignmentPass::new(self, day).run();
        for player in self
            .teams
            .teams
            .iter_mut()
            .flat_map(|t| t.players.players.iter_mut())
        {
            match assignments.iter().find(|a| a.player_id == player.id) {
                Some(found) => player.on_matchday_assigned(day, found.team_id),
                None if player.assigned_side_on(day).is_some() => {
                    player.on_matchday_assignment_withdrawn()
                }
                None => {}
            }
        }
    }
}

struct SideAssignment {
    player_id: u32,
    team_id: u32,
}

struct MatchdayAssignmentPass<'a> {
    club: &'a Club,
    day: NaiveDate,
    /// The club's sides with a fixture on the day, most senior first.
    sides: Vec<&'a Team>,
    /// The men the first team would name on the day or the day after it.
    held: FxHashSet<u32>,
    taken: FxHashSet<u32>,
    /// Sides already sent a goalkeeper for the day.
    sent_a_keeper: FxHashSet<u32>,
    assignments: Vec<SideAssignment>,
}

impl<'a> MatchdayAssignmentPass<'a> {
    /// The most first-team players one fixture of another side takes, so
    /// the side's own players keep most of its shirts.
    const RELEASED_PER_FIXTURE: usize = 3;
    /// Days without a match from any side before a man is short of football:
    /// he was not used in his side's last fixture.
    const IDLE_DAYS: u16 = 7;
    /// Match sharpness, 0..20, below which a man back from an absence
    /// still needs a game.
    const SHARP: f32 = 15.0;
    /// The part of his planned football a man has missed before he counts
    /// as short of it.
    const SHORT: f32 = 0.5;
    /// The shortest bench that names a second goalkeeper.
    const SECOND_KEEPER_BENCH: u8 = 12;

    fn new(club: &'a Club, day: NaiveDate) -> Self {
        let mut sides: Vec<&Team> = club
            .teams
            .teams
            .iter()
            .filter(|t| t.fixture_window.plays_on(day))
            .collect();
        sides.sort_by_key(|t| (t.team_type.seniority(), t.id));
        MatchdayAssignmentPass {
            club,
            day,
            sides,
            held: FxHashSet::default(),
            taken: FxHashSet::default(),
            sent_a_keeper: FxHashSet::default(),
            assignments: Vec::new(),
        }
    }

    fn run(mut self) -> Vec<SideAssignment> {
        if self.sides.is_empty() {
            return self.assignments;
        }
        self.held = self.held_by_the_first_team();
        self.find_keepers();
        self.release_from_the_first_team();
        self.assignments
    }

    fn assign(&mut self, player: &Player, side: &Team) {
        self.taken.insert(player.id);
        if player.positions.is_goalkeeper() {
            self.sent_a_keeper.insert(side.id);
        }
        self.assignments.push(SideAssignment {
            player_id: player.id,
            team_id: side.id,
        });
    }

    /// Whether a man can be named in a fixture of `family`, a development
    /// fixture when there is none.
    fn can_play(player: &Player, family: Option<CompetitionFamily>) -> bool {
        !player.player_attributes.is_injured
            && !player.statuses.is_on_international_duty()
            && !player.move_is_agreed()
            && family.is_none_or(|family| !player.is_suspended_in(family))
    }

    fn may_register(player: &Player, side: &Team, day: NaiveDate) -> bool {
        side.team_type
            .registration_age_cap()
            .is_none_or(|cap| player.age(day) <= cap)
    }

    fn free_keepers(&self, team: &'a Team, family: Option<CompetitionFamily>) -> Vec<&'a Player> {
        team.players
            .iter()
            .filter(|p| p.positions.is_goalkeeper())
            .filter(|p| !self.taken.contains(&p.id) && Self::can_play(p, family))
            .collect()
    }

    /// A side with no goalkeeper for the day takes one from the nearest
    /// side that can spare him. A side with a fixture of its own keeps
    /// one, and the first team keeps the men it would name. Of the two
    /// sides the more senior has the better keeper.
    fn find_keepers(&mut self) {
        for side in self.sides.clone() {
            let family = side.fixture_window.competes_on(self.day);
            if !self.free_keepers(side, family).is_empty() {
                continue;
            }
            let rank = side.team_type.seniority();
            let spare = self
                .club
                .teams
                .teams
                .iter()
                .filter(|donor| donor.id != side.id)
                .flat_map(|donor| {
                    let needs_one = donor.fixture_window.plays_on(self.day);
                    let kept = if needs_one {
                        self.free_keepers(donor, donor.fixture_window.competes_on(self.day))
                    } else {
                        Vec::new()
                    };
                    self.free_keepers(donor, family)
                        .into_iter()
                        .filter(|p| !self.held.contains(&p.id))
                        .filter(move |p| !needs_one || kept.iter().any(|other| other.id != p.id))
                        .map(move |p| (donor, p))
                })
                .filter(|(_, p)| Self::may_register(p, side, self.day))
                .min_by_key(|(donor, p)| {
                    let level = AbilityEstimator::observable_level(p);
                    let donor_rank = donor.team_type.seniority();
                    (
                        donor_rank.abs_diff(rank),
                        if rank < donor_rank {
                            u8::MAX - level
                        } else {
                            level
                        },
                        p.id,
                    )
                });
            if let Some((_, keeper)) = spare {
                self.assign(keeper, side);
            }
        }
    }

    /// The first team's manager lets the men he has no use for that day go
    /// and play for the club's next side, when they are short of football.
    fn release_from_the_first_team(&mut self) {
        let Some(main) = self.club.teams.main() else {
            return;
        };
        let plan = &main.staffs.head_coach().squad_plan;
        let mut short: Vec<(&Player, f32)> = main
            .players
            .iter()
            .filter(|p| !self.taken.contains(&p.id) && !self.held.contains(&p.id))
            .filter(|p| !p.is_force_match_selection)
            .filter(|p| p.player_attributes.days_since_last_match >= Self::IDLE_DAYS)
            .filter_map(|p| Self::football_needed(p, plan).map(|need| (p, need)))
            .collect();
        short.sort_by(|a, b| {
            let idle = |p: &Player| p.player_attributes.days_since_last_match;
            b.1.total_cmp(&a.1)
                .then_with(|| idle(b.0).cmp(&idle(a.0)))
                .then_with(|| a.0.id.cmp(&b.0.id))
        });

        for side in self.sides.clone() {
            if side.team_type == TeamType::Main {
                continue;
            }
            let family = side.fixture_window.competes_on(self.day);
            let mut released = 0usize;
            for (player, _) in &short {
                if released == Self::RELEASED_PER_FIXTURE {
                    break;
                }
                if self.taken.contains(&player.id)
                    || (player.positions.is_goalkeeper() && self.sent_a_keeper.contains(&side.id))
                    || !Self::can_play(player, family)
                    || !Self::may_register(player, side, self.day)
                {
                    continue;
                }
                self.assign(player, side);
                released += 1;
            }
        }
    }

    /// How short of football a first-team player is, when he is: he has
    /// had less than half of what the plan gave him, or he is back from an
    /// absence and not yet sharp.
    fn football_needed(player: &Player, plan: &CoachSquadPlan) -> Option<f32> {
        let shortfall = plan
            .entry(player.id)
            .map_or(0.0, |entry| entry.shortfall(player));
        let returning = player.player_attributes.is_in_recovery()
            && player.skills.physical.match_readiness < Self::SHARP;
        (shortfall >= Self::SHORT || returning)
            .then_some(shortfall + f32::from(u8::from(returning)))
    }

    /// The men the first team would name on the day or the day after it,
    /// when it has a fixture on either: a full matchday squad of its fit
    /// players, its goalkeepers among them, in the order the plan ranks
    /// them.
    fn held_by_the_first_team(&self) -> FxHashSet<u32> {
        let Some(main) = self.club.teams.main() else {
            return FxHashSet::default();
        };
        let fixture = [self.day, self.day + Duration::days(1)]
            .into_iter()
            .find_map(|day| main.fixture_window.competes_on(day));
        let Some(family) = fixture else {
            return FxHashSet::default();
        };
        let bench = match family {
            CompetitionFamily::Continental => ContinentalRegulations::BENCH_SIZE,
            CompetitionFamily::Domestic => LeagueRegulations::BENCH_TOP_DIVISIONS,
        };
        let keepers = 2 + usize::from(bench >= Self::SECOND_KEEPER_BENCH);
        let outfield = DEFAULT_SQUAD_SIZE + usize::from(bench) - keepers;

        let plan = &main.staffs.head_coach().squad_plan;
        let mut fit: Vec<&Player> = main
            .players
            .iter()
            .filter(|p| Self::can_play(p, Some(family)))
            .collect();
        fit.sort_by(|a, b| {
            let rank = |p: &Player| {
                plan.entry(p.id)
                    .map_or((u8::MAX, 0.0), |entry| (entry.place, entry.share))
            };
            let (place_a, share_a) = rank(a);
            let (place_b, share_b) = rank(b);
            place_a
                .cmp(&place_b)
                .then_with(|| share_b.total_cmp(&share_a))
                .then_with(|| a.id.cmp(&b.id))
        });
        let named = |keeper: bool, seats: usize| {
            fit.iter()
                .filter(move |p| p.positions.is_goalkeeper() == keeper)
                .take(seats)
                .map(|p| p.id)
        };
        named(true, keepers).chain(named(false, outfield)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::model::builder::PlayerBuilder;
    use crate::club::staff::PlanReading;
    use crate::club::team::fitness::fixture_window::FixtureRun;
    use crate::r#match::squad::selection::model::{
        EligibilityDecision, EligibilityEvaluator, FixtureEligibility,
    };
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, MatchTacticType, PersonAttributes,
        PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, StaffCollection, Tactics, TeamBuilder, TeamCollection, TeamReputation,
        TrainingSchedule,
    };
    use chrono::NaiveTime;

    const MAIN: u32 = 1;
    const OTHER: u32 = 2;
    const THIRD: u32 = 3;

    /// A club the evening before a fixture.
    struct Eve;

    impl Eve {
        fn today() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 10, 9).unwrap()
        }

        fn tomorrow() -> NaiveDate {
            Self::today() + Duration::days(1)
        }

        fn player(id: u32, position: PlayerPositionType, age: i32, level: u8) -> Player {
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("P".to_string(), format!("L{id}")))
                .birth_date(NaiveDate::from_ymd_opt(2026 - age, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(level))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 20,
                    }],
                })
                .player_attributes(PlayerAttributes {
                    current_ability: level,
                    condition: 9500,
                    ..Default::default()
                })
                .build()
                .unwrap()
        }

        fn keeper(id: u32, age: i32, level: u8) -> Player {
            Self::player(id, PlayerPositionType::Goalkeeper, age, level)
        }

        fn injured(mut player: Player) -> Player {
            player.player_attributes.is_injured = true;
            player
        }

        /// Two keepers and `outfield` men of 27 in a 4-4-2's shirts, the
        /// best first.
        fn first_team(outfield: usize) -> Vec<Player> {
            let shirts = Tactics::new(MatchTacticType::T442);
            let shirts: Vec<PlayerPositionType> = shirts
                .positions()
                .iter()
                .copied()
                .filter(|p| *p != PlayerPositionType::Goalkeeper)
                .collect();
            let mut players = vec![Self::keeper(100, 27, 150), Self::keeper(101, 27, 120)];
            for i in 0..outfield {
                let level = 150 - (i as u8) * 2;
                players.push(Self::player(
                    110 + i as u32,
                    shirts[i % shirts.len()],
                    27,
                    level,
                ));
            }
            players
        }

        fn team(id: u32, team_type: TeamType, players: Vec<Player>) -> Team {
            TeamBuilder::new()
                .id(id)
                .league_id(Some(id))
                .club_id(1)
                .name(format!("team-{id}"))
                .slug(format!("team-{id}"))
                .team_type(team_type)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(100, 100, 100))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        fn competes(mut team: Team, days_ahead: i64) -> Team {
            let day = Self::today() + Duration::days(days_ahead);
            team.fixture_window.domestic = FixtureRun::dated(&[day], &[], 0.6);
            team
        }

        fn plays_a_development_fixture(mut team: Team) -> Team {
            team.fixture_window.development = FixtureRun::dated(&[Self::tomorrow()], &[], 0.1);
            team
        }

        /// The first team's coach draws his plan, and the season then goes
        /// by without `short` playing a minute of it.
        fn planned(mut main: Team, short: &[u32]) -> Team {
            let shape = Tactics::new(MatchTacticType::T442);
            let reading = PlanReading::of(&main.players, &shape, Self::today());
            main.staffs.squad_planner_mut().squad_plan.revise(&reading);
            for player in main.players.players.iter_mut() {
                if short.contains(&player.id) {
                    player.happiness.eligible_official_matches_since_join = 40;
                    player.player_attributes.days_since_last_match = 21;
                }
            }
            main
        }

        fn club(teams: Vec<Team>) -> Club {
            Club::new(
                1,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(teams),
                ClubFacilities::default(),
            )
        }

        fn side_of(club: &Club, player_id: u32) -> Option<u32> {
            club.teams
                .teams
                .iter()
                .flat_map(|t| t.players.iter())
                .find(|p| p.id == player_id)
                .and_then(|p| p.assigned_side_on(Self::tomorrow()))
        }
    }

    #[test]
    fn a_first_team_with_no_fit_keeper_takes_one_and_the_donor_keeps_one() {
        let mut players = Eve::first_team(16);
        players[0].player_attributes.is_injured = true;
        players[1].player_attributes.is_injured = true;
        let main = Eve::competes(Eve::team(MAIN, TeamType::Main, players), 1);
        let u21 = Eve::plays_a_development_fixture(Eve::team(
            OTHER,
            TeamType::U21,
            vec![Eve::keeper(200, 20, 110), Eve::keeper(201, 19, 95)],
        ));
        let mut club = Eve::club(vec![main, u21]);
        club.assign_matchday_sides(Eve::today());

        assert_eq!(
            Eve::side_of(&club, 200),
            Some(MAIN),
            "the better of the two goes up"
        );
        assert_eq!(
            Eve::side_of(&club, 201),
            None,
            "the under-21s keep a keeper of their own"
        );
    }

    #[test]
    fn a_donor_with_one_keeper_and_a_fixture_of_its_own_lends_nobody() {
        let mut players = Eve::first_team(16);
        players[0].player_attributes.is_injured = true;
        players[1].player_attributes.is_injured = true;
        let main = Eve::competes(Eve::team(MAIN, TeamType::Main, players), 1);
        let u21 = Eve::plays_a_development_fixture(Eve::team(
            OTHER,
            TeamType::U21,
            vec![Eve::keeper(200, 20, 110)],
        ));
        let mut club = Eve::club(vec![main, u21]);
        club.assign_matchday_sides(Eve::today());
        assert_eq!(Eve::side_of(&club, 200), None);
    }

    #[test]
    fn a_youth_friendly_is_found_a_keeper_from_the_side_above() {
        let u18 = Eve::plays_a_development_fixture(Eve::team(
            THIRD,
            TeamType::U18,
            vec![
                Eve::injured(Eve::keeper(300, 17, 80)),
                Eve::player(301, PlayerPositionType::DefenderCenter, 17, 80),
            ],
        ));
        let u19 = Eve::team(
            OTHER,
            TeamType::U19,
            vec![Eve::keeper(200, 18, 90), Eve::keeper(201, 18, 85)],
        );
        let main = Eve::team(MAIN, TeamType::Main, Eve::first_team(16));
        let mut club = Eve::club(vec![main, u19, u18]);
        club.assign_matchday_sides(Eve::today());

        assert_eq!(
            Eve::side_of(&club, 201),
            Some(THIRD),
            "the nearest side's keeper, and the lesser of its two"
        );
        assert_eq!(Eve::side_of(&club, 200), None);
        assert_eq!(Eve::side_of(&club, 100), None, "not the first team's");
    }

    #[test]
    fn a_club_with_no_fit_keeper_anywhere_assigns_nobody() {
        let mut players = Eve::first_team(16);
        players[0].player_attributes.is_injured = true;
        players[1].player_attributes.is_injured = true;
        let main = Eve::competes(Eve::team(MAIN, TeamType::Main, players), 1);
        let u21 = Eve::team(
            OTHER,
            TeamType::U21,
            vec![Eve::injured(Eve::keeper(200, 20, 110))],
        );
        let mut club = Eve::club(vec![main, u21]);
        club.assign_matchday_sides(Eve::today());
        assert!(
            club.teams
                .teams
                .iter()
                .flat_map(|t| t.players.iter())
                .all(|p| p.matchday_assignment.is_none())
        );
    }

    /// The last man of a deep first team: the plan gives him football and
    /// he has had none of it.
    const SHORT: u32 = 110 + 23;

    fn b_side() -> Team {
        Eve::competes(
            Eve::team(
                OTHER,
                TeamType::B,
                vec![Eve::keeper(200, 22, 100), Eve::keeper(201, 21, 90)],
            ),
            1,
        )
    }

    #[test]
    fn a_prospect_short_of_football_goes_to_the_b_side_when_the_first_team_is_idle() {
        let mut players = Eve::first_team(24);
        players
            .iter_mut()
            .find(|p| p.id == SHORT)
            .unwrap()
            .birth_date = NaiveDate::from_ymd_opt(2007, 1, 1).unwrap();
        let main = Eve::planned(Eve::team(MAIN, TeamType::Main, players), &[SHORT]);
        let mut club = Eve::club(vec![main, b_side()]);
        club.assign_matchday_sides(Eve::today());
        assert_eq!(Eve::side_of(&club, SHORT), Some(OTHER));
    }

    #[test]
    fn a_man_back_from_an_absence_plays_for_the_reserves_first() {
        let mut players = Eve::first_team(16);
        let regular = players.iter_mut().find(|p| p.id == 112).unwrap();
        regular.player_attributes.recovery_days_remaining = 14;
        regular.player_attributes.days_since_last_match = 60;
        regular.skills.physical.match_readiness = 6.0;
        // The first team's next fixture is three days off.
        let main = Eve::competes(
            Eve::planned(Eve::team(MAIN, TeamType::Main, players), &[]),
            3,
        );
        let reserves = Eve::competes(
            Eve::team(OTHER, TeamType::Reserve, vec![Eve::keeper(200, 22, 100)]),
            1,
        );
        let mut club = Eve::club(vec![main, reserves]);
        club.assign_matchday_sides(Eve::today());
        assert_eq!(Eve::side_of(&club, 112), Some(OTHER));
    }

    #[test]
    fn a_thin_first_team_playing_the_next_day_releases_nobody_it_would_name() {
        // Eighteen fit men: every one of them is in tomorrow's squad or the
        // day after's.
        let short = 110 + 15;
        let main = Eve::competes(
            Eve::planned(
                Eve::team(MAIN, TeamType::Main, Eve::first_team(16)),
                &[short],
            ),
            2,
        );
        let mut club = Eve::club(vec![main, b_side()]);
        club.assign_matchday_sides(Eve::today());
        assert_eq!(Eve::side_of(&club, short), None);
    }

    #[test]
    fn a_senior_is_not_released_to_an_age_limited_side_he_cannot_register_for() {
        let mut players = Eve::first_team(24);
        players
            .iter_mut()
            .find(|p| p.id == SHORT)
            .unwrap()
            .birth_date = NaiveDate::from_ymd_opt(2002, 1, 1).unwrap();
        let main = Eve::planned(Eve::team(MAIN, TeamType::Main, players), &[SHORT]);
        let u19 = Eve::plays_a_development_fixture(Eve::team(
            OTHER,
            TeamType::U19,
            vec![Eve::keeper(200, 18, 90)],
        ));
        let mut club = Eve::club(vec![main, u19]);
        club.assign_matchday_sides(Eve::today());
        assert_eq!(
            Eve::side_of(&club, SHORT),
            None,
            "a man of 24 and an under-19 fixture"
        );
    }

    #[test]
    fn a_released_man_is_with_one_side_only_on_a_day_both_play() {
        let main = Eve::competes(
            Eve::planned(
                Eve::team(MAIN, TeamType::Main, Eve::first_team(24)),
                &[SHORT],
            ),
            1,
        );
        let mut club = Eve::club(vec![main, b_side()]);
        club.assign_matchday_sides(Eve::today());
        assert_eq!(
            Eve::side_of(&club, SHORT),
            Some(OTHER),
            "outside the first team's twenty"
        );

        let player = club
            .teams
            .teams
            .iter()
            .flat_map(|t| t.players.iter())
            .find(|p| p.id == SHORT)
            .unwrap();
        let asked_by = |team_id: u32| {
            EligibilityEvaluator::evaluate(
                player,
                &FixtureEligibility {
                    team_id,
                    ..Default::default()
                },
                Eve::tomorrow(),
                false,
            )
        };
        assert!(matches!(
            asked_by(MAIN),
            EligibilityDecision::HardBlocked { .. }
        ));
        assert!(matches!(asked_by(OTHER), EligibilityDecision::Eligible));
    }

    #[test]
    fn the_first_teams_bench_keeper_stays_on_a_day_it_plays() {
        // The deputy has had none of his football, and the first team
        // still names him tomorrow.
        let main = Eve::competes(
            Eve::planned(Eve::team(MAIN, TeamType::Main, Eve::first_team(24)), &[101]),
            1,
        );
        let mut club = Eve::club(vec![main, b_side()]);
        club.assign_matchday_sides(Eve::today());
        assert_eq!(Eve::side_of(&club, 101), None);
    }

    #[test]
    fn a_regular_a_match_behind_is_not_sent_to_the_next_side_in_an_idle_fortnight() {
        let mut main = Eve::planned(Eve::team(MAIN, TeamType::Main, Eve::first_team(24)), &[]);
        let regular = main
            .players
            .players
            .iter_mut()
            .find(|p| p.id == 112)
            .unwrap();
        regular.happiness.eligible_official_matches_since_join = 20;
        regular.happiness.official_minutes_since_join = 15 * 90;
        regular.player_attributes.days_since_last_match = 16;
        let mut club = Eve::club(vec![main, b_side()]);
        club.assign_matchday_sides(Eve::today());
        assert_eq!(
            Eve::side_of(&club, 112),
            None,
            "he has had most of his football"
        );
    }

    #[test]
    fn a_fixture_moved_onto_the_day_takes_back_the_men_the_side_had_let_go() {
        // The first team was idle tomorrow as far as it knew, and its
        // deputy keeper went to the B side. Its cup tie is then moved.
        let main = Eve::competes(
            Eve::planned(Eve::team(MAIN, TeamType::Main, Eve::first_team(24)), &[101]),
            4,
        );
        let mut club = Eve::club(vec![main, b_side()]);
        club.assign_matchday_sides(Eve::today());
        assert_eq!(Eve::side_of(&club, 101), Some(OTHER));

        let nominal = Eve::today() + Duration::days(4);
        club.on_fixture_rearranged(MAIN, nominal, Eve::tomorrow(), Eve::tomorrow());
        assert_eq!(Eve::side_of(&club, 101), None);
    }

    #[test]
    fn a_fixture_moved_off_the_day_sends_back_the_men_lent_for_it() {
        let main = Eve::planned(
            Eve::team(MAIN, TeamType::Main, Eve::first_team(24)),
            &[SHORT],
        );
        let mut club = Eve::club(vec![main, b_side()]);
        club.assign_matchday_sides(Eve::today());
        assert_eq!(Eve::side_of(&club, SHORT), Some(OTHER));

        let later = Eve::tomorrow() + Duration::days(2);
        club.on_fixture_rearranged(OTHER, Eve::tomorrow(), later, Eve::tomorrow());
        assert_eq!(Eve::side_of(&club, SHORT), None);
    }

    #[test]
    fn a_first_team_whose_fixture_is_moved_onto_the_day_chooses_its_keeper_first() {
        let mut players = Eve::first_team(16);
        players.retain(|p| !p.positions.is_goalkeeper());
        let main = Eve::competes(Eve::team(MAIN, TeamType::Main, players), 4);
        let u20 = Eve::team(OTHER, TeamType::U20, vec![Eve::keeper(200, 19, 100)]);
        let u18 = Eve::plays_a_development_fixture(Eve::team(
            THIRD,
            TeamType::U18,
            vec![Eve::player(301, PlayerPositionType::DefenderCenter, 17, 80)],
        ));
        let mut club = Eve::club(vec![main, u20, u18]);
        club.assign_matchday_sides(Eve::today());
        assert_eq!(
            Eve::side_of(&club, 200),
            Some(THIRD),
            "the only side known to play"
        );

        let nominal = Eve::today() + Duration::days(4);
        club.on_fixture_rearranged(MAIN, nominal, Eve::tomorrow(), Eve::tomorrow());
        assert_eq!(Eve::side_of(&club, 200), Some(MAIN));
    }

    #[test]
    fn a_fixture_takes_a_few_first_team_players_at_most() {
        let short: Vec<u32> = (110 + 20..110 + 28).collect();
        let main = Eve::planned(Eve::team(MAIN, TeamType::Main, Eve::first_team(28)), &short);
        let mut club = Eve::club(vec![main, b_side()]);
        club.assign_matchday_sides(Eve::today());
        let released = short
            .iter()
            .filter(|id| Eve::side_of(&club, **id) == Some(OTHER))
            .count();
        assert_eq!(released, MatchdayAssignmentPass::RELEASED_PER_FIXTURE);
    }
}
