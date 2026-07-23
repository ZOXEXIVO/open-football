pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::slug::player_history_slug;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use core::league::ScheduleItem;
use core::league::schedule::cup;
use core::r#match::player::statistics::MatchStatisticType;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Deserialize)]
pub struct CupGetRequest {
    pub lang: String,
    pub cup_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "cups/get/index.html")]
pub struct CupGetTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
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
    pub cup_slug: String,
    pub active_tab: &'static str,
    /// True once the bracket has resolved to a single winner.
    pub is_decided: bool,
    pub champion_name: String,
    pub champion_slug: String,
    pub entrants: usize,
    pub rounds_count: usize,
    /// Label of the furthest round reached, shown in the hero while the cup
    /// is still being contested (empty once `is_decided`, or if no rounds).
    pub stage_label: String,
    /// The full knockout bracket, one entry per round (earliest first).
    pub bracket: Vec<CupRound>,
    pub top_scorers: Vec<CupPlayerStat>,
    pub top_assisters: Vec<CupPlayerStat>,
}

pub struct CupRound {
    pub label: String,
    pub ties: Vec<CupTie>,
    /// Teams that advanced this round without playing (top-seed byes).
    pub byes: Vec<CupByeTeam>,
}

pub struct CupTie {
    pub match_id: String,
    pub home_name: String,
    pub home_slug: String,
    pub away_name: String,
    pub away_slug: String,
    /// Goals for each side as display strings; `None` while the tie is
    /// unplayed. Split per-team (rather than a combined "2 – 1" string) so
    /// the bracket can render each team's score on its own row.
    pub home_goals: Option<String>,
    pub away_goals: Option<String>,
    /// Shootout tallies; `Some` only when the tie was settled on penalties.
    pub home_pens: Option<String>,
    pub away_pens: Option<String>,
    pub home_winner: bool,
    pub away_winner: bool,
    pub played: bool,
}

pub struct CupByeTeam {
    pub name: String,
    pub slug: String,
}

pub struct CupPlayerStat {
    pub player_slug: String,
    pub player_name: String,
    pub team_name: String,
    pub team_slug: String,
    pub played: u16,
    pub stat_value: String,
}

/// Stage label for a knockout round, derived from the number of ties it
/// holds: 1 → Final, 2 → Semi-finals, 4 → Quarter-finals, otherwise the
/// "Round of N" the field size implies.
fn cup_round_label(i18n: &I18n, ties: usize) -> String {
    match ties {
        1 => i18n.t("final").to_string(),
        2 => i18n.t("semi_finals").to_string(),
        4 => i18n.t("quarter_finals").to_string(),
        n => format!("{} {}", i18n.t("round_of"), n * 2),
    }
}

