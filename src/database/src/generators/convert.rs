use crate::loaders::NationalCompetitionEntity;
use core::{
    CompetitionScope, NationalCompetitionConfig, QualifyingConfig, QualifyingPosition,
    QualifyingZoneConfig, ScheduleConfig, ScheduleDate, TournamentConfig,
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
        cycle_years: entity.cycle_years,
        cycle_offset: entity.cycle_offset,
        qualifying,
        tournament,
        schedule,
    }
}
