pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::PlayerPositionType;
use core::SimulatorData;
use serde::Deserialize;

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

    let tactics = team.tactics();
    let formation_name = tactics.tactic_type.display_name().to_string();
    let formation_positions = tactics.positions();

    // Match best players to formation positions
    let mut formation_players: Vec<FormationPlayer> = Vec::new();
    let mut used_player_ids: Vec<u32> = Vec::new();

    for required_pos in formation_positions.iter() {
        let is_gk_slot = *required_pos == PlayerPositionType::Goalkeeper;
        // Find best available player for this position
        let players = team.players();
        let best_player = players
            .iter()
            .filter(|p| !used_player_ids.contains(&p.id))
            .filter(|p| p.is_ready_for_match())
            .filter(|p| p.positions.is_goalkeeper() == is_gk_slot)
            .max_by_key(|p| {
                let pos_level = p.positions.get_level(*required_pos) as i32;
                let ability = p.player_attributes.current_ability as i32;
                pos_level * 10 + ability
            });

        if let Some(player) = best_player {
            used_player_ids.push(player.id);
            let ca = player.player_attributes.current_ability as f32 / 20.0;
            formation_players.push(FormationPlayer {
                player_id: player.id,
                slug: player.slug(),
                last_name: player.full_name.display_last_name().to_string(),
                is_generated: player.is_generated(),
                rating: format!("{:.1}", ca.min(10.0)),
                css_class: position_to_css_class(required_pos),
            });
        }
    }

    // Pull the last few in-match shapes the team actually used so the
    // tactics screen can show "Plan: 4-3-3 — Last used: 4-5-1 vs Spurs
    // (shifted at min 72), 4-4-2, 4-3-3" instead of pretending the
    // persistent plan was the only shape that ever ran. Each chip
    // shows whether the shape was a kept plan or an in-match shift,
    // and (when the rival id can be resolved cheaply) tags the
    // opponent. Most-recent-first, capped at 5.
    let recent_used_shapes: Vec<RecentUsedShape> = team
        .match_history
        .items()
        .iter()
        .rev()
        .filter_map(|m| {
            let final_tac = m.tactic_used?;
            // Use the engine-recorded starting tactic when available
            // (i.e. the team's actual plan at kickoff, regardless of
            // what their *current* persistent plan happens to be —
            // those can drift across weeks). Fall back to "shift only
            // when the canonical history field disagrees with itself".
            let is_shift = m.shape_changed();
            let opponent_label = simulator_data
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
        .collect();

    let (neighbor_teams, country_leagues) =
        get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let league_refs: Vec<(&str, &str)> = country_leagues
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
    let menu_sections = views::team_menu(
        &menu_params,
        &neighbor_refs,
        &team.slug,
        &league_refs,
        team.team_type == core::TeamType::Main,
    );
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
        show_academy_tab: team.team_type == core::TeamType::Main
            || team.team_type == core::TeamType::U18,
        formation_name,
        formation_players,
        recent_used_shapes,
    })
}

fn position_to_css_class(pos: &PlayerPositionType) -> String {
    match pos {
        PlayerPositionType::Goalkeeper => "pos-gk".to_string(),
        PlayerPositionType::Sweeper => "pos-sw".to_string(),
        PlayerPositionType::DefenderLeft => "pos-dl".to_string(),
        PlayerPositionType::DefenderCenterLeft => "pos-dcl".to_string(),
        PlayerPositionType::DefenderCenter => "pos-dc".to_string(),
        PlayerPositionType::DefenderCenterRight => "pos-dcr".to_string(),
        PlayerPositionType::DefenderRight => "pos-dr".to_string(),
        PlayerPositionType::DefensiveMidfielder => "pos-dm".to_string(),
        PlayerPositionType::WingbackLeft => "pos-wl".to_string(),
        PlayerPositionType::WingbackRight => "pos-wr".to_string(),
        PlayerPositionType::MidfielderLeft => "pos-ml".to_string(),
        PlayerPositionType::MidfielderCenterLeft => "pos-mcl".to_string(),
        PlayerPositionType::MidfielderCenter => "pos-mc".to_string(),
        PlayerPositionType::MidfielderCenterRight => "pos-mcr".to_string(),
        PlayerPositionType::MidfielderRight => "pos-mr".to_string(),
        PlayerPositionType::AttackingMidfielderLeft => "pos-aml".to_string(),
        PlayerPositionType::AttackingMidfielderCenter => "pos-amc".to_string(),
        PlayerPositionType::AttackingMidfielderRight => "pos-amr".to_string(),
        PlayerPositionType::ForwardLeft => "pos-fl".to_string(),
        PlayerPositionType::ForwardCenter => "pos-fc".to_string(),
        PlayerPositionType::ForwardRight => "pos-fr".to_string(),
        PlayerPositionType::Striker => "pos-st".to_string(),
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
