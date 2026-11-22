pub mod routes;

use crate::leagues::address::LeagueAddress;
use crate::player::{PlayerPageChrome, PlayerPageColors};
use crate::shared::slug::{PlayerPage, PlayerSlug};
use crate::views::{MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::{PlayerAwardsCount, SimulatorData};
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

/// Award cards split into the page's five groups.
struct AwardCardGroups {
    weekly: Vec<AwardCard>,
    monthly: Vec<AwardCard>,
    season: Vec<AwardCard>,
    silverware: Vec<AwardCard>,
    global: Vec<AwardCard>,
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

/// Career-wide hero strip at the top of the page — lifetime totals
/// rolled up into one row with a banner and four category tiles.
pub struct SummaryBlock {
    pub weekly_total: u32,
    pub monthly_total: u32,
    pub season_total: u32,
    pub global_total: u32,
    pub has_weekly: bool,
    pub has_monthly: bool,
    pub has_season: bool,
    pub has_global: bool,
    pub total: u32,
}

/// One league section on the Awards tab. Sorted most-recent-first.
/// `league_id == None` is the global bucket (Continental / World POY)
/// — only rendered when the player has actually won one of those.
/// The past-12-months chart lives at the page level (right under the
/// Career Summary hero) and is no longer tied to a single league.
pub struct LeagueBlock {
    pub league_name: String,
    /// Raw slug — preserved for callers / tests that need the
    /// underlying identifier. Templates link via the precomputed
    /// `awards_url` instead so /cups vs /leagues routing is owned by
    /// the server (and not duplicated across each card).
    #[allow(dead_code)]
    pub league_slug: String,
    pub league_id: Option<u32>,
    /// True when this block is a domestic cup (FA Cup, Copa del Rey,
    /// …). Cups don't have a league awards page — they route through
    /// `/cups/<slug>`. Templates use this to pick the right link target
    /// and to apply silverware-tinted styling.
    pub is_cup: bool,
    /// Precomputed link target for the league/cup header + per-card
    /// chip. Empty when the block points at the global Continental /
    /// World POY bucket (no per-league destination).
    pub awards_url: String,
    /// Country card shown to the left of the league name when the
    /// block is a real league (not the global / Continental bucket).
    /// `code` is the lowercased ISO code used by the flag sprite.
    pub country_code: String,
    pub country_name: String,
    pub country_slug: String,
    pub weekly_cards: Vec<AwardCard>,
    pub monthly_cards: Vec<AwardCard>,
    pub season_cards: Vec<AwardCard>,
    pub silverware_cards: Vec<AwardCard>,
    pub global_cards: Vec<AwardCard>,
    pub has_weekly: bool,
    pub has_monthly: bool,
    pub has_season: bool,
    pub has_silverware: bool,
    pub has_global: bool,
    pub total: u32,
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
    pub is_on_loan: bool,
    pub is_injured: bool,
    pub is_unhappy: bool,
    pub is_force_match_selection: bool,
    pub is_on_watchlist: bool,
    pub events_count: usize,
    pub interested_clubs_count: usize,
    pub awards_count: u32,
    pub news_count: usize,
    pub summary: SummaryBlock,
    /// Career-wide past-12-months chart, rendered right under the
    /// Career Summary hero. Aggregates every league.
    pub chart_month_bars: Vec<MonthBar>,
    pub chart_month_max: u16,
    pub has_chart: bool,
    pub league_blocks: Vec<LeagueBlock>,
}

pub struct PlayerAwardsPage;

impl PlayerAwardsPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<PlayerAwardsRequest>,
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
            "/awards",
        )? {
            PlayerPage::Found {
                player,
                team,
                canonical_slug,
            } => (player, team, canonical_slug),
            PlayerPage::Redirect(r) => return Ok(r),
        };

        let counts = &player.awards_count;
        let summary = SummaryBlock::build(counts, &i18n);
        let now = simulator_data.date.date();
        let (chart_month_bars, chart_month_max) = MonthBar::chart(&counts.timeline, now, &i18n);
        let has_chart = !chart_month_bars.is_empty();
        let league_blocks = LeagueBlock::blocks(counts, simulator_data, &i18n);
        let chrome = PlayerPageChrome::build(
            simulator_data,
            i18n,
            player,
            team_opt,
            &route_params.lang,
            &canonical,
            "awards",
            PlayerPageColors::Grey,
        )?;

        Ok(PlayerAwardsTemplate {
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
            is_on_loan: chrome.is_on_loan,
            is_injured: chrome.is_injured,
            is_unhappy: chrome.is_unhappy,
            is_force_match_selection: chrome.is_force_match_selection,
            is_on_watchlist: chrome.is_on_watchlist,
            events_count: chrome.events_count,
            interested_clubs_count: chrome.interested_clubs_count,
            awards_count: chrome.awards_count,
            news_count: chrome.news_count,
            summary,
            chart_month_bars,
            chart_month_max,
            has_chart,
            league_blocks,
        }
        .into_response())
    }
}

