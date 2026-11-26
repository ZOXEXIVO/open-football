pub mod routes;

use crate::leagues::address::LeagueAddress;
use crate::player::{PlayerPageChrome, PlayerPageColors};
use crate::shared::friendly_source::FriendlySourceSlug;
use crate::shared::slug::{PlayerPage, PlayerSlug};
use crate::views::{MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::utils::FormattingUtils;
use core::{
    LiveCupSlice, PlayerLiveStatsInput, PlayerStatCompetitionKind, PlayerStatistics,
    PlayerStatisticsProjection, SimulatorData,
};
use serde::Deserialize;
use std::collections::HashMap;

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
    pub sub_title_flag: Option<SubTitleFlag>,
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
    /// Every club name the table shows, once each. The Club cells plant a
    /// zero-height copy of the whole set, which is what makes each name box
    /// as wide as the longest name and holds the column's left edge.
    pub club_name_sizers: Vec<String>,
    pub totals: PlayerHistoryStats,
    pub is_goalkeeper: bool,
    pub is_on_loan: bool,
    pub is_injured: bool,
    pub is_unhappy: bool,
    pub is_force_match_selection: bool,
    pub watch_key: String,
    pub events_count: usize,
    pub interested_clubs_count: usize,
    pub awards_count: u32,
    pub news_count: usize,
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
    /// Empty when no league answers to the row's slug any more.
    pub league_url: String,
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
    /// The side's division season by season, oldest first, resolved to
    /// display labels. Read through [`TeamLocationInfo::league_for_season`]
    /// so a row for a season the club spent in another division is never
    /// labelled with the one it sits in today.
    pub league_by_season: Vec<(u16, String, String)>,
    /// Most recent season the record above closed; past it it is silent.
    pub last_recorded_season: Option<u16>,
}

impl TeamLocationInfo {
    /// The division to print against a season. The recorded spell wins for
    /// any season the record covers; `league_name` / `league_slug` (the
    /// side's CURRENT division) answers outside it — correct for the
    /// campaign in progress, and the best available answer for a season
    /// older than the record.
    fn league_for_season(&self, season_start_year: u16) -> (String, String) {
        if self
            .last_recorded_season
            .is_some_and(|last| last >= season_start_year)
            && let Some((_, name, slug)) = self
                .league_by_season
                .iter()
                .rev()
                .find(|(year, _, _)| *year <= season_start_year)
        {
            return (name.clone(), slug.clone());
        }
        (self.league_name.clone(), self.league_slug.clone())
    }

    fn find(data: &SimulatorData, team_slug: &str) -> Option<TeamLocationInfo> {
        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for t in &club.teams.teams {
                        if t.slug == team_slug {
                            let league = t
                                .league_id
                                .and_then(|lid| {
                                    country.leagues.leagues.iter().find(|l| l.id == lid)
                                })
                                .or_else(|| {
                                    country.leagues.leagues.iter().find(|l| {
                                        l.table.rows.iter().any(|row| row.team_id == t.id)
                                    })
                                })
                                .or_else(|| country.leagues.leagues.first());

                            let (league_name, league_slug) = league
                                .map(|l| (l.name.clone(), l.slug.clone()))
                                .unwrap_or_default();

                            let league_by_season = t
                                .league_history
                                .spells()
                                .iter()
                                .filter_map(|spell| {
                                    country
                                        .leagues
                                        .leagues
                                        .iter()
                                        .find(|l| l.id == spell.league_id)
                                        .map(|l| {
                                            (
                                                spell.season_start_year,
                                                l.name.clone(),
                                                l.slug.clone(),
                                            )
                                        })
                                })
                                .collect();

                            return Some(TeamLocationInfo {
                                country_code: country.code.clone(),
                                country_name: country.name.clone(),
                                country_slug: country.slug.clone(),
                                league_name,
                                league_slug,
                                league_by_season,
                                last_recorded_season: t.league_history.last_recorded_season(),
                            });
                        }
                    }
                }
            }
        }
        None
    }
}

pub struct PlayerHistoryPage;

