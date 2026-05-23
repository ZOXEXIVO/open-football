pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::slug::{PlayerPage, resolve_player_page};
use crate::player::decisions::PlayerDecisionsCounter;
use crate::player::events::PlayerEventsCounter;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::{PlayerAwardsCount, PlayerStatusType, SimulatorData};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PlayerAwardsRequest {
    pub lang: String,
    pub player_slug: String,
}

/// One dashboard tile. Title + count + a CSS modifier so the template
/// can tint the tile per award family without duplicating markup.
pub struct AwardCard {
    pub title: String,
    pub count: u16,
    pub tone: &'static str,
    pub icon: &'static str,
}

/// One bar in the past-12-months chart at the bottom of the page.
/// `height_pct` is the bar's height as a percentage of the largest bar
/// in the set — we precompute it server-side so the chart renders
/// without JS. Label is split into `month` / `year` so the template
/// can stack them on two lines.
pub struct MonthBar {
    pub month: String,
    pub year: String,
    pub count: u16,
    pub height_pct: u32,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/awards/index.html")]
pub struct PlayerAwardsTemplate {
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
    pub active_tab: &'static str,
    pub player_id: u32,
    pub player_slug: String,
    pub club_id: u32,
    pub is_on_loan: bool,
    pub is_injured: bool,
    pub is_unhappy: bool,
    pub is_force_match_selection: bool,
    pub is_on_watchlist: bool,
    pub events_count: usize,
    pub decisions_count: usize,
    pub interested_clubs_count: usize,
    pub awards_count: u32,
    pub weekly_cards: Vec<AwardCard>,
    pub monthly_cards: Vec<AwardCard>,
    pub season_cards: Vec<AwardCard>,
    pub global_cards: Vec<AwardCard>,
    pub has_weekly: bool,
    pub has_monthly: bool,
    pub has_season: bool,
    pub has_global: bool,
    pub month_bars: Vec<MonthBar>,
    pub month_max: u16,
    pub has_chart: bool,
}

pub async fn player_awards_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<PlayerAwardsRequest>,
) -> ApiResult<Response> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let (player, team_opt, canonical) = match resolve_player_page(
        simulator_data,
        &route_params.player_slug,
        &route_params.lang,
        "/awards",
    )? {
        PlayerPage::Found {
            player,
            team,
            canonical_slug,
        } => (player, team, canonical_slug),
        PlayerPage::Redirect(r) => return Ok(r),
    };

    let (neighbor_teams, country_leagues) = if let Some(team) = team_opt {
        get_neighbor_teams(team.club_id, simulator_data, &i18n)?
    } else {
        (Vec::new(), Vec::new())
    };
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let league_refs: Vec<(&str, &str)> = country_leagues
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();

    let title = format!(
        "{} {}",
        player.full_name.display_first_name(),
        player.full_name.display_last_name()
    );

    let counts = &player.awards_count;
    let (weekly_cards, monthly_cards, season_cards, global_cards) = build_cards(counts, &i18n);
    let has_weekly = weekly_cards.iter().any(|c| c.count > 0);
    let has_monthly = monthly_cards.iter().any(|c| c.count > 0);
    let has_season = season_cards.iter().any(|c| c.count > 0);
    let has_global = global_cards.iter().any(|c| c.count > 0);
    let (month_bars, month_max) = build_month_bars(counts, simulator_data.date.date());

    Ok(PlayerAwardsTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        title,
        sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
        sub_title_suffix: String::new(),
        sub_title: team_opt.map(|t| t.name.clone()).unwrap_or_else(|| {
            if player.is_retired() {
                i18n.t("retired").to_string()
            } else {
                i18n.t("free_agent").to_string()
            }
        }),
        sub_title_link: team_opt
            .map(|t| format!("/{}/teams/{}", &route_params.lang, &t.slug))
            .unwrap_or_default(),
        sub_title_country_code: String::new(),
        header_color: team_opt
            .and_then(|t| {
                simulator_data
                    .club(t.club_id)
                    .map(|c| c.colors.background.clone())
            })
            .unwrap_or_else(|| "#808080".to_string()),
        foreground_color: team_opt
            .and_then(|t| {
                simulator_data
                    .club(t.club_id)
                    .map(|c| c.colors.foreground.clone())
            })
            .unwrap_or_else(|| "#ffffff".to_string()),
        menu_sections: if let Some(team) = team_opt {
            let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
            let current_path = format!("/{}/teams/{}", &route_params.lang, &team.slug);
            let mp = views::MenuParams {
                i18n: &i18n,
                lang: &route_params.lang,
                current_path: &current_path,
                country_name: cn,
                country_slug: cs,
            };
            views::team_menu(&mp, &neighbor_refs, &team.slug, &league_refs, false)
        } else {
            Vec::new()
        },
        i18n,
        lang: route_params.lang.clone(),
        active_tab: "awards",
        player_id: player.id,
        player_slug: canonical,
        club_id: team_opt.map(|t| t.club_id).unwrap_or(0),
        is_on_loan: player.is_on_loan(),
        is_injured: player.player_attributes.is_injured,
        is_unhappy: player.statuses.get().contains(&PlayerStatusType::Unh),
        is_force_match_selection: player.is_force_match_selection,
        is_on_watchlist: simulator_data.watchlist.contains(&player.id),
        events_count: PlayerEventsCounter::count(player),
        decisions_count: PlayerDecisionsCounter::count_recent(player, simulator_data.date.date()),
        interested_clubs_count: simulator_data.clubs_interested_in_player(player.id).len(),
        awards_count: counts.total(),
        weekly_cards,
        monthly_cards,
        season_cards,
        global_cards,
        has_weekly,
        has_monthly,
        has_season,
        has_global,
        has_chart: !month_bars.is_empty(),
        month_bars,
        month_max,
    }
    .into_response())
}

