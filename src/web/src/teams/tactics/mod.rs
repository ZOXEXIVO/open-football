pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
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
    pub i18n: crate::I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub formation_name: String,
    pub formation_players: Vec<FormationPlayer>,
}

pub struct FormationPlayer {
    pub id: u32,
    pub last_name: String,
    pub position_short: String,
    pub css_class: String,
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
        // Find best available player for this position
        let players = team.players();
        let best_player = players
            .iter()
            .filter(|p| !used_player_ids.contains(&p.id))
            .filter(|p| p.is_ready_for_match())
            .max_by_key(|p| {
                let pos_level = p.positions.get_level(*required_pos) as i32;
                let ability = p.player_attributes.current_ability as i32;
                pos_level * 10 + ability
            });

        if let Some(player) = best_player {
            used_player_ids.push(player.id);
            formation_players.push(FormationPlayer {
                id: player.id,
                last_name: player.full_name.last_name.clone(),
                position_short: required_pos.get_short_name().to_string(),
                css_class: position_to_css_class(required_pos),
            });
        }
    }

    let neighbor_teams: Vec<(String, String)> = get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    let menu_sections = views::team_menu(&i18n, &route_params.lang, &neighbor_refs, &team.slug, &format!("/{}/teams/{}/tactics", &route_params.lang, &team.slug));
    let title = if team.team_type == core::TeamType::Main { team.name.clone() } else { format!("{} - {}", team.name, i18n.t(team.team_type.as_i18n_key())) };

    Ok(TeamTacticsTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        i18n,
        lang: route_params.lang.clone(),
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: league.map(|l| l.name.clone()).unwrap_or_default(),
        sub_title_link: league.map(|l| format!("/{}/leagues/{}", &route_params.lang, &l.slug)).unwrap_or_default(),
        header_color: simulator_data.club(team.club_id).map(|c| c.colors.background.clone()).unwrap_or_default(),
        foreground_color: simulator_data.club(team.club_id).map(|c| c.colors.foreground.clone()).unwrap_or_default(),
        menu_sections,
        team_slug: team.slug.clone(),
        formation_name,
        formation_players,
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
    i18n: &crate::I18n,
) -> Result<Vec<(String, String)>, ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let mut teams: Vec<(String, String, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| (i18n.t(team.team_type.as_i18n_key()).to_string(), team.slug.clone(), team.reputation.world))
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    Ok(teams
        .into_iter()
        .map(|(name, slug, _)| (name, slug))
        .collect())
}
