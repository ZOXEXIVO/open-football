use crate::DatabaseEntity;
use core::league::{DayMonthPeriod, League, LeagueFinancials, LeagueGroup, LeagueSettings};
use core::{Club, TeamType};
use std::str::FromStr;

use super::DatabaseGenerator;

impl DatabaseGenerator {
    pub(super) fn generate_leagues(country_id: u32, country_reputation: u16, data: &DatabaseEntity) -> Vec<League> {
        data
            .leagues
            .iter()
            .filter(|l| l.country_id == country_id)
            .map(|league| {
                let financials = LeagueFinancials::from_reputation_and_tier(
                    league.reputation, league.tier, country_reputation,
                );
                let settings = LeagueSettings {
                    season_starting_half: DayMonthPeriod {
                        from_day: league.settings.season_starting_half.from_day,
                        from_month: league.settings.season_starting_half.from_month,
                        to_day: league.settings.season_starting_half.to_day,
                        to_month: league.settings.season_starting_half.to_month,
                    },
                    season_ending_half: DayMonthPeriod {
                        from_day: league.settings.season_ending_half.from_day,
                        from_month: league.settings.season_ending_half.from_month,
                        to_day: league.settings.season_ending_half.to_day,
                        to_month: league.settings.season_ending_half.to_month,
                    },
                    tier: league.tier,
                    promotion_spots: league.promotion_spots,
                    relegation_spots: league.relegation_spots,
                    league_group: league.league_group.as_ref().map(|g| LeagueGroup {
                        name: g.name.clone(),
                        competition: g.competition.clone(),
                        total_groups: g.total_groups,
                    }),
                };

                let mut l = League::new(league.id, league.name.clone(), league.slug.clone(), league.country_id, league.reputation, settings, false);
                l.financials = financials;
                l
            })
            .collect()
    }

    pub(super) fn create_subteams_leagues(country_id: u32, clubs: &mut [Club], leagues: &mut Vec<League>, data: &DatabaseEntity) {
        // Build a map: club_id → parent league_id (from the club's Main team)
        let club_league_map: Vec<(u32, u32)> = clubs
            .iter()
            .filter_map(|club| {
                let main_league_id = club.teams.teams
                    .iter()
                    .find(|t| t.team_type == TeamType::Main)
                    .and_then(|t| t.league_id)?;
                Some((club.id, main_league_id))
            })
            .collect();

        // Snapshot parent leagues to create subleagues per configured team type
        let parent_leagues: Vec<(u32, String, String, u16, LeagueSettings)> = leagues
            .iter()
            .map(|l| (l.id, l.name.clone(), l.slug.clone(), l.reputation, l.settings.clone()))
            .collect();

        for (parent_id, parent_name, parent_slug, parent_rep, parent_settings) in &parent_leagues {
            // Find sub_leagues_competitions config from the league entity
            let team_types: Vec<TeamType> = data.leagues
                .iter()
                .find(|l| l.id == *parent_id)
                .map(|l| {
                    l.sub_leagues_competitions.iter()
                        .filter_map(|s| TeamType::from_str(s).ok())
                        .collect()
                })
                .unwrap_or_default();

            for team_type in &team_types {
                // Check if any club in this parent league has this team type
                let has_type = clubs.iter().any(|club| {
                    club_league_map.iter().any(|(cid, lid)| *cid == club.id && lid == parent_id)
                        && club.teams.teams.iter().any(|t| t.team_type == *team_type)
                });

                if !has_type {
                    continue;
                }

                // Deterministic league ID offset per team type
                let type_offset = match team_type {
                    TeamType::U18 => 100000,
                    TeamType::U19 => 110000,
                    TeamType::U20 => 120000,
                    TeamType::U21 => 130000,
                    TeamType::U23 => 140000,
                    _ => continue,
                };

                let youth_league_id = parent_id + type_offset;
                let youth_reputation = (parent_rep / 10).max(100);
                let type_label = format!("{}", team_type);
                let type_slug = type_label.to_lowercase();

                let youth_settings = LeagueSettings {
                    season_starting_half: parent_settings.season_starting_half,
                    season_ending_half: parent_settings.season_ending_half,
                    tier: 99,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                };

                let youth_league = League::new(
                    youth_league_id,
                    format!("{} {}", parent_name, type_label),
                    format!("{}-{}", parent_slug, type_slug),
                    country_id,
                    youth_reputation,
                    youth_settings,
                    true,
                );

                leagues.push(youth_league);

                // Assign matching teams to this youth league
                for club in clubs.iter_mut() {
                    let is_in_parent = club_league_map.iter().any(|(cid, lid)| *cid == club.id && lid == parent_id);
                    if !is_in_parent {
                        continue;
                    }
                    for team in &mut club.teams.teams {
                        if team.team_type == *team_type {
                            team.league_id = Some(youth_league_id);
                        }
                    }
                }
            }
        }
    }
}
