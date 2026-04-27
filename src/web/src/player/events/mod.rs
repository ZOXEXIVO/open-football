pub mod routes;

use crate::common::default_handler::{CSS_VERSION, COMPUTER_NAME};
use crate::common::slug::{resolve_player_page, PlayerPage};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::HappinessEventType;
use core::PlayerStatusType;
use core::SimulatorData;
use serde::Deserialize;

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &I18n,
) -> Result<(Vec<(String, String)>, Vec<(String, String)>), ApiError> {
    let club = data.club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;
    let teams = views::neighbor_teams(club, i18n);
    let mut country_leagues: Vec<(u32, String, String)> = data.country_by_club(club_id)
        .map(|country| country.leagues.leagues.iter().filter(|l| !l.friendly)
            .map(|l| (l.id, l.name.clone(), l.slug.clone())).collect())
        .unwrap_or_default();
    country_leagues.sort_by_key(|(id, _, _)| *id);
    Ok((
        teams,
        country_leagues.into_iter().map(|(_, name, slug)| (name, slug)).collect(),
    ))
}

#[derive(Deserialize)]
pub struct PlayerEventsRequest {
    pub lang: String,
    pub player_slug: String,
}

pub struct PlayerEventDto {
    pub description: String,
    pub is_positive: bool,
    pub is_negative: bool,
    pub is_big: bool,
    pub days_ago: u16,
    /// Partner player display info (name + slug) for events that name a
    /// specific teammate. The template renders a link when present.
    pub partner_name: Option<String>,
    pub partner_slug: Option<String>,
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
    pub events: Vec<PlayerEventDto>,
}

pub async fn player_events_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<PlayerEventsRequest>,
) -> ApiResult<Response> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let (player, team_opt, canonical) = match resolve_player_page(
        simulator_data,
        &route_params.player_slug,
        &route_params.lang,
        "/events",
    )? {
        PlayerPage::Found { player, team, canonical_slug } => (player, team, canonical_slug),
        PlayerPage::Redirect(r) => return Ok(r),
    };

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

    let events = build_events(player, &i18n, simulator_data);

    Ok(PlayerEventsTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        title,
        sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
        sub_title_suffix: String::new(),
        sub_title: team_opt
            .map(|t| t.name.clone())
            .unwrap_or_else(|| {
                if player.is_retired() {
                    i18n.t("retired").to_string()
                } else {
                    i18n.t("free_agent").to_string()
                }
            }),
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
        player_slug: canonical,
        club_id: team_opt.map(|t| t.club_id).unwrap_or(0),
        is_on_loan: player.is_on_loan(),
        is_injured: player.player_attributes.is_injured,
        is_unhappy: player.statuses.get().contains(&PlayerStatusType::Unh),
        is_force_match_selection: player.is_force_match_selection,
        is_on_watchlist: simulator_data.watchlist.contains(&player.id),
        events,
    }.into_response())
}

