pub mod routes;

use crate::leagues::address::{LeagueAddress, LeaguePage};
use crate::leagues::newspaper::LeagueNewspaperCounter;
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::shared::slug::PlayerSlug;
use crate::views::{self, MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::Country;
use core::PlayerFieldPositionGroup;
use core::SimulatorData;
use core::league::League;
use core::league::{
    MonthlyAwardsSnapshot, MonthlyPlayerAward, MonthlyStatLeader, PlayerOfTheWeekAward,
    SeasonAwardsSnapshot, TeamOfTheWeekAward, TeamOfTheWeekSlot, TeamOfTheYearAward,
};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct LeagueAwardsRequest {
    pub lang: String,
    pub country_slug: String,
    pub league_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "leagues/awards/index.html")]
pub struct LeagueAwardsTemplate {
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
    /// Every league template that renders the tabbar carries this —
    /// there is no shared base struct, so a new league tab must add it
    /// or the template fails to compile.
    pub newspaper_count: usize,
    pub hero_player: Option<AwardPlayerCard>,
    pub young_hero_player: Option<AwardPlayerCard>,
    pub player_of_month: Option<AwardPlayerCard>,
    pub young_player_of_month: Option<AwardPlayerCard>,
    pub team_of_week: Option<TeamOfWeekView>,
    pub young_team_of_week: Option<TeamOfWeekView>,
    pub team_of_month: Option<TeamOfWeekView>,
    pub young_team_of_month: Option<TeamOfWeekView>,
    pub team_of_year: Option<TeamOfYearView>,
    pub monthly_label: String,
    pub monthly_matches_count: u32,
    pub monthly_top_scorers: Vec<StatLeaderItem>,
    pub monthly_top_assists: Vec<StatLeaderItem>,
    pub monthly_best_ratings: Vec<StatLeaderItem>,
    pub recent_weeks: Vec<RecentWeekItem>,
    pub monthly_archive: Vec<MonthlyArchiveItem>,
    pub season_highlights: Vec<SeasonHighlightItem>,
}

pub struct AwardPlayerCard {
    pub player_id: u32,
    pub player_slug: String,
    pub player_name: String,
    pub player_generated: bool,
    pub club_name: String,
    pub club_slug: String,
    pub date_label: String,
    pub matches_played: u8,
    pub goals: u8,
    pub assists: u8,
    pub average_rating: String,
    pub score: String,
}

pub struct TeamOfWeekView {
    pub date_label: String,
    pub all_slots: Vec<TeamOfWeekSlotView>,
}

pub struct TeamOfYearView {
    pub year_label: String,
    pub all_slots: Vec<TeamOfWeekSlotView>,
}

pub struct TeamOfWeekSlotView {
    pub player_id: u32,
    pub player_slug: String,
    pub player_name: String,
    pub player_first_name: String,
    pub player_last_name: String,
    pub player_generated: bool,
    pub club_name: String,
    pub club_slug: String,
    pub average_rating: String,
    pub position_class: String,
}

pub struct RecentWeekItem {
    pub player_id: u32,
    pub player_slug: String,
    pub player_name: String,
    pub player_generated: bool,
    pub club_name: String,
    pub club_slug: String,
    pub date_label: String,
    pub goals: u8,
    pub assists: u8,
    pub average_rating: String,
}

/// One row in the top-scorers / top-assists / best-ratings tables
/// shown on the Monthly tab.
pub struct StatLeaderItem {
    pub player_id: u32,
    pub player_slug: String,
    pub player_name: String,
    pub player_generated: bool,
    pub club_name: String,
    pub club_slug: String,
    pub matches_played: u8,
    pub goals: u8,
    pub assists: u8,
    pub average_rating: String,
}

/// Compact summary of one archived month (PoM + Young PoM + top
/// stat leaders + match count) for the Monthly Archive table.
pub struct MonthlyArchiveItem {
    pub month_label: String,
    pub matches_count: u32,
    pub player_of_month: Option<MonthlyNamedAward>,
    pub young_player_of_month: Option<MonthlyNamedAward>,
    pub top_scorer: Option<StatLeaderItem>,
    pub top_assist: Option<StatLeaderItem>,
    pub best_rating: Option<StatLeaderItem>,
}

