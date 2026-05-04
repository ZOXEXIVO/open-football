pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::slug::player_history_slug;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Datelike;
use core::Person;
use core::PlayerFieldPositionGroup;
use core::SimulatorData;
use core::league::{
    AwardAggregator, CandidateAggregate, MonthlyPlayerAward, PlayerOfTheWeekAward,
    SeasonAwardsSnapshot, TeamOfTheWeekAward, TeamOfTheWeekSelector, TeamOfTheWeekSlot,
};
use core::shared::FullName;
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
    pub recent_weeks: Vec<RecentWeekItem>,
    pub recent_months: Vec<RecentMonthItem>,
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

pub struct RecentMonthItem {
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
    pub kind_label: String,
    pub is_young: bool,
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

    let player_of_month = league
        .awards
        .player_of_month
        .last()
        .map(|a| build_player_card_from_monthly(simulator_data, a));

    let young_player_of_month = league
        .awards
        .young_player_of_month
        .last()
        .map(|a| build_player_card_from_monthly(simulator_data, a));

    let team_of_week = league
        .awards
        .team_of_week
        .last()
        .map(|t| build_team_of_week(simulator_data, t));

    let now = simulator_data.date.date();
    let month_start = now
        .with_day(1)
        .unwrap_or(now)
        .checked_sub_months(chrono::Months::new(1))
        .unwrap_or(now);
    let month_end = now;
    let month_aggregate = AwardAggregator::aggregate(
        league.matches.iter_in_range(month_start, month_end),
    );
    let month_label = month_start.format("%B %Y").to_string();

    let team_of_month = build_pitch_view_from_aggregate(
        simulator_data,
        &month_aggregate,
        |_id| true,
        month_label.clone(),
    );

    let young_team_of_month = build_pitch_view_from_aggregate(
        simulator_data,
        &month_aggregate,
        |id| {
            simulator_data
                .player(id)
                .map(|p| p.age(now) <= 21)
                .unwrap_or(false)
        },
        month_label,
    );

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

    let recent_months: Vec<RecentMonthItem> = {
        let mut combined: Vec<(&MonthlyPlayerAward, bool)> = league
            .awards
            .player_of_month
            .iter()
            .rev()
            .skip(1)
            .take(6)
            .map(|a| (a, false))
            .chain(
                league
                    .awards
                    .young_player_of_month
                    .iter()
                    .rev()
                    .take(4)
                    .map(|a| (a, true)),
            )
            .collect();
        combined.sort_by(|a, b| b.0.month_end_date.cmp(&a.0.month_end_date));
        combined
            .into_iter()
            .take(8)
            .map(|(a, is_young)| build_recent_month_item(simulator_data, &i18n, a, is_young))
            .collect()
    };

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
        recent_weeks,
        recent_months,
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

fn build_recent_month_item(
    data: &SimulatorData,
    i18n: &I18n,
    award: &MonthlyPlayerAward,
    is_young: bool,
) -> RecentMonthItem {
    let kind_label = if is_young {
        i18n.t("young_player_of_month").to_string()
    } else {
        i18n.t("player_of_month").to_string()
    };
    RecentMonthItem {
        player_id: award.player_id,
        player_slug: player_history_slug(data, award.player_id, &award.player_name),
        player_name: award.player_name.clone(),
        player_generated: is_player_generated(data, award.player_id),
        club_name: award.club_name.clone(),
        club_slug: award.club_slug.clone(),
        date_label: award.month_end_date.format("%b %Y").to_string(),
        matches_played: award.matches_played,
        goals: award.goals,
        assists: award.assists,
        average_rating: format!("{:.2}", award.average_rating),
        kind_label,
        is_young,
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

/// Pick an XI from a window aggregate (e.g. past month). `eligibility`
/// gates which player ids are considered (used by Young Team of the Month
/// to enforce age <= 21). Returns `None` if no valid XI can be assembled.
fn build_pitch_view_from_aggregate(
    data: &SimulatorData,
    aggregate: &HashMap<u32, CandidateAggregate>,
    eligibility: impl Fn(u32) -> bool,
    date_label: String,
) -> Option<TeamOfWeekView> {
    let filtered: HashMap<u32, CandidateAggregate> = aggregate
        .iter()
        .filter(|(id, _)| eligibility(**id))
        .map(|(id, agg)| (*id, *agg))
        .collect();
    if filtered.is_empty() {
        return None;
    }
    // Min two appearances filters out one-match wonders for a monthly XI.
    let picks = TeamOfTheWeekSelector::pick_with_min_apps(&filtered, 2);
    if picks.is_empty() {
        return None;
    }

    let mut by_group: HashMap<PlayerFieldPositionGroup, Vec<&(u32, PlayerFieldPositionGroup, f32, CandidateAggregate)>> =
        HashMap::new();
    for p in &picks {
        by_group.entry(p.1).or_default().push(p);
    }

    let mut all_slots: Vec<TeamOfWeekSlotView> = Vec::new();
    for (group, position_classes) in FORMATION_442 {
        if let Some(group_picks) = by_group.get(group) {
            for (idx, pick) in group_picks.iter().enumerate() {
                let pos_class = position_classes
                    .get(idx)
                    .copied()
                    .unwrap_or("")
                    .to_string();
                let (player_id, _, _, agg) = **pick;
                if let Some(slot) = build_totw_slot_from_pick(data, player_id, agg, pos_class) {
                    all_slots.push(slot);
                }
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

fn build_totw_slot_from_pick(
    data: &SimulatorData,
    player_id: u32,
    agg: CandidateAggregate,
    position_class: String,
) -> Option<TeamOfWeekSlotView> {
    let (player, team) = data.player_with_team(player_id)?;
    let full_name = player.full_name.to_string();
    let (first_name, last_name) = split_display_name(&player.full_name);
    Some(TeamOfWeekSlotView {
        player_id,
        player_slug: player.slug(),
        player_name: full_name,
        player_first_name: first_name,
        player_last_name: last_name,
        player_generated: player.is_generated(),
        club_name: team.name.clone(),
        club_slug: team.slug.clone(),
        average_rating: format!("{:.2}", agg.average_rating()),
        position_class,
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
/// (first, last). For Brazilian-style single-word display names — nickname
/// (`Ronaldinho`) or mononym (`Bremer`) — return (`name`, ``) so the
/// pitch label renders the name in the smaller "first-name" font instead
/// of the bold "last-name" font.
fn split_full_name(full: &str) -> (String, String) {
    let trimmed = full.trim();
    if let Some(idx) = trimmed.rfind(' ') {
        let first = trimmed[..idx].trim().to_string();
        let last = trimmed[idx + 1..].trim().to_string();
        if !first.is_empty() && !last.is_empty() {
            return (first, last);
        }
    }
    (trimmed.to_string(), String::new())
}

/// Split a structured `FullName` for the pitch label, mirroring
/// `split_full_name` for the recorded path: nickname / mononym players
/// flow into the small "first-name" slot, regular players keep their
/// first/last split.
fn split_display_name(name: &FullName) -> (String, String) {
    let display_first = name.display_first_name();
    let display_last = name.display_last_name();
    if display_first.is_empty() {
        return (display_last.to_string(), String::new());
    }
    (display_first.to_string(), display_last.to_string())
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