/// Big events: rare, career-visible moments. The threshold is "would a
/// player remember this in retirement" — silverware, career milestones,
/// promotion/relegation, transfer dramas, captaincy hand-overs. Routine
/// match talk, fan/media noise, role oscillation, and gradual integration
/// sit below the bar even when their immediate magnitude is non-trivial.
///
/// Trimmed entries (used to be big, now demoted):
/// - `PlayerOfTheMatch` — fires for stars several times a season; common.
/// - `DerbyDefeat` — squad-wide on every rivalry loss; recurs.
/// - `RedCardFallout` — recurs across a career, particularly for the
///   physical centre-backs / hot-headed strikers it most applies to.
/// - `QualifiedForEurope` — routine at elite clubs; the heuristic gate
///   already only fires it at the upper end of league-rep tiers.
/// - `WonStartingPlace` / `LostStartingPlace` — sticky one-shots, but
///   they oscillate over a career and the UI can't tell whether this is
///   a 19-year-old's breakthrough or a journeyman's third such event.
fn is_big_event(event_type: &HappinessEventType) -> bool {
    matches!(
        event_type,
        // ── Career / silverware ──────────────────────────────
        HappinessEventType::TrophyWon
            | HappinessEventType::Relegated
            | HappinessEventType::PromotionCelebration
            | HappinessEventType::CupFinalDefeat
            // ── Status / role hand-overs ─────────────────────
            | HappinessEventType::CaptaincyAwarded
            | HappinessEventType::CaptaincyRemoved
            | HappinessEventType::YouthBreakthrough
            // SquadRegistrationOmitted is reserved (no emit site yet —
            // see HappinessEventType docs). Removed from is_big_event
            // until a real registration-window emitter exists.
            // ── Contract / transfer drama ────────────────────
            | HappinessEventType::ContractRenewal
            | HappinessEventType::ContractTerminated
            | HappinessEventType::DreamMove
            | HappinessEventType::DreamMoveCollapsed
            | HappinessEventType::JoiningElite
            | HappinessEventType::AmbitionShock
            // ── Match-day milestones ─────────────────────────
            | HappinessEventType::FirstClubGoal
            | HappinessEventType::DerbyHero
            // ── Manager / national team ──────────────────────
            | HappinessEventType::ManagerDeparture
            | HappinessEventType::NationalTeamCallup
            | HappinessEventType::NationalTeamDropped
            | HappinessEventType::PromiseBroken
            | HappinessEventType::PromiseKept
            // ── Dressing-room ──────────────────────────────
            | HappinessEventType::CloseFriendSold
            | HappinessEventType::MentorDeparted
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
        HappinessEventType::ManagerDeparture => "event_manager_departure",
        HappinessEventType::NationalTeamCallup => "event_national_team_callup",
        HappinessEventType::NationalTeamDropped => "event_national_team_dropped",
        HappinessEventType::ShirtNumberPromotion => "event_shirt_number_promotion",
        HappinessEventType::ControversyIncident => "event_controversy_incident",
        HappinessEventType::FirstClubGoal => "event_first_club_goal",
        HappinessEventType::DecisiveGoal => "event_decisive_goal",
        HappinessEventType::SubstituteImpact => "event_substitute_impact",
        HappinessEventType::CleanSheetPride => "event_clean_sheet_pride",
        HappinessEventType::CostlyMistake => "event_costly_mistake",
        HappinessEventType::RedCardFallout => "event_red_card_fallout",
        HappinessEventType::DerbyHero => "event_derby_hero",
        HappinessEventType::DerbyWin => "event_derby_win",
        HappinessEventType::DerbyDefeat => "event_derby_defeat",
        HappinessEventType::TrophyWon => "event_trophy_won",
        HappinessEventType::CupFinalDefeat => "event_cup_final_defeat",
        HappinessEventType::PromotionCelebration => "event_promotion_celebration",
        HappinessEventType::RelegationFear => "event_relegation_fear",
        HappinessEventType::Relegated => "event_relegated",
        HappinessEventType::QualifiedForEurope => "event_qualified_for_europe",
        HappinessEventType::WonStartingPlace => "event_won_starting_place",
        HappinessEventType::LostStartingPlace => "event_lost_starting_place",
        HappinessEventType::CaptaincyAwarded => "event_captaincy_awarded",
        HappinessEventType::CaptaincyRemoved => "event_captaincy_removed",
        HappinessEventType::YouthBreakthrough => "event_youth_breakthrough",
        HappinessEventType::SquadRegistrationOmitted => "event_squad_registration_omitted",
        HappinessEventType::WantedByBiggerClub => "event_wanted_by_bigger_club",
        HappinessEventType::TransferBidRejected => "event_transfer_bid_rejected",
        HappinessEventType::DreamMoveCollapsed => "event_dream_move_collapsed",
        HappinessEventType::FanPraise => "event_fan_praise",
        HappinessEventType::FanCriticism => "event_fan_criticism",
        HappinessEventType::MediaPraise => "event_media_praise",
        HappinessEventType::MediaCriticism => "event_media_criticism",
        HappinessEventType::CloseFriendSold => "event_close_friend_sold",
        HappinessEventType::CompatriotJoined => "event_compatriot_joined",
        HappinessEventType::MentorDeparted => "event_mentor_departed",
        HappinessEventType::LanguageProgress => "event_language_progress",
    }
}

fn build_events(
    player: &core::Player,
    i18n: &I18n,
    simulator_data: &SimulatorData,
) -> Vec<PlayerEventDto> {
    let mut events: Vec<_> = player
        .happiness
        .recent_events
        .iter()
        .filter(|e| e.event_type != HappinessEventType::GoodTraining)
        // Suppress partner-style events that lost track of the partner:
        // showing "Bonded with a teammate" without naming who is confusing.
        .filter(|e| !is_partner_required(&e.event_type) || e.partner_player_id.is_some())
        .take(100)
        .map(|e| {
            let key = event_type_to_i18n_key(&e.event_type);
            let (partner_name, partner_slug) = e
                .partner_player_id
                .and_then(|pid| resolve_partner(simulator_data, pid))
                .map(|(n, s)| (Some(n), Some(s)))
                .unwrap_or((None, None));
            PlayerEventDto {
                description: i18n.t(key).to_string(),
                is_positive: e.magnitude > 0.0,
                is_negative: e.magnitude < 0.0,
                is_big: is_big_event(&e.event_type),
                days_ago: e.days_ago,
                partner_name,
                partner_slug,
            }
        })
        .collect();

    events.sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
    events
}

/// Events that don't make sense without a named partner. If the event was
/// emitted without a partner id (legacy data, generic emit site), it gets
/// filtered out of the player's history view.
fn is_partner_required(event_type: &HappinessEventType) -> bool {
    matches!(
        event_type,
        HappinessEventType::TeammateBonding | HappinessEventType::ConflictWithTeammate
    )
}

/// Resolve `(display_name, canonical_slug)` for the partner player. The
/// slug must be the canonical `{id}-{name}` form produced by
/// `Player::slug()` so the rendered `/players/{slug}` URL parses back to
/// a real player id — using `FullName::slug()` here would strip the id
/// prefix and the link would 404. Returns `None` when the partner can no
/// longer be located (retired and aged out, or never existed); the event
/// is then filtered out so we don't show a dangling link.
fn resolve_partner(data: &SimulatorData, partner_id: u32) -> Option<(String, String)> {
    let p = data.player(partner_id).or_else(|| data.retired_player(partner_id))?;
    let display = format!(
        "{} {}",
        p.full_name.display_first_name(),
        p.full_name.display_last_name()
    );
    Some((display, p.slug()))
}
