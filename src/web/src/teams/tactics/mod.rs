pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::MatchHistoryItem;
use core::Player;
use core::PlayerPositionType;
use core::SimulatorData;
use core::Tactics;
use core::Team;
use core::TeamType;
use serde::Deserialize;
use std::borrow::Cow;

#[derive(Deserialize)]
pub struct TeamTacticsGetRequest {
    lang: String,
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/tactics/index.html")]
pub struct TeamTacticsTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub i18n: I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub active_tab: &'static str,
    pub show_finances_tab: bool,
    pub show_academy_tab: bool,
    pub formation_name: String,
    pub formation_players: Vec<FormationPlayer>,
    /// Most recent in-match shapes the team actually used. Lets the
    /// view show "Plan: 4-3-3 — Last used: 4-5-1, 4-4-2, 4-4-2" so the
    /// user can see when the coach is shifting shape mid-match.
    pub recent_used_shapes: Vec<RecentUsedShape>,
}

pub struct FormationPlayer {
    pub player_id: u32,
    pub slug: String,
    pub last_name: String,
    pub is_generated: bool,
    pub rating: String,
    pub css_class: String,
}

pub struct RecentUsedShape {
    pub date_label: String,
    pub formation_name: String,
    /// Cheap rival display label ("vs Spurs"), or empty when the rival
    /// id can't be resolved. Resolved at render time off the same
    /// SimulatorData snapshot the rest of the page uses — no extra
    /// lookups, no async fan-out.
    pub opponent_label: String,
    /// True when the team's final shape differed from what they
    /// kicked off with. Drives the accent-coloured chip + "shift"
    /// label so the user can see *when* the manager pivoted.
    pub is_shift: bool,
    /// Sim-minute the first shape change fired (only stamped when
    /// `is_shift == true`). Empty string when no change.
    pub change_minute_label: String,
}

pub async fn team_tactics_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamTacticsGetRequest>,
) -> ApiResult<impl IntoResponse> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let i18n = state.i18n.for_lang(&route_params.lang);

    let team_id = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?
        .slug_indexes
        .get_team_by_slug(&route_params.team_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Team '{}' not found", route_params.team_slug))
        })?;

    let team = simulator_data
        .team(team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))?;

    let league = team.league_id.and_then(|id| simulator_data.league(id));

    let last_match = LastMatchLookup::find(team);
    let lineup = TacticsLineupBuilder::build(team, last_match);
    let formation_name = lineup.formation_name;
    let formation_players = lineup.players;

    let recent_used_shapes = RecentShapesView::build(team, simulator_data);

    let neighborhood = TeamNeighborhood::for_club(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighborhood
        .teams
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let league_refs: Vec<(&str, &str)> = neighborhood
        .leagues
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();

    let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
    let current_path = format!("/{}/teams/{}/tactics", &route_params.lang, &team.slug);
    let menu_params = views::MenuParams {
        i18n: &i18n,
        lang: &route_params.lang,
        current_path: &current_path,
        country_name: cn,
        country_slug: cs,
    };
    let menu_sections = views::team_menu(&menu_params, &neighbor_refs, &league_refs);
    let title = team.name.clone();
    let league_title = league
        .map(|l| views::league_display_name(l, &i18n, simulator_data))
        .unwrap_or_default();

    Ok(TeamTacticsTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        i18n,
        lang: route_params.lang.clone(),
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: league_title,
        sub_title_link: league
            .map(|l| format!("/{}/leagues/{}", &route_params.lang, &l.slug))
            .unwrap_or_default(),
        sub_title_country_code: String::new(),
        header_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.background.clone())
            .unwrap_or_default(),
        foreground_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.foreground.clone())
            .unwrap_or_default(),
        menu_sections,
        team_slug: team.slug.clone(),
        active_tab: "tactics",
        show_finances_tab: team.team_type.is_own_team(),
        show_academy_tab: team.team_type == TeamType::Main
            || team.team_type == TeamType::U18,
        formation_name,
        formation_players,
        recent_used_shapes,
    })
}

struct PositionCss;

impl PositionCss {
    fn slot_class(pos: &PlayerPositionType) -> &'static str {
        match pos {
            PlayerPositionType::Goalkeeper => "pos-gk",
            PlayerPositionType::Sweeper => "pos-sw",
            PlayerPositionType::DefenderLeft => "pos-dl",
            PlayerPositionType::DefenderCenterLeft => "pos-dcl",
            PlayerPositionType::DefenderCenter => "pos-dc",
            PlayerPositionType::DefenderCenterRight => "pos-dcr",
            PlayerPositionType::DefenderRight => "pos-dr",
            PlayerPositionType::DefensiveMidfielder => "pos-dm",
            PlayerPositionType::WingbackLeft => "pos-wl",
            PlayerPositionType::WingbackRight => "pos-wr",
            PlayerPositionType::MidfielderLeft => "pos-ml",
            PlayerPositionType::MidfielderCenterLeft => "pos-mcl",
            PlayerPositionType::MidfielderCenter => "pos-mc",
            PlayerPositionType::MidfielderCenterRight => "pos-mcr",
            PlayerPositionType::MidfielderRight => "pos-mr",
            PlayerPositionType::AttackingMidfielderLeft => "pos-aml",
            PlayerPositionType::AttackingMidfielderCenter => "pos-amc",
            PlayerPositionType::AttackingMidfielderRight => "pos-amr",
            PlayerPositionType::ForwardLeft => "pos-fl",
            PlayerPositionType::ForwardCenter => "pos-fc",
            PlayerPositionType::ForwardRight => "pos-fr",
            PlayerPositionType::Striker => "pos-st",
        }
    }
}

