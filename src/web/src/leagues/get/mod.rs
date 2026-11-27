mod fixtures;
mod highlights;
pub mod routes;
#[cfg(test)]
mod tests;

use self::fixtures::{LeagueFixtures, MatchFanContext};
use self::highlights::LeagueHighlights;
use crate::leagues::address::{LeagueAddress, LeaguePage};
use crate::leagues::newspaper::LeagueNewspaperCounter;
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::shared::slug::PlayerSlug;
use crate::views::{self, MenuSection, SubTitleFlag};
use crate::{ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Duration;
use core::continent::{CompetitionTier, ContinentalQualification, QualificationBand};
use core::league::{
    League, LeagueLadder, LeagueTableRow as CoreTableRow, PlayoffStage, ScheduleItem,
    TieBreakPolicy,
};
use core::r#match::{GoalDetail, MatchStatisticType, Score};
use core::{Country, SimulatorData};
use serde::Deserialize;
use std::cmp::Reverse;
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct LeagueGetRequest {
    pub lang: String,
    pub country_slug: String,
    pub league_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "leagues/get/index.html")]
pub struct LeagueGetTemplate {
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
    pub league_url: String,
    /// Monthly editions on the division's shelf, for the tabbar badge.
    pub newspaper_count: usize,
    /// Split-season leagues label their standings with the current
    /// tournament and append the annual aggregate below.
    pub split_season: bool,
    /// Current tournament's display name (Torneo Apertura/Clausura);
    /// empty for regular leagues.
    pub tournament_label: String,
    /// Annual aggregate across every zone of the competition, best-first;
    /// empty for regular leagues.
    pub annual_table_rows: Vec<LeagueTableRow>,
    pub annual_legend: Vec<StandingsLegendItem>,
    pub table_rows: Vec<LeagueTableRow>,
    pub legend: Vec<StandingsLegendItem>,
    pub current_tour_schedule: Vec<TourSchedule>,
    pub highlights_url: Option<String>,
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
    pub league_url: String,
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
    fan_context: MatchFanContext,
}

#[allow(dead_code)]
pub struct LeagueTableGoalscorer {
    pub id: u32,
    pub name: String,
    pub time: String,
    pub auto_goal: bool,
}

pub struct StandingsLegendItem {
    pub zone: &'static str,
    pub label: String,
}

#[derive(Clone, Copy, PartialEq)]
enum StandingsZone {
    Continental(usize),
    Promotion,
    Relegation,
}

/// The coloured bands down the side of a standings table: continental
/// places, promotion and relegation, each only where this league really
/// has them.
struct StandingsZones {
    continental: Vec<QualificationBand>,
    promoted: usize,
    relegated: usize,
}

impl StandingsZones {
    fn tier_class(tier: &CompetitionTier) -> &'static str {
        match tier {
            CompetitionTier::ChampionsLeague => "ucl",
            CompetitionTier::EuropaLeague => "uel",
            CompetitionTier::ConferenceLeague => "uecl",
            CompetitionTier::CopaLibertadores => "copa",
        }
    }

    fn zone(&self, index: usize, rows: usize) -> Option<StandingsZone> {
        if index >= rows.saturating_sub(self.relegated) {
            return Some(StandingsZone::Relegation);
        }
        if let Some(i) = self.continental.iter().position(|b| b.contains(index)) {
            return Some(StandingsZone::Continental(i));
        }
        (index < self.promoted).then_some(StandingsZone::Promotion)
    }

    fn zone_class(&self, zone: StandingsZone) -> &'static str {
        match zone {
            StandingsZone::Continental(i) => Self::tier_class(&self.continental[i].tier),
            StandingsZone::Promotion => "pro",
            StandingsZone::Relegation => "rel",
        }
    }

    /// Stamps each row's zone and returns the legend for the zones that
    /// actually appear, in table order.
    fn apply(&self, rows: &mut [LeagueTableRow], i18n: &I18n) -> Vec<StandingsLegendItem> {
        let count = rows.len();
        let mut seen: Vec<StandingsZone> = Vec::new();
        for (index, row) in rows.iter_mut().enumerate() {
            if let Some(zone) = self.zone(index, count) {
                row.zone = self.zone_class(zone);
                if !seen.contains(&zone) {
                    seen.push(zone);
                }
            }
        }
        seen.into_iter()
            .map(|zone| StandingsLegendItem {
                zone: self.zone_class(zone),
                label: match zone {
                    StandingsZone::Continental(i) => {
                        i18n.t(self.continental[i].tier.as_i18n_key()).to_string()
                    }
                    StandingsZone::Promotion => i18n.t("promotion").to_string(),
                    StandingsZone::Relegation => i18n.t("relegation").to_string(),
                },
            })
            .collect()
    }
}