fn build_cards(
    counts: &PlayerAwardsCount,
    i18n: &I18n,
) -> (
    Vec<AwardCard>,
    Vec<AwardCard>,
    Vec<AwardCard>,
    Vec<AwardCard>,
) {
    let card =
        |title_key: &str, count: u16, tone: &'static str, icon: &'static str| AwardCard {
            title: i18n.t(title_key).to_string(),
            count,
            tone,
            icon,
        };

    // Icon mapping uses Font Awesome glyphs already in the page's icon
    // pack. Team awards take group icons; senior individual awards take
    // a trophy/star/medal; young variants overlay a graduate / seedling.
    let weekly = vec![
        card(
            "team_of_the_week",
            counts.team_of_the_week,
            "weekly",
            "fa-people-group",
        ),
        card(
            "young_team_of_the_week",
            counts.young_team_of_the_week,
            "weekly young",
            "fa-user-graduate",
        ),
        card(
            "player_of_the_week",
            counts.player_of_the_week,
            "weekly",
            "fa-star",
        ),
        card(
            "young_player_of_the_week",
            counts.young_player_of_the_week,
            "weekly young",
            "fa-seedling",
        ),
    ];

    let monthly = vec![
        card(
            "team_of_the_month",
            counts.team_of_the_month,
            "monthly",
            "fa-people-group",
        ),
        card(
            "young_team_of_the_month",
            counts.young_team_of_the_month,
            "monthly young",
            "fa-user-graduate",
        ),
        card(
            "player_of_the_month",
            counts.player_of_the_month,
            "monthly",
            "fa-medal",
        ),
        card(
            "young_player_of_the_month",
            counts.young_player_of_the_month,
            "monthly young",
            "fa-award",
        ),
    ];

    let season = vec![
        card(
            "team_of_the_season",
            counts.team_of_the_season,
            "season",
            "fa-trophy",
        ),
        card(
            "team_of_the_year",
            counts.team_of_the_year,
            "season",
            "fa-trophy",
        ),
        card(
            "player_of_the_season",
            counts.player_of_the_season,
            "season",
            "fa-crown",
        ),
        card(
            "young_player_of_the_season",
            counts.young_player_of_the_season,
            "season young",
            "fa-crown",
        ),
        card(
            "top_scorer",
            counts.league_top_scorer,
            "season",
            "fa-futbol",
        ),
        card(
            "top_assists",
            counts.league_top_assists,
            "season",
            "fa-handshake",
        ),
        card(
            "golden_glove",
            counts.league_golden_glove,
            "season",
            "fa-hand-back-fist",
        ),
    ];

    let global = vec![
        card(
            "continental_player_of_the_year",
            counts.continental_player_of_year,
            "global",
            "fa-globe-europe",
        ),
        card(
            "world_player_of_the_year",
            counts.world_player_of_year,
            "global",
            "fa-earth-americas",
        ),
    ];

    (weekly, monthly, season, global)
}

