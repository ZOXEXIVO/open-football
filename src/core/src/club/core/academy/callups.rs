use crate::Club;
use crate::TeamType;
use crate::club::academy::ClubAcademy;
use crate::club::player::language::{Language, PlayerLanguage};
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::deal::reason::TransferReason;
use crate::transfers::{CompletedTransfer, TransferType};
use chrono::NaiveDate;
use log::debug;

impl Club {
    /// Weekly rescue for a youth squad that cannot put a team on the
    /// pitch.
    ///
    /// The academy → youth pathway used to have exactly one door and it
    /// opened one morning a year, on the country's season-start day.
    /// That is fine for a club whose U18 already exists; it is useless
    /// for the large majority that begin a new world with an empty or
    /// half-empty one, because the source data carries no youth records
    /// for them. Those clubs sat on a full academy — thirty-odd boys,
    /// training every week — while their U18 played short, in some
    /// countries for eleven months, because the season had started three
    /// weeks before the world did.
    ///
    /// So: when a youth squad is under eleven, the club promotes from
    /// its own academy the way a real one does, down to age fourteen,
    /// and only as far as a fielding eleven plus three substitutes.
    /// Everything about it is bounded — one rescue a month, never past
    /// the academy's own [`ClubAcademy::call_up_capacity`], and it stops
    /// the moment the squad can field a team. The seasonal graduation
    /// round is untouched: this is the club not forfeiting, not a second
    /// throughput channel.
    ///
    /// Returns the transfer records so the country layer files them the
    /// same way it files graduation day.
    pub(in crate::club::core) fn process_youth_emergency_callups(
        &mut self,
        date: NaiveDate,
        country_code: &str,
    ) -> Vec<CompletedTransfer> {
        let mut budget = self.academy.emergency_allowance(date);
        if budget == 0 {
            return Vec::new();
        }

        let mut transfers = Vec::new();
        let main_team_name = self
            .teams
            .main()
            .map(|t| t.name.clone())
            .unwrap_or_else(|| self.name.clone());

        // Largest hole first. The old order was "lowest bracket first",
        // which is the right instinct about WHERE academy boys belong and
        // the wrong rule for a rescue: a club whose U18 is one man short
        // and whose U20 has four spent the whole budget on the U18 and
        // left the U20 unable to field a side — and a squad that never
        // reaches eleven is the squad the promotion guard then refuses to
        // let anybody out of. The deficit is the severity ladder; ties keep
        // the old lowest-bracket preference.
        let mut brackets: Vec<(TeamType, usize, usize)> = TeamType::YOUTH_PROGRESSION
            .iter()
            .filter_map(|team_type| {
                let idx = self.teams.index_of_type(*team_type)?;
                let squad = self.teams.teams[idx].players.len();
                (squad < ClubAcademy::EMERGENCY_YOUTH_SIZE).then_some((*team_type, idx, squad))
            })
            .collect();
        brackets.sort_by_key(|(_, _, squad)| *squad);

        for (team_type, idx, squad) in brackets {
            if budget == 0 {
                break;
            }
            // Continuous in the size of the hole: an empty squad gets
            // fourteen, a squad of nine gets five. No severity ladder —
            // the deficit already is one.
            let wanted = ClubAcademy::EMERGENCY_YOUTH_TARGET
                .saturating_sub(squad)
                .min(budget);
            let called_up = self.academy.emergency_call_up(date, wanted);
            if called_up.is_empty() {
                continue;
            }
            budget = budget.saturating_sub(called_up.len());

            debug!(
                "academy {}: {} emergency call-ups into {:?} (squad was {})",
                self.name,
                called_up.len(),
                team_type,
                squad
            );

            for mut player in called_up {
                if player.languages.is_empty() {
                    player.languages = Language::from_country_code(country_code)
                        .into_iter()
                        .map(PlayerLanguage::native)
                        .collect();
                }

                transfers.push(
                    CompletedTransfer::new(
                        player.id,
                        player.full_name.to_string(),
                        0,
                        0,
                        "Academy".to_string(),
                        self.id,
                        main_team_name.clone(),
                        date,
                        CurrencyValue::new(0.0, Currency::Usd),
                        TransferType::Free,
                    )
                    .with_reason(TransferReason::key(
                        "signing_reason_academy_emergency_callup",
                    )),
                );
                self.teams.teams[idx].players.add(player);
            }
        }

        if !transfers.is_empty() {
            self.academy.record_emergency_call_up(date);
        }
        transfers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Player;
    use crate::academy::ClubAcademy;
    use crate::shared::Location;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PeopleNameGeneratorData,
        PlayerCollection, PlayerGenerator, PlayerPositionType, StaffCollection, TeamBuilder,
        TeamCollection, TeamReputation, TrainingSchedule,
    };
    use chrono::Datelike;

    struct Fixture;

    impl Fixture {
        fn date() -> NaiveDate {
            // A Monday well away from any season boundary, so nothing but
            // the weekly emergency pass could be moving players.
            NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()
        }

        fn names() -> PeopleNameGeneratorData {
            PeopleNameGeneratorData {
                first_names: vec!["Test".into()],
                last_names: vec!["Prospect".into()],
                nicknames: vec![],
            }
        }

        fn prospect(age: u8) -> Player {
            let date = Self::date();
            let mut player = PlayerGenerator::generate(
                1,
                date,
                PlayerPositionType::MidfielderCenter,
                10,
                &Self::names(),
            );
            player.birth_date = NaiveDate::from_ymd_opt(date.year() - age as i32, 1, 1).unwrap();
            player.player_attributes.condition = 8500;
            player.player_attributes.jadedness = 0;
            player.player_attributes.is_injured = false;
            player
        }