pub struct LeagueTableRow {
    pub zone: &'static str,
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
pub struct LeagueGetPage<'a> {
    data: &'a SimulatorData,
    i18n: I18n,
    lang: String,
    league: &'a League,
    country: &'a Country,
}

impl<'a> LeagueGetPage<'a> {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<LeagueGetRequest>,
    ) -> ApiResult<axum::response::Response> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let highlights_enabled = state.ai.connection().await.is_some();
        let world = state.world.snapshot();

        let simulator_data = world.as_ref().unwrap();

        let (league, country) = match LeagueAddress::resolve(
            simulator_data,
            &route_params.lang,
            &route_params.country_slug,
            &route_params.league_slug,
            "",
        )? {
            LeaguePage::Found(league, country) => (league, country),
            LeaguePage::Moved(response) => return Ok(response),
        };
        let league_id = league.id;

        // Domestic cups and grouped-competition playoffs have their own
        // bracket pages; bounce there so this handler only ever renders a
        // standings table.
        if league.is_cup {
            let section = if country.playoffs.iter().any(|p| p.league.id == league_id) {
                "playoffs"
            } else {
                "cups"
            };
            return Ok(axum::response::Redirect::to(&format!(
                "/{}/{}/{}",
                route_params.lang, section, league.slug
            ))
            .into_response());
        }

        let page = LeagueGetPage {
            data: simulator_data,
            i18n,
            lang: route_params.lang,
            league,
            country,
        };
        Ok(page.render(highlights_enabled).into_response())
    }

    fn render(self, highlights_enabled: bool) -> LeagueGetTemplate {
        let ladder = LeagueLadder::new(&self.country.leagues.leagues);
        let continental = self
            .data
            .continent(self.country.continent_id)
            .map(|continent| ContinentalQualification::bands_for_league(continent, self.league))
            .unwrap_or_default();

        let mut table_rows: Vec<LeagueTableRow> = self
            .league
            .table
            .get()
            .iter()
            .map(|t| LeagueTableRow::from_core(self.data, t))
            .collect();

        // Split-season context: label the live standings with the current
        // tournament (Apertura until the flip, Clausura after) and build the
        // annual aggregate across every zone of the competition — the same
        // aggregate the relegation pipeline reads.
        let split_season = self.league.settings.split_season;
        let legend = StandingsZones {
            continental,
            promoted: ladder.promoted_from_table(self.league),
            relegated: ladder.relegated_from_table(self.league),
        }
        .apply(&mut table_rows, &self.i18n);

        let (tournament_label, mut annual_table_rows) = self.split_season_rows(split_season);

        let annual_legend = StandingsZones {
            continental: Vec::new(),
            promoted: 0,
            relegated: ladder.relegated_from_annual(self.league),
        }
        .apply(&mut annual_table_rows, &self.i18n);

        let current_tour_schedule = LeagueFixtures::schedule(self.data, self.league);
        let competition_reputation = self.competition_reputation();
        let stats = LeaguePlayerStats::collect(&self);

        let league_title = views::Menus::league_display_name(self.league, &self.i18n, self.data);
        let league_url = LeagueAddress::new(&self.country.slug, &self.league.slug).url(&self.lang);
        let highlights_url = if highlights_enabled {
            LeagueHighlights::from_schedule(self.league, &current_tour_schedule)
                .map(|highlights| highlights.url(&league_url))
        } else {
            None
        };

        LeagueGetTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            title: league_title,
            sub_title_prefix: String::new(),
            sub_title_suffix: String::new(),
            sub_title: self.country.name.clone(),
            sub_title_link: format!("/{}/countries/{}", self.lang, self.country.slug),
            sub_title_flag: Some(SubTitleFlag::of(self.country, &self.lang)),
            header_color: self.country.background_color.clone(),
            foreground_color: self.country.foreground_color.clone(),
            menu_sections: self.menu_sections(&league_url),
            league_url,
            newspaper_count: LeagueNewspaperCounter::count(self.league),
            split_season,
            tournament_label,
            annual_table_rows,
            annual_legend,
            table_rows,
            legend,
            current_tour_schedule,
            highlights_url,
            competition_reputation,
            top_scorers: stats.top_scorers,
            top_assisters: stats.top_assisters,
            top_rated: stats.top_rated,
            lang: self.lang,
            i18n: self.i18n,
        }
    }

    fn split_season_rows(&self, split_season: bool) -> (String, Vec<LeagueTableRow>) {
        if !split_season {
            return (String::new(), Vec::new());
        }

        let league_id = self.league.id;
        let second_stage = self.league.split_first_table.is_some();
        let stage = if second_stage {
            PlayoffStage::SecondStage
        } else {
            PlayoffStage::FirstStage
        };
        let tournament_label = self
            .country
            .playoffs
            .iter()
            .find(|p| p.stage == stage && p.group_league_ids.contains(&league_id))
            .map(|p| p.league.name.clone())
            .unwrap_or_else(|| {
                if second_stage {
                    "Torneo Clausura".to_string()
                } else {
                    "Torneo Apertura".to_string()
                }
            });

        let competition = self
            .league
            .settings
            .league_group
            .as_ref()
            .map(|g| g.competition.clone());
        let mut annual: Vec<CoreTableRow> = self
            .country
            .leagues
            .leagues
            .iter()
            .filter(|l| match (&l.settings.league_group, &competition) {
                (Some(g), Some(c)) => !l.friendly && &g.competition == c,
                _ => l.id == league_id,
            })
            .flat_map(|l| l.annual_table_rows())
            .collect();
        let policy = TieBreakPolicy::fifa_default();
        annual.sort_by(|a, b| policy.compare(a, b));

        let annual_rows: Vec<LeagueTableRow> = annual
            .iter()
            .filter_map(|t| {
                self.data
                    .team(t.team_id)
                    .map(|_| LeagueTableRow::from_core(self.data, t))
            })
            .collect();
        (tournament_label, annual_rows)
    }

    fn competition_reputation(&self) -> Vec<CompetitionReputationItem> {
        let mut reputation_data: Vec<(u16, String, String, String, String)> = self
            .data
            .continents
            .iter()
            .flat_map(|continent| &continent.countries)
            .flat_map(|country| {
                let lang = &self.lang;
                country
                    .leagues
                    .leagues
                    .iter()
                    .filter(|l| !l.friendly)
                    .map(move |league| {
                        (
                            league.reputation,
                            league.name.clone(),
                            LeagueAddress::new(&country.slug, &league.slug).url(lang),
                            country.name.clone(),
                            country.code.clone(),
                        )
                    })
            })
            .collect();

        reputation_data.sort_by_key(|r| Reverse(r.0));

        reputation_data
            .into_iter()
            .take(10)
            .map(|(_, league_name, league_url, country_name, country_code)| {
                CompetitionReputationItem {
                    league_name,
                    league_url,
                    country_name,
                    country_code,
                }
            })
            .collect()
    }

    fn resolve_team_for_player(
        &self,
        player_id: u32,
        league_table: &[CoreTableRow],
    ) -> Option<(String, String)> {
        let (_player, roster_team) = self.data.player_with_team(player_id)?;
        let club = self.data.club(roster_team.club_id)?;
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
    }

    fn menu_sections(&self, league_url: &str) -> Vec<MenuSection> {
        let mut cl: Vec<(u32, &str, &str)> = self
            .country
            .leagues
            .leagues
            .iter()
            .filter(|l| !l.friendly)
            .map(|l| (l.id, l.name.as_str(), l.slug.as_str()))
            .collect();
        cl.sort_by_key(|(id, _, _)| *id);
        let cl_refs: Vec<(&str, &str)> = cl.iter().map(|(_, n, s)| (*n, *s)).collect();
        let mp = views::MenuParams {
            i18n: &self.i18n,
            lang: &self.lang,
            current_path: league_url,
            country_name: &self.country.name,
            country_slug: &self.country.slug,
        };
        // The domestic cup links to its dedicated /cups/ bracket page,
        // the playoffs to /playoffs/.
        views::Menus::league_menu(
            &mp,
            &cl_refs,
            self.country
                .domestic_cup
                .as_ref()
                .map(|c| (c.league.name.as_str(), c.league.slug.as_str())),
            &self
                .country
                .playoffs
                .iter()
                .map(|p| (p.league.name.as_str(), p.league.slug.as_str()))
                .collect::<Vec<_>>(),
        )
    }
}