impl PlayerHistoryPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<PlayerHistoryRequest>,
    ) -> ApiResult<Response> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let guard = state.data.read().await;

        let simulator_data = guard
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let (player, team_opt, canonical) = match PlayerSlug::resolve_page(
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

        let view = HistoryView::build(player, team_opt, simulator_data, &i18n, &route_params.lang);
        let chrome = PlayerPageChrome::build(
            simulator_data,
            i18n,
            player,
            team_opt,
            &route_params.lang,
            &canonical,
            "history",
            PlayerPageColors::Grey,
        )?;
        let chrome = if team_opt.is_none() {
            chrome.without_club_activity()
        } else {
            chrome
        };

        Ok(PlayerHistoryTemplate {
            css_version: chrome.css_version,
            computer_name: chrome.computer_name,
            cpu_brand: chrome.cpu_brand,
            cores_count: chrome.cores_count,
            title: chrome.title,
            sub_title_prefix: chrome.sub_title_prefix,
            sub_title_suffix: chrome.sub_title_suffix,
            sub_title: chrome.sub_title,
            sub_title_link: chrome.sub_title_link,
            sub_title_flag: chrome.sub_title_flag,
            header_color: chrome.header_color,
            foreground_color: chrome.foreground_color,
            menu_sections: chrome.menu_sections,
            i18n: chrome.i18n,
            lang: chrome.lang,
            active_tab: chrome.active_tab,
            player_id: chrome.player_id,
            player_slug: chrome.player_slug,
            club_id: chrome.club_id,
            items: view.items,
            club_name_sizers: view.club_name_sizers,
            totals: view.totals,
            is_goalkeeper: view.is_goalkeeper,
            is_on_loan: chrome.is_on_loan,
            is_injured: chrome.is_injured,
            is_unhappy: chrome.is_unhappy,
            is_force_match_selection: chrome.is_force_match_selection,
            watch_key: chrome.watch_key,
            events_count: chrome.events_count,
            interested_clubs_count: chrome.interested_clubs_count,
            awards_count: chrome.awards_count,
            news_count: chrome.news_count,
        }
        .into_response())
    }
}

type BreakdownKey = (u16, String, String);

struct HistoryView {
    items: Vec<PlayerHistorySeasonItem>,
    club_name_sizers: Vec<String>,
    totals: PlayerHistoryStats,
    is_goalkeeper: bool,
}

impl HistoryView {
    fn build(
        player: &core::Player,
        team_opt: Option<&core::Team>,
        data: &SimulatorData,
        i18n: &I18n,
        lang: &str,
    ) -> Self {
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
        let friendly_source_slug = FriendlySourceSlug::resolve(player, team_opt, data);
        let live_input = PlayerLiveStatsInput {
            league: live_league,
            friendly: &player.friendly_statistics,
            cups: &live_cups,
            friendly_source_slug: &friendly_source_slug,
        };
        let view = PlayerStatisticsProjection::player_history_rows(
            &player.statistics_history,
            &live_input,
            data.date.date(),
        );
        let career_totals = PlayerStatisticsProjection::player_history_totals(&view);
        let breakdowns = PlayerStatisticsProjection::player_history_breakdowns(
            &player.statistics_history,
            &live_input,
            data.date.date(),
        );
        let breakdown_index = Self::breakdown_index(breakdowns, data, i18n);
        let items = Self::items(view, &breakdown_index, data, i18n, lang);
        let club_name_sizers = Self::club_name_sizers(&items);

        Self {
            items,
            club_name_sizers,
            totals: PlayerHistoryStats::from(&career_totals),
            is_goalkeeper: player.position().is_goalkeeper(),
        }
    }

