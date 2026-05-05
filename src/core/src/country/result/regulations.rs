//! Per-season squad registration enforcement.
//!
//! Runs once on the season-start tick. For each club in the country,
//! we apply the country's `foreign_player_limit` to the main team's
//! roster — the weakest excess foreigners get marked as
//! Unregistered (`PlayerStatusType::Unr`) and receive the
//! `SquadRegistrationOmitted` happiness event so the player feed
//! reflects the snub.
//!
//! Salary cap and homegrown requirements are surfaced as warnings via
//! the `CountryRegulations` helpers but not yet auto-fixed at the
//! squad level — they tie into transfer-time enforcement and the FFP
//! lifecycle, both of which live closer to the financial pipeline.

use super::CountryResult;
use crate::PlayerStatusType;
use crate::club::HappinessEventType;
use crate::simulator::SimulatorData;
use chrono::NaiveDate;
use log::debug;

impl CountryResult {
    /// Walk every club in `country_id`'s main team and drop the
    /// weakest foreign surplus from the registered squad. The omitted
    /// player gets:
    ///   * `PlayerStatusType::Unr` added to their statuses (squad
    ///     selection already filters Unr out via the existing status
    ///     gate).
    ///   * A `SquadRegistrationOmitted` happiness event so the
    ///     feedback is durable in the player history.
    pub(super) fn enforce_squad_registration(
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) {
        // Snapshot the country's foreign-limit configuration up front
        // so we can release the read borrow before mutating clubs.
        let (foreign_limit, club_country_id) = match data.country(country_id) {
            Some(c) => (c.regulations.foreign_player_limit, c.id),
            None => return,
        };
        if foreign_limit.is_none() {
            // No rule configured — nothing to enforce.
            return;
        }

        let Some(country) = data.country_mut(country_id) else {
            return;
        };

        for club in &mut country.clubs {
            // Only the main team is registered with the league. Reserve /
            // youth squads have their own rosters and aren't filtered.
            let Some(main_team) = club.teams.main_mut() else {
                continue;
            };
            // Build a borrow of player references to feed the rule helper.
            let player_refs: Vec<&crate::Player> =
                main_team.players.players.iter().collect();
            let omitted_ids =
                country.regulations.omitted_for_foreign_limit(&player_refs, club_country_id);
            drop(player_refs);

            if omitted_ids.is_empty() {
                continue;
            }
            debug!(
                "📋 Squad registration: club {} omits {} foreign players",
                club.id,
                omitted_ids.len()
            );
            for player in &mut main_team.players.players {
                if !omitted_ids.contains(&player.id) {
                    continue;
                }
                if !player.statuses.get().contains(&PlayerStatusType::Unr) {
                    player.statuses.add(date, PlayerStatusType::Unr);
                }
                player
                    .happiness
                    .add_event_default_with_cooldown(
                        HappinessEventType::SquadRegistrationOmitted,
                        365,
                    );
            }
        }
        // Suppress unused-variable warning when the path stays generic.
        let _ = foreign_limit;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::builder::PlayerBuilder;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        Club, ClubColors, ClubFinances, ClubStatus, Country, CountryRegulations, PersonAttributes,
        PlayerAttributes, PlayerCollection, PlayerPositions, PlayerSkills, StaffCollection,
        TeamBuilder, TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn make_player(id: u32, country_id: u32, ability: u8) -> crate::Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = ability;
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(d(1995, 1, 1))
            .country_id(country_id)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions { positions: vec![] })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    fn make_training_schedule() -> TrainingSchedule {
        use chrono::NaiveTime;
        TrainingSchedule::new(
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
        )
    }

    #[test]
    fn omitted_for_foreign_limit_picks_weakest_excess() {
        let mut regs = CountryRegulations::new();
        regs.foreign_player_limit = Some(1);
        let players = vec![
            make_player(1, 1, 100),  // domestic — keeps slot
            make_player(2, 99, 70),  // foreign weak — drop
            make_player(3, 99, 150), // foreign strong — keep
        ];
        let refs: Vec<&crate::Player> = players.iter().collect();
        let omitted = regs.omitted_for_foreign_limit(&refs, 1);
        assert_eq!(omitted, vec![2]);
    }

    /// Smoke test that the season-start hook actually walks a country's
    /// clubs without panicking and applies the Unr status to omitted
    /// players. Detailed semantics of "who gets dropped" are covered
    /// by the rules-helper tests above.
    #[test]
    fn enforce_squad_registration_marks_omitted_player_as_unr() {
        // Country with a strict foreign limit of 0 — every non-domestic
        // player must be omitted.
        let mut regulations = CountryRegulations::new();
        regulations.foreign_player_limit = Some(0);

        let domestic = make_player(1, 1, 100);
        let foreigner = make_player(2, 99, 90);
        let team = TeamBuilder::new()
            .id(10)
            .league_id(Some(1))
            .club_id(100)
            .name("T".to_string())
            .slug("t".to_string())
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(vec![domestic, foreigner]))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(make_training_schedule())
            .build()
            .unwrap();
        let club = Club::new(
            100,
            "Club".to_string(),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![team]),
            crate::ClubFacilities::default(),
        );
        let league = League::new(
            1,
            "L".to_string(),
            "l".to_string(),
            1,
            5000,
            LeagueSettings {
                season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                tier: 1,
                promotion_spots: 0,
                relegation_spots: 0,
                league_group: None,
            },
            false,
        );
        let country = Country::builder()
            .id(1)
            .code("EN".to_string())
            .slug("en".to_string())
            .name("England".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(vec![league]))
            .clubs(vec![club])
            .regulations(regulations)
            .build()
            .unwrap();

        // Build SimulatorData with this single country.
        let mut sim = SimulatorData::new(
            chrono::NaiveDateTime::new(
                d(2032, 8, 1),
                chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            ),
            vec![crate::continent::Continent::new(
                1,
                "Europe".to_string(),
                vec![country],
                Vec::new(),
            )],
            crate::competitions::GlobalCompetitions::new(Vec::new()),
        );

        CountryResult::enforce_squad_registration(&mut sim, 1, d(2032, 8, 1));

        // Foreigner (id 2) must now carry Unr; domestic (id 1) does not.
        let foreigner = sim.player(2).unwrap();
        assert!(foreigner.statuses.get().contains(&PlayerStatusType::Unr));
        let domestic = sim.player(1).unwrap();
        assert!(!domestic.statuses.get().contains(&PlayerStatusType::Unr));
    }
}