impl LeagueTableRow {
    fn from_core(data: &SimulatorData, row: &CoreTableRow) -> LeagueTableRow {
        let team = data.team(row.team_id).unwrap();
        LeagueTableRow {
            zone: "",
            team_name: team.name.clone(),
            team_slug: team.slug.clone(),
            played: row.played,
            win: row.win,
            draft: row.draft,
            lost: row.lost,
            goal_scored: row.goal_scored,
            goal_concerned: row.goal_concerned,
            points: row.points,
        }
    }
}

impl LeagueScheduleItem {
    fn from_item(data: &SimulatorData, item: &ScheduleItem) -> LeagueScheduleItem {
        let home_team = data.team(item.home_team_id).unwrap();
        let away_team = data.team(item.away_team_id).unwrap();

        LeagueScheduleItem {
            match_id: item.id.clone(),
            result: item.result.as_ref().map(|res| {
                LeagueScheduleItemResult::from_result(data, item, res, home_team, away_team)
            }),
            home_team_name: home_team.name.clone(),
            home_team_slug: home_team.slug.clone(),
            away_team_name: away_team.name.clone(),
            away_team_slug: away_team.slug.clone(),
        }
    }
}

impl LeagueScheduleItemResult {
    fn from_result(
        data: &SimulatorData,
        item: &ScheduleItem,
        res: &Score,
        home_team: &core::Team,
        away_team: &core::Team,
    ) -> LeagueScheduleItemResult {
        let details: Vec<&GoalDetail> = res
            .details
            .iter()
            .filter(|detail| detail.stat_type == MatchStatisticType::Goal)
            .collect();

        LeagueScheduleItemResult {
            fan_context: MatchFanContext::for_teams(data, home_team, away_team),
            home_goals: if item.home_team_id == res.home_team.team_id {
                res.home_team.get()
            } else {
                res.away_team.get()
            },
            home_goalscorers: details
                .iter()
                .filter_map(|detail| {
                    let player = data.player(detail.player_id)?;
                    home_team
                        .players
                        .contains(player.id)
                        .then(|| LeagueTableGoalscorer::from_goal(detail, player))
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
                    let player = data.player(detail.player_id)?;
                    away_team
                        .players
                        .contains(player.id)
                        .then(|| LeagueTableGoalscorer::from_goal(detail, player))
                })
                .collect(),
        }
    }
}

