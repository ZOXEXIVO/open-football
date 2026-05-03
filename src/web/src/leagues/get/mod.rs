pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CSS_VERSION};
use crate::common::slug::player_history_slug;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Duration;
use core::league::ScheduleTour;
use core::r#match::GoalDetail;
use core::r#match::player::statistics::MatchStatisticType;
use itertools::*;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct LeagueGetRequest {
    pub lang: String,
    pub league_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "leagues/get/index.html")]
pub struct LeagueGetTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: I18n,
    pub lang: String,
    pub league_slug: String,
    pub table_rows: Vec<LeagueTableRow>,
    pub current_tour_schedule: Vec<TourSchedule>,
    pub competition_reputation: Vec<CompetitionReputationItem>,
    pub top_scorers: Vec<LeaguePlayerStatItem>,
    pub top_assisters: Vec<LeaguePlayerStatItem>,
    pub top_rated: Vec<LeaguePlayerStatItem>,
}

pub struct LeaguePlayerStatItem {
    pub player_slug: String,
    pub player_name: String,
    pub team_name: String,
    pub team_slug: String,
    pub played: u16,
    pub stat_value: String,
}

pub struct CompetitionReputationItem {
    pub league_name: String,
    pub league_slug: String,
    pub country_name: String,
    pub country_code: String,
}

pub struct TourSchedule {
    pub date: String,
    pub matches: Vec<LeagueScheduleItem>,
}

pub struct LeagueScheduleItem {
    pub match_id: String,
    pub home_team_name: String,
    pub home_team_slug: String,
    pub away_team_name: String,
    pub away_team_slug: String,
    pub result: Option<LeagueScheduleItemResult>,
}

#[allow(dead_code)]
pub struct LeagueScheduleItemResult {
    pub home_goals: u8,
    pub home_goalscorers: Vec<LeagueTableGoalscorer>,
    pub away_goals: u8,
    pub away_goalscorers: Vec<LeagueTableGoalscorer>,
}

#[allow(dead_code)]
pub struct LeagueTableGoalscorer {
    pub id: u32,
    pub name: String,
    pub time: String,
    pub auto_goal: bool,
}

pub struct LeagueTableRow {
    pub team_name: String,
    pub team_slug: String,
    pub played: u8,
    pub win: u8,
    pub draft: u8,
    pub lost: u8,
    pub goal_scored: i32,
    pub goal_concerned: i32,
    pub points: u8,
}

