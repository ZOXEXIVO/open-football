use std::collections::HashMap;

use crate::loaders::{
    CountryCorridorEntity, CountryTransferProfileEntity, NationalCompetitionEntity,
};
use core::transfers::{CorridorWeight, CountryTransferProfile, DiasporaShare};
use core::{
    CompetitionScope, NationalCompetitionConfig, NationalTeamLevel, QualifyingConfig,
    QualifyingPosition, QualifyingZoneConfig, ScheduleConfig, ScheduleDate, TournamentConfig,
};

/// Convert a database NationalCompetitionEntity to a runtime NationalCompetitionConfig
pub fn convert_national_competition(
    entity: &NationalCompetitionEntity,
) -> NationalCompetitionConfig {
    let scope = match entity.scope.as_str() {
        "global" => CompetitionScope::Global,
        "continental" => CompetitionScope::Continental,
        _ => CompetitionScope::Continental,
    };

    // Missing / null / "senior" => Senior (backward-compatible default).
    let team_level = match entity.team_level.as_deref() {
        Some("u21") => NationalTeamLevel::Under21,
        _ => NationalTeamLevel::Senior,
    };

    let qualifying = QualifyingConfig {
        zones: entity
            .qualifying
            .zones
            .iter()
            .map(|z| {
                let qualifiers_per_group = z
                    .qualifiers_per_group
                    .iter()
                    .map(|pos| match pos.as_str() {
                        "winner" => QualifyingPosition::Winner,
                        "runner_up" => QualifyingPosition::RunnerUp,
                        _ => QualifyingPosition::Winner,
                    })
                    .collect();

                QualifyingZoneConfig {
                    continent_id: z.continent_id,
                    spots: z.spots,
                    max_groups: z.max_groups,
                    teams_per_group_target: z.teams_per_group_target,
                    qualifiers_per_group,
                    best_runners_up: z.best_runners_up,
                    best_third_placed: z.best_third_placed,
                }
            })
            .collect(),
    };

    let tournament = TournamentConfig {
        total_teams: entity.tournament.total_teams,
        group_count: entity.tournament.group_count,
        teams_per_group: entity.tournament.teams_per_group,
        advance_per_group: entity.tournament.advance_per_group,
        best_third_placed: entity.tournament.best_third_placed,
    };

    let schedule = ScheduleConfig {
        qualifying_dates: entity
            .schedule
            .qualifying_dates
            .iter()
            .map(|sd| ScheduleDate {
                month: sd.month,
                day: sd.day,
                year_offset: sd.year_offset,
            })
            .collect(),
        tournament_group_dates: entity
            .schedule
            .tournament_group_dates
            .iter()
            .map(|sd| ScheduleDate {
                month: sd.month,
                day: sd.day,
                year_offset: sd.year_offset,
            })
            .collect(),
        tournament_knockout_dates: entity
            .schedule
            .tournament_knockout_dates
            .iter()
            .map(|sd| ScheduleDate {
                month: sd.month,
                day: sd.day,
                year_offset: sd.year_offset,
            })
            .collect(),
    };

    NationalCompetitionConfig {
        id: entity.id,
        name: entity.name.clone(),
        short_name: entity.short_name.clone(),
        scope,
        continent_id: entity.continent_id,
        team_level,
        cycle_years: entity.cycle_years,
        cycle_offset: entity.cycle_offset,
        qualifying,
        tournament,
        schedule,
    }
}

/// Continent id used by the bundled UEFA U21 example competition.
const UEFA_CONTINENT_ID: u32 = 1;

/// Built-in UEFA U21 Championship config.
///
/// The compiled `database.db` predates the `team_level` field, so there
/// is no data-driven U21 competition to load yet. This helper supplies a
/// single example so U21 squads, schedules, and stats have something to
/// exercise in-sim. Two-year cycle starting on the next even year;
/// qualifying matchdays land inside the regular September/October/
/// November/March international breaks, finals in the June window.
pub fn uefa_u21_championship_config() -> NationalCompetitionConfig {
    let qualifying = QualifyingConfig {
        zones: vec![QualifyingZoneConfig {
            continent_id: UEFA_CONTINENT_ID,
            spots: 16,
            max_groups: 8,
            teams_per_group_target: 5,
            qualifiers_per_group: vec![QualifyingPosition::Winner, QualifyingPosition::RunnerUp],
            best_runners_up: 0,
            best_third_placed: 0,
        }],
    };

    let tournament = TournamentConfig {
        total_teams: 16,
        group_count: 4,
        teams_per_group: 4,
        advance_per_group: 2,
        best_third_placed: 0,
    };

    let qd = |month: u32, day: u32, year_offset: i32| ScheduleDate {
        month,
        day,
        year_offset,
    };

    let schedule = ScheduleConfig {
        // Qualifying matchdays inside the regular break windows
        // (Sep 4-12, Oct 9-17, Nov 13-21, Mar 20-28).
        qualifying_dates: vec![
            qd(9, 6, 0),
            qd(9, 9, 0),
            qd(10, 11, 0),
            qd(10, 14, 0),
            qd(11, 15, 0),
            qd(11, 18, 0),
            qd(3, 22, 1),
            qd(3, 25, 1),
        ],
        // Finals in June of the tournament year (qualifying_start + 2).
        tournament_group_dates: vec![qd(6, 11, 2), qd(6, 14, 2), qd(6, 17, 2)],
        tournament_knockout_dates: vec![
            qd(6, 24, 2),
            qd(6, 28, 2),
            qd(7, 2, 2),
            qd(7, 5, 2),
            qd(7, 8, 2),
        ],
    };

    NationalCompetitionConfig {
        id: 9_001,
        name: "UEFA U21 Championship".to_string(),
        short_name: "U21 EURO".to_string(),
        scope: CompetitionScope::Continental,
        continent_id: Some(UEFA_CONTINENT_ID),
        team_level: NationalTeamLevel::Under21,
        cycle_years: 2,
        cycle_offset: 0,
        qualifying,
        tournament,
        schedule,
    }
}

