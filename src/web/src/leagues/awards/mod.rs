pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::slug::player_history_slug;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::PlayerFieldPositionGroup;
use core::SimulatorData;
use core::league::{
    MonthlyAwardsSnapshot, MonthlyPlayerAward, MonthlyStatLeader, PlayerOfTheWeekAward,
    SeasonAwardsSnapshot, TeamOfTheWeekAward, TeamOfTheWeekSlot,
};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct LeagueAwardsRequest {
    pub lang: String,
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
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: I18n,
    pub lang: String,
    pub league_slug: String,
    pub hero_player: Option<AwardPlayerCard>,
    pub player_of_month: Option<AwardPlayerCard>,
    pub young_player_of_month: Option<AwardPlayerCard>,
    pub team_of_week: Option<TeamOfWeekView>,
    pub team_of_month: Option<TeamOfWeekView>,
    pub young_team_of_month: Option<TeamOfWeekView>,
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

pub async fn league_awards_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<LeagueAwardsRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let indexes = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?;

    let league_id = indexes
        .slug_indexes
        .get_league_by_slug(&route_params.league_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!("League '{}' not found", route_params.league_slug))
        })?;

    let league = simulator_data
        .league(league_id)
        .ok_or_else(|| ApiError::NotFound(format!("League with ID {} not found", league_id)))?;

    let country = simulator_data.country(league.country_id).ok_or_else(|| {
        ApiError::NotFound(format!("Country with ID {} not found", league.country_id))
    })?;

    let league_title = views::league_display_name(&league, &i18n, simulator_data);

    let hero_player = league
        .player_of_week
        .latest()
        .map(|a| build_player_card_from_pow(simulator_data, a));

    let team_of_week = league
        .awards
        .team_of_week
        .last()
        .map(|t| build_team_of_week(simulator_data, t));

    // Monthly tab is driven entirely by the most recent archived
    // snapshot (built in `MonthlyAwardsTick`). Empty months are not
    // recorded so this is automatically the latest meaningful month
    // — works for winter-break leagues, split seasons, and summer
    // calendars without any per-league special-casing.
    let snapshot = league.awards.latest_monthly_snapshot();

    let monthly_label = snapshot
        .map(|s| s.month_start_date.format("%B %Y").to_string())
        .unwrap_or_default();
    let monthly_matches_count = snapshot.map(|s| s.matches_count).unwrap_or(0);

    let player_of_month = snapshot
        .and_then(|s| s.player_of_month.as_ref())
        .map(|a| build_player_card_from_monthly(simulator_data, a));

    let young_player_of_month = snapshot
        .and_then(|s| s.young_player_of_month.as_ref())
        .map(|a| build_player_card_from_monthly(simulator_data, a));

    let team_of_month = snapshot.and_then(|s| {
        build_team_view_from_slots(
            simulator_data,
            &s.team_of_month,
            monthly_label.clone(),
        )
    });

    let young_team_of_month = snapshot.and_then(|s| {
        build_team_view_from_slots(
            simulator_data,
            &s.young_team_of_month,
            monthly_label.clone(),
        )
    });

    let monthly_top_scorers: Vec<StatLeaderItem> = snapshot
        .map(|s| {
            s.top_scorers
                .iter()
                .map(|l| build_stat_leader_item(simulator_data, l))
                .collect()
        })
        .unwrap_or_default();
    let monthly_top_assists: Vec<StatLeaderItem> = snapshot
        .map(|s| {
            s.top_assists
                .iter()
                .map(|l| build_stat_leader_item(simulator_data, l))
                .collect()
        })
        .unwrap_or_default();
    let monthly_best_ratings: Vec<StatLeaderItem> = snapshot
        .map(|s| {
            s.best_ratings
                .iter()
                .map(|l| build_stat_leader_item(simulator_data, l))
                .collect()
        })
        .unwrap_or_default();

    let mut recent_weeks: Vec<RecentWeekItem> = league
        .player_of_week
        .items()
        .iter()
        .rev()
        .skip(1)
        .take(8)
        .map(|a| build_recent_week_item(simulator_data, a))
        .collect();
    recent_weeks.reserve(0);

    let monthly_archive: Vec<MonthlyArchiveItem> = league
        .awards
        .monthly_awards
        .iter()
        .rev()
        .skip(1) // first is shown in the Monthly tab
        .take(8)
        .map(|s| build_monthly_archive_item(simulator_data, s))
        .collect();

    let season_highlights: Vec<SeasonHighlightItem> = league
        .awards
        .season_awards
        .iter()
        .rev()
        .take(3)
        .map(|s| build_season_highlight(simulator_data, s))
        .collect();

    Ok(LeagueAwardsTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        title: format!("{} — {}", league_title, i18n.t("awards")),
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
            let current_path =
                format!("/{}/leagues/{}/awards", &route_params.lang, &league.slug);
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
        hero_player,
        player_of_month,
        young_player_of_month,
        team_of_week,
        team_of_month,
        young_team_of_month,
        monthly_label,
        monthly_matches_count,
        monthly_top_scorers,
        monthly_top_assists,
        monthly_best_ratings,
        recent_weeks,
        monthly_archive,
        season_highlights,
        lang: route_params.lang,
        i18n,
    })
}