        fn training_schedule() -> TrainingSchedule {
            use chrono::NaiveTime;
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            )
        }

        fn team(id: u32, name: &str, team_type: TeamType, players: Vec<Player>) -> crate::Team {
            TeamBuilder::new()
                .id(id)
                .league_id(Some(1))
                .club_id(100)
                .name(name.to_string())
                .slug(name.to_lowercase())
                .team_type(team_type)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(Self::training_schedule())
                .build()
                .unwrap()
        }

        /// Main + U18 + U20 — the commonest shape in the shipped world,
        /// and the one the annual round only ever feeds at one end.
        fn club(u18: usize, u20: usize, academy_ages: &[u8]) -> Club {
            let mut academy = ClubAcademy::new(8);
            for age in academy_ages {
                academy.players.add(Self::prospect(*age));
            }
            let teams = vec![
                Self::team(10, "Main", TeamType::Main, Vec::new()),
                Self::team(
                    11,
                    "U18",
                    TeamType::U18,
                    (0..u18).map(|_| Self::prospect(17)).collect(),
                ),
                Self::team(
                    12,
                    "U20",
                    TeamType::U20,
                    (0..u20).map(|_| Self::prospect(19)).collect(),
                ),
            ];
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                academy,
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(teams),
                ClubFacilities::default(),
            )
        }

        fn squad(club: &Club, team_type: TeamType) -> usize {
            club.teams
                .teams
                .iter()
                .find(|t| t.team_type == team_type)
                .map(|t| t.players.len())
                .unwrap_or(0)
        }
    }

    #[test]
    fn empty_youth_squad_is_filled_from_the_academy_the_same_week() {
        // The bug this exists for: a new world whose U18 has no players
        // and whose country's season started three weeks before the
        // simulation did, leaving a full academy and an empty team sheet
        // for eleven months.
        let date = Fixture::date();
        let mut club = Fixture::club(0, 12, &[15; 40]);

        let transfers = club.process_youth_emergency_callups(date, "en");

        assert_eq!(
            Fixture::squad(&club, TeamType::U18),
            ClubAcademy::EMERGENCY_YOUTH_TARGET,
            "an empty U18 is topped up to a fielding eleven plus subs"
        );
        assert_eq!(transfers.len(), ClubAcademy::EMERGENCY_YOUTH_TARGET);
        assert!(
            transfers
                .iter()
                .all(|t| t.reason.key == "signing_reason_academy_emergency_callup"),
            "the history line must read as an emergency call-up, not graduation day"
        );
        assert_eq!(
            Fixture::squad(&club, TeamType::U20),
            12,
            "a U20 that can already field a team is left alone"
        );
    }

    #[test]
    fn a_healthy_youth_squad_is_left_alone() {
        let date = Fixture::date();
        let mut club = Fixture::club(11, 14, &[15; 40]);
        let before = club.academy.players.players.len();

        let transfers = club.process_youth_emergency_callups(date, "en");

        assert!(transfers.is_empty(), "eleven players can field a team");
        assert_eq!(club.academy.players.players.len(), before);
    }

    #[test]
    fn both_short_squads_are_fed_lowest_bracket_first() {
        let date = Fixture::date();
        let mut club = Fixture::club(0, 0, &[15; 60]);

        club.process_youth_emergency_callups(date, "en");

        assert_eq!(
            Fixture::squad(&club, TeamType::U18),
            ClubAcademy::EMERGENCY_YOUTH_TARGET
        );
        assert_eq!(
            Fixture::squad(&club, TeamType::U20),
            ClubAcademy::EMERGENCY_YOUTH_TARGET,
            "the U20 gets the rest of the month's budget"
        );
    }

    #[test]
    fn the_rescue_runs_once_a_month_however_often_it_is_asked() {
        let date = Fixture::date();
        let mut club = Fixture::club(0, 12, &[15; 40]);

        assert!(!club.process_youth_emergency_callups(date, "en").is_empty());
        let after_first = club.academy.players.players.len();

        // Next Monday, and the one after: even a squad pushed straight
        // back under the line waits for the new month rather than
        // draining the academy every week.
        club.teams.teams[1].players.players.clear();
        for next in [
            NaiveDate::from_ymd_opt(2026, 9, 14).unwrap(),
            NaiveDate::from_ymd_opt(2026, 9, 21).unwrap(),
        ] {
            assert!(
                club.process_youth_emergency_callups(next, "en").is_empty(),
                "the academy answers one emergency a month"
            );
            assert_eq!(club.academy.players.players.len(), after_first);
        }

        let next_month = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        assert!(
            !club
                .process_youth_emergency_callups(next_month, "en")
                .is_empty(),
            "the budget reopens the following month"
        );
    }

    #[test]
    fn a_club_with_nothing_left_to_give_does_not_invent_players() {
        // Academy already at its bootstrap line: the rescue must decline
        // rather than drain it and let the backfill mint a year group.
        let date = Fixture::date();
        let mut club = Fixture::club(0, 0, &[15; 8]);
        let before = club.academy.players.players.len();

        let transfers = club.process_youth_emergency_callups(date, "en");

        assert!(transfers.is_empty());
        assert_eq!(club.academy.players.players.len(), before);
        assert_eq!(Fixture::squad(&club, TeamType::U18), 0);
    }
}
