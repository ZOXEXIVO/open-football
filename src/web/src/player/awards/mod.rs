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

/// Career-wide summary at the top of the page — lifetime counts
/// across every league the player has ever appeared in. Same card
/// layout as a [`LeagueBlock`] but with no chart and no league
/// header link.
pub struct SummaryBlock {
    pub weekly_cards: Vec<AwardCard>,
    pub monthly_cards: Vec<AwardCard>,
    pub season_cards: Vec<AwardCard>,
    pub global_cards: Vec<AwardCard>,
    pub has_weekly: bool,
    pub has_monthly: bool,
    pub has_season: bool,
    pub has_global: bool,
    pub total: u32,
}

/// One league section on the Awards tab. The first block in the list
/// (sorted most-recent-first) is the player's "current league" and
/// renders both the cards and the 12-month chart; the rest show only
/// the lifetime counts won at that league. `league_id == None` is the
/// global bucket (Continental / World POY) — only rendered when the
/// player has actually won one of those.
pub struct LeagueBlock {
    pub league_name: String,
    pub league_slug: String,
    pub league_id: Option<u32>,
    /// Country card shown to the left of the league name when the
    /// block is a real league (not the global / Continental bucket).
    /// `code` is the lowercased ISO code used by the flag sprite.
    pub country_code: String,
    pub country_name: String,
    pub country_slug: String,
    pub weekly_cards: Vec<AwardCard>,
    pub monthly_cards: Vec<AwardCard>,
    pub season_cards: Vec<AwardCard>,
    pub global_cards: Vec<AwardCard>,
    pub has_weekly: bool,
    pub has_monthly: bool,
    pub has_season: bool,
    pub has_global: bool,
    pub total: u32,
    /// Populated only on the leading block (most recent league). Other
    /// blocks render their counts but skip the chart panel.
    pub month_bars: Vec<MonthBar>,
    pub month_max: u16,
    pub has_chart: bool,
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
    pub summary: SummaryBlock,
    pub league_blocks: Vec<LeagueBlock>,
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
    let summary = build_summary(counts, &i18n);
    let league_blocks = build_league_blocks(counts, simulator_data, &i18n);

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
        summary,
        league_blocks,
    }
    .into_response())
}

fn build_summary(counts: &PlayerAwardsCount, i18n: &I18n) -> SummaryBlock {
    let totals = LeagueAwardTotals::from_lifetime(counts);
    let (weekly, monthly, season, global) = build_cards(&totals, i18n);
    SummaryBlock {
        has_weekly: weekly.iter().any(|c| c.count > 0),
        has_monthly: monthly.iter().any(|c| c.count > 0),
        has_season: season.iter().any(|c| c.count > 0),
        has_global: global.iter().any(|c| c.count > 0),
        weekly_cards: weekly,
        monthly_cards: monthly,
        season_cards: season,
        global_cards: global,
        total: totals.total(),
    }
}

/// Per-league derived totals, computed by counting the timeline.
/// Mirrors the lifetime [`PlayerAwardsCount`] field layout but only
/// over awards won at a specific league_id.
#[derive(Default)]
struct LeagueAwardTotals {
    player_of_the_week: u16,
    young_player_of_the_week: u16,
    team_of_the_week: u16,
    young_team_of_the_week: u16,
    player_of_the_month: u16,
    young_player_of_the_month: u16,
    team_of_the_month: u16,
    young_team_of_the_month: u16,
    team_of_the_season: u16,
    team_of_the_year: u16,
    player_of_the_season: u16,
    young_player_of_the_season: u16,
    league_top_scorer: u16,
    league_top_assists: u16,
    league_golden_glove: u16,
    continental_player_of_year: u16,
    world_player_of_year: u16,
}

impl LeagueAwardTotals {
    /// Lifetime totals (across every league) sourced directly from
    /// the player's counter struct — used by the career-summary
    /// header so it doesn't need to re-walk the timeline.
    fn from_lifetime(counts: &PlayerAwardsCount) -> Self {
        Self {
            player_of_the_week: counts.player_of_the_week,
            young_player_of_the_week: counts.young_player_of_the_week,
            team_of_the_week: counts.team_of_the_week,
            young_team_of_the_week: counts.young_team_of_the_week,
            player_of_the_month: counts.player_of_the_month,
            young_player_of_the_month: counts.young_player_of_the_month,
            team_of_the_month: counts.team_of_the_month,
            young_team_of_the_month: counts.young_team_of_the_month,
            team_of_the_season: counts.team_of_the_season,
            team_of_the_year: counts.team_of_the_year,
            player_of_the_season: counts.player_of_the_season,
            young_player_of_the_season: counts.young_player_of_the_season,
            league_top_scorer: counts.league_top_scorer,
            league_top_assists: counts.league_top_assists,
            league_golden_glove: counts.league_golden_glove,
            continental_player_of_year: counts.continental_player_of_year,
            world_player_of_year: counts.world_player_of_year,
        }
    }

