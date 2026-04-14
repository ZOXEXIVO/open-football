pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::HappinessEventType;
use core::SimulatorData;
use serde::Deserialize;

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &crate::I18n,
) -> Result<(Vec<(String, String)>, Vec<(String, String)>), ApiError> {
    let club = data.club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;
    let club_name = &club.name;
    let mut teams: Vec<(String, String, u16)> = club.teams.teams.iter()
        .map(|team| (format!("{}  |  {}", club_name, i18n.t(team.team_type.as_i18n_key())), team.slug.clone(), team.reputation.world))
        .collect();
    teams.sort_by(|a, b| b.2.cmp(&a.2));
    let mut country_leagues: Vec<(u32, String, String)> = data.country_by_club(club_id)
        .map(|country| country.leagues.leagues.iter().filter(|l| !l.friendly)
            .map(|l| (l.id, l.name.clone(), l.slug.clone())).collect())
        .unwrap_or_default();
    country_leagues.sort_by_key(|(id, _, _)| *id);
    Ok((
        teams.into_iter().map(|(name, slug, _)| (name, slug)).collect(),
        country_leagues.into_iter().map(|(_, name, slug)| (name, slug)).collect(),
    ))
}

#[derive(Deserialize)]
pub struct PlayerEventsRequest {
    pub lang: String,
    pub player_id: u32,
}

pub struct PlayerEventDto {
    pub description: String,
    pub is_positive: bool,
    pub is_negative: bool,
    pub is_big: bool,
    pub days_ago: u16,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/events/index.html")]
pub struct PlayerEventsTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: crate::I18n,
    pub lang: String,
    pub active_tab: &'static str,
    pub player_id: u32,
    pub club_id: u32,
    pub is_on_loan: bool,
    pub is_injured: bool,
    pub events: Vec<PlayerEventDto>,
}

pub async fn player_events_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<PlayerEventsRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let active = simulator_data.player_with_team(route_params.player_id);
    let player = if let Some((p, _)) = active {
        p
    } else if let Some(p) = simulator_data.retired_player(route_params.player_id) {
        p
    } else {
        return Err(ApiError::NotFound(format!(
            "Player with ID {} not found",
            route_params.player_id
        )));
    };
    let team_opt = active.map(|(_, t)| t);

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

    let events = build_events(player, &i18n);

    Ok(PlayerEventsTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        computer_name: &crate::common::default_handler::COMPUTER_NAME,
        title,
        sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
        sub_title_suffix: String::new(),
        sub_title: team_opt
            .map(|t| t.name.clone())
            .unwrap_or_else(|| "Retired".to_string()),
        sub_title_link: team_opt
            .map(|t| format!("/{}/teams/{}", &route_params.lang, &t.slug))
            .unwrap_or_default(),
        sub_title_country_code: String::new(),
        header_color: team_opt
            .and_then(|t| {
                simulator_data
                    .club(t.club_id)
                    .map(|c| c.colors.background.clone())
            })
            .unwrap_or_else(|| "#808080".to_string()),
        foreground_color: team_opt
            .and_then(|t| {
                simulator_data
                    .club(t.club_id)
                    .map(|c| c.colors.foreground.clone())
            })
            .unwrap_or_else(|| "#ffffff".to_string()),
        menu_sections: if let Some(team) = team_opt {
            let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
            let current_path = format!("/{}/teams/{}", &route_params.lang, &team.slug);
            let mp = views::MenuParams {
                i18n: &i18n,
                lang: &route_params.lang,
                current_path: &current_path,
                country_name: cn,
                country_slug: cs,
            };
            views::team_menu(
                &mp,
                &neighbor_refs,
                &team.slug,
                &league_refs,
                team.team_type == core::TeamType::Main,
            )
        } else {
            Vec::new()
        },
        i18n,
        lang: route_params.lang.clone(),
        active_tab: "events",
        player_id: player.id,
        club_id: team_opt.map(|t| t.club_id).unwrap_or(0),
        is_on_loan: player.is_on_loan(),
        is_injured: player.player_attributes.is_injured,
        events,
    })
}