impl LeagueTableGoalscorer {
    fn from_goal(detail: &GoalDetail, player: &core::Player) -> LeagueTableGoalscorer {
        LeagueTableGoalscorer {
            id: detail.player_id,
            name: player.full_name.to_string(),
            time: format!(
                "('{})",
                Duration::new((detail.time / 1000) as i64, 0)
                    .unwrap()
                    .num_minutes()
            ),
            auto_goal: detail.is_auto_goal,
        }
    }
}

struct LeaguePlayerStats {
    top_scorers: Vec<LeaguePlayerStatItem>,
    top_assisters: Vec<LeaguePlayerStatItem>,
    top_rated: Vec<LeaguePlayerStatItem>,
}

impl LeaguePlayerStats {
    fn collect(page: &LeagueGetPage<'_>) -> LeaguePlayerStats {
        let tally = LeagueTally::collect(page.league);
        let league_table = page.league.table.get();

        let scorers = Self::tally_rows(page, league_table, &tally.apps, &tally.goals);
        let assisters = Self::tally_rows(page, league_table, &tally.apps, &tally.assists);
        let ratings = Self::rating_rows(page, league_table, &tally.apps);

        LeaguePlayerStats {
            top_scorers: Self::items(page, scorers, |(_, _, _, _, played, goals)| {
                (*played, goals.to_string())
            }),
            top_assisters: Self::items(page, assisters, |(_, _, _, _, played, assists)| {
                (*played, assists.to_string())
            }),
            top_rated: ratings
                .into_iter()
                .take(10)
                .map(
                    |(player_id, player_name, team_name, team_slug, played, rating)| {
                        LeaguePlayerStatItem {
                            player_slug: PlayerSlug::history(page.data, player_id, &player_name),
                            player_name,
                            team_name,
                            team_slug,
                            played,
                            stat_value: format!("{:.2}", rating),
                        }
                    },
                )
                .collect(),
        }
    }