    /// Index the per-row breakdowns by the same key `player_history_rows`
    /// groups on so each main row can find its own per-competition lines.
    /// Labels are pre-built here so the template only renders strings.
    fn breakdown_index(
        breakdowns: Vec<core::PlayerHistoryRowBreakdown>,
        data: &SimulatorData,
        i18n: &I18n,
    ) -> HashMap<BreakdownKey, Vec<PlayerHistoryCompetitionStats>> {
        breakdowns
            .into_iter()
            .map(|b| {
                let key: BreakdownKey = (b.season_start_year, b.team_slug, b.league_slug.clone());
                let row_league_slug = b.league_slug;
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
                            Self::resolve_label(data, i18n, "", c.competition_kind)
                        } else {
                            Self::resolve_label(data, i18n, &c.competition_slug, c.competition_kind)
                        };
                        PlayerHistoryCompetitionStats {
                            label,
                            stats: PlayerHistoryStats::from(&c.statistics),
                        }
                    })
                    .collect();
                (key, comps)
            })
            .collect()
    }

    /// Resolve a breakdown row's slug into the league / cup display name.
    /// For League and Friendly the slug is a league_slug (the senior
    /// league, or for youth-aliased Friendly the youth league), which the
    /// simulator's slug index can resolve. For Continental Cup the slug
    /// is one of the four well-known continental constants — those
    /// aren't in the slug index, so we fall back to an i18n key derived
    /// by replacing hyphens with underscores. Domestic cups land in the
    /// slug index too because cups register a league slug at startup.
    fn resolve_label(
        data: &SimulatorData,
        i18n: &I18n,
        slug: &str,
        kind: PlayerStatCompetitionKind,
    ) -> String {
        let kind_key = match kind {
            PlayerStatCompetitionKind::League => "league",
            PlayerStatCompetitionKind::ContinentalCup => "continental_cup",
            PlayerStatCompetitionKind::DomesticCup => "domestic_cup",
            PlayerStatCompetitionKind::Friendly => "friendly",
        };
        if !slug.is_empty() {
            if let Some(name) = data
                .indexes
                .as_ref()
                .and_then(|idx| idx.slug_indexes.get_league_by_slug(slug))
                .and_then(|id| data.league(id))
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
    }

    fn items(
        view: Vec<core::PlayerHistoryRow>,
        breakdown_index: &HashMap<BreakdownKey, Vec<PlayerHistoryCompetitionStats>>,
        data: &SimulatorData,
        i18n: &I18n,
        lang: &str,
    ) -> Vec<PlayerHistorySeasonItem> {
        let mut location_cache: HashMap<String, TeamLocationInfo> = HashMap::new();
        view.into_iter()
            .map(|item| {
                Self::season_item(item, breakdown_index, &mut location_cache, data, i18n, lang)
            })
            .collect()
    }

    fn season_item(
        item: core::PlayerHistoryRow,
        breakdown_index: &HashMap<BreakdownKey, Vec<PlayerHistoryCompetitionStats>>,
        location_cache: &mut HashMap<String, TeamLocationInfo>,
        data: &SimulatorData,
        i18n: &I18n,
        lang: &str,
    ) -> PlayerHistorySeasonItem {
        let location = if !item.team_slug.is_empty() {
            if !location_cache.contains_key(&item.team_slug)
                && let Some(info) = TeamLocationInfo::find(data, &item.team_slug)
            {
                location_cache.insert(item.team_slug.clone(), info);
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
        // The row's own stamp wins: it was written from the spell,
        // which recorded the division when the spell opened — during
        // the season, so it is that season's division even for a
        // borrower on a loan row. Only when the row carries no stamp
        // does the club's own record answer, and it answers PER SEASON.
        //
        // The old fallback was the club's league *today*, which is
        // right exactly until the club is promoted or relegated — after
        // that every earlier season on every one of its players' pages
        // silently re-labels itself to the new division. A club moves
        // between divisions; its history must not move with it.
        let (league_name, league_slug) = if !item.league_name.is_empty() {
            (item.league_name, item.league_slug)
        } else {
            location
                .map(|l| l.league_for_season(item.season.start_year))
                .unwrap_or_default()
        };

        // Every season row gets a breakdown so the dropdown icon and
        // accordion render consistently. When the projection doesn't
        // have one (synthetic gap-fill rows from fill_career_gaps, or a
        // key mismatch we haven't seen before), synthesise a single
        // League line from the row's own stats so the expansion still
        // shows the row's numbers under a labelled competition — under
        // the season's OWN division, resolved just above.
        let breakdown = breakdown_index
            .get(&breakdown_key)
            .cloned()
            .unwrap_or_else(|| {
                vec![PlayerHistoryCompetitionStats {
                    label: Self::resolve_label(
                        data,
                        i18n,
                        &league_slug,
                        PlayerStatCompetitionKind::League,
                    ),
                    stats: PlayerHistoryStats::from(&item.statistics),
                }]
            });

        PlayerHistorySeasonItem {
            season: item.season.display,
            team_name: item.team_name,
            team_slug: item.team_slug,
            is_loan: item.is_loan,
            transfer_fee: match item.transfer_fee {
                Some(f) if f > 0.0 => FormattingUtils::format_money(f),
                Some(_) => i18n.t("fee_free").to_string(),
                None => String::new(),
            },
            stats: PlayerHistoryStats::from(&item.statistics),
            country_code: location.map(|l| l.country_code.clone()).unwrap_or_default(),
            country_name: location.map(|l| l.country_name.clone()).unwrap_or_default(),
            country_slug: location.map(|l| l.country_slug.clone()).unwrap_or_default(),
            league_name,
            league_url: LeagueAddress::by_slug(data, &league_slug)
                .map(|address| address.url(lang))
                .unwrap_or_default(),
            breakdown,
        }
    }

    /// One entry per distinct club, in the order the rows introduce them. A
    /// career visits a handful of clubs, so a linear `contains` is cheaper
    /// than reaching for a set.
    fn club_name_sizers(items: &[PlayerHistorySeasonItem]) -> Vec<String> {
        let mut club_name_sizers: Vec<String> = Vec::new();
        for item in items {
            if !item.team_name.is_empty() && !club_name_sizers.contains(&item.team_name) {
                club_name_sizers.push(item.team_name.clone());
            }
        }
        club_name_sizers
    }
}

impl PlayerHistoryStats {
    fn from(stats: &PlayerStatistics) -> PlayerHistoryStats {
        PlayerHistoryStats {
            played: stats.played,
            played_subs: stats.played_subs,
            goals: stats.goals,
            assists: stats.assists,
            player_of_the_match: stats.player_of_the_match,
            average_rating: stats.display_average_rating(),
            conceded: stats.conceded,
            clean_sheets: stats.clean_sheets,
        }
    }
}