struct LastMatchLookup;

impl LastMatchLookup {
    /// Most recent match that recorded both a final tactic AND a
    /// starting XI — the canonical "what did the team really do last
    /// time out?" reference for the tactics page. Older history items
    /// predating the recording return as `None`.
    fn find(team: &Team) -> Option<&MatchHistoryItem> {
        team.match_history
            .items()
            .iter()
            .rev()
            .find(|m| m.tactic_used.is_some() && !m.starting_eleven.is_empty())
    }
}

struct TacticsLineup {
    formation_name: String,
    players: Vec<FormationPlayer>,
}

struct TacticsLineupBuilder;

impl TacticsLineupBuilder {
    /// Pick the formation + XI shown on the tactics pitch. Prefers the
    /// last match's actual shape and starters; falls back to the
    /// team's persistent plan and a best-available pick when no usable
    /// history exists. Recorded starters who have left the club fall
    /// through to the best-available filler for that slot.
    fn build(team: &Team, last_match: Option<&MatchHistoryItem>) -> TacticsLineup {
        let tactics: Cow<'_, Tactics> = match last_match.and_then(|m| m.tactic_used) {
            Some(tac) => Cow::Owned(Tactics::new(tac)),
            None => team.tactics(),
        };
        let formation_name = tactics.tactic_type.display_name().to_string();
        let formation_positions = tactics.positions();

        let players = team.players();
        let mut recorded_pool: Vec<(u32, PlayerPositionType)> = last_match
            .map(|m| m.starting_eleven.clone())
            .unwrap_or_default();
        let mut used: Vec<u32> = Vec::new();
        let mut out: Vec<FormationPlayer> = Vec::new();

        for required_pos in formation_positions.iter() {
            let chosen = Self::take_recorded(&mut recorded_pool, required_pos, &players, &used)
                .or_else(|| Self::pick_best(required_pos, &players, &used));
            if let Some(player) = chosen {
                used.push(player.id);
                out.push(FormationPlayer::from_pick(player, required_pos));
            }
        }

        TacticsLineup {
            formation_name,
            players: out,
        }
    }

    fn take_recorded<'a>(
        pool: &mut Vec<(u32, PlayerPositionType)>,
        slot: &PlayerPositionType,
        players: &[&'a Player],
        used: &[u32],
    ) -> Option<&'a Player> {
        let idx = pool.iter().position(|(_, s)| s == slot)?;
        let (player_id, _) = pool.remove(idx);
        players
            .iter()
            .find(|p| p.id == player_id)
            .filter(|p| !used.contains(&p.id))
            .copied()
    }

    fn pick_best<'a>(
        slot: &PlayerPositionType,
        players: &[&'a Player],
        used: &[u32],
    ) -> Option<&'a Player> {
        let is_gk_slot = *slot == PlayerPositionType::Goalkeeper;
        players
            .iter()
            .filter(|p| !used.contains(&p.id))
            .filter(|p| p.is_ready_for_match())
            .filter(|p| p.positions.is_goalkeeper() == is_gk_slot)
            .max_by_key(|p| {
                let pos_level = p.positions.get_level(*slot) as i32;
                let ability = p.player_attributes.current_ability as i32;
                pos_level * 10 + ability
            })
            .copied()
    }
}

impl FormationPlayer {
    fn from_pick(player: &Player, slot: &PlayerPositionType) -> Self {
        let ca = player.player_attributes.current_ability as f32 / 20.0;
        FormationPlayer {
            player_id: player.id,
            slug: player.slug(),
            last_name: player.full_name.display_last_name().to_string(),
            is_generated: player.is_generated(),
            rating: format!("{:.1}", ca.min(10.0)),
            css_class: PositionCss::slot_class(slot).to_string(),
        }
    }
}

struct RecentShapesView;

impl RecentShapesView {
    /// Most-recent-first list of in-match shapes (capped at 5). Each
    /// chip records whether the coach shifted shape mid-match and, if
    /// the rival is still resolvable on the current sim snapshot,
    /// labels the opponent. Drops items the engine never tagged with
    /// a tactic (e.g. friendlies in some pipelines).
    fn build(team: &Team, data: &SimulatorData) -> Vec<RecentUsedShape> {
        team.match_history
            .items()
            .iter()
            .rev()
            .filter_map(|m| {
                let final_tac = m.tactic_used?;
                let is_shift = m.shape_changed();
                let opponent_label = data
                    .team(m.rival_team_id)
                    .map(|t| format!("vs {}", t.name))
                    .unwrap_or_default();
                let change_minute_label = if is_shift {
                    m.tactic_change_minute
                        .map(|min| format!("min {}", min))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                Some(RecentUsedShape {
                    date_label: m.date.format("%d %b").to_string(),
                    formation_name: final_tac.display_name().to_string(),
                    opponent_label,
                    is_shift,
                    change_minute_label,
                })
            })
            .take(5)
            .collect()
    }
}

struct TeamNeighborhood {
    teams: Vec<(String, String)>,
    leagues: Vec<(String, String)>,
}

impl TeamNeighborhood {
    fn for_club(club_id: u32, data: &SimulatorData, i18n: &I18n) -> Result<Self, ApiError> {
        let club = data.club(club_id).ok_or_else(|| {
            ApiError::InternalError(format!("Club with ID {} not found", club_id))
        })?;

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

        Ok(TeamNeighborhood {
            teams,
            leagues: country_leagues
                .into_iter()
                .map(|(_, name, slug)| (name, slug))
                .collect(),
        })
    }
}