impl SummaryBlock {
    fn build(counts: &PlayerAwardsCount, i18n: &I18n) -> SummaryBlock {
        let totals = LeagueAwardTotals::from_lifetime(counts);
        let AwardCardGroups {
            weekly,
            monthly,
            season,
            silverware,
            global,
        } = AwardCardGroups::build(&totals, i18n);
        let sum = |cards: &[AwardCard]| -> u32 { cards.iter().map(|c| c.count as u32).sum() };
        let weekly_total = sum(&weekly);
        let monthly_total = sum(&monthly);
        // Silverware totals fold into the season tile on the summary hero —
        // a trophy medal is a season-scoped honour. The per-league block
        // below still renders silverware in its own group.
        let season_total = sum(&season) + sum(&silverware);
        let global_total = sum(&global);
        SummaryBlock {
            has_weekly: weekly_total > 0,
            has_monthly: monthly_total > 0,
            has_season: season_total > 0,
            has_global: global_total > 0,
            weekly_total,
            monthly_total,
            season_total,
            global_total,
            total: totals.total(),
        }
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
    domestic_cup_winner: u16,
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
            domestic_cup_winner: counts.domestic_cup_winner,
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
            K::DomesticCupWinner => &mut self.domestic_cup_winner,
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
            + self.domestic_cup_winner as u32
    }
}

impl AwardCardGroups {
    fn build(totals: &LeagueAwardTotals, i18n: &I18n) -> AwardCardGroups {
        Self {
            weekly: Self::weekly(totals, i18n),
            monthly: Self::monthly(totals, i18n),
            season: Self::season(totals, i18n),
            silverware: Self::silverware(totals, i18n),
            global: Self::global(totals, i18n),
        }
    }

    fn card(
        i18n: &I18n,
        title_key: &str,
        count: u16,
        tone: &'static str,
        icon: &'static str,
    ) -> AwardCard {
        AwardCard {
            title: i18n.t(title_key).to_string(),
            count,
            tone,
            icon,
        }
    }

    // Icon mapping uses Font Awesome glyphs already in the page's icon
    // pack. Team awards take group icons; senior individual awards take
    // a trophy/star/medal; young variants overlay a graduate / seedling.
    fn weekly(totals: &LeagueAwardTotals, i18n: &I18n) -> Vec<AwardCard> {
        vec![
            Self::card(
                i18n,
                "team_of_the_week",
                totals.team_of_the_week,
                "weekly",
                "fa-people-group",
            ),
            Self::card(
                i18n,
                "young_team_of_the_week",
                totals.young_team_of_the_week,
                "weekly young",
                "fa-user-graduate",
            ),
            Self::card(
                i18n,
                "player_of_the_week",
                totals.player_of_the_week,
                "weekly",
                "fa-star",
            ),
            Self::card(
                i18n,
                "young_player_of_the_week",
                totals.young_player_of_the_week,
                "weekly young",
                "fa-seedling",
            ),
        ]
    }

    fn monthly(totals: &LeagueAwardTotals, i18n: &I18n) -> Vec<AwardCard> {
        vec![
            Self::card(
                i18n,
                "team_of_the_month",
                totals.team_of_the_month,
                "monthly",
                "fa-people-group",
            ),
            Self::card(
                i18n,
                "young_team_of_the_month",
                totals.young_team_of_the_month,
                "monthly young",
                "fa-user-graduate",
            ),
            Self::card(
                i18n,
                "player_of_the_month",
                totals.player_of_the_month,
                "monthly",
                "fa-medal",
            ),
            Self::card(
                i18n,
                "young_player_of_the_month",
                totals.young_player_of_the_month,
                "monthly young",
                "fa-award",
            ),
        ]
    }