pub async fn league_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<LeagueGetRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard.as_ref().unwrap();

    let league_id = simulator_data
        .indexes
        .as_ref()
        .unwrap()
        .slug_indexes
        .get_league_by_slug(&route_params.league_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "League with slug {} not found",
                route_params.league_slug
            ))
        })?;

    let league = simulator_data.league(league_id).unwrap();
    let country = simulator_data.country(league.country_id).unwrap();
    let league_table = league.table.get();

    let table_rows: Vec<LeagueTableRow> = league_table
        .iter()
        .map(|t| {
            let team_data = simulator_data.team_data(t.team_id).unwrap();
            LeagueTableRow {
                team_name: team_data.name.clone(),
                team_slug: team_data.slug.clone(),
                played: t.played,
                win: t.win,
                draft: t.draft,
                lost: t.lost,
                goal_scored: t.goal_scored,
                goal_concerned: t.goal_concerned,
                points: t.points,
            }
        })
        .collect();

    let now = simulator_data.date.date() + Duration::days(3);

    let mut current_tour: Option<&ScheduleTour> = None;

    for tour in league.schedule.tours.iter() {
        if now >= tour.start_date() && now <= tour.end_date() {
            current_tour = Some(tour);
        }
    }

    if current_tour.is_none() {
        for tour in league.schedule.tours.iter() {
            if now >= tour.end_date() {
                current_tour = Some(tour);
            }
        }
    }

    let mut current_tour_schedule = Vec::new();

    if let Some(tour) = current_tour {
        for (key, group) in &tour.items.iter().chunk_by(|t| t.date.date()) {
            let tour_schedule = TourSchedule {
                date: key.format("%d.%m.%Y").to_string(),
                matches: group
                    .map(|item| {
                        let home_team_data = simulator_data.team_data(item.home_team_id).unwrap();
                        let home_team = simulator_data.team(item.home_team_id).unwrap();
                        let away_team_data = simulator_data.team_data(item.away_team_id).unwrap();
                        let away_team = simulator_data.team(item.away_team_id).unwrap();

                        LeagueScheduleItem {
                            match_id: item.id.clone(),
                            result: item.result.as_ref().map(|res| {
                                let details: Vec<&GoalDetail> = res
                                    .details
                                    .iter()
                                    .filter(|detail| detail.stat_type == MatchStatisticType::Goal)
                                    .collect();

                                LeagueScheduleItemResult {
                                    home_goals: if item.home_team_id == res.home_team.team_id {
                                        res.home_team.get()
                                    } else {
                                        res.away_team.get()
                                    },
                                    home_goalscorers: details
                                        .iter()
                                        .filter_map(|detail| {
                                            let player = simulator_data.player(detail.player_id)?;
                                            if home_team.players.contains(player.id) {
                                                Some(LeagueTableGoalscorer {
                                                    id: detail.player_id,
                                                    name: player.full_name.to_string(),
                                                    time: format!(
                                                        "('{})",
                                                        Duration::new(
                                                            (detail.time / 1000) as i64,
                                                            0
                                                        )
                                                        .unwrap()
                                                        .num_minutes()
                                                    ),
                                                    auto_goal: detail.is_auto_goal,
                                                })
                                            } else {
                                                None
                                            }
                                        })
                                        .collect(),
                                    away_goals: if item.away_team_id == res.away_team.team_id {
                                        res.away_team.get()
                                    } else {
                                        res.home_team.get()
                                    },
                                    away_goalscorers: details
                                        .iter()
                                        .filter_map(|detail| {
                                            let player = simulator_data.player(detail.player_id)?;
                                            if away_team.players.contains(player.id) {
                                                Some(LeagueTableGoalscorer {
                                                    id: detail.player_id,
                                                    name: player.full_name.to_string(),
                                                    time: format!(
                                                        "('{})",
                                                        Duration::new(
                                                            (detail.time / 1000) as i64,
                                                            0
                                                        )
                                                        .unwrap()
                                                        .num_minutes()
                                                    ),
                                                    auto_goal: detail.is_auto_goal,
                                                })
                                            } else {
                                                None
                                            }
                                        })
                                        .collect(),
                                }
                            }),
                            home_team_name: home_team_data.name.clone(),
                            home_team_slug: home_team_data.slug.clone(),
                            away_team_name: away_team_data.name.clone(),
                            away_team_slug: away_team_data.slug.clone(),
                        }
                    })
                    .collect(),
            };
            current_tour_schedule.push(tour_schedule);
        }
    }

    let mut reputation_data: Vec<(u16, String, String, String, String)> = simulator_data
        .continents
        .iter()
        .flat_map(|continent| &continent.countries)
        .flat_map(|country| {
            country
                .leagues
                .leagues
                .iter()
                .filter(|l| !l.friendly)
                .map(move |league| {
                    (
                        league.reputation,
                        league.name.clone(),
                        league.slug.clone(),
                        country.name.clone(),
                        country.code.clone(),
                    )
                })
        })
        .collect();

    reputation_data.sort_by(|a, b| b.0.cmp(&a.0));

    let competition_reputation: Vec<CompetitionReputationItem> = reputation_data
        .into_iter()
        .take(10)
        .map(
            |(_, league_name, league_slug, country_name, country_code)| CompetitionReputationItem {
                league_name,
                league_slug,
                country_name,
                country_code,
            },
        )
        .collect();

    // League-scoped goal & assist tally — count from this league's
    // own match results, not from `player.statistics` (which is the
    // player's full competitive bucket and would also include any
    // other league spell from the same season). Also: a force-pinned
    // youth player only appears in goal-detail records of THIS
    // league's matches when he actually played here, so this is the
    // single source that gets him on the right top-scorer list.
    let mut goals_per_player: HashMap<u32, u16> = HashMap::new();
    let mut assists_per_player: HashMap<u32, u16> = HashMap::new();
    let mut apps_per_player: HashMap<u32, u16> = HashMap::new();

    for tour in &league.schedule.tours {
        for item in &tour.items {
            let Some(score) = &item.result else {
                continue;
            };
            for d in &score.details {
                match d.stat_type {
                    MatchStatisticType::Goal if !d.is_auto_goal => {
                        *goals_per_player.entry(d.player_id).or_insert(0) += 1;
                    }
                    MatchStatisticType::Assist => {
                        *assists_per_player.entry(d.player_id).or_insert(0) += 1;
                    }
                    _ => {}
                }
            }
        }
    }

    let resolve_team_for_player = |player_id: u32| -> Option<(String, String)> {
        let (_player, roster_team) = simulator_data.player_with_team(player_id)?;
        let club = simulator_data.club(roster_team.club_id)?;
        // Prefer the player's actual roster team if it's in this
        // league. Otherwise (force-pinned youth, or rare reserve
        // entries), fall back to the club's team that participates
        // in this league.
        if league_table.iter().any(|r| r.team_id == roster_team.id) {
            return Some((roster_team.name.clone(), roster_team.slug.clone()));
        }
        for team in &club.teams.teams {
            if league_table.iter().any(|r| r.team_id == team.id) {
                return Some((team.name.clone(), team.slug.clone()));
            }
        }
        None
    };

    let mut scorer_data: Vec<(u32, String, String, String, u16, u16)> = Vec::new();
    let mut assister_data: Vec<(u32, String, String, String, u16, u16)> = Vec::new();
    let mut rating_data: Vec<(u32, String, String, String, u16, f32)> = Vec::new();

    for (&pid, &goals) in &goals_per_player {
        let Some(player) = simulator_data.player(pid) else {
            continue;
        };
        let Some((team_name, team_slug)) = resolve_team_for_player(pid) else {
            continue;
        };
        let played = player.statistics.played + player.statistics.played_subs;
        apps_per_player.insert(pid, played);
        scorer_data.push((
            pid,
            player.full_name.to_string(),
            team_name,
            team_slug,
            played,
            goals,
        ));
    }

    for (&pid, &assists) in &assists_per_player {
        let Some(player) = simulator_data.player(pid) else {
            continue;
        };
        let Some((team_name, team_slug)) = resolve_team_for_player(pid) else {
            continue;
        };
        let played = *apps_per_player
            .entry(pid)
            .or_insert_with(|| player.statistics.played + player.statistics.played_subs);
        assister_data.push((
            pid,
            player.full_name.to_string(),
            team_name,
            team_slug,
            played,
            assists,
        ));
    }

    // Average rating still flows from `player.statistics.average_rating`
    // — per-league rating tracking would need new storage on the
    // player. Iterate over the same effective rosters we use for
    // matchday selection so force-pinned youth players surface here
    // too.
    let mut seen_in_table: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for table_row in league_table {
        if let Some(team) = simulator_data.team(table_row.team_id) {
            let team_name = team.name.clone();
            let team_slug = team.slug.clone();

            let mut effective: Vec<&core::Player> = team.players.players.iter().collect();
            if team.team_type == core::TeamType::Main {
                if let Some(club) = simulator_data.club(team.club_id) {
                    for sibling in &club.teams.teams {
                        if sibling.id == team.id {
                            continue;
                        }
                        for p in &sibling.players.players {
                            if p.is_force_match_selection && !effective.iter().any(|q| q.id == p.id)
                            {
                                effective.push(p);
                            }
                        }
                    }
                }
            }

            for player in effective {
                if !seen_in_table.insert(player.id) {
                    continue;
                }
                let played = player.statistics.played + player.statistics.played_subs;
                if played > 0 && player.statistics.average_rating > 0.0 {
                    rating_data.push((
                        player.id,
                        player.full_name.to_string(),
                        team_name.clone(),
                        team_slug.clone(),
                        played,
                        player.statistics.average_rating,
                    ));
                }
            }
        }
    }

    scorer_data.sort_by(|a, b| b.5.cmp(&a.5));
    assister_data.sort_by(|a, b| b.5.cmp(&a.5));
    rating_data.sort_by(|a, b| b.5.partial_cmp(&a.5).unwrap_or(std::cmp::Ordering::Equal));

    let top_scorers: Vec<LeaguePlayerStatItem> = scorer_data
        .into_iter()
        .take(10)
        .map(
            |(player_id, player_name, team_name, team_slug, played, goals)| LeaguePlayerStatItem {
                player_slug: player_history_slug(simulator_data, player_id, &player_name),
                player_name,
                team_name,
                team_slug,
                played,
                stat_value: goals.to_string(),
            },
        )
        .collect();

    let top_assisters: Vec<LeaguePlayerStatItem> = assister_data
        .into_iter()
        .take(10)
        .map(
            |(player_id, player_name, team_name, team_slug, played, assists)| {
                LeaguePlayerStatItem {
                    player_slug: player_history_slug(simulator_data, player_id, &player_name),
                    player_name,
                    team_name,
                    team_slug,
                    played,
                    stat_value: assists.to_string(),
                }
            },
        )
        .collect();

    let top_rated: Vec<LeaguePlayerStatItem> = rating_data
        .into_iter()
        .take(10)
        .map(
            |(player_id, player_name, team_name, team_slug, played, rating)| LeaguePlayerStatItem {
                player_slug: player_history_slug(simulator_data, player_id, &player_name),
                player_name,
                team_name,
                team_slug,
                played,
                stat_value: format!("{:.2}", rating),
            },
        )
        .collect();

    let league_title = views::league_display_name(&league, &i18n, simulator_data);

    Ok(LeagueGetTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        title: league_title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: country.name.clone(),
        sub_title_link: format!("/{}/countries/{}", &route_params.lang, &country.slug),
        sub_title_country_code: country.code.clone(),
        header_color: country.background_color.clone(),
        foreground_color: country.foreground_color.clone(),
        menu_sections: {
            let mut cl: Vec<(u32, &str, &str)> = country
                .leagues
                .leagues
                .iter()
                .filter(|l| !l.friendly)
                .map(|l| (l.id, l.name.as_str(), l.slug.as_str()))
                .collect();
            cl.sort_by_key(|(id, _, _)| *id);
            let cl_refs: Vec<(&str, &str)> = cl.iter().map(|(_, n, s)| (*n, *s)).collect();
            let current_path = format!("/{}/leagues/{}", &route_params.lang, &league.slug);
            let mp = views::MenuParams {
                i18n: &i18n,
                lang: &route_params.lang,
                current_path: &current_path,
                country_name: &country.name,
                country_slug: &country.slug,
            };
            views::league_menu(&mp, &league.slug, &cl_refs)
        },
        league_slug: league.slug.clone(),
        table_rows,
        current_tour_schedule,
        competition_reputation,
        top_scorers,
        top_assisters,
        top_rated,
        lang: route_params.lang,
        i18n,
    })
}
