//! The club's goalkeeping department pass.
//!
//! The department reasons about every keeper the club owns, so the review
//! has to run above a single squad — the first team's succession problem is
//! routinely sitting in the under-eighteens, and the under-eighteens'
//! surplus keeper is routinely the reserve side's answer.
//!
//! Two phases, for one borrow reason and one modelling reason. The room is
//! assembled from the squads read-only; the plan is then written onto the
//! man who owns it. And that separation is the honest one: the census is a
//! fact about the club, the plan is somebody's opinion about it.

use chrono::NaiveDate;

use crate::club::staff::goalkeeping::{
    GoalkeepingDepartment, KeeperCoachAuthority, KeeperRoom, KeeperRoomPlan, KeeperSelectionBrief,
    KeeperTier,
};
use crate::{Club, Player, TeamType};

impl Club {
    /// Every keeper at the club, read as one group.
    pub fn keeper_room(&self, date: NaiveDate) -> KeeperRoom {
        KeeperRoom::assemble(
            self.teams
                .teams
                .iter()
                .flat_map(|t| t.players.iter().map(move |p| (t.id, t.team_type, p))),
            date,
        )
    }

    /// The goalkeeping department's monthly review.
    ///
    /// Returns the keepers whose standing in the room changed, so the caller
    /// can tell them where they stand. A club with no keepers, or with no
    /// staff at all, is a no-op — and every consumer of the plan falls back
    /// to the behaviour it had before the department existed.
    pub fn review_goalkeeping_department(&mut self, date: NaiveDate) -> Vec<(u32, KeeperTier)> {
        let Some(main) = self.teams.main() else {
            return Vec::new();
        };
        if main.staffs.goalkeeping_lead().is_none() {
            return Vec::new();
        }
        {
            let plan = &main.staffs.goalkeeping_lead().expect("checked").keeper_plan;
            if !plan.due_for_review(date) {
                return Vec::new();
            }
        }

        let room = self.keeper_room(date);
        if room.is_empty() {
            return Vec::new();
        }

        // Read the two men involved before taking the plan, so the borrow of
        // the staff list ends before the write.
        let main_idx = match self
            .teams
            .teams
            .iter()
            .position(|t| t.team_type == TeamType::Main)
        {
            Some(idx) => idx,
            None => return Vec::new(),
        };
        let (authority, previous) = {
            let staffs = &self.teams.teams[main_idx].staffs;
            if staffs.goalkeeping_lead().is_none() {
                return Vec::new();
            }
            // The standing plan, wherever it is currently held — the holder
            // changes when the club hires a specialist or replaces a
            // manager, and the club's pecking order must not reset because
            // the paperwork moved desks.
            let mut plan = staffs.keeper_plan().cloned().unwrap_or_default();

            // How his last request turned out, applied before his standing
            // is read: the manager weighs the man he has been listening to,
            // not the man he was a month ago.
            plan.credit(GoalkeepingDepartment::credibility_delta(&plan, &room, date));

            let head_coach = staffs.head_coach();
            let authority = KeeperCoachAuthority::read(
                staffs.goalkeeper_coach(),
                head_coach,
                plan.credibility(),
            );
            (authority, plan)
        };

        let Some(outcome) = GoalkeepingDepartment::review(&room, &previous, authority, date) else {
            return Vec::new();
        };

        let mut plan = previous;
        let changes = plan.commit(outcome, date);

        let lead_id = self.teams.teams[main_idx]
            .staffs
            .goalkeeping_lead()
            .map(|s| s.id);
        if let Some(lead) = self.teams.teams[main_idx].staffs.goalkeeping_lead_mut() {
            lead.keeper_plan = plan;
        }
        // Exactly one live plan. Without this, a plan left on a previous
        // holder would keep answering `keeper_plan()` after the desk moved.
        if let Some(lead_id) = lead_id {
            for staff in self.teams.teams[main_idx].staffs.iter_mut() {
                if staff.id != lead_id && !staff.keeper_plan.is_empty() {
                    staff.keeper_plan.clear();
                }
            }
        }
        changes
    }

    /// The department's word as a matchday squad hears it. `None` when
    /// nobody has reviewed the room yet, in which case keeper selection
    /// behaves exactly as it always did.
    pub fn keeper_selection_brief(&self, date: NaiveDate) -> Option<KeeperSelectionBrief> {
        let plan = self.keeper_plan()?;
        let brief = KeeperSelectionBrief::from_plan(plan, date);
        if brief.is_silent() { None } else { Some(brief) }
    }

    /// The goalkeeping department's standing plan, wherever it is held.
    pub fn keeper_plan(&self) -> Option<&KeeperRoomPlan> {
        self.teams.main().and_then(|t| t.staffs.keeper_plan())
    }

