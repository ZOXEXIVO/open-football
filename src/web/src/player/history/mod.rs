pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::friendly_source::FriendlySourceSlug;
use crate::common::slug::{PlayerPage, resolve_player_page};
use crate::player::decisions::PlayerDecisionsCounter;
use crate::player::events::PlayerEventsCounter;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::utils::FormattingUtils;
use core::{
    LiveCupSlice, PlayerLiveStatsInput, PlayerStatCompetitionKind, PlayerStatistics,
    PlayerStatisticsProjection, PlayerStatusType, SimulatorData,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PlayerHistoryRequest {
    pub lang: String,
    pub player_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/history/index.html")]
pub struct PlayerHistoryTemplate {
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
    pub items: Vec<PlayerHistorySeasonItem>,
    pub totals: PlayerHistoryStats,
    pub is_goalkeeper: bool,
    pub is_on_loan: bool,
    pub is_injured: bool,
    pub is_unhappy: bool,
    pub is_force_match_selection: bool,
    pub is_on_watchlist: bool,
    pub events_count: usize,
    pub decisions_count: usize,
    pub interested_clubs_count: usize,
    pub awards_count: u32,
}

pub struct PlayerHistorySeasonItem {
    pub season: String,
    pub team_name: String,
    pub team_slug: String,
    pub is_loan: bool,
    pub transfer_fee: String,
    pub stats: PlayerHistoryStats,
    pub country_code: String,
    pub country_name: String,
    pub country_slug: String,
    pub league_name: String,
    pub league_slug: String,
    pub breakdown: Vec<PlayerHistoryCompetitionStats>,
}

#[derive(Clone)]
pub struct PlayerHistoryStats {
    pub played: u16,
    pub played_subs: u16,
    pub goals: u16,
    pub assists: u16,
    pub player_of_the_match: u8,
    pub average_rating: String,
    pub conceded: u16,
    pub clean_sheets: u16,
}

#[derive(Clone)]
pub struct PlayerHistoryCompetitionStats {
    pub label: String,
    pub stats: PlayerHistoryStats,
}

struct TeamLocationInfo {
    pub country_code: String,
    pub country_name: String,
    pub country_slug: String,
    pub league_name: String,
    pub league_slug: String,
}

fn find_team_location(simulator_data: &SimulatorData, team_slug: &str) -> Option<TeamLocationInfo> {
    for continent in &simulator_data.continents {
        for country in &continent.countries {
            for club in &country.clubs {
                for t in &club.teams.teams {
                    if t.slug == team_slug {
                        let league = t
                            .league_id
                            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                            .or_else(|| {
                                country
                                    .leagues
                                    .leagues
                                    .iter()
                                    .find(|l| l.table.rows.iter().any(|row| row.team_id == t.id))
                            })
                            .or_else(|| country.leagues.leagues.first());

                        let (league_name, league_slug) = league
                            .map(|l| (l.name.clone(), l.slug.clone()))
                            .unwrap_or_default();

                        return Some(TeamLocationInfo {
                            country_code: country.code.clone(),
                            country_name: country.name.clone(),
                            country_slug: country.slug.clone(),
                            league_name,
                            league_slug,
                        });
                    }
                }
            }
        }
    }
    None
}

pub async fn player_history_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<PlayerHistoryRequest>,
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
        "/history",
    )? {
        PlayerPage::Found {
            player,
            team,
            canonical_slug,
        } => (player, team, canonical_slug),
        PlayerPage::Redirect(r) => return Ok(r),
    };

    let has_no_team = team_opt.is_none();

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

    // History rows come from the central projection. The projection
    // reads `player.statistics` (live league counter) for the active
    // spell's current-season row and the per-competition live cup
    // slices for continental-cup folding; both sources are passed
    // through the same `PlayerLiveStatsInput` shape so the renderer
    // never reaches into history internals.
    //
    // Retired players don't have an active spell, but the projection
    // still tolerates empty live inputs and renders only the frozen
    // history rows.
    let empty_live = PlayerStatistics::default();
    let live_league = if team_opt.is_some() {
        &player.statistics
    } else {
        &empty_live
    };
    let live_friendly = &player.friendly_statistics;
    let live_cups: Vec<LiveCupSlice<'_>> = player
        .cup_statistics_by_competition
        .iter()
        .map(|c| LiveCupSlice {
            competition_slug: c.competition_slug.as_str(),
            competition_name: String::new(),
            statistics: &c.statistics,
        })
        .collect();
    // Source slug for the live Friendly entry — shared with the Overview
    // page so both label the Friendly row from the same league (see
    // `FriendlySourceSlug` for the lookup order).
    let friendly_source_slug: String =
        FriendlySourceSlug::resolve(player, team_opt, simulator_data);
    let live_input = PlayerLiveStatsInput {
        league: live_league,
        friendly: live_friendly,
        cups: &live_cups,
        friendly_source_slug: &friendly_source_slug,
    };
    let view = PlayerStatisticsProjection::player_history_rows(
        &player.statistics_history,
        &live_input,
        simulator_data.date.date(),
    );
    let career_totals = PlayerStatisticsProjection::player_history_totals(&view);
    let breakdowns = PlayerStatisticsProjection::player_history_breakdowns(
        &player.statistics_history,
        &live_input,
        simulator_data.date.date(),
    );

    let mut location_cache: std::collections::HashMap<String, TeamLocationInfo> =
        std::collections::HashMap::new();

    // Index the per-row breakdowns by the same key
    // `player_history_rows` groups on so each main row can find its
    // own per-competition lines. Pre-build the i18n labels here so the
    // template only renders strings.
    let to_history_stats = |stats: &PlayerStatistics| PlayerHistoryStats {
        played: stats.played,
        played_subs: stats.played_subs,
        goals: stats.goals,
        assists: stats.assists,
        player_of_the_match: stats.player_of_the_match,
        average_rating: core::PlayerStatistics::format_rating(stats.average_rating),
        conceded: stats.conceded,
        clean_sheets: stats.clean_sheets,
    };
    // Resolve a breakdown row's slug into the league / cup display name.
    // For League and Friendly the slug is a league_slug (the senior
    // league, or for youth-aliased Friendly the youth league), which the
    // simulator's slug index can resolve. For Continental Cup the slug
    // is one of the four well-known continental constants — those
    // aren't in the slug index, so we fall back to an i18n key derived
    // by replacing hyphens with underscores. Domestic cups land in the
    // slug index too because cups register a league slug at startup.
    // Legacy aggregated rows (slug == row's main league_slug for a
    // Cup kind) are detected by the caller and fall through to the
    // generic kind label.
    let resolve_label = |slug: &str, kind: PlayerStatCompetitionKind| -> String {
        let kind_key = match kind {
            PlayerStatCompetitionKind::League => "league",
            PlayerStatCompetitionKind::ContinentalCup => "continental_cup",
            PlayerStatCompetitionKind::DomesticCup => "domestic_cup",
            PlayerStatCompetitionKind::Friendly => "friendly",
        };
        if !slug.is_empty() {
            if let Some(name) = simulator_data
                .indexes
                .as_ref()
                .and_then(|idx| idx.slug_indexes.get_league_by_slug(slug))
                .and_then(|id| simulator_data.league(id))
                .map(|l| l.name.clone())
            {
                return name;
            }
            let key = slug.replace('-', "_");
            let translated = i18n.t(&key);
            if translated != key.as_str() {
                return translated.to_string();
            }
        }
        i18n.t(kind_key).to_string()
    };

    // Match records group by (year, team, league) — loan flag is row
    // metadata, not part of the grouping. Keeping this in sync with the
    // projection's grouping is what makes the breakdown lookup robust.
    type BreakdownKey = (u16, String, String);
    let breakdown_index: std::collections::HashMap<
        BreakdownKey,
        Vec<PlayerHistoryCompetitionStats>,
    > = breakdowns
        .into_iter()
        .map(|b| {
            let key: BreakdownKey = (b.season_start_year, b.team_slug, b.league_slug.clone());
            let row_league_slug = b.league_slug.clone();
            let comps = b
                .competitions
                .into_iter()
                .map(|c| {
                    // Slug-matches-row defaults to the generic kind
                    // label for non-League kinds. Three patterns
                    // land here and all want "Cup" / "Continental
                    // Cup" / "Friendly" rather than the senior
                    // league name:
                    //   1. Legacy aggregated cup entries (written
                    //      before per-cup recording) — slug was set
                    //      to the team's league_slug.
                    //   2. Senior pre-season Friendly — no
                    //      specific source league; the recorder
                    //      defaults to the team's league_slug.
                    //   3. New cup entries that wound up with the
                    //      team's league slug for any reason.
                    // For a youth-aliased Friendly the slug is the
                    // YOUTH team's league_slug, which differs from
                    // the row's main league_slug; that path
                    // resolves to the youth league name.
                    let use_generic_label =
                        !matches!(c.competition_kind, PlayerStatCompetitionKind::League)
                            && c.competition_slug == row_league_slug;
                    let label = if use_generic_label {
                        resolve_label("", c.competition_kind)
                    } else {
                        resolve_label(&c.competition_slug, c.competition_kind)
                    };
                    PlayerHistoryCompetitionStats {
                        label,
                        stats: to_history_stats(&c.statistics),
                    }
                })
                .collect();
            (key, comps)
        })
        .collect();

    let items: Vec<PlayerHistorySeasonItem> = view
        .into_iter()
        .map(|item| {
            let location = if !item.team_slug.is_empty() {
                if !location_cache.contains_key(&item.team_slug) {
                    if let Some(info) = find_team_location(simulator_data, &item.team_slug) {
                        location_cache.insert(item.team_slug.clone(), info);
                    }
                }
                location_cache.get(&item.team_slug)
            } else {
                None
            };

            // Look up the breakdown using the row's ORIGINAL league_slug
            // — the breakdown index is keyed on the slug the projection
            // grouped with, before any web-side fallback below.
            let breakdown_key: BreakdownKey = (
                item.season.start_year,
                item.team_slug.clone(),
                item.league_slug.clone(),
            );
            // Every season row gets a breakdown so the dropdown icon
            // and accordion render consistently. When the projection
            // doesn't have one (synthetic gap-fill rows from
            // fill_career_gaps, or a key mismatch we haven't seen
            // before), synthesise a single League line from the
            // row's own stats so the expansion still shows the row's
            // numbers under a labelled competition.
            let fallback_league_slug = item.league_slug.clone();
            let breakdown = breakdown_index
                .get(&breakdown_key)
                .cloned()
                .unwrap_or_else(|| {
                    vec![PlayerHistoryCompetitionStats {
                        label: resolve_label(
                            &fallback_league_slug,
                            PlayerStatCompetitionKind::League,
                        ),
                        stats: to_history_stats(&item.statistics),
                    }]
                });

            // If league name is empty, fall back to team's current league
            let (league_name, league_slug) = if !item.league_name.is_empty() {
                (item.league_name, item.league_slug)
            } else {
                location
                    .map(|l| (l.league_name.clone(), l.league_slug.clone()))
                    .unwrap_or_default()
            };

            PlayerHistorySeasonItem {
                season: item.season.display,
                team_name: item.team_name,
                team_slug: item.team_slug,
                is_loan: item.is_loan,
                transfer_fee: match item.transfer_fee {
                    Some(f) if f > 0.0 => FormattingUtils::format_money(f),
                    Some(_) => "Free".to_string(),
                    None => String::new(),
                },
                stats: to_history_stats(&item.statistics),
                country_code: location.map(|l| l.country_code.clone()).unwrap_or_default(),
                country_name: location.map(|l| l.country_name.clone()).unwrap_or_default(),
                country_slug: location.map(|l| l.country_slug.clone()).unwrap_or_default(),
                league_name,
                league_slug,
                breakdown,
            }
        })
        .collect();

    let totals = to_history_stats(&career_totals);

    if has_no_team {
        let sub_title = if player.is_retired() {
            i18n.t("retired").to_string()
        } else {
            i18n.t("free_agent").to_string()
        };
        Ok(PlayerHistoryTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            title,
            sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
            sub_title_suffix: String::new(),
            sub_title,
            sub_title_link: String::new(),
            sub_title_country_code: String::new(),
            header_color: "#808080".to_string(),
            foreground_color: "#ffffff".to_string(),
            menu_sections: Vec::new(),
            i18n,
            lang: route_params.lang.clone(),
            active_tab: "history",
            player_id: player.id,
            player_slug: canonical.clone(),
            club_id: 0,
            items,
            totals,
            is_goalkeeper: player.position().is_goalkeeper(),
            is_on_loan: false,
            is_injured: false,
            is_unhappy: false,
            is_force_match_selection: player.is_force_match_selection,
            is_on_watchlist: simulator_data.watchlist.contains(&player.id),
            events_count: PlayerEventsCounter::count(player),
            decisions_count: PlayerDecisionsCounter::count_recent(
                player,
                simulator_data.date.date(),
            ),
            interested_clubs_count: simulator_data.clubs_interested_in_player(player.id).len(),
            awards_count: player.awards_count.total(),
        }
        .into_response())
    } else {
        let team = team_opt.unwrap();

        Ok(PlayerHistoryTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            title,
            sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
            sub_title_suffix: String::new(),
            sub_title: team.name.clone(),
            sub_title_link: format!("/{}/teams/{}", &route_params.lang, &team.slug),
            sub_title_country_code: String::new(),
            header_color: simulator_data
                .club(team.club_id)
                .map(|c| c.colors.background.clone())
                .unwrap_or_default(),
            foreground_color: simulator_data
                .club(team.club_id)
                .map(|c| c.colors.foreground.clone())
                .unwrap_or_default(),
            menu_sections: {
                let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
                let current_path = format!("/{}/teams/{}", &route_params.lang, &team.slug);
                let mp = views::MenuParams {
                    i18n: &i18n,
                    lang: &route_params.lang,
                    current_path: &current_path,
                    country_name: cn,
                    country_slug: cs,
                };
                views::team_menu(&mp, &neighbor_refs, &league_refs)
            },
            i18n,
            lang: route_params.lang.clone(),
            active_tab: "history",
            player_id: player.id,
            player_slug: canonical,
            club_id: team.club_id,
            items,
            totals,
            is_goalkeeper: player.position().is_goalkeeper(),
            is_on_loan: player.is_on_loan(),
            is_injured: player.player_attributes.is_injured,
            is_unhappy: player.statuses.get().contains(&PlayerStatusType::Unh),
            is_force_match_selection: player.is_force_match_selection,
            is_on_watchlist: simulator_data.watchlist.contains(&player.id),
            events_count: PlayerEventsCounter::count(player),
            decisions_count: PlayerDecisionsCounter::count_recent(
                player,
                simulator_data.date.date(),
            ),
            interested_clubs_count: simulator_data.clubs_interested_in_player(player.id).len(),
            awards_count: player.awards_count.total(),
        }
        .into_response())
    }
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