    fn tally_rows(
        page: &LeagueGetPage<'_>,
        league_table: &[CoreTableRow],
        apps: &HashMap<u32, u16>,
        tally: &HashMap<u32, u16>,
    ) -> Vec<(u32, String, String, String, u16, u16)> {
        let mut rows: Vec<(u32, String, String, String, u16, u16)> = Vec::new();
        for (&pid, &value) in tally {
            let Some(player) = page.data.player(pid) else {
                continue;
            };
            let Some((team_name, team_slug)) = page.resolve_team_for_player(pid, league_table)
            else {
                continue;
            };
            let played = apps.get(&pid).copied().unwrap_or(0);
            rows.push((
                pid,
                player.full_name.to_string(),
                team_name,
                team_slug,
                played,
                value,
            ));
        }
        rows.sort_by_key(|r| Reverse(r.5));
        rows
    }

    fn items<F>(
        page: &LeagueGetPage<'_>,
        rows: Vec<(u32, String, String, String, u16, u16)>,
        value: F,
    ) -> Vec<LeaguePlayerStatItem>
    where
        F: Fn(&(u32, String, String, String, u16, u16)) -> (u16, String),
    {
        rows.into_iter()
            .take(10)
            .map(|row| {
                let (played, stat_value) = value(&row);
                LeaguePlayerStatItem {
                    player_slug: PlayerSlug::history(page.data, row.0, &row.1),
                    player_name: row.1,
                    team_name: row.2,
                    team_slug: row.3,
                    played,
                    stat_value,
                }
            })
            .collect()
    }