    fn add(&mut self, kind: core::AwardReputationKind) {
        use core::AwardReputationKind as K;
        let slot = match kind {
            K::PlayerOfTheWeek => &mut self.player_of_the_week,
            K::YoungPlayerOfTheWeek => &mut self.young_player_of_the_week,
            K::TeamOfTheWeekSelection => &mut self.team_of_the_week,
            K::YoungTeamOfTheWeekSelection => &mut self.young_team_of_the_week,
            K::PlayerOfTheMonth => &mut self.player_of_the_month,
            K::YoungPlayerOfTheMonth => &mut self.young_player_of_the_month,
            K::TeamOfTheMonthSelection => &mut self.team_of_the_month,
            K::YoungTeamOfTheMonthSelection => &mut self.young_team_of_the_month,
            K::TeamOfTheSeasonSelection => &mut self.team_of_the_season,
            K::TeamOfTheYearSelection => &mut self.team_of_the_year,
            K::PlayerOfTheSeason => &mut self.player_of_the_season,
            K::YoungPlayerOfTheSeason => &mut self.young_player_of_the_season,
            K::LeagueTopScorer => &mut self.league_top_scorer,
            K::LeagueTopAssists => &mut self.league_top_assists,
            K::LeagueGoldenGlove => &mut self.league_golden_glove,
            K::ContinentalPlayerOfYear => &mut self.continental_player_of_year,
            K::WorldPlayerOfYear => &mut self.world_player_of_year,
        };
        *slot = slot.saturating_add(1);
    }

    fn total(&self) -> u32 {
        self.player_of_the_week as u32
            + self.young_player_of_the_week as u32
            + self.team_of_the_week as u32
            + self.young_team_of_the_week as u32
            + self.player_of_the_month as u32
            + self.young_player_of_the_month as u32
            + self.team_of_the_month as u32
            + self.young_team_of_the_month as u32
            + self.team_of_the_season as u32
            + self.team_of_the_year as u32
            + self.player_of_the_season as u32
            + self.young_player_of_the_season as u32
            + self.league_top_scorer as u32
            + self.league_top_assists as u32
            + self.league_golden_glove as u32
            + self.continental_player_of_year as u32
            + self.world_player_of_year as u32
    }
}

fn build_cards(
    totals: &LeagueAwardTotals,
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
            totals.team_of_the_week,
            "weekly",
            "fa-people-group",
        ),
        card(
            "young_team_of_the_week",
            totals.young_team_of_the_week,
            "weekly young",
            "fa-user-graduate",
        ),
        card(
            "player_of_the_week",
            totals.player_of_the_week,
            "weekly",
            "fa-star",
        ),
        card(
            "young_player_of_the_week",
            totals.young_player_of_the_week,
            "weekly young",
            "fa-seedling",
        ),
    ];

    let monthly = vec![
        card(
            "team_of_the_month",
            totals.team_of_the_month,
            "monthly",
            "fa-people-group",
        ),
        card(
            "young_team_of_the_month",
            totals.young_team_of_the_month,
            "monthly young",
            "fa-user-graduate",
        ),
        card(
            "player_of_the_month",
            totals.player_of_the_month,
            "monthly",
            "fa-medal",
        ),
        card(
            "young_player_of_the_month",
            totals.young_player_of_the_month,
            "monthly young",
            "fa-award",
        ),
    ];

    let season = vec![
        card(
            "team_of_the_season",
            totals.team_of_the_season,
            "season",
            "fa-trophy",
        ),
        card(
            "team_of_the_year",
            totals.team_of_the_year,
            "season",
            "fa-trophy",
        ),
        card(
            "player_of_the_season",
            totals.player_of_the_season,
            "season",
            "fa-crown",
        ),
        card(
            "young_player_of_the_season",
            totals.young_player_of_the_season,
            "season young",
            "fa-crown",
        ),
        card(
            "top_scorer",
            totals.league_top_scorer,
            "season",
            "fa-futbol",
        ),
        card(
            "top_assists",
            totals.league_top_assists,
            "season",
            "fa-handshake",
        ),
        card(
            "golden_glove",
            totals.league_golden_glove,
            "season",
            "fa-hand-back-fist",
        ),
    ];

    let global = vec![
        card(
            "continental_player_of_the_year",
            totals.continental_player_of_year,
            "global",
            "fa-globe-europe",
        ),
        card(
            "world_player_of_the_year",
            totals.world_player_of_year,
            "global",
            "fa-earth-americas",
        ),
    ];

    (weekly, monthly, season, global)
}