pub async fn cup_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<CupGetRequest>,
) -> ApiResult<Response> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;
    let simulator_data = guard.as_ref().unwrap();

    let league_id = simulator_data
        .indexes
        .as_ref()
        .unwrap()
        .slug_indexes
        .get_league_by_slug(&route_params.cup_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Cup with slug {} not found", route_params.cup_slug))
        })?;

    let league = simulator_data.league(league_id).unwrap();

    // The cup route only serves domestic cups. A normal league slug here
    // (or any non-cup competition) is bounced to the standings page.
    if !league.is_cup {
        return Ok(
            Redirect::to(&format!("/{}/leagues/{}", route_params.lang, league.slug))
                .into_response(),
        );
    }

    let country = simulator_data.country(league.country_id).unwrap();

    // Grouped-competition playoffs also run through cup-flagged leagues but
    // render on their own bracket page.
    if country.playoffs.iter().any(|p| p.league.id == league_id) {
        return Ok(
            Redirect::to(&format!("/{}/playoffs/{}", route_params.lang, league.slug))
                .into_response(),
        );
    }

    let tours = &league.schedule.tours;

    // Build one fixture card from a bracket tie. The schedule item's
    // home/away ids may sit in either slot of the stored `Score`, so map
    // goals (and the shootout tally) back through the recorded `team_id`s,
    // mirroring the league page's `map_item`.
    let build_tie = |item: &ScheduleItem| -> CupTie {
        let (home_name, home_slug) = simulator_data
            .team_data(item.home_team_id)
            .map(|d| (d.name.clone(), d.slug.clone()))
            .unwrap_or_default();
        let (away_name, away_slug) = simulator_data
            .team_data(item.away_team_id)
            .map(|d| (d.name.clone(), d.slug.clone()))
            .unwrap_or_default();

        let winner = cup::tie_winner(item);

        let (home_goals, away_goals, home_pens, away_pens) = match item.result.as_ref() {
            Some(res) => {
                let home_first = item.home_team_id == res.home_team.team_id;
                let (hg, ag) = if home_first {
                    (res.home_team.get(), res.away_team.get())
                } else {
                    (res.away_team.get(), res.home_team.get())
                };
                let (hp, ap) = if res.had_shootout() {
                    let (home_so, away_so) = if home_first {
                        (res.home_shootout, res.away_shootout)
                    } else {
                        (res.away_shootout, res.home_shootout)
                    };
                    (Some(home_so.to_string()), Some(away_so.to_string()))
                } else {
                    (None, None)
                };
                (Some(hg.to_string()), Some(ag.to_string()), hp, ap)
            }
            None => (None, None, None, None),
        };

        CupTie {
            match_id: item.id.clone(),
            home_name,
            home_slug,
            away_name,
            away_slug,
            home_goals,
            away_goals,
            home_pens,
            away_pens,
            home_winner: winner == Some(item.home_team_id),
            away_winner: winner == Some(item.away_team_id),
            played: item.result.is_some(),
        }
    };

    // Byes per round: a team that turns up in round r+1 without having won a
    // tie in round r advanced on a bye. Computed generically from the
    // bracket — no extra core state. The final round has no successor and so
    // no byes.
    let mut byes_per_round: Vec<Vec<u32>> = Vec::with_capacity(tours.len());
    for (r, tour) in tours.iter().enumerate() {
        let mut byes = Vec::new();
        if let Some(next) = tours.get(r + 1) {
            let winners: HashSet<u32> = tour.items.iter().filter_map(cup::tie_winner).collect();
            let mut seen = HashSet::new();
            for item in &next.items {
                for tid in [item.home_team_id, item.away_team_id] {
                    if !winners.contains(&tid) && seen.insert(tid) {
                        byes.push(tid);
                    }
                }
            }
        }
        byes_per_round.push(byes);
    }

    let bracket: Vec<CupRound> = tours
        .iter()
        .enumerate()
        .map(|(r, tour)| CupRound {
            label: cup_round_label(&i18n, tour.items.len()),
            ties: tour.items.iter().map(|item| build_tie(item)).collect(),
            byes: byes_per_round[r]
                .iter()
                .filter_map(|&tid| {
                    simulator_data.team_data(tid).map(|d| CupByeTeam {
                        name: d.name.clone(),
                        slug: d.slug.clone(),
                    })
                })
                .collect(),
        })
        .collect();

    // Champion: the last round is the final once it holds a single, played
    // tie. `tie_winner` resolves a level score on the shootout tally.
    let champion_id = tours
        .last()
        .filter(|t| t.items.len() == 1)
        .and_then(|t| t.items.first())
        .and_then(cup::tie_winner);
    let is_decided = champion_id.is_some();
    let (champion_name, champion_slug) = champion_id
        .and_then(|id| simulator_data.team_data(id))
        .map(|d| (d.name.clone(), d.slug.clone()))
        .unwrap_or_default();

    let entrants = tours
        .first()
        .map(|first| {
            let mut set: HashSet<u32> = HashSet::new();
            for item in &first.items {
                set.insert(item.home_team_id);
                set.insert(item.away_team_id);
            }
            set.extend(byes_per_round[0].iter().copied());
            set.len()
        })
        .unwrap_or(0);
    let rounds_count = tours.len();

    // While the cup is live, the furthest round drawn is its current stage
    // (e.g. "Quarter-finals"); blanked once a champion is known.
    let stage_label = if is_decided {
        String::new()
    } else {
        bracket.last().map(|r| r.label.clone()).unwrap_or_default()
    };

    // Cup-scoped player tallies, read from this competition's own match
    // records (same source the league page uses; see leagues/get for why
    // `player.statistics` is unreliable here). Teams resolve directly from
    // the player's roster since a cup has no standings table.
    let mut goals_per_player: HashMap<u32, u16> = HashMap::new();
    let mut assists_per_player: HashMap<u32, u16> = HashMap::new();
    let mut apps_per_player: HashMap<u32, u16> = HashMap::new();

    for mr in league.matches.iter() {
        for d in &mr.score.details {
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
        if let Some(details) = &mr.details {
            for side in [&details.left_team_players, &details.right_team_players] {
                for &pid in side.main.iter().chain(side.substitutes_used.iter()) {
                    *apps_per_player.entry(pid).or_insert(0) += 1;
                }
            }
        }
    }

    let build_stats = |tally: &HashMap<u32, u16>| -> Vec<CupPlayerStat> {
        let mut rows: Vec<(u32, String, String, String, u16, u16)> = Vec::new();
        for (&pid, &value) in tally {
            let Some(player) = simulator_data.player(pid) else {
                continue;
            };
            let Some((_p, team)) = simulator_data.player_with_team(pid) else {
                continue;
            };
            let played = apps_per_player.get(&pid).copied().unwrap_or(0);
            rows.push((
                pid,
                player.full_name.to_string(),
                team.name.clone(),
                team.slug.clone(),
                played,
                value,
            ));
        }
        rows.sort_by(|a, b| b.5.cmp(&a.5));
        rows.into_iter()
            .take(10)
            .map(
                |(pid, player_name, team_name, team_slug, played, value)| CupPlayerStat {
                    player_slug: player_history_slug(simulator_data, pid, &player_name),
                    player_name,
                    team_name,
                    team_slug,
                    played,
                    stat_value: value.to_string(),
                },
            )
            .collect()
    };

    let top_scorers = build_stats(&goals_per_player);
    let top_assisters = build_stats(&assists_per_player);

    let title = views::league_display_name(&league, &i18n, simulator_data);
    let current_path = format!("/{}/cups/{}", &route_params.lang, &league.slug);
    let country_leagues: Vec<(&str, &str)> = country
        .leagues
        .leagues
        .iter()
        .filter(|l| !l.friendly)
        .map(|l| (l.name.as_str(), l.slug.as_str()))
        .collect();

    let menu_sections = {
        let mp = views::MenuParams {
            i18n: &i18n,
            lang: &route_params.lang,
            current_path: &current_path,
            country_name: &country.name,
            country_slug: &country.slug,
        };
        views::cup_menu(
            &mp,
            &league.slug,
            &country_leagues,
            &league.name,
            &country
                .playoffs
                .iter()
                .map(|p| (p.league.name.as_str(), p.league.slug.as_str()))
                .collect::<Vec<_>>(),
            country.continent_id,
        )
    };

    Ok(CupGetTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: country.name.clone(),
        sub_title_link: format!("/{}/countries/{}", &route_params.lang, &country.slug),
        sub_title_country_code: country.code.clone(),
        header_color: country.background_color.clone(),
        foreground_color: country.foreground_color.clone(),
        menu_sections,
        cup_slug: league.slug.clone(),
        active_tab: "bracket",
        is_decided,
        champion_name,
        champion_slug,
        entrants,
        rounds_count,
        stage_label,
        bracket,
        top_scorers,
        top_assisters,
        lang: route_params.lang,
        i18n,
    }
    .into_response())
}