    fn season(totals: &LeagueAwardTotals, i18n: &I18n) -> Vec<AwardCard> {
        vec![
            Self::card(
                i18n,
                "team_of_the_season",
                totals.team_of_the_season,
                "season",
                "fa-trophy",
            ),
            Self::card(
                i18n,
                "team_of_the_year",
                totals.team_of_the_year,
                "season",
                "fa-trophy",
            ),
            Self::card(
                i18n,
                "player_of_the_season",
                totals.player_of_the_season,
                "season",
                "fa-crown",
            ),
            Self::card(
                i18n,
                "young_player_of_the_season",
                totals.young_player_of_the_season,
                "season young",
                "fa-crown",
            ),
            Self::card(
                i18n,
                "top_scorer",
                totals.league_top_scorer,
                "season",
                "fa-futbol",
            ),
            Self::card(
                i18n,
                "top_assists",
                totals.league_top_assists,
                "season",
                "fa-handshake",
            ),
            Self::card(
                i18n,
                "golden_glove",
                totals.league_golden_glove,
                "season",
                "fa-hand-back-fist",
            ),
        ]
    }

    // Silverware — team-level trophy medals. Distinct from individual
    // season awards (top scorer / POS / etc.) so the page can render
    // them under their own header with a trophy tint.
    fn silverware(totals: &LeagueAwardTotals, i18n: &I18n) -> Vec<AwardCard> {
        vec![Self::card(
            i18n,
            "domestic_cup_winner",
            totals.domestic_cup_winner,
            "silverware",
            "fa-trophy",
        )]
    }

    fn global(totals: &LeagueAwardTotals, i18n: &I18n) -> Vec<AwardCard> {
        vec![
            Self::card(
                i18n,
                "continental_player_of_the_year",
                totals.continental_player_of_year,
                "global",
                "fa-globe-europe",
            ),
            Self::card(
                i18n,
                "world_player_of_the_year",
                totals.world_player_of_year,
                "global",
                "fa-earth-americas",
            ),
        ]
    }
}

impl LeagueBlock {
    /// Group the lifetime timeline into per-league blocks, sorted by
    /// most-recent-award-date desc. The leading block (current league)
    /// also carries the past-12-months chart; trailing blocks render
    /// counts only. The `None` bucket — Continental / World POY — only
    /// appears when the player has won one of those.
    fn blocks(counts: &PlayerAwardsCount, data: &SimulatorData, i18n: &I18n) -> Vec<LeagueBlock> {
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

        let mut blocks: Vec<LeagueBlock> = Vec::with_capacity(keys.len());
        for league_id in keys.iter() {
            let totals = totals_by_league.get(league_id).expect("populated above");
            blocks.push(Self::block(*league_id, totals, data, i18n));
        }

        blocks
    }

    fn block(
        league_id: Option<u32>,
        totals: &LeagueAwardTotals,
        data: &SimulatorData,
        i18n: &I18n,
    ) -> LeagueBlock {
        let AwardCardGroups {
            weekly,
            monthly,
            season,
            silverware,
            global,
        } = AwardCardGroups::build(totals, i18n);
        let has_weekly = weekly.iter().any(|c| c.count > 0);
        let has_monthly = monthly.iter().any(|c| c.count > 0);
        let has_season = season.iter().any(|c| c.count > 0);
        let has_silverware = silverware.iter().any(|c| c.count > 0);
        let has_global = global.iter().any(|c| c.count > 0);

        // Resolve league + cup flag + country card. `data.league(id)`
        // already covers both standings leagues and the country's
        // domestic cup (which lives outside the league collection), so
        // `is_cup` reflects whichever side resolved the id.
        let (league_name, league_slug, is_cup, country_code, country_name, country_slug) =
            Self::identity(league_id, data, i18n);

        // Domestic cups don't have an awards subpage — link to the
        // bracket route instead. Leagues keep their existing awards
        // tab. Global bucket (empty slug) renders without a link.
        let awards_url = Self::awards_url(&league_slug, is_cup, &country_slug);

        LeagueBlock {
            league_name,
            league_slug,
            league_id,
            is_cup,
            awards_url,
            country_code,
            country_name,
            country_slug,
            weekly_cards: weekly,
            monthly_cards: monthly,
            season_cards: season,
            silverware_cards: silverware,
            global_cards: global,
            has_weekly,
            has_monthly,
            has_season,
            has_silverware,
            has_global,
            total: totals.total(),
        }
    }