/// Big events: those that visibly impact the player's career or mood
fn is_big_event(event_type: &HappinessEventType) -> bool {
    matches!(
        event_type,
        HappinessEventType::PlayerOfTheMatch
            | HappinessEventType::ManagerDiscipline
            | HappinessEventType::ManagerCriticism
            | HappinessEventType::ContractOffer
            | HappinessEventType::ContractRenewal
            | HappinessEventType::InjuryReturn
            | HappinessEventType::LoanListingAccepted
            | HappinessEventType::ConflictWithTeammate
            | HappinessEventType::DressingRoomSpeech
            | HappinessEventType::SettledIntoSquad
            | HappinessEventType::FeelingIsolated
            | HappinessEventType::AmbitionShock
            | HappinessEventType::SalaryShock
            | HappinessEventType::RoleMismatch
            | HappinessEventType::DreamMove
            | HappinessEventType::SalaryBoost
            | HappinessEventType::JoiningElite
            | HappinessEventType::ContractTerminated
    )
}

pub fn event_type_to_i18n_key(event_type: &HappinessEventType) -> &'static str {
    match event_type {
        HappinessEventType::ManagerPraise => "event_manager_praise",
        HappinessEventType::ManagerDiscipline => "event_manager_discipline",
        HappinessEventType::ManagerPlayingTimePromise => "event_playing_time_promise",
        HappinessEventType::ManagerCriticism => "event_manager_criticism",
        HappinessEventType::ManagerEncouragement => "event_manager_encouragement",
        HappinessEventType::ManagerTacticalInstruction => "event_manager_tactical_instruction",
        HappinessEventType::GoodTraining => "event_good_training",
        HappinessEventType::PoorTraining => "event_poor_training",
        HappinessEventType::MatchSelection => "event_match_selection",
        HappinessEventType::MatchDropped => "event_match_dropped",
        HappinessEventType::ContractOffer => "event_contract_offer",
        HappinessEventType::ContractRenewal => "event_contract_renewal",
        HappinessEventType::InjuryReturn => "event_injury_return",
        HappinessEventType::SquadStatusChange => "event_squad_status_change",
        HappinessEventType::LackOfPlayingTime => "event_lack_of_playing_time",
        HappinessEventType::LoanListingAccepted => "event_loan_listing_accepted",
        HappinessEventType::PlayerOfTheMatch => "event_player_of_the_match",
        HappinessEventType::TeammateBonding => "event_teammate_bonding",
        HappinessEventType::ConflictWithTeammate => "event_conflict_with_teammate",
        HappinessEventType::DressingRoomSpeech => "event_dressing_room_speech",
        HappinessEventType::SettledIntoSquad => "event_settled_into_squad",
        HappinessEventType::FeelingIsolated => "event_feeling_isolated",
        HappinessEventType::SalaryGapNoticed => "event_salary_gap_noticed",
        HappinessEventType::PromiseKept => "event_promise_kept",
        HappinessEventType::PromiseBroken => "event_promise_broken",
        HappinessEventType::AmbitionShock => "event_ambition_shock",
        HappinessEventType::SalaryShock => "event_salary_shock",
        HappinessEventType::RoleMismatch => "event_role_mismatch",
        HappinessEventType::DreamMove => "event_dream_move",
        HappinessEventType::SalaryBoost => "event_salary_boost",
        HappinessEventType::JoiningElite => "event_joining_elite",
        HappinessEventType::ContractTerminated => "event_contract_terminated",
    }
}

fn build_events(player: &core::Player, i18n: &crate::I18n) -> Vec<PlayerEventDto> {
    let mut events: Vec<_> = player
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type != HappinessEventType::GoodTraining)
        .take(100)
        .map(|e| {
            let key = event_type_to_i18n_key(&e.event_type);
            PlayerEventDto {
                description: i18n.t(key).to_string(),
                is_positive: e.magnitude > 0.0,
                is_negative: e.magnitude < 0.0,
                is_big: is_big_event(&e.event_type),
                days_ago: e.days_ago,
            }
        })
        .collect();

    events.sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
    events
}