pub struct MonthlyNamedAward {
    pub player_slug: String,
    pub player_name: String,
    pub club_name: String,
    pub club_slug: String,
}

pub struct SeasonHighlightItem {
    pub season_label: String,
    pub player_of_season: Option<SeasonNamedAward>,
    pub young_player_of_season: Option<SeasonNamedAward>,
    pub top_scorer: Option<SeasonNamedAward>,
    pub top_assists: Option<SeasonNamedAward>,
    pub golden_glove: Option<SeasonNamedAward>,
}

pub struct SeasonNamedAward {
    pub player_id: u32,
    pub player_slug: String,
    pub player_name: String,
    pub player_generated: bool,
    pub club_name: String,
    pub club_slug: String,
}

pub struct LeagueAwardsPage<'a> {
    data: &'a SimulatorData,
    i18n: I18n,
    lang: String,
    league: &'a League,
    country: &'a Country,
}

impl<'a> LeagueAwardsPage<'a> {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<LeagueAwardsRequest>,
    ) -> ApiResult<Response> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let guard = state.data.read().await;

        let simulator_data = guard
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let (league, country) = match LeagueAddress::resolve(
            simulator_data,
            &route_params.lang,
            &route_params.country_slug,
            &route_params.league_slug,
            "/awards",
        )? {
            LeaguePage::Found(league, country) => (league, country),
            LeaguePage::Moved(response) => return Ok(response),
        };

        let page = LeagueAwardsPage {
            data: simulator_data,
            i18n,
            lang: route_params.lang,
            league,
            country,
        };
        Ok(page.render().into_response())
    }

    fn render(self) -> LeagueAwardsTemplate {
        let league_url = LeagueAddress::new(&self.country.slug, &self.league.slug).url(&self.lang);
        let league_title = views::Menus::league_display_name(self.league, &self.i18n, self.data);

        let (hero_player, young_hero_player) = self.heroes();
        let (team_of_week, young_team_of_week, team_of_year) = self.teams();
        let monthly = self.monthly();

        LeagueAwardsTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            title: format!("{} — {}", league_title, self.i18n.t("awards")),
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
            hero_player,
            young_hero_player,
            player_of_month: monthly.player_of_month,
            young_player_of_month: monthly.young_player_of_month,
            team_of_week,
            young_team_of_week,
            team_of_month: monthly.team_of_month,
            young_team_of_month: monthly.young_team_of_month,
            team_of_year,
            monthly_label: monthly.label,
            monthly_matches_count: monthly.matches_count,
            monthly_top_scorers: monthly.top_scorers,
            monthly_top_assists: monthly.top_assists,
            monthly_best_ratings: monthly.best_ratings,
            recent_weeks: self.recent_weeks(),
            monthly_archive: self.monthly_archive(),
            season_highlights: self.season_highlights(),
            lang: self.lang,
            i18n: self.i18n,
        }
    }

    fn heroes(&self) -> (Option<AwardPlayerCard>, Option<AwardPlayerCard>) {
        let hero_player = self
            .league
            .player_of_week
            .latest()
            .map(|a| AwardPlayerCard::from_pow(self.data, &self.i18n, a));

        let young_hero_player = self
            .league
            .awards
            .young_player_of_week
            .last()
            .map(|a| AwardPlayerCard::from_pow(self.data, &self.i18n, a));

        (hero_player, young_hero_player)
    }

    fn teams(
        &self,
    ) -> (
        Option<TeamOfWeekView>,
        Option<TeamOfWeekView>,
        Option<TeamOfYearView>,
    ) {
        let team_of_week = self
            .league
            .awards
            .team_of_week
            .last()
            .map(|t| TeamOfWeekView::from_weekly(self.data, &self.i18n, t));

        let young_team_of_week = self
            .league
            .awards
            .young_team_of_week
            .last()
            .map(|t| TeamOfWeekView::from_weekly(self.data, &self.i18n, t));

        let team_of_year = self
            .league
            .awards
            .team_of_year
            .last()
            .and_then(|t| TeamOfYearView::from_yearly(self.data, t));

        (team_of_week, young_team_of_week, team_of_year)
    }

    /// Monthly tab is driven entirely by the most recent archived
    /// snapshot (built in `MonthlyAwardsTick`). Empty months are not
    /// recorded so this is automatically the latest meaningful month
    /// — works for winter-break leagues, split seasons, and summer
    /// calendars without any per-league special-casing.
    fn monthly(&self) -> MonthlyAwardsView {
        let snapshot = self.league.awards.latest_monthly_snapshot();

        let label = snapshot
            .map(|s| self.i18n.format_month_year(s.month_start_date))
            .unwrap_or_default();
        let matches_count = snapshot.map(|s| s.matches_count).unwrap_or(0);

        let player_of_month = snapshot
            .and_then(|s| s.player_of_month.as_ref())
            .map(|a| AwardPlayerCard::from_monthly(self.data, &self.i18n, a));

        let young_player_of_month = snapshot
            .and_then(|s| s.young_player_of_month.as_ref())
            .map(|a| AwardPlayerCard::from_monthly(self.data, &self.i18n, a));

        let team_of_month = snapshot.and_then(|s| {
            TeamOfWeekView::from_slots(
                self.data,
                &s.team_of_month,
                self.i18n.format_month_year(s.month_start_date),
            )
        });

        let young_team_of_month = snapshot.and_then(|s| {
            TeamOfWeekView::from_slots(
                self.data,
                &s.young_team_of_month,
                self.i18n.format_month_year(s.month_start_date),
            )
        });

        let top_scorers = Self::leaders(self.data, snapshot, |s| &s.top_scorers);
        let top_assists = Self::leaders(self.data, snapshot, |s| &s.top_assists);
        let best_ratings = Self::leaders(self.data, snapshot, |s| &s.best_ratings);

        MonthlyAwardsView {
            label,
            matches_count,
            player_of_month,
            young_player_of_month,
            team_of_month,
            young_team_of_month,
            top_scorers,
            top_assists,
            best_ratings,
        }
    }

    fn leaders(
        data: &SimulatorData,
        snapshot: Option<&MonthlyAwardsSnapshot>,
        field: impl Fn(&MonthlyAwardsSnapshot) -> &Vec<MonthlyStatLeader>,
    ) -> Vec<StatLeaderItem> {
        snapshot
            .map(|s| {
                field(s)
                    .iter()
                    .map(|l| StatLeaderItem::from_leader(data, l))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn recent_weeks(&self) -> Vec<RecentWeekItem> {
        let mut recent_weeks: Vec<RecentWeekItem> = self
            .league
            .player_of_week
            .items()
            .iter()
            .rev()
            .skip(1)
            .take(8)
            .map(|a| RecentWeekItem::from_pow(self.data, &self.i18n, a))
            .collect();
        recent_weeks.reserve(0);
        recent_weeks
    }

    fn monthly_archive(&self) -> Vec<MonthlyArchiveItem> {
        self.league
            .awards
            .monthly_awards
            .iter()
            .rev()
            .skip(1) // first is shown in the Monthly tab
            .take(8)
            .map(|s| MonthlyArchiveItem::from_snapshot(self.data, &self.i18n, s))
            .collect()
    }

    fn season_highlights(&self) -> Vec<SeasonHighlightItem> {
        self.league
            .awards
            .season_awards
            .iter()
            .rev()
            .take(3)
            .map(|s| SeasonHighlightItem::from_snapshot(self.data, s))
            .collect()
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
        let current_path = format!("{}/awards", league_url);
        let mp = views::MenuParams {
            i18n: &self.i18n,
            lang: &self.lang,
            current_path: &current_path,
            country_name: &self.country.name,
            country_slug: &self.country.slug,
        };
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

struct MonthlyAwardsView {
    label: String,
    matches_count: u32,
    player_of_month: Option<AwardPlayerCard>,
    young_player_of_month: Option<AwardPlayerCard>,
    team_of_month: Option<TeamOfWeekView>,
    young_team_of_month: Option<TeamOfWeekView>,
    top_scorers: Vec<StatLeaderItem>,
    top_assists: Vec<StatLeaderItem>,
    best_ratings: Vec<StatLeaderItem>,
}

/// The player identity facts every award card shares: whether this is a
/// generated player, resolved through live and retired records alike.
struct AwardPlayer;

impl AwardPlayer {
    fn is_generated(data: &SimulatorData, player_id: u32) -> bool {
        if let Some(player) = data.player(player_id) {
            return player.is_generated();
        }
        if let Some(player) = data.retired_player(player_id) {
            return player.is_generated();
        }
        true
    }
}

impl AwardPlayerCard {
    fn from_pow(
        data: &SimulatorData,
        i18n: &I18n,
        award: &PlayerOfTheWeekAward,
    ) -> AwardPlayerCard {
        AwardPlayerCard {
            player_id: award.player_id,
            player_slug: PlayerSlug::history(data, award.player_id, &award.player_name),
            player_name: award.player_name.clone(),
            player_generated: AwardPlayer::is_generated(data, award.player_id),
            club_name: award.club_name.clone(),
            club_slug: award.club_slug.clone(),
            date_label: i18n.format_date(award.week_end_date),
            matches_played: award.matches_played,
            goals: award.goals,
            assists: award.assists,
            average_rating: format!("{:.2}", award.average_rating),
            score: format!("{:.1}", award.score),
        }
    }

    fn from_monthly(
        data: &SimulatorData,
        i18n: &I18n,
        award: &MonthlyPlayerAward,
    ) -> AwardPlayerCard {
        AwardPlayerCard {
            player_id: award.player_id,
            player_slug: PlayerSlug::history(data, award.player_id, &award.player_name),
            player_name: award.player_name.clone(),
            player_generated: AwardPlayer::is_generated(data, award.player_id),
            club_name: award.club_name.clone(),
            club_slug: award.club_slug.clone(),
            date_label: i18n.format_month_year(award.month_end_date),
            matches_played: award.matches_played,
            goals: award.goals,
            assists: award.assists,
            average_rating: format!("{:.2}", award.average_rating),
            score: format!("{:.1}", award.score),
        }
    }
}

impl RecentWeekItem {
    fn from_pow(data: &SimulatorData, i18n: &I18n, award: &PlayerOfTheWeekAward) -> RecentWeekItem {
        RecentWeekItem {
            player_id: award.player_id,
            player_slug: PlayerSlug::history(data, award.player_id, &award.player_name),
            player_name: award.player_name.clone(),
            player_generated: AwardPlayer::is_generated(data, award.player_id),
            club_name: award.club_name.clone(),
            club_slug: award.club_slug.clone(),
            date_label: i18n.format_day_month(award.week_end_date),
            goals: award.goals,
            assists: award.assists,
            average_rating: format!("{:.2}", award.average_rating),
        }
    }
}

impl StatLeaderItem {
    fn from_leader(data: &SimulatorData, leader: &MonthlyStatLeader) -> StatLeaderItem {
        StatLeaderItem {
            player_id: leader.player_id,
            player_slug: PlayerSlug::history(data, leader.player_id, &leader.player_name),
            player_name: leader.player_name.clone(),
            player_generated: AwardPlayer::is_generated(data, leader.player_id),
            club_name: leader.club_name.clone(),
            club_slug: leader.club_slug.clone(),
            matches_played: leader.matches_played,
            goals: leader.goals,
            assists: leader.assists,
            average_rating: format!("{:.2}", leader.average_rating),
        }
    }
}

impl MonthlyNamedAward {
    fn from_player_award(data: &SimulatorData, award: &MonthlyPlayerAward) -> MonthlyNamedAward {
        MonthlyNamedAward {
            player_slug: PlayerSlug::history(data, award.player_id, &award.player_name),
            player_name: award.player_name.clone(),
            club_name: award.club_name.clone(),
            club_slug: award.club_slug.clone(),
        }
    }
}

impl MonthlyArchiveItem {
    fn from_snapshot(
        data: &SimulatorData,
        i18n: &I18n,
        snapshot: &MonthlyAwardsSnapshot,
    ) -> MonthlyArchiveItem {
        MonthlyArchiveItem {
            month_label: i18n.format_month_year(snapshot.month_start_date),
            matches_count: snapshot.matches_count,
            player_of_month: snapshot
                .player_of_month
                .as_ref()
                .map(|a| MonthlyNamedAward::from_player_award(data, a)),
            young_player_of_month: snapshot
                .young_player_of_month
                .as_ref()
                .map(|a| MonthlyNamedAward::from_player_award(data, a)),
            top_scorer: snapshot
                .top_scorers
                .first()
                .map(|l| StatLeaderItem::from_leader(data, l)),
            top_assist: snapshot
                .top_assists
                .first()
                .map(|l| StatLeaderItem::from_leader(data, l)),
            best_rating: snapshot
                .best_ratings
                .first()
                .map(|l| StatLeaderItem::from_leader(data, l)),
        }
    }
}

/// 4-4-2 position classes per group, in the order defenders/mids/fwds get
/// assigned. The pitch CSS positions a `.pos-*` class into a fixed slot.
const FORMATION_442: &[(PlayerFieldPositionGroup, &[&str])] = &[
    (PlayerFieldPositionGroup::Goalkeeper, &["pos-gk"]),
    (
        PlayerFieldPositionGroup::Defender,
        &["pos-dl", "pos-dcl", "pos-dcr", "pos-dr"],
    ),
    (
        PlayerFieldPositionGroup::Midfielder,
        &["pos-ml", "pos-mcl", "pos-mcr", "pos-mr"],
    ),
    (PlayerFieldPositionGroup::Forward, &["pos-stl", "pos-str"]),
];

struct TeamSheet;

impl TeamSheet {
    fn slots(data: &SimulatorData, slots: &[TeamOfTheWeekSlot]) -> Vec<TeamOfWeekSlotView> {
        let mut by_group: HashMap<PlayerFieldPositionGroup, Vec<&TeamOfTheWeekSlot>> =
            HashMap::new();
        for s in slots {
            by_group.entry(s.position_group).or_default().push(s);
        }

        let mut all_slots: Vec<TeamOfWeekSlotView> = Vec::new();
        for (group, position_classes) in FORMATION_442 {
            if let Some(group_slots) = by_group.get(group) {
                for (idx, slot) in group_slots.iter().enumerate() {
                    let pos_class = position_classes.get(idx).copied().unwrap_or("").to_string();
                    all_slots.push(TeamOfWeekSlotView::from_slot(data, slot, pos_class));
                }
            }
        }
        all_slots
    }
}

impl TeamOfWeekView {
    fn from_weekly(
        data: &SimulatorData,
        i18n: &I18n,
        award: &TeamOfTheWeekAward,
    ) -> TeamOfWeekView {
        TeamOfWeekView {
            date_label: i18n.format_date(award.week_end_date),
            all_slots: TeamSheet::slots(data, &award.slots),
        }
    }

    /// Build the Team-of-Month pitch view from an archived slot list.
    /// Returns `None` for empty slot lists so the template can render the
    /// "no awards yet" placeholder instead of an empty pitch.
    fn from_slots(
        data: &SimulatorData,
        slots: &[TeamOfTheWeekSlot],
        date_label: String,
    ) -> Option<TeamOfWeekView> {
        if slots.is_empty() {
            return None;
        }
        let all_slots = TeamSheet::slots(data, slots);
        if all_slots.is_empty() {
            return None;
        }
        Some(TeamOfWeekView {
            date_label,
            all_slots,
        })
    }
}

impl TeamOfYearView {
    fn from_yearly(data: &SimulatorData, award: &TeamOfTheYearAward) -> Option<TeamOfYearView> {
        if award.slots.is_empty() {
            return None;
        }
        let all_slots = TeamSheet::slots(data, &award.slots);
        if all_slots.is_empty() {
            return None;
        }
        Some(TeamOfYearView {
            year_label: award.year.to_string(),
            all_slots,
        })
    }
}

impl TeamOfWeekSlotView {
    fn from_slot(
        data: &SimulatorData,
        slot: &TeamOfTheWeekSlot,
        position_class: String,
    ) -> TeamOfWeekSlotView {
        let (first_name, last_name) = Self::split_full_name(&slot.player_name);
        TeamOfWeekSlotView {
            player_id: slot.player_id,
            player_slug: PlayerSlug::history(data, slot.player_id, &slot.player_name),
            player_name: slot.player_name.clone(),
            player_first_name: first_name,
            player_last_name: last_name,
            player_generated: AwardPlayer::is_generated(data, slot.player_id),
            club_name: slot.club_name.clone(),
            club_slug: slot.club_slug.clone(),
            average_rating: format!("{:.2}", slot.average_rating),
            position_class,
        }
    }

    /// Split a denormalised "First Last" string (as built by the simulator's
    /// `format!("{} {}", display_first_name, display_last_name)`) back into
    /// (first, last). Mononym / nickname players (`Batxi`, `Ronaldinho`,
    /// `Bremer`) flow into the last-name slot so the pitch label renders
    /// them in the bold "last-name" font.
    fn split_full_name(full: &str) -> (String, String) {
        let trimmed = full.trim();
        if let Some(idx) = trimmed.rfind(' ') {
            let first = trimmed[..idx].trim().to_string();
            let last = trimmed[idx + 1..].trim().to_string();
            if !first.is_empty() && !last.is_empty() {
                return (first, last);
            }
        }
        (String::new(), trimmed.to_string())
    }
}

impl SeasonHighlightItem {
    fn from_snapshot(data: &SimulatorData, snapshot: &SeasonAwardsSnapshot) -> SeasonHighlightItem {
        let end_year = snapshot.season_end_date.format("%Y").to_string();
        let end_yy = end_year.chars().rev().take(2).collect::<Vec<_>>();
        let end_yy: String = end_yy.into_iter().rev().collect();
        let start_year: i32 = end_year.parse::<i32>().map(|y| y - 1).unwrap_or(0);
        let season_label = format!("{}/{}", start_year, end_yy);
        SeasonHighlightItem {
            season_label,
            player_of_season: snapshot
                .player_of_season
                .and_then(|id| SeasonNamedAward::from_player(data, id)),
            young_player_of_season: snapshot
                .young_player_of_season
                .and_then(|id| SeasonNamedAward::from_player(data, id)),
            top_scorer: snapshot
                .top_scorer
                .and_then(|id| SeasonNamedAward::from_player(data, id)),
            top_assists: snapshot
                .top_assists
                .and_then(|id| SeasonNamedAward::from_player(data, id)),
            golden_glove: snapshot
                .golden_glove
                .and_then(|id| SeasonNamedAward::from_player(data, id)),
        }
    }
}

impl SeasonNamedAward {
    fn from_player(data: &SimulatorData, player_id: u32) -> Option<SeasonNamedAward> {
        if let Some((player, team)) = data.player_with_team(player_id) {
            let player_name = player.full_name.to_string();
            return Some(SeasonNamedAward {
                player_id,
                player_slug: PlayerSlug::history(data, player_id, &player_name),
                player_name,
                player_generated: player.is_generated(),
                club_name: team.name.clone(),
                club_slug: team.slug.clone(),
            });
        }
        let player = data.retired_player(player_id)?;
        let player_name = player.full_name.to_string();
        Some(SeasonNamedAward {
            player_id,
            player_slug: PlayerSlug::history(data, player_id, &player_name),
            player_name,
            player_generated: player.is_generated(),
            club_name: String::new(),
            club_slug: String::new(),
        })
    }
}