fn is_player_generated(data: &SimulatorData, player_id: u32) -> bool {
    if let Some(player) = data.player(player_id) {
        return player.is_generated();
    }
    if let Some(player) = data.retired_player(player_id) {
        return player.is_generated();
    }
    true
}

fn build_player_card_from_pow(
    data: &SimulatorData,
    award: &PlayerOfTheWeekAward,
) -> AwardPlayerCard {
    AwardPlayerCard {
        player_id: award.player_id,
        player_slug: player_history_slug(data, award.player_id, &award.player_name),
        player_name: award.player_name.clone(),
        player_generated: is_player_generated(data, award.player_id),
        club_name: award.club_name.clone(),
        club_slug: award.club_slug.clone(),
        date_label: award.week_end_date.format("%d %b %Y").to_string(),
        matches_played: award.matches_played,
        goals: award.goals,
        assists: award.assists,
        average_rating: format!("{:.2}", award.average_rating),
        score: format!("{:.1}", award.score),
    }
}

fn build_player_card_from_monthly(
    data: &SimulatorData,
    award: &MonthlyPlayerAward,
) -> AwardPlayerCard {
    AwardPlayerCard {
        player_id: award.player_id,
        player_slug: player_history_slug(data, award.player_id, &award.player_name),
        player_name: award.player_name.clone(),
        player_generated: is_player_generated(data, award.player_id),
        club_name: award.club_name.clone(),
        club_slug: award.club_slug.clone(),
        date_label: award.month_end_date.format("%B %Y").to_string(),
        matches_played: award.matches_played,
        goals: award.goals,
        assists: award.assists,
        average_rating: format!("{:.2}", award.average_rating),
        score: format!("{:.1}", award.score),
    }
}

fn build_recent_week_item(data: &SimulatorData, award: &PlayerOfTheWeekAward) -> RecentWeekItem {
    RecentWeekItem {
        player_id: award.player_id,
        player_slug: player_history_slug(data, award.player_id, &award.player_name),
        player_name: award.player_name.clone(),
        player_generated: is_player_generated(data, award.player_id),
        club_name: award.club_name.clone(),
        club_slug: award.club_slug.clone(),
        date_label: award.week_end_date.format("%d %b").to_string(),
        goals: award.goals,
        assists: award.assists,
        average_rating: format!("{:.2}", award.average_rating),
    }
}

fn build_stat_leader_item(
    data: &SimulatorData,
    leader: &MonthlyStatLeader,
) -> StatLeaderItem {
    StatLeaderItem {
        player_id: leader.player_id,
        player_slug: player_history_slug(data, leader.player_id, &leader.player_name),
        player_name: leader.player_name.clone(),
        player_generated: is_player_generated(data, leader.player_id),
        club_name: leader.club_name.clone(),
        club_slug: leader.club_slug.clone(),
        matches_played: leader.matches_played,
        goals: leader.goals,
        assists: leader.assists,
        average_rating: format!("{:.2}", leader.average_rating),
    }
}

fn build_monthly_named_award(
    data: &SimulatorData,
    award: &MonthlyPlayerAward,
) -> MonthlyNamedAward {
    MonthlyNamedAward {
        player_slug: player_history_slug(data, award.player_id, &award.player_name),
        player_name: award.player_name.clone(),
        club_name: award.club_name.clone(),
        club_slug: award.club_slug.clone(),
    }
}