    /// Average rating still flows from the global league bucket — per-league
    /// rating tracking would need new storage on the player. Iterate over
    /// the same effective rosters we use for matchday selection so force-
    /// pinned youth players surface here too. The value we feed into the
    /// ranking is the sample-size-regressed one so the league's top-rated
    /// table is gated by both the minimum-apps filter below *and* the
    /// positional reliability regression — a debut-month 8.2 shouldn't
    /// outrank a 30-app 7.4 starter.
    fn rating_rows(
        page: &LeagueGetPage<'_>,
        league_table: &[CoreTableRow],
        apps: &HashMap<u32, u16>,
    ) -> Vec<(u32, String, String, String, u16, f32)> {
        let mut rating_data: Vec<(u32, String, String, String, u16, f32)> = Vec::new();
        let mut seen_in_table: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for table_row in league_table {
            if let Some(team) = page.data.team(table_row.team_id) {
                let team_name = team.name.clone();
                let team_slug = team.slug.clone();

                let mut effective: Vec<&core::Player> = team.players.players.iter().collect();
                if team.team_type == core::TeamType::Main
                    && let Some(club) = page.data.club(team.club_id)
                {
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

                for player in effective {
                    if !seen_in_table.insert(player.id) {
                        continue;
                    }
                    // Use league-scoped apps from `apps` so the
                    // count matches the top-scorers/assisters columns.
                    // Roster members who haven't featured score 0 here and
                    // are excluded by the `played > 0` filter below.
                    let played = apps.get(&player.id).copied().unwrap_or(0);
                    if played > 0 && player.statistics.average_rating > 0.0 {
                        let pos = player.position().position_group();
                        rating_data.push((
                            player.id,
                            player.full_name.to_string(),
                            team_name.clone(),
                            team_slug.clone(),
                            played,
                            player.statistics.average_rating_realistic(pos),
                        ));
                    }
                }
            }
        }

        // Gate top_rated on a minimum appearance count so a single
        // standout match doesn't outrank a season of consistent play.
        // Threshold scales with the league's leader so the cut adapts
        // to season progress (low early, strict late).
        let max_played_in_league = apps.values().copied().max().unwrap_or(0);
        let min_rated_apps = std::cmp::max(5, max_played_in_league / 2);
        rating_data.retain(|(_, _, _, _, played, _)| *played >= min_rated_apps);
        rating_data.sort_by(|a, b| b.5.partial_cmp(&a.5).unwrap_or(std::cmp::Ordering::Equal));
        rating_data
    }
}

struct LeagueTally {
    goals: HashMap<u32, u16>,
    assists: HashMap<u32, u16>,
    apps: HashMap<u32, u16>,
}

impl LeagueTally {
    /// League-scoped tallies — read goals / assists / apps from this
    /// league's own match records, not from `player.statistics`.
    /// `player.statistics` is the live per-spell bucket and gets wiped
    /// on mid-season transfer / loan and at every season-end snapshot,
    /// while the league's match records keep goal events and lineups.
    /// Reading apps from `player.statistics.played` produced the
    /// "0 Apps – 22 Goals" top-scorer row when the bucket got reset
    /// under a player whose goal events still sat in the league.
    ///
    /// `league.matches` is reset at season start (see `League::simulate`)
    /// and trimmed to a 3-season window globally, so every entry walked
    /// here belongs to the current season.
    fn collect(league: &League) -> LeagueTally {
        let mut goals: HashMap<u32, u16> = HashMap::new();
        let mut assists: HashMap<u32, u16> = HashMap::new();
        let mut apps: HashMap<u32, u16> = HashMap::new();

        for mr in league.matches.iter() {
            for d in &mr.score.details {
                match d.stat_type {
                    MatchStatisticType::Goal if !d.is_auto_goal => {
                        *goals.entry(d.player_id).or_insert(0) += 1;
                    }
                    MatchStatisticType::Assist => {
                        *assists.entry(d.player_id).or_insert(0) += 1;
                    }
                    _ => {}
                }
            }
            if let Some(details) = &mr.details {
                for side in [&details.left_team_players, &details.right_team_players] {
                    for &pid in side.main.iter().chain(side.substitutes_used.iter()) {
                        *apps.entry(pid).or_insert(0) += 1;
                    }
                }
            }
        }
        LeagueTally {
            goals,
            assists,
            apps,
        }
    }
}