    fn identity(
        league_id: Option<u32>,
        data: &SimulatorData,
        i18n: &I18n,
    ) -> (String, String, bool, String, String, String) {
        match league_id {
            Some(id) => {
                let league = data.league(id);
                let (name, slug, is_cup_flag, country_id) = league
                    .map(|l| (l.name.clone(), l.slug.clone(), l.is_cup, Some(l.country_id)))
                    .unwrap_or_else(|| {
                        (
                            i18n.t("awards_unknown_league").to_string(),
                            String::new(),
                            false,
                            None,
                        )
                    });
                let country = country_id.and_then(|cid| data.country(cid));
                let (code, cname, cslug) = country
                    .map(|c| (c.code.clone(), c.name.clone(), c.slug.clone()))
                    .unwrap_or_default();
                (name, slug, is_cup_flag, code, cname, cslug)
            }
            None => (
                i18n.t("awards_section_global").to_string(),
                String::new(),
                false,
                String::new(),
                String::new(),
                String::new(),
            ),
        }
    }

    fn awards_url(league_slug: &str, is_cup: bool, country_slug: &str) -> String {
        if league_slug.is_empty() {
            String::new()
        } else if is_cup {
            format!("/cups/{}", league_slug)
        } else {
            format!(
                "{}/awards",
                LeagueAddress::new(country_slug, league_slug).path()
            )
        }
    }
}

impl MonthBar {
    /// Career-wide chart data — bucket the lifetime timeline by calendar
    /// month for the past 12 months relative to `now`, aggregating every
    /// league. Always returns exactly 12 bars (current month last) so the
    /// chart's x-axis is stable when the player went quiet. Returns an
    /// empty vec only when the player has zero awards total (chart panel
    /// is then hidden).
    fn chart(
        timeline: &[core::AwardTimelineEntry],
        now: chrono::NaiveDate,
        i18n: &I18n,
    ) -> (Vec<MonthBar>, u16) {
        use chrono::Datelike;

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

        use std::collections::HashMap;
        let mut counts_by_key: HashMap<(i32, u32), u16> = HashMap::new();
        let mut any = false;
        for entry in timeline {
            any = true;
            let key = (entry.date.year(), entry.date.month());
            let slot = counts_by_key.entry(key).or_insert(0);
            *slot = slot.saturating_add(1);
        }
        if !any {
            return (Vec::new(), 0);
        }

        let month_short = |m: u32| -> String {
            let key = match m {
                1 => "month_jan",
                2 => "month_feb",
                3 => "month_mar",
                4 => "month_apr",
                5 => "month_may",
                6 => "month_jun",
                7 => "month_jul",
                8 => "month_aug",
                9 => "month_sep",
                10 => "month_oct",
                11 => "month_nov",
                12 => "month_dec",
                _ => "",
            };
            i18n.t(key).to_string()
        };

        let bars_data: Vec<(String, i32, u16)> = keys
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
                    month,
                    year: year.to_string(),
                    count,
                    height_pct,
                }
            })
            .collect();

        (bars, max_count)
    }
}

#[cfg(test)]
mod tests {
    /// Every locale shipped under `assets/i18n` must carry the
    /// `domestic_cup_winner` translation key. Without this, the player
    /// awards page would surface the raw key as a fallback for any
    /// non-English locale — visibly broken UI for cup-winning squads.
    #[test]
    fn every_locale_has_domestic_cup_winner_key() {
        // Each locale's body is included at compile time so the check
        // travels with the binary. Loading via the runtime I18nManager
        // would tie the test to filesystem layout under CI; a static
        // include is both faster and more deterministic.
        let locales: &[(&str, &str)] = &[
            ("en", include_str!("../../../assets/i18n/en.json")),
            ("de", include_str!("../../../assets/i18n/de.json")),
            ("es", include_str!("../../../assets/i18n/es.json")),
            ("fr", include_str!("../../../assets/i18n/fr.json")),
            ("ja", include_str!("../../../assets/i18n/ja.json")),
            ("pt", include_str!("../../../assets/i18n/pt.json")),
            ("ru", include_str!("../../../assets/i18n/ru.json")),
            ("tr", include_str!("../../../assets/i18n/tr.json")),
            ("zh", include_str!("../../../assets/i18n/zh.json")),
        ];
        for (lang, body) in locales {
            // Match the key with its JSON quoting so we don't false-
            // positive on a substring like `domestic_cup_winner_medal`.
            assert!(
                body.contains("\"domestic_cup_winner\""),
                "locale {} is missing the `domestic_cup_winner` key",
                lang
            );
            assert!(
                body.contains("\"awards_section_silverware\""),
                "locale {} is missing the `awards_section_silverware` key",
                lang
            );
        }
    }
}