/// Bucket the lifetime timeline by calendar month for the past 12
/// months relative to `now`. Always returns exactly 12 bars (current
/// month last) so the chart's x-axis is stable even when the player
/// went quiet — empty months render as a flat track. Returns
/// `(bars, max_count)`; max may be zero if no awards fell in the
/// window. The bar list is empty only when the player has no awards
/// at all (so the chart panel is hidden).
fn build_month_bars(counts: &PlayerAwardsCount, now: chrono::NaiveDate) -> (Vec<MonthBar>, u16) {
    if counts.timeline.is_empty() {
        return (Vec::new(), 0);
    }
    use chrono::Datelike;

    // Build the 12 (year, month) keys ending at `now`'s month. Stored
    // in chronological order so the rightmost bar is "this month".
    let mut keys: Vec<(i32, u32)> = Vec::with_capacity(12);
    let mut year = now.year();
    let mut month = now.month() as i32;
    for _ in 0..12 {
        keys.push((year, month as u32));
        month -= 1;
        if month == 0 {
            month = 12;
            year -= 1;
        }
    }
    keys.reverse();

    // Index counts per (year, month).
    use std::collections::HashMap;
    let mut counts_by_key: HashMap<(i32, u32), u16> = HashMap::new();
    for entry in &counts.timeline {
        let key = (entry.date.year(), entry.date.month());
        let slot = counts_by_key.entry(key).or_insert(0);
        *slot = slot.saturating_add(1);
    }

    let month_short = |m: u32| -> &'static str {
        match m {
            1 => "Jan",
            2 => "Feb",
            3 => "Mar",
            4 => "Apr",
            5 => "May",
            6 => "Jun",
            7 => "Jul",
            8 => "Aug",
            9 => "Sep",
            10 => "Oct",
            11 => "Nov",
            12 => "Dec",
            _ => "",
        }
    };

    let bars_data: Vec<(&'static str, i32, u16)> = keys
        .iter()
        .map(|key| {
            let count = counts_by_key.get(key).copied().unwrap_or(0);
            (month_short(key.1), key.0, count)
        })
        .collect();

    let max_count = bars_data.iter().map(|(_, _, c)| *c).max().unwrap_or(0);

    let bars = bars_data
        .into_iter()
        .map(|(month, year, count)| {
            let height_pct = if max_count == 0 {
                0
            } else {
                ((count as u32 * 100) / max_count as u32).min(100)
            };
            MonthBar {
                month: month.to_string(),
                year: year.to_string(),
                count,
                height_pct,
            }
        })
        .collect();

    (bars, max_count)
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &I18n,
) -> Result<(Vec<(String, String)>, Vec<(String, String)>), ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;
    let teams = views::neighbor_teams(club, i18n);
    let mut country_leagues: Vec<(u32, String, String)> = data
        .country_by_club(club_id)
        .map(|country| {
            country
                .leagues
                .leagues
                .iter()
                .filter(|l| !l.friendly)
                .map(|l| (l.id, l.name.clone(), l.slug.clone()))
                .collect()
        })
        .unwrap_or_default();
    country_leagues.sort_by_key(|(id, _, _)| *id);
    Ok((
        teams,
        country_leagues
            .into_iter()
            .map(|(_, name, slug)| (name, slug))
            .collect(),
    ))
}