    /// Academy keepers the department wants in the senior matchday pool.
    ///
    /// The first rung of the real introduction ladder, and the one the sim
    /// had no way of climbing: a young keeper trains and travels with the
    /// first team long before he plays for it, and until now the only route
    /// into a senior squad for a keeper was an injury crisis. Ordered with
    /// the nominated keeper first, and returned as players so the matchday
    /// caller can drop them straight into its reserve pool.
    pub fn keeper_call_ups(&self, main_team_id: u32, date: NaiveDate) -> Vec<&Player> {
        let Some(plan) = self.keeper_plan() else {
            return Vec::new();
        };
        let nominated = plan.nominated(date);

        let mut picks: Vec<(&Player, bool)> = Vec::new();
        for team in self.teams.teams.iter().filter(|t| t.id != main_team_id) {
            for player in team.players.iter() {
                if !player.positions.is_goalkeeper() || player.is_on_loan() {
                    continue;
                }
                let Some(tier) = plan.tier_of(player.id) else {
                    continue;
                };
                let wanted = tier == KeeperTier::Pathway || nominated == Some(player.id);
                if wanted {
                    picks.push((player, nominated == Some(player.id)));
                }
            }
        }
        // The nominated keeper leads: he is the one the department has
        // actually asked for.
        picks.sort_by(|a, b| b.1.cmp(&a.1));
        picks.into_iter().map(|(p, _)| p).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::club::staff::goalkeeping::{KeeperAdvice, KeeperSuccession};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, PlayerAttributes,
        PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, Staff, StaffClubContract, StaffCollection, StaffPosition, StaffStatus,
        StaffStub, Team, TeamBuilder, TeamCollection, TeamReputation, TrainingSchedule,
    };
    use chrono::NaiveTime;

    /// A club with a real keeper room spread across its squads.
    struct Gk;

    impl Gk {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()
        }