fn build_monthly_archive_item(
    data: &SimulatorData,
    snapshot: &MonthlyAwardsSnapshot,
) -> MonthlyArchiveItem {
    MonthlyArchiveItem {
        month_label: snapshot.month_start_date.format("%b %Y").to_string(),
        matches_count: snapshot.matches_count,
        player_of_month: snapshot
            .player_of_month
            .as_ref()
            .map(|a| build_monthly_named_award(data, a)),
        young_player_of_month: snapshot
            .young_player_of_month
            .as_ref()
            .map(|a| build_monthly_named_award(data, a)),
        top_scorer: snapshot
            .top_scorers
            .first()
            .map(|l| build_stat_leader_item(data, l)),
        top_assist: snapshot
            .top_assists
            .first()
            .map(|l| build_stat_leader_item(data, l)),
        best_rating: snapshot
            .best_ratings
            .first()
            .map(|l| build_stat_leader_item(data, l)),
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

fn build_team_of_week(
    data: &SimulatorData,
    award: &TeamOfTheWeekAward,
) -> TeamOfWeekView {
    let mut by_group: HashMap<PlayerFieldPositionGroup, Vec<&TeamOfTheWeekSlot>> = HashMap::new();
    for s in &award.slots {
        by_group.entry(s.position_group).or_default().push(s);
    }

    let mut all_slots: Vec<TeamOfWeekSlotView> = Vec::new();
    for (group, position_classes) in FORMATION_442 {
        if let Some(group_slots) = by_group.get(group) {
            for (idx, slot) in group_slots.iter().enumerate() {
                let pos_class = position_classes
                    .get(idx)
                    .copied()
                    .unwrap_or("")
                    .to_string();
                all_slots.push(build_totw_slot(data, slot, pos_class));
            }
        }
    }

    TeamOfWeekView {
        date_label: award.week_end_date.format("%d %b %Y").to_string(),
        all_slots,
    }
}

/// Build the Team-of-Month pitch view from an archived slot list.
/// Returns `None` for empty slot lists so the template can render the
/// "no awards yet" placeholder instead of an empty pitch.
fn build_team_view_from_slots(
    data: &SimulatorData,
    slots: &[TeamOfTheWeekSlot],
    date_label: String,
) -> Option<TeamOfWeekView> {
    if slots.is_empty() {
        return None;
    }
    let mut by_group: HashMap<PlayerFieldPositionGroup, Vec<&TeamOfTheWeekSlot>> = HashMap::new();
    for s in slots {
        by_group.entry(s.position_group).or_default().push(s);
    }
    let mut all_slots: Vec<TeamOfWeekSlotView> = Vec::new();
    for (group, position_classes) in FORMATION_442 {
        if let Some(group_slots) = by_group.get(group) {
            for (idx, slot) in group_slots.iter().enumerate() {
                let pos_class = position_classes
                    .get(idx)
                    .copied()
                    .unwrap_or("")
                    .to_string();
                all_slots.push(build_totw_slot(data, slot, pos_class));
            }
        }
    }
    if all_slots.is_empty() {
        return None;
    }
    Some(TeamOfWeekView {
        date_label,
        all_slots,
    })
}

fn build_totw_slot(
    data: &SimulatorData,
    slot: &TeamOfTheWeekSlot,
    position_class: String,
) -> TeamOfWeekSlotView {
    let (first_name, last_name) = split_full_name(&slot.player_name);
    TeamOfWeekSlotView {
        player_id: slot.player_id,
        player_slug: player_history_slug(data, slot.player_id, &slot.player_name),
        player_name: slot.player_name.clone(),
        player_first_name: first_name,
        player_last_name: last_name,
        player_generated: is_player_generated(data, slot.player_id),
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

fn build_season_highlight(
    data: &SimulatorData,
    snapshot: &SeasonAwardsSnapshot,
) -> SeasonHighlightItem {
    let end_year = snapshot.season_end_date.format("%Y").to_string();
    let end_yy = end_year.chars().rev().take(2).collect::<Vec<_>>();
    let end_yy: String = end_yy.into_iter().rev().collect();
    let start_year: i32 = end_year.parse::<i32>().map(|y| y - 1).unwrap_or(0);
    let season_label = format!("{}/{}", start_year, end_yy);
    SeasonHighlightItem {
        season_label,
        player_of_season: snapshot
            .player_of_season
            .and_then(|id| build_named_award(data, id)),
        young_player_of_season: snapshot
            .young_player_of_season
            .and_then(|id| build_named_award(data, id)),
        top_scorer: snapshot
            .top_scorer
            .and_then(|id| build_named_award(data, id)),
        top_assists: snapshot
            .top_assists
            .and_then(|id| build_named_award(data, id)),
        golden_glove: snapshot
            .golden_glove
            .and_then(|id| build_named_award(data, id)),
    }
}

fn build_named_award(data: &SimulatorData, player_id: u32) -> Option<SeasonNamedAward> {
    if let Some((player, team)) = data.player_with_team(player_id) {
        let player_name = player.full_name.to_string();
        return Some(SeasonNamedAward {
            player_id,
            player_slug: player_history_slug(data, player_id, &player_name),
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
        player_slug: player_history_slug(data, player_id, &player_name),
        player_name,
        player_generated: player.is_generated(),
        club_name: String::new(),
        club_slug: String::new(),
    })
}
