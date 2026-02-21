pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Duration;
use core::league::ScheduleTour;
use core::r#match::GoalDetail;
use core::r#match::player::statistics::MatchStatisticType;
use itertools::*;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct LeagueGetRequest {
    pub lang: String,
    pub league_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "leagues/get/index.html")]
pub struct LeagueGetTemplate {
    pub css_version: &'static str,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: crate::I18n,
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
    pub player_id: u32,
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
        .ok_or_else(|| ApiError::NotFound(format!("League with slug {} not found", route_params.league_slug)))?;

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
                                                        Duration::new((detail.time / 1000) as i64, 0)
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
                                                        Duration::new((detail.time / 1000) as i64, 0)
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
            country.leagues.leagues.iter().map(move |league| {
                (league.reputation, league.name.clone(), league.slug.clone(), country.name.clone(), country.code.clone())
            })
        })
        .collect();

    reputation_data.sort_by(|a, b| b.0.cmp(&a.0));

    let competition_reputation: Vec<CompetitionReputationItem> = reputation_data
        .into_iter()
        .take(20)
        .map(|(_, league_name, league_slug, country_name, country_code)| {
            CompetitionReputationItem {
                league_name,
                league_slug,
                country_name,
                country_code,
            }
        })
        .collect();

    // Collect player statistics from all teams in this league
    let mut scorer_data: Vec<(u32, String, String, String, u16, u16)> = Vec::new(); // (player_id, name, team_name, team_slug, played, goals)
    let mut assister_data: Vec<(u32, String, String, String, u16, u16)> = Vec::new();
    let mut rating_data: Vec<(u32, String, String, String, u16, f32)> = Vec::new();

    for table_row in league_table {
        if let Some(team) = simulator_data.team(table_row.team_id) {
            let team_name = team.name.clone();
            let team_slug = team.slug.clone();
            for player in &team.players.players {
                let played = player.statistics.played + player.statistics.played_subs;
                if player.statistics.goals > 0 {
                    scorer_data.push((
                        player.id,
                        player.full_name.to_string(),
                        team_name.clone(),
                        team_slug.clone(),
                        played,
                        player.statistics.goals,
                    ));
                }
                if player.statistics.assists > 0 {
                    assister_data.push((
                        player.id,
                        player.full_name.to_string(),
                        team_name.clone(),
                        team_slug.clone(),
                        played,
                        player.statistics.assists,
                    ));
                }
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
        .map(|(player_id, player_name, team_name, team_slug, played, goals)| LeaguePlayerStatItem {
            player_id,
            player_name,
            team_name,
            team_slug,
            played,
            stat_value: goals.to_string(),
        })
        .collect();

    let top_assisters: Vec<LeaguePlayerStatItem> = assister_data
        .into_iter()
        .take(10)
        .map(|(player_id, player_name, team_name, team_slug, played, assists)| LeaguePlayerStatItem {
            player_id,
            player_name,
            team_name,
            team_slug,
            played,
            stat_value: assists.to_string(),
        })
        .collect();

    let top_rated: Vec<LeaguePlayerStatItem> = rating_data
        .into_iter()
        .take(10)
        .map(|(player_id, player_name, team_name, team_slug, played, rating)| LeaguePlayerStatItem {
            player_id,
            player_name,
            team_name,
            team_slug,
            played,
            stat_value: format!("{:.2}", rating),
        })
        .collect();

    Ok(LeagueGetTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        title: league.name.clone(),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: country.name.clone(),
        sub_title_link: format!("/{}/countries/{}", &route_params.lang, &country.slug),
        sub_title_country_code: country.code.clone(),
        header_color: country.background_color.clone(),
        foreground_color: country.foreground_color.clone(),
        menu_sections: {
            let mut cl: Vec<(u32, &str, &str)> = country.leagues.leagues.iter().map(|l| (l.id, l.name.as_str(), l.slug.as_str())).collect();
            cl.sort_by_key(|(id, _, _)| *id);
            let cl_refs: Vec<(&str, &str)> = cl.iter().map(|(_, n, s)| (*n, *s)).collect();
            views::league_menu(&i18n, &route_params.lang, &country.name, &country.slug, &league.slug, &format!("/{}/leagues/{}", &route_params.lang, &league.slug), &cl_refs)
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