/// Turn one country's shipped transfer card into the runtime profile.
///
/// Two things happen here and nowhere else: country CODES become country
/// IDS (the data files name each other by code; the simulator works in ids),
/// and each corridor list is normalised by its own maximum so a country's
/// top corridor reads 1.0 whatever scale it was authored on. An entry naming
/// a country this database does not carry is dropped — silently, because the
/// compiler already fails the build on an unknown code and a drop here can
/// only mean a database older than its cards.
pub fn convert_country_transfers(
    entity: Option<&CountryTransferProfileEntity>,
    country_id_by_code: &HashMap<String, u32>,
) -> CountryTransferProfile {
    let Some(entity) = entity else {
        return CountryTransferProfile::default();
    };

    let corridors = |rows: &[CountryCorridorEntity]| -> Vec<CorridorWeight> {
        let max = rows.iter().map(|r| r.weight).fold(0.0_f32, f32::max);
        if max <= 0.0 {
            return Vec::new();
        }
        rows.iter()
            .filter_map(|row| {
                let country_id = *country_id_by_code.get(&row.country.to_ascii_lowercase())?;
                Some(CorridorWeight {
                    country_id,
                    weight: (row.weight / max).clamp(0.0, 1.0),
                    money: row.is_money(),
                })
            })
            .collect()
    };

    CountryTransferProfile {
        import: corridors(&entity.import),
        export: corridors(&entity.export),
        diaspora: entity
            .diaspora
            .iter()
            .filter_map(|row| {
                let country_id = *country_id_by_code.get(&row.country.to_ascii_lowercase())?;
                Some(DiasporaShare {
                    country_id,
                    share: row.share.clamp(0.0, 1.0),
                })
            })
            .collect(),
        foreign_share: entity.foreign_share.clamp(0.0, 1.0),
        authored: entity.source == "authored",
    }
}

#[cfg(test)]
mod transfer_profile_tests {
    use super::*;

    fn codes() -> HashMap<String, u32> {
        [("br", 1u32), ("tr", 2), ("sa", 3)]
            .into_iter()
            .map(|(code, id)| (code.to_string(), id))
            .collect()
    }

    #[test]
    fn weights_normalise_to_the_list_maximum() {
        let entity = CountryTransferProfileEntity {
            code: "tr".into(),
            source: "authored".into(),
            import: vec![
                CountryCorridorEntity {
                    country: "br".into(),
                    weight: 30.0,
                    kind: None,
                },
                CountryCorridorEntity {
                    country: "sa".into(),
                    weight: 6.0,
                    kind: Some("money".into()),
                },
            ],
            export: Vec::new(),
            diaspora: Vec::new(),
            foreign_share: 0.58,
        };
        let profile = convert_country_transfers(Some(&entity), &codes());
        assert_eq!(profile.import_weight(1), Some(1.0));
        assert_eq!(profile.import_weight(3), Some(0.2));
        assert!(profile.is_money_corridor(3));
        assert!(!profile.is_money_corridor(1));
        assert!(profile.authored);
    }

    #[test]
    fn an_absent_card_yields_the_neutral_profile() {
        let profile = convert_country_transfers(None, &codes());
        assert!(profile.import.is_empty());
        assert_eq!(profile.foreign_share, 0.0);
    }

    #[test]
    fn entries_naming_an_unknown_country_are_dropped() {
        let entity = CountryTransferProfileEntity {
            code: "tr".into(),
            source: "derived".into(),
            import: vec![
                CountryCorridorEntity {
                    country: "br".into(),
                    weight: 10.0,
                    kind: None,
                },
                CountryCorridorEntity {
                    country: "zz".into(),
                    weight: 8.0,
                    kind: None,
                },
            ],
            export: Vec::new(),
            diaspora: Vec::new(),
            foreign_share: 0.3,
        };
        let profile = convert_country_transfers(Some(&entity), &codes());
        assert_eq!(profile.import.len(), 1);
    }
}