        fn keeper(id: u32, age: u8, level: u8) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = level;
            attrs.condition = 9500;
            let mut person = PersonAttributes::default();
            person.professionalism = 15.0;
            person.ambition = 15.0;
            let birth = NaiveDate::from_ymd_opt(2026 - age as i32, 1, 1).unwrap();
            let mut player = PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("K".to_string(), format!("G{id}")))
                .birth_date(birth)
                .country_id(1)
                .attributes(person)
                .skills(PlayerSkills::flat_for_ability(level))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::Goalkeeper,
                        level: 20,
                    }],
                })
                .player_attributes(attrs)
                .build()
                .unwrap();
            player.contract = Some(PlayerClubContract::new(
                10_000,
                NaiveDate::from_ymd_opt(2029, 6, 1).unwrap(),
            ));
            player
        }

        /// A goalkeeping coach who knows his job.
        fn specialist(id: u32) -> Staff {
            let mut staff = StaffStub::default();
            staff.id = id;
            staff.contract = Some(StaffClubContract::new(
                50_000,
                NaiveDate::from_ymd_opt(2029, 6, 1).unwrap(),
                StaffPosition::GoalkeeperCoach,
                StaffStatus::Active,
            ));
            let gk = &mut staff.staff_attributes.goalkeeping;
            gk.shot_stopping = 18;
            gk.handling = 18;
            gk.distribution = 16;
            let knowledge = &mut staff.staff_attributes.knowledge;
            knowledge.judging_player_ability = 17;
            knowledge.judging_player_potential = 17;
            let mental = &mut staff.staff_attributes.mental;
            mental.man_management = 15;
            mental.adaptability = 15;
            staff
        }

        /// A manager, so a club with no specialist still has somebody to
        /// hold the plan.
        fn manager_only(id: u32) -> Staff {
            let mut staff = StaffStub::default();
            staff.id = id;
            staff.contract = Some(StaffClubContract::new(
                80_000,
                NaiveDate::from_ymd_opt(2029, 6, 1).unwrap(),
                StaffPosition::Manager,
                StaffStatus::Active,
            ));
            let mental = &mut staff.staff_attributes.mental;
            mental.man_management = 16;
            mental.adaptability = 15;
            staff
        }

        fn team(id: u32, tt: TeamType, players: Vec<Player>, staffs: Vec<Staff>) -> Team {
            TeamBuilder::new()
                .id(id)
                .league_id(Some(1))
                .club_id(100)
                .name(format!("t{id}"))
                .slug(format!("t{id}"))
                .team_type(tt)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(staffs))
                .reputation(TeamReputation::new(6000, 6000, 6000))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        /// An ageing number one, a deputy in his prime, a veteran third, and
        /// a boy in the under-eighteens: the room the department exists for.
        fn club(with_specialist: bool) -> Club {
            let mut staffs = vec![Gk::manager_only(800)];
            if with_specialist {
                staffs.push(Gk::specialist(900));
            }
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(1_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![
                    Gk::team(
                        10,
                        TeamType::Main,
                        vec![
                            Gk::keeper(1, 34, 150),
                            Gk::keeper(2, 28, 138),
                            Gk::keeper(3, 37, 118),
                        ],
                        staffs,
                    ),
                    Gk::team(11, TeamType::U18, vec![Gk::keeper(4, 18, 106)], Vec::new()),
                ]),
                ClubFacilities::default(),
            )
        }
    }

    #[test]
    fn the_review_reads_every_squad_and_writes_the_order_onto_the_specialist() {
        let mut club = Gk::club(true);
        assert!(club.keeper_plan().is_none(), "no opinion before the review");

        let changes = club.review_goalkeeping_department(Gk::date());
        assert!(!changes.is_empty(), "every standing is fresh news");

        let plan = club.keeper_plan().expect("the review wrote a plan");
        assert_eq!(plan.number_one(), Some(1));
        assert_eq!(plan.deputy(), Some(2));
        assert_eq!(plan.third(), Some(3));
        assert!(
            plan.tier_of(4).is_some(),
            "the under-eighteens keeper is in the plan too"
        );
        assert!(
            plan.authority() > 0.5,
            "a strong specialist under an open manager carries real weight: {}",
            plan.authority()
        );
    }

    #[test]
    fn the_review_stands_for_a_month() {
        let mut club = Gk::club(true);
        club.review_goalkeeping_department(Gk::date());
        let again = club.review_goalkeeping_department(Gk::date());
        assert!(
            again.is_empty(),
            "a plan revised today is not revised again tomorrow"
        );
    }

    #[test]
    fn the_matchday_brief_carries_the_order_and_the_nomination() {
        let mut club = Gk::club(true);
        club.review_goalkeeping_department(Gk::date());

        let brief = club
            .keeper_selection_brief(Gk::date())
            .expect("a reviewed room has something to say");
        assert_eq!(brief.number_one, Some(1));
        assert_eq!(brief.deputy, Some(2));
        assert!(brief.authority > 0.0);
        assert_eq!(brief.nominated, Some(4));
        assert!(
            brief.selection_adjustment(4, 0.20) > brief.selection_adjustment(1, 0.20),
            "in a fixture the club can afford, the boy is the pick"
        );
    }

    #[test]
    fn the_nominated_academy_keeper_joins_the_senior_matchday_pool() {
        let mut club = Gk::club(true);
        club.review_goalkeeping_department(Gk::date());

        let called_up: Vec<u32> = club
            .keeper_call_ups(10, Gk::date())
            .iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(called_up, vec![4], "he travels with the first team");
    }

    #[test]
    fn an_ageing_number_one_opens_a_succession_and_the_veteran_third_keeps_his_job() {
        let mut club = Gk::club(true);
        club.review_goalkeeping_department(Gk::date());
        let plan = club.keeper_plan().unwrap();

        assert!(plan.succession() >= KeeperSuccession::Watch);
        assert!(
            plan.advice_for(1)
                .any(|r| r.advice == KeeperAdvice::OpenTheSuccession),
            "a thirty-four-year-old number one is a question worth asking"
        );
        assert!(
            plan.advice_for(3)
                .any(|r| r.advice == KeeperAdvice::KeepHimAsTheSeniorVoice),
            "the veteran third keeper has a job, and it is not being sold"
        );
    }

    #[test]
    fn a_club_with_no_backroom_at_all_has_no_department() {
        let mut club = Gk::club(false);
        club.teams.teams[0].staffs = StaffCollection::new(Vec::new());
        let changes = club.review_goalkeeping_department(Gk::date());
        assert!(changes.is_empty());
        assert!(club.keeper_plan().is_none());
        assert!(club.keeper_selection_brief(Gk::date()).is_none());
        assert!(club.keeper_call_ups(10, Gk::date()).is_empty());
    }

    #[test]
    fn a_nomination_the_manager_acted_on_raises_the_department_standing() {
        let mut club = Gk::club(true);
        club.review_goalkeeping_department(Gk::date());
        let before = club.keeper_plan().unwrap().credibility();
        assert_eq!(club.keeper_plan().unwrap().nominated(Gk::date()), Some(4));

        // He played. A month later the department is reviewed again.
        for team in club.teams.teams.iter_mut() {
            if let Some(player) = team.players.players.iter_mut().find(|p| p.id == 4) {
                player.statistics.played = 2;
            }
        }
        let next = Gk::date() + chrono::Duration::days(31);
        club.review_goalkeeping_department(next);

        assert!(
            club.keeper_plan().unwrap().credibility() > before,
            "a request that turned into minutes is why he gets listened to next time"
        );
    }

    #[test]
    fn the_pecking_order_survives_the_plan_changing_hands() {
        // Reviewed by a manager with no specialist on the books.
        let mut club = Gk::club(false);
        club.review_goalkeeping_department(Gk::date());
        let order = club.keeper_plan().map(|p| p.number_one());
        assert_eq!(order, Some(Some(1)));

        // The club then hires a goalkeeping coach: the desk moves, the
        // club's number one does not.
        club.teams.teams[0].staffs.push(Gk::specialist(900));
        assert_eq!(
            club.keeper_plan().map(|p| p.number_one()),
            Some(Some(1)),
            "a change of holder is not a change of number one"
        );

        let next = Gk::date() + chrono::Duration::days(31);
        club.review_goalkeeping_department(next);
        assert_eq!(club.keeper_plan().map(|p| p.number_one()), Some(Some(1)));

        let holders = club.teams.teams[0]
            .staffs
            .iter()
            .filter(|s| !s.keeper_plan.is_empty())
            .count();
        assert_eq!(holders, 1, "exactly one live plan at the club");
    }
}