/// Group the lifetime timeline into per-league blocks, sorted by
/// most-recent-award-date desc. The leading block (current league)
/// also carries the past-12-months chart; trailing blocks render
/// counts only. The `None` bucket — Continental / World POY — only
/// appears when the player has won one of those.
fn build_league_blocks(
    counts: &PlayerAwardsCount,
    data: &SimulatorData,
    i18n: &I18n,
) -> Vec<LeagueBlock> {
    use std::collections::HashMap;

    let mut totals_by_league: HashMap<Option<u32>, LeagueAwardTotals> = HashMap::new();
    let mut latest_by_league: HashMap<Option<u32>, chrono::NaiveDate> = HashMap::new();
    for entry in &counts.timeline {
        let key = entry.league_id;
        totals_by_league.entry(key).or_default().add(entry.kind);
        latest_by_league
            .entry(key)
            .and_modify(|d| {
                if entry.date > *d {
                    *d = entry.date;
                }
            })
            .or_insert(entry.date);
    }

    // Convert into Vec and sort newest-first by latest award date.
    let mut keys: Vec<Option<u32>> = totals_by_league.keys().copied().collect();
    keys.sort_by(|a, b| {
        let da = latest_by_league.get(a).copied();
        let db = latest_by_league.get(b).copied();
        db.cmp(&da)
    });

    let now = data.date.date();
    let mut blocks: Vec<LeagueBlock> = Vec::with_capacity(keys.len());
    for (idx, league_id) in keys.iter().enumerate() {
        let totals = totals_by_league.get(league_id).expect("populated above");
        let (weekly, monthly, season, global) = build_cards(totals, i18n);
        let has_weekly = weekly.iter().any(|c| c.count > 0);
        let has_monthly = monthly.iter().any(|c| c.count > 0);
        let has_season = season.iter().any(|c| c.count > 0);
        let has_global = global.iter().any(|c| c.count > 0);

        let (league_name, league_slug, country_code, country_name, country_slug) = match league_id
        {
            Some(id) => {
                let league = data.league(*id);
                let (name, slug, country_id) = league
                    .map(|l| (l.name.clone(), l.slug.clone(), Some(l.country_id)))
                    .unwrap_or_else(|| {
                        (i18n.t("awards_unknown_league").to_string(), String::new(), None)
                    });
                let country = country_id.and_then(|cid| data.country(cid));
                let (code, cname, cslug) = country
                    .map(|c| (c.code.clone(), c.name.clone(), c.slug.clone()))
                    .unwrap_or_default();
                (name, slug, code, cname, cslug)
            }
            None => (
                i18n.t("awards_section_global").to_string(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
        };

        // Only the leading league (most recent) gets the chart.
        let (month_bars, month_max) = if idx == 0 {
            build_month_bars_for_league(&counts.timeline, *league_id, now)
        } else {
            (Vec::new(), 0)
        };

        let has_chart = !month_bars.is_empty();

        blocks.push(LeagueBlock {
            league_name,
            league_slug,
            league_id: *league_id,
            country_code,
            country_name,
            country_slug,
            weekly_cards: weekly,
            monthly_cards: monthly,
            season_cards: season,
            global_cards: global,
            has_weekly,
            has_monthly,
            has_season,
            has_global,
            total: totals.total(),
            month_bars,
            month_max,
            has_chart,
        });
    }

    blocks
}

/// Bucket the lifetime timeline by calendar month for the past 12
/// months relative to `now`, filtered to a specific `league_id`.
/// Always returns exactly 12 bars (current month last) so the chart's
/// x-axis is stable even when the player went quiet — empty months
/// render as a flat track. Returns `(bars, max_count)`; max may be
/// zero if no awards fell in the window. The bar list is empty only
/// when the league had zero awards in the period (the chart panel is
/// then hidden).
fn build_month_bars_for_league(
    timeline: &[core::AwardTimelineEntry],
    league_id: Option<u32>,
    now: chrono::NaiveDate,
) -> (Vec<MonthBar>, u16) {
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

    // Index counts per (year, month), restricted to this league.
    use std::collections::HashMap;
    let mut counts_by_key: HashMap<(i32, u32), u16> = HashMap::new();
    let mut any_in_league = false;
    for entry in timeline {
        if entry.league_id != league_id {
            continue;
        }
        any_in_league = true;
        let key = (entry.date.year(), entry.date.month());
        let slot = counts_by_key.entry(key).or_insert(0);
        *slot = slot.saturating_add(1);
    }
    if !any_in_league {
        return (Vec::new(), 0);
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

