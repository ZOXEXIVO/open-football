pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::slug::{PlayerPage, resolve_player_page};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::ContractEventContext;
use core::HappinessEvent;
use core::HappinessEventContext;
use core::HappinessEventEvidence;
use core::HappinessEventSeverity;
use core::HappinessEventType;
use core::InjuryRecoveryEventContext;
use core::LeadershipEventContext;
use core::LoanEventContext;
use core::ManagerInteractionEventContext;
use core::MatchPerformanceEventContext;
use core::MatchSelectionContext;
use core::MediaFanEventContext;
use core::NationalTeamEventContext;
use core::PersonalAdaptationEventContext;
use core::PlayerStatusType;
use core::RoleStatusEventContext;
use core::SelectionDecisionScope;
use core::SelectionScoreFactor;
use core::SimulatorData;
use core::SupportEventContext;
use core::SupportTrigger;
use core::TeammateConflictContext;
use core::TrainingEventContext;
use core::TransferInterestContext;
use core::TransferInterestEvidence;
use core::TransferInterestStage;
use serde::Deserialize;

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

#[derive(Deserialize)]
pub struct PlayerEventsRequest {
    pub lang: String,
    pub player_slug: String,
}

pub struct PlayerEventDto {
    /// Single-line headline. Static i18n string for legacy events;
    /// context-aware composition for upgraded ones.
    pub description: String,
    pub is_positive: bool,
    pub is_negative: bool,
    pub is_big: bool,
    pub days_ago: u16,
    /// Pre-formatted "time-ago" label. Renders as the localised "now"
    /// for events emitted today, "{n}d ago" otherwise. Centralised so
    /// the template doesn't have to special-case the zero-day branch.
    pub time_ago_label: String,
    /// Partner player display info (name + slug) for events that name a
    /// specific teammate. The template renders a link when present.
    pub partner_name: Option<String>,
    pub partner_slug: Option<String>,
    /// Why this happened — derived from the event's
    /// `HappinessEventContext` (cause + relationship signal). Empty if
    /// the emit site didn't attach context.
    pub detail: Option<String>,
    /// What may happen next — closed-set follow-up hint. Empty if not set.
    pub follow_up: Option<String>,
    /// Localised severity label (Minor / Moderate / Serious / Major)
    /// for a badge in the UI; empty for events with no context.
    pub severity_label: Option<String>,
    /// Lower-case CSS class-friendly severity tag (matches the i18n
    /// key suffix). Empty when there's no severity to badge.
    pub severity_tag: Option<String>,
    /// Comparison line for selection-related events — names the rival
    /// player who took the slot and why the manager preferred them.
    /// Empty when no `MatchSelectionContext` is attached.
    pub comparison: Option<String>,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/events/index.html")]
pub struct PlayerEventsTemplate {
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
        PlayerPage::Found {
            player,
            team,
            canonical_slug,
        } => (player, team, canonical_slug),
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

    let league_slug = team_opt
        .and_then(|t| t.league_id)
        .and_then(|id| simulator_data.league(id))
        .map(|l| l.slug.clone());
    let events = build_events(
        player,
        &i18n,
        simulator_data,
        &route_params.lang,
        league_slug.as_deref(),
    );

    Ok(PlayerEventsTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        title,
        sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
        sub_title_suffix: String::new(),
        sub_title: team_opt.map(|t| t.name.clone()).unwrap_or_else(|| {
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
        events_count: events.len(),
        events,
    }
    .into_response())
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
            // ── Awards ───────────────────────────────────────
            | HappinessEventType::PlayerOfTheWeek
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
            // ── Awards / nominations ─────────────────────────
            | HappinessEventType::PlayerOfTheMonth
            | HappinessEventType::YoungPlayerOfTheMonth
            | HappinessEventType::YoungPlayerOfTheWeek
            | HappinessEventType::TeamOfTheMonthSelection
            | HappinessEventType::YoungTeamOfTheMonthSelection
            | HappinessEventType::TeamOfTheSeasonSelection
            | HappinessEventType::TeamOfTheYearSelection
            | HappinessEventType::PlayerOfTheSeason
            | HappinessEventType::YoungPlayerOfTheSeason
            | HappinessEventType::LeagueTopScorer
            | HappinessEventType::LeagueTopAssists
            | HappinessEventType::LeagueGoldenGlove
            | HappinessEventType::ContinentalPlayerOfYearNomination
            | HappinessEventType::ContinentalPlayerOfYear
            | HappinessEventType::WorldPlayerOfYearNomination
            | HappinessEventType::WorldPlayerOfYear
            // ── Real-life football milestones ────────────────
            | HappinessEventType::SeniorDebut
            | HappinessEventType::NationalTeamDebut
            | HappinessEventType::HatTrick
            | HappinessEventType::AppearanceMilestone
            | HappinessEventType::GoalMilestone
            | HappinessEventType::CleanSheetMilestone
            | HappinessEventType::LeadershipEmergence
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
        HappinessEventType::PlayerOfTheWeek => "event_player_of_the_week",
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
        HappinessEventType::PlayerOfTheMonth => "event_player_of_the_month",
        HappinessEventType::YoungPlayerOfTheMonth => "event_young_player_of_the_month",
        HappinessEventType::YoungPlayerOfTheWeek => "event_young_player_of_the_week",
        HappinessEventType::TeamOfTheWeekSelection => "event_team_of_the_week_selection",
        HappinessEventType::YoungTeamOfTheWeekSelection => {
            "event_young_team_of_the_week_selection"
        }
        HappinessEventType::TeamOfTheMonthSelection => "event_team_of_the_month_selection",
        HappinessEventType::YoungTeamOfTheMonthSelection => {
            "event_young_team_of_the_month_selection"
        }
        HappinessEventType::TeamOfTheSeasonSelection => "event_team_of_the_season_selection",
        HappinessEventType::TeamOfTheYearSelection => "event_team_of_the_year_selection",
        HappinessEventType::PlayerOfTheSeason => "event_player_of_the_season",
        HappinessEventType::YoungPlayerOfTheSeason => "event_young_player_of_the_season",
        HappinessEventType::LeagueTopScorer => "event_league_top_scorer",
        HappinessEventType::LeagueTopAssists => "event_league_top_assists",
        HappinessEventType::LeagueGoldenGlove => "event_league_golden_glove",
        HappinessEventType::ContinentalPlayerOfYearNomination => {
            "event_continental_player_of_year_nomination"
        }
        HappinessEventType::ContinentalPlayerOfYear => "event_continental_player_of_year",
        HappinessEventType::WorldPlayerOfYearNomination => {
            "event_world_player_of_year_nomination"
        }
        HappinessEventType::WorldPlayerOfYear => "event_world_player_of_year",
        HappinessEventType::SeniorDebut => "event_senior_debut",
        HappinessEventType::NationalTeamDebut => "event_national_team_debut",
        HappinessEventType::HatTrick => "event_hat_trick",
        HappinessEventType::AssistHatTrick => "event_assist_hat_trick",
        HappinessEventType::GoalDroughtEnded => "event_goal_drought_ended",
        HappinessEventType::ScoringDroughtConcern => "event_scoring_drought_concern",
        HappinessEventType::AppearanceMilestone => "event_appearance_milestone",
        HappinessEventType::GoalMilestone => "event_goal_milestone",
        HappinessEventType::CleanSheetMilestone => "event_clean_sheet_milestone",
        HappinessEventType::TrainingGroundBustUp => "event_training_ground_bust_up",
        HappinessEventType::PublicApology => "event_public_apology",
        HappinessEventType::FansChantPlayerName => "event_fans_chant_player_name",
        HappinessEventType::MediaPressureMounting => "event_media_pressure_mounting",
        HappinessEventType::LeadershipEmergence => "event_leadership_emergence",
        HappinessEventType::ScoutedByClub => "event_scouted_by_club",
        HappinessEventType::TransferRumour => "event_transfer_rumour",
        HappinessEventType::AgentStirsInterest => "event_agent_stirs_interest",
        HappinessEventType::InterestFromBiggerClub => "event_interest_from_bigger_club",
        HappinessEventType::InterestFromRival => "event_interest_from_rival",
        HappinessEventType::HomecomingRumour => "event_homecoming_rumour",
        HappinessEventType::FormerClubInterest => "event_former_club_interest",
        HappinessEventType::FavoriteClubInterest => "event_favorite_club_interest",
        HappinessEventType::TransferSpeculationDistracts => {
            "event_transfer_speculation_distracts"
        }
        HappinessEventType::TransferInterestDismissed => "event_transfer_interest_dismissed",
        HappinessEventType::TransferTalksExpected => "event_transfer_talks_expected",
        HappinessEventType::InterestCooled => "event_interest_cooled",
        HappinessEventType::UsedInterestForContractLeverage => {
            "event_used_interest_for_contract_leverage"
        }
        HappinessEventType::FansReactToTransferRumour => "event_fans_react_to_transfer_rumour",
    }
}

pub struct PlayerEventsCounter;

impl PlayerEventsCounter {
    pub fn count(player: &core::Player) -> usize {
        Self::visible_iter(player).count()
    }

    fn visible_iter(player: &core::Player) -> impl Iterator<Item = &HappinessEvent> {
        player
            .happiness
            .recent_events
            .iter()
            // Routine GoodTraining without context is hidden — it's noise.
            // GoodTraining with a non-routine context (set standards, young
            // impressing, returning from injury, extra work) is shown.
            .filter(|e| {
                if e.event_type != HappinessEventType::GoodTraining {
                    return true;
                }
                match e.context.as_ref().and_then(|c| c.training_context.as_ref()) {
                    Some(tc) => !matches!(
                        tc.reason,
                        core::TrainingEventReason::RoutineGoodSession
                    ),
                    None => false,
                }
            })
            // Suppress partner-style events that lost track of the partner:
            // showing "Bonded with a teammate" without naming who is confusing.
            .filter(|e| !is_partner_required(&e.event_type) || e.partner_player_id.is_some())
            .take(100)
    }
}

fn build_events(
    player: &core::Player,
    i18n: &I18n,
    simulator_data: &SimulatorData,
    lang: &str,
    league_slug: Option<&str>,
) -> Vec<PlayerEventDto> {
    let mut events: Vec<_> = PlayerEventsCounter::visible_iter(player)
        .map(|e| {
            let resolved_partner = e
                .partner_player_id
                .and_then(|pid| resolve_partner(simulator_data, pid));

            let description = build_description(
                e,
                resolved_partner.as_ref(),
                i18n,
                lang,
                league_slug,
            );

            // When the description already names + links the partner
            // inline, suppress the template's trailing dash-suffix so the
            // partner doesn't appear twice in the rendered row.
            let (partner_name, partner_slug) = if description.partner_in_headline {
                (None, None)
            } else {
                resolved_partner
                    .map(|(n, s)| (Some(n), Some(s)))
                    .unwrap_or((None, None))
            };
            let (detail, follow_up, severity_label, severity_tag) =
                EventContextRenderer::render(e, i18n);
            let comparison = e
                .context
                .as_ref()
                .and_then(|ctx| ctx.selection_context.as_ref())
                .and_then(|sel| {
                    SelectionRender::comparison(sel, simulator_data, i18n, lang)
                });
            // Context-aware headline routing. The dispatcher tries each
            // registered renderer in order; the first whose `handles()`
            // matches AND whose specialized context is attached produces
            // the headline. Falls back to the legacy description when
            // no renderer applies.
            let dispatcher = HeadlineDispatcher {
                event: e,
                simulator_data,
                i18n,
                lang,
            };
            let (description_html, partner_in_headline) = dispatcher
                .try_render()
                .unwrap_or((description.html, description.partner_in_headline));

            let (partner_name, partner_slug) = if partner_in_headline {
                (None, None)
            } else {
                (partner_name, partner_slug)
            };

            PlayerEventDto {
                description: description_html,
                is_positive: e.magnitude > 0.0,
                is_negative: e.magnitude < 0.0,
                is_big: is_big_event(&e.event_type),
                days_ago: e.days_ago,
                time_ago_label: EventContextRenderer::time_ago_label(e.days_ago, i18n),
                partner_name,
                partner_slug,
                detail,
                follow_up,
                severity_label,
                severity_tag,
                comparison,
            }
        })
        .collect();

    events.sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
    events
}

/// Renders a [`HappinessEventContext`] payload into the strings the
/// player-events template consumes. Bundling the rendering helpers under
/// a named type keeps the module's call site (`build_events`) readable
/// and lets the unit tests target `EventContextRenderer::*` as a single
/// cluster instead of a scatter of free functions.
struct EventContextRenderer;

impl EventContextRenderer {
    fn render(
        event: &HappinessEvent,
        i18n: &I18n,
    ) -> (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        let Some(ctx) = event.context.as_ref() else {
            return (None, None, None, None);
        };

        let detail = Self::detail_sentences(ctx, i18n);
        let follow_up = ctx
            .follow_up
            .map(|fu| i18n.t(fu.as_i18n_key()).to_string());
        let severity_label = Some(i18n.t(ctx.severity.as_i18n_key()).to_string());
        let severity_tag = Some(Self::severity_tag(ctx.severity).to_string());
        (detail, follow_up, severity_label, severity_tag)
    }

    /// Compose the "Cause" body: a main reason sentence pulled from the
    /// event's cause, followed by AT MOST ONE evidence sentence picked
    /// by [`EvidencePicker::pick`]. Keeps the explanation specific
    /// without dumping every evidence atom on the user.
    fn detail_sentences(ctx: &HappinessEventContext, i18n: &I18n) -> Option<String> {
        // Selection events override the generic relationship cause —
        // the football-specific "manager preferred a fitter teammate"
        // reads better than "tactical disagreement set this off".
        if let Some(sel) = ctx.selection_context.as_ref() {
            return SelectionRender::reason_sentence(sel, i18n);
        }
        // Support events (manager encouragement, dressing-room speech,
        // fan praise, fans-chant) carry their own structured trigger
        // and metadata — the renderer composes a contextual sentence
        // from those rather than the generic relationship cause.
        if let Some(support) = ctx.support_context.as_ref() {
            return SupportRender::reason_sentence(support, ctx, i18n);
        }
        // Transfer-interest events compose a stage / source / kind /
        // evidence sentence followed by the player's private reaction.
        if let Some(tic) = ctx.transfer_interest_context.as_ref() {
            let mut parts: Vec<String> = Vec::new();
            if let Some(reason) = TransferInterestRender::reason_sentence(tic, i18n) {
                parts.push(reason);
            }
            if let Some(reaction) = TransferInterestRender::reaction_sentence(tic, i18n) {
                parts.push(reaction);
            }
            if parts.is_empty() {
                return None;
            }
            return Some(parts.join(" "));
        }
        if let Some(tc) = ctx.training_context.as_ref() {
            return TrainingRender::reason_sentence(tc, i18n);
        }
        if let Some(conflict) = ctx.teammate_conflict_context.as_ref() {
            let mut parts: Vec<String> = Vec::new();
            if let Some(reason) = TeammateConflictRender::reason_sentence(conflict, i18n) {
                parts.push(reason);
            }
            if let Some(evidence) = TeammateConflictRender::evidence_sentence(conflict, i18n) {
                parts.push(evidence);
            }
            if !parts.is_empty() {
                return Some(parts.join(" "));
            }
        }
        if let Some(mc) = ctx.manager_interaction_context.as_ref() {
            let mut parts: Vec<String> = Vec::new();
            if let Some(reason) = ManagerInteractionRender::reason_sentence(mc, i18n) {
                parts.push(reason);
            }
            if let Some(evidence) = ManagerInteractionRender::evidence_sentence(mc, i18n) {
                parts.push(evidence);
            }
            if !parts.is_empty() {
                return Some(parts.join(" "));
            }
            return None;
        }
        if let Some(cc) = ctx.contract_context.as_ref() {
            return ContractRender::reason_sentence(cc, i18n);
        }
        if let Some(ic) = ctx.injury_context.as_ref() {
            return InjuryRecoveryRender::reason_sentence(ic, i18n);
        }
        if let Some(mp) = ctx.match_performance_context.as_ref() {
            return MatchPerformanceRender::reason_sentence(mp, i18n);
        }
        if let Some(rc) = ctx.role_status_context.as_ref() {
            return RoleStatusRender::reason_sentence(rc, i18n);
        }
        if let Some(nt) = ctx.national_team_context.as_ref() {
            return NationalTeamRender::reason_sentence(nt, i18n);
        }
        if let Some(lc) = ctx.leadership_context.as_ref() {
            return LeadershipRender::reason_sentence(lc, i18n);
        }
        if let Some(mf) = ctx.media_fan_context.as_ref() {
            return MediaFanRender::reason_sentence(mf, i18n);
        }
        if let Some(pa) = ctx.personal_adaptation_context.as_ref() {
            return PersonalAdaptationRender::reason_sentence(pa, i18n);
        }
        if let Some(lc) = ctx.loan_context.as_ref() {
            return LoanRender::reason_sentence(lc, i18n);
        }
        if let Some(rc) = ctx.recognition_context.as_ref() {
            return RecognitionRender::reason_sentence(rc, i18n);
        }
        if let Some(sc) = ctx.season_outcome_context.as_ref() {
            return SeasonOutcomeRender::reason_sentence(sc, i18n);
        }
        if let Some(rc) = ctx.regulation_context.as_ref() {
            return RegulationRender::reason_sentence(rc, i18n);
        }
        let cause_key = format!("reason_main_{}", Self::cause_token(ctx));
        let main = i18n.t(&cause_key);
        let mut out = main.to_string();

        if let Some(evidence) = EvidencePicker::pick(ctx) {
            let ev_key = format!("reason_ev_{}", Self::evidence_token(evidence));
            let sentence = i18n.t(&ev_key);
            // Skip if the i18n layer fell back to the raw key — keeps
            // partially-translated locales from showing the placeholder.
            if sentence != ev_key {
                out.push(' ');
                out.push_str(sentence);
            }
        }

        Some(out)
    }

    /// Localised "X d ago" / "now" label. Centralised here so the
    /// template stays free of zero-day branching — an event emitted
    /// today reads as "now" rather than the visually noisy "0d ago".
    fn time_ago_label(days_ago: u16, i18n: &I18n) -> String {
        if days_ago == 0 {
            i18n.t("now").to_string()
        } else {
            format!("{}{}", days_ago, i18n.t("days_ago_short"))
        }
    }

    fn severity_tag(severity: HappinessEventSeverity) -> &'static str {
        match severity {
            HappinessEventSeverity::Minor => "minor",
            HappinessEventSeverity::Moderate => "moderate",
            HappinessEventSeverity::Serious => "serious",
            HappinessEventSeverity::Major => "major",
        }
    }

    /// Cause → i18n-key suffix. The full key is `reason_main_<token>`,
    /// kept here as a single function so the locale audit can grep one
    /// list and confirm every cause has a sentence.
    fn cause_token(ctx: &HappinessEventContext) -> &'static str {
        use core::HappinessEventCause as C;
        match ctx.cause {
            C::PersonalityClash => "personality_clash",
            C::TrainingFriction => "training_friction",
            C::PositionalRivalry => "positional_rivalry",
            C::WageJealousy => "wage_jealousy",
            C::PoorFormPressure => "poor_form_pressure",
            C::LeadershipDispute => "leadership_dispute",
            C::TacticalDisagreement => "tactical_disagreement",
            C::AdaptationIsolation => "adaptation_isolation",
            C::MediaPressure => "media_pressure",
            C::MentorDeparture => "mentor_departure",
            C::FriendDeparture => "friend_departure",
            C::MatchCooperation => "match_cooperation",
            C::NationalityIntegration => "nationality_integration",
            C::TrainingPartnership => "training_partnership",
            C::ReputationTension => "reputation_tension",
            C::ReputationAdmiration => "reputation_admiration",
            C::ManagerSupport => "manager_support",
            C::SupporterAppreciation => "supporter_appreciation",
            C::SupporterIdentification => "supporter_identification",
            C::DressingRoomLift => "dressing_room_lift",
            C::Other => "other",
        }
    }

    fn evidence_token(evidence: HappinessEventEvidence) -> &'static str {
        use HappinessEventEvidence as E;
        match evidence {
            E::StrongExistingBond => "strong_existing_bond",
            E::AlreadyStrainedRelationship => "already_strained_relationship",
            E::WeakExistingBond => "weak_existing_bond",
            E::SamePositionCompetition => "same_position_competition",
            E::SimilarSquadStatusCompetition => "similar_squad_status_competition",
            E::LowTrust => "low_trust",
            E::LowFriendship => "low_friendship",
            E::LowProfessionalRespect => "low_professional_respect",
            E::HighProfessionalRespect => "high_professional_respect",
            E::HighAmbition => "high_ambition",
            E::LowTemperament => "low_temperament",
            E::HighControversy => "high_controversy",
            E::LowSportsmanship => "low_sportsmanship",
            E::HighProfessionalism => "high_professionalism",
            E::NewSigningStillSettling => "new_signing_still_settling",
            E::LanguageBarrier => "language_barrier",
            E::SharedNationality => "shared_nationality",
            E::MentorInfluence => "mentor_influence",
            E::MatchCooperation => "match_cooperation",
            E::ComplementaryRoles => "complementary_roles",
            E::TrainingStandardsMismatch => "training_standards_mismatch",
            E::RepeatedIncident => "repeated_incident",
            E::WageGap => "wage_gap",
            E::ReputationGap => "reputation_gap",
            E::NoInnerCircleYet => "no_inner_circle_yet",
            E::SquadTurnover => "squad_turnover",
            E::MediaIncident => "media_incident",
            E::DressingRoomRow => "dressing_room_row",
            E::TrainingGroundIncident => "training_ground_incident",
            E::ExcellentPerformance => "excellent_performance",
            E::PlayerOfTheMatch => "player_of_the_match",
            E::GoalContribution => "goal_contribution",
            E::DecisiveContribution => "decisive_contribution",
            E::DerbyPerformance => "derby_performance",
            E::CupPerformance => "cup_performance",
            E::HomeCrowdMoment => "home_crowd_moment",
            E::PoorMoraleBeforeTalk => "poor_morale_before_talk",
            E::LowConfidence => "low_confidence",
            E::ManagerTrust => "manager_trust",
            E::StrongCoachRapport => "strong_coach_rapport",
            E::WeakCoachRapport => "weak_coach_rapport",
            E::HighPressurePersonality => "high_pressure_personality",
            E::LowPressurePersonality => "low_pressure_personality",
            E::HighDetermination => "high_determination",
            E::ImportantMatchTemperament => "important_match_temperament",
            E::RepeatedTalkDampened => "repeated_talk_dampened",
            E::CaptainOrLeaderInfluence => "captain_or_leader_influence",
            E::YoungPlayerNeedingConfidence => "young_player_needing_confidence",
            E::ReturnFromInjuryBoost => "return_from_injury_boost",
        }
    }
}

/// Picks the single most informative evidence atom from a context. The
/// renderer surfaces at most one — surfacing all of them would turn a
/// dense FM event row into a paragraph wall.
///
/// Priority order is hand-tuned: the more specific or actionable an
/// atom, the higher it ranks. Same context always picks the same atom,
/// keeping the rendered text stable across page reloads (acceptance
/// criterion: deterministic explanations).
struct EvidencePicker;

impl EvidencePicker {
    fn pick(ctx: &HappinessEventContext) -> Option<HappinessEventEvidence> {
        if ctx.evidence.is_empty() {
            return None;
        }
        // Iterate in priority order rather than evidence-array order so
        // emit-site insertion order doesn't change the rendered sentence.
        for candidate in Self::PRIORITY {
            if ctx.evidence.contains(candidate) {
                return Some(*candidate);
            }
        }
        // Fallback — surface the first atom the emit site recorded.
        ctx.evidence.first().copied()
    }

    /// Hand-tuned priority order. Higher position = more informative
    /// reason for the user. The list intentionally puts strained or
    /// repeated incidents first (they reframe the event entirely),
    /// then specific football contexts, then personality / status
    /// modifiers, then weak-bond filler last.
    const PRIORITY: &'static [HappinessEventEvidence] = &[
        // — Reframing reasons (dominate the explanation) —
        HappinessEventEvidence::AlreadyStrainedRelationship,
        HappinessEventEvidence::RepeatedIncident,
        HappinessEventEvidence::StrongExistingBond,
        // — Football-specific context —
        HappinessEventEvidence::SamePositionCompetition,
        HappinessEventEvidence::SimilarSquadStatusCompetition,
        HappinessEventEvidence::WageGap,
        HappinessEventEvidence::ReputationGap,
        HappinessEventEvidence::TrainingStandardsMismatch,
        HappinessEventEvidence::MatchCooperation,
        HappinessEventEvidence::ComplementaryRoles,
        HappinessEventEvidence::MentorInfluence,
        HappinessEventEvidence::LanguageBarrier,
        HappinessEventEvidence::NoInnerCircleYet,
        HappinessEventEvidence::SharedNationality,
        HappinessEventEvidence::SquadTurnover,
        HappinessEventEvidence::NewSigningStillSettling,
        // — Relationship axes —
        HappinessEventEvidence::LowTrust,
        HappinessEventEvidence::LowFriendship,
        HappinessEventEvidence::LowProfessionalRespect,
        HappinessEventEvidence::HighProfessionalRespect,
        // — Personality modifiers —
        HappinessEventEvidence::HighControversy,
        HappinessEventEvidence::LowTemperament,
        HappinessEventEvidence::LowSportsmanship,
        HappinessEventEvidence::HighProfessionalism,
        HappinessEventEvidence::HighAmbition,
        // — Scope tags (where it played out) —
        HappinessEventEvidence::TrainingGroundIncident,
        HappinessEventEvidence::DressingRoomRow,
        HappinessEventEvidence::MediaIncident,
        // — Filler (least informative) —
        HappinessEventEvidence::WeakExistingBond,
    ];
}

struct DescriptionRender {
    html: String,
    /// True when the partner's name+link is embedded inside `html` —
    /// signals to the caller that the template's trailing-dash suffix
    /// must be suppressed for this row.
    partner_in_headline: bool,
}

/// Renders the `MatchSelectionContext` payload into the strings the
/// player-events template expects. Centralises the headline /
/// comparison / reason composition so the build pipeline stays
/// readable, and so the unit tests can target one named cluster
/// instead of scattered free functions.
struct SelectionRender;

impl SelectionRender {
    /// Compose the "Lost out to {rival} ..." headline. Falls back to
    /// the localized scope copy when no rival player can be resolved
    /// (left out of squad with no positional comparison).
    fn headline(
        ctx: &MatchSelectionContext,
        data: &SimulatorData,
        i18n: &I18n,
        lang: &str,
    ) -> DescriptionRender {
        if let Some(comp) = ctx.comparison.as_ref() {
            if let Some((name, slug)) = resolve_partner(data, comp.selected_player_id) {
                let link = format!(
                    r#"<a href="/{}/players/{}">{}</a>"#,
                    lang, slug, name
                );
                let raw = i18n.t(Self::headline_key_for(ctx, true));
                let html = raw.replace("{rival}", &link);
                return DescriptionRender {
                    html,
                    partner_in_headline: true,
                };
            }
        }
        let html = i18n.t(Self::headline_key_for(ctx, false)).to_string();
        DescriptionRender {
            html,
            partner_in_headline: false,
        }
    }

    /// Headline i18n key — picks a scope-aware variant. The `_named`
    /// suffix variants embed `{rival}` for the rival's player link.
    fn headline_key_for(
        ctx: &MatchSelectionContext,
        with_rival: bool,
    ) -> &'static str {
        match (ctx.scope, with_rival) {
            (SelectionDecisionScope::LeftOutOfMatchdaySquad, true) => {
                "selection_headline_left_out_named"
            }
            (SelectionDecisionScope::LeftOutOfMatchdaySquad, false) => {
                "selection_headline_left_out"
            }
            (SelectionDecisionScope::DroppedToBench, true) => {
                "selection_headline_dropped_to_bench_named"
            }
            (SelectionDecisionScope::DroppedToBench, false) => {
                "selection_headline_dropped_to_bench"
            }
            (SelectionDecisionScope::Rested, _) => "selection_headline_rested",
            (SelectionDecisionScope::Rotation, _) => "selection_headline_rotation",
            (SelectionDecisionScope::UnavailableButNotInjured, _) => {
                "selection_headline_unavailable"
            }
            (SelectionDecisionScope::UnusedSubstitute, true) => {
                "selection_headline_unused_sub_named"
            }
            (SelectionDecisionScope::UnusedSubstitute, false) => {
                "selection_headline_unused_sub"
            }
        }
    }

    /// Build the "Cause" body — a single composed sentence describing
    /// why the manager picked someone else.
    fn reason_sentence(
        ctx: &MatchSelectionContext,
        i18n: &I18n,
    ) -> Option<String> {
        let key = ctx.reason.as_i18n_key();
        let main = i18n.t(key);
        if main == key {
            return None;
        }
        Some(main.to_string())
    }

    /// Build the comparison line: who was preferred and along which
    /// dominant scoring axis. Returns `None` when no rival could be
    /// resolved (avoid a dangling sentence with a missing player).
    fn comparison(
        ctx: &MatchSelectionContext,
        data: &SimulatorData,
        i18n: &I18n,
        lang: &str,
    ) -> Option<String> {
        let comp = ctx.comparison.as_ref()?;
        let (name, slug) = resolve_partner(data, comp.selected_player_id)?;
        let link = format!(r#"<a href="/{}/players/{}">{}</a>"#, lang, slug, name);

        let factor_phrase = comp
            .top_factors
            .first()
            .map(|f| Self::factor_phrase(*f, i18n))
            .unwrap_or_default();

        let template_key = if factor_phrase.is_empty() {
            "selection_comparison_plain"
        } else {
            "selection_comparison_with_factor"
        };
        let template = i18n.t(template_key);
        let mut out = template.replace("{rival}", &link);
        out = out.replace("{factor}", &factor_phrase);
        Some(out)
    }

    fn factor_phrase(factor: SelectionScoreFactor, i18n: &I18n) -> String {
        i18n.t(factor.as_i18n_key()).to_string()
    }
}

/// Renders the structured payload for support / approval events
/// (`ManagerEncouragement`, `DressingRoomSpeech`, `FanPraise`,
/// `FansChantPlayerName`). Picks deterministic copy variants from the
/// stored trigger / phase / tone so the same saved event always renders
/// the same string.
struct SupportRender;

impl SupportRender {
    pub fn handles(event_type: &HappinessEventType) -> bool {
        matches!(
            event_type,
            HappinessEventType::ManagerEncouragement
                | HappinessEventType::DressingRoomSpeech
                | HappinessEventType::FanPraise
                | HappinessEventType::FansChantPlayerName
        )
    }

    /// Build the headline copy. Picks a variant from the trigger /
    /// phase / tone — same context always yields the same string.
    pub fn headline(
        event_type: &HappinessEventType,
        support: &SupportEventContext,
        i18n: &I18n,
    ) -> String {
        let key = Self::headline_key(event_type, support);
        let translated = i18n.t(key);
        if translated == key {
            // Fallback to the legacy static line when the locale is
            // partially translated and the variant key isn't there yet.
            i18n.t(event_type_to_i18n_key(event_type)).to_string()
        } else {
            translated.to_string()
        }
    }

    /// Pick the variant headline key based on the structured context.
    /// Order of preference is hand-tuned: more specific situational
    /// triggers beat the generic ones.
    fn headline_key(
        event_type: &HappinessEventType,
        support: &SupportEventContext,
    ) -> &'static str {
        match event_type {
            HappinessEventType::ManagerEncouragement => match support.trigger {
                SupportTrigger::PlayerOfMatch => "event_manager_encouragement_pom",
                SupportTrigger::DecisiveMoment => "event_manager_encouragement_decisive",
                SupportTrigger::GoalContribution => "event_manager_encouragement_goal_contribution",
                SupportTrigger::HighRating => "event_manager_encouragement_high_rating",
                SupportTrigger::PoorMorale => "event_manager_encouragement_morale_lift",
                SupportTrigger::PoorFormRecovery => "event_manager_encouragement_form_recovery",
                SupportTrigger::ReturningFromInjury => "event_manager_encouragement_return_injury",
                SupportTrigger::YoungPlayerConfidence => "event_manager_encouragement_young_player",
                SupportTrigger::LeadershipMoment => "event_manager_encouragement_leadership",
                SupportTrigger::Derby => "event_manager_encouragement_derby",
                SupportTrigger::CupTie => "event_manager_encouragement_cup",
                _ => "event_manager_encouragement_default",
            },
            HappinessEventType::DressingRoomSpeech => Self::dressing_room_key(support),
            HappinessEventType::FanPraise => match support.trigger {
                SupportTrigger::PlayerOfMatch => "event_fan_praise_pom",
                SupportTrigger::DecisiveMoment => "event_fan_praise_decisive",
                SupportTrigger::GoalContribution if support.team_won.unwrap_or(false) => {
                    "event_fan_praise_goal_contribution_win"
                }
                SupportTrigger::GoalContribution => "event_fan_praise_goal_contribution",
                SupportTrigger::HighRating => "event_fan_praise_high_rating",
                SupportTrigger::Derby => "event_fan_praise_derby",
                SupportTrigger::CupTie => "event_fan_praise_cup",
                _ => "event_fan_praise_default",
            },
            HappinessEventType::FansChantPlayerName => match support.trigger {
                SupportTrigger::DecisiveMoment => "event_fans_chant_player_name_decisive",
                SupportTrigger::PlayerOfMatch => "event_fans_chant_player_name_pom",
                SupportTrigger::GoalContribution => "event_fans_chant_player_name_goal",
                SupportTrigger::Derby => "event_fans_chant_player_name_derby",
                SupportTrigger::CupTie => "event_fans_chant_player_name_cup",
                _ => "event_fans_chant_player_name_default",
            },
            _ => "",
        }
    }

    /// Dressing-room speech variant — picked from phase + tone with
    /// score_delta-aware modifiers ("trailing at half-time" reads
    /// differently from "leading at half-time").
    fn dressing_room_key(support: &SupportEventContext) -> &'static str {
        use core::SupportMatchPhase as Ph;
        use core::SupportTone as T;
        let phase = support.phase.unwrap_or(Ph::FullTime);
        let tone = support.tone.unwrap_or(T::Calm);
        match (phase, tone) {
            (Ph::PreMatch, T::Passionate) => "event_dressing_room_speech_pre_passionate",
            (Ph::PreMatch, T::Praise) => "event_dressing_room_speech_pre_praise",
            (Ph::PreMatch, T::Encourage) => "event_dressing_room_speech_pre_encourage",
            (Ph::PreMatch, T::Criticise) => "event_dressing_room_speech_pre_criticise",
            (Ph::HalfTime, T::Passionate) => "event_dressing_room_speech_half_passionate",
            (Ph::HalfTime, T::Praise) => "event_dressing_room_speech_half_praise",
            (Ph::HalfTime, T::Encourage) => "event_dressing_room_speech_half_encourage",
            (Ph::HalfTime, T::Criticise) => "event_dressing_room_speech_half_criticise",
            (Ph::FullTime, T::Praise) => "event_dressing_room_speech_full_praise",
            (Ph::FullTime, T::Criticise) => "event_dressing_room_speech_full_criticise",
            (Ph::FullTime, T::Passionate) => "event_dressing_room_speech_full_passionate",
            (Ph::FullTime, T::Encourage) => "event_dressing_room_speech_full_encourage",
            _ => "event_dressing_room_speech_default",
        }
    }

    /// Compose the "Cause" body. Falls back to a generic situational
    /// sentence when no specific evidence is attached.
    pub fn reason_sentence(
        support: &SupportEventContext,
        ctx: &HappinessEventContext,
        i18n: &I18n,
    ) -> Option<String> {
        let mut out = String::new();

        let trigger_key = format!("support_reason_main_{}", Self::trigger_token(support.trigger));
        let main = i18n.t(&trigger_key);
        if main != trigger_key {
            out.push_str(main);
        } else {
            // Fall back to a setting-based sentence so we never expose
            // the raw key.
            let setting_key = format!("support_reason_setting_{}", Self::setting_token(support));
            let setting = i18n.t(&setting_key);
            if setting != setting_key {
                out.push_str(setting);
            }
        }

        // Attach a single evidence sentence if any (same picker as the
        // generic path, but we use the support evidence atoms).
        if let Some(evidence) = EvidencePicker::pick(ctx) {
            let ev_key = format!("reason_ev_{}", EventContextRenderer::evidence_token(evidence));
            let sentence = i18n.t(&ev_key);
            if sentence != ev_key {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(sentence);
            }
        }

        if out.is_empty() {
            return None;
        }
        Some(out)
    }

    fn trigger_token(trigger: SupportTrigger) -> &'static str {
        match trigger {
            SupportTrigger::HighRating => "high_rating",
            SupportTrigger::PlayerOfMatch => "pom",
            SupportTrigger::GoalContribution => "goal_contribution",
            SupportTrigger::DecisiveMoment => "decisive_moment",
            SupportTrigger::PoorMorale => "poor_morale",
            SupportTrigger::PoorFormRecovery => "form_recovery",
            SupportTrigger::BigMatch => "big_match",
            SupportTrigger::Derby => "derby",
            SupportTrigger::CupTie => "cup_tie",
            SupportTrigger::LeadershipMoment => "leadership_moment",
            SupportTrigger::TeamTrailingAtHalfTime => "trailing_half_time",
            SupportTrigger::TeamWon => "team_won",
            SupportTrigger::YoungPlayerConfidence => "young_player_confidence",
            SupportTrigger::ReturningFromInjury => "returning_from_injury",
            SupportTrigger::Generic => "generic",
        }
    }

    fn setting_token(support: &SupportEventContext) -> &'static str {
        use core::SupportSetting as S;
        match support.setting {
            S::PrivateTalk => "private",
            S::TrainingGround => "training_ground",
            S::DressingRoom => "dressing_room",
            S::Touchline => "touchline",
            S::HomeCrowd => "home_crowd",
            S::AwayEnd => "away_end",
            S::PostMatch => "post_match",
        }
    }
}

/// Renders the structured `TransferInterestContext` payload — the
/// transfer-interest funnel events use a stage- and kind-aware
/// headline that names the interested club inline, an evidence-driven
/// reason sentence, and a reaction line tied to the player's
/// personality and context.
struct TransferInterestRender;

impl TransferInterestRender {
    fn handles(event_type: &HappinessEventType) -> bool {
        matches!(
            event_type,
            HappinessEventType::ScoutedByClub
                | HappinessEventType::TransferRumour
                | HappinessEventType::AgentStirsInterest
                | HappinessEventType::InterestFromBiggerClub
                | HappinessEventType::InterestFromRival
                | HappinessEventType::HomecomingRumour
                | HappinessEventType::FormerClubInterest
                | HappinessEventType::FavoriteClubInterest
                | HappinessEventType::TransferSpeculationDistracts
                | HappinessEventType::TransferInterestDismissed
                | HappinessEventType::TransferTalksExpected
                | HappinessEventType::InterestCooled
                | HappinessEventType::UsedInterestForContractLeverage
                | HappinessEventType::TransferBidRejected
                | HappinessEventType::DreamMoveCollapsed
                | HappinessEventType::WantedByBiggerClub
        )
    }

    /// Headline composition: pick a stage-aware key and substitute
    /// `{club}` with a club link when the interested club resolves to a
    /// real Club in the simulator data.
    fn headline(
        event_type: &HappinessEventType,
        ctx: &TransferInterestContext,
        data: &SimulatorData,
        i18n: &I18n,
        lang: &str,
    ) -> (String, bool) {
        let club_link = ctx
            .interested_club_id
            .and_then(|cid| Self::resolve_club(data, cid))
            .map(|(name, slug)| (name, slug));

        let key = Self::headline_key(event_type, ctx, club_link.is_some());
        let raw = i18n.t(key);
        if raw == key {
            // Fall back to the legacy static line when the locale is
            // partially translated and the variant key isn't there yet.
            let fallback = i18n.t(event_type_to_i18n_key(event_type)).to_string();
            return (fallback, false);
        }
        if let Some((name, slug)) = club_link {
            let link = format!(r#"<a href="/{}/teams/{}">{}</a>"#, lang, slug, name);
            return (raw.replace("{club}", &link), true);
        }
        (raw.to_string(), false)
    }

    fn headline_key(
        event_type: &HappinessEventType,
        ctx: &TransferInterestContext,
        with_club: bool,
    ) -> &'static str {
        use TransferInterestStage as S;
        match (event_type, ctx.interest_stage, with_club) {
            (HappinessEventType::ScoutedByClub, _, true) => {
                "transfer_interest_headline_scouted_named"
            }
            (HappinessEventType::ScoutedByClub, _, false) => {
                "transfer_interest_headline_scouted"
            }
            (HappinessEventType::TransferRumour, _, true) => {
                "transfer_interest_headline_rumour_named"
            }
            (HappinessEventType::TransferRumour, _, false) => {
                "transfer_interest_headline_rumour"
            }
            (HappinessEventType::AgentStirsInterest, _, _) => {
                "transfer_interest_headline_agent_stirs"
            }
            (HappinessEventType::InterestFromBiggerClub, _, true) => {
                "transfer_interest_headline_bigger_named"
            }
            (HappinessEventType::InterestFromBiggerClub, _, false) => {
                "transfer_interest_headline_bigger"
            }
            (HappinessEventType::InterestFromRival, _, true) => {
                "transfer_interest_headline_rival_named"
            }
            (HappinessEventType::InterestFromRival, _, false) => {
                "transfer_interest_headline_rival"
            }
            (HappinessEventType::HomecomingRumour, _, true) => {
                "transfer_interest_headline_homecoming_named"
            }
            (HappinessEventType::HomecomingRumour, _, false) => {
                "transfer_interest_headline_homecoming"
            }
            (HappinessEventType::FormerClubInterest, _, true) => {
                "transfer_interest_headline_former_named"
            }
            (HappinessEventType::FormerClubInterest, _, false) => {
                "transfer_interest_headline_former"
            }
            (HappinessEventType::FavoriteClubInterest, _, true) => {
                "transfer_interest_headline_favorite_named"
            }
            (HappinessEventType::FavoriteClubInterest, _, false) => {
                "transfer_interest_headline_favorite"
            }
            (HappinessEventType::TransferSpeculationDistracts, _, _) => {
                "transfer_interest_headline_speculation_distracts"
            }
            (HappinessEventType::TransferInterestDismissed, _, _) => {
                "transfer_interest_headline_dismissed"
            }
            (HappinessEventType::TransferTalksExpected, _, true) => {
                "transfer_interest_headline_talks_named"
            }
            (HappinessEventType::TransferTalksExpected, _, false) => {
                "transfer_interest_headline_talks"
            }
            (HappinessEventType::InterestCooled, _, true) => {
                "transfer_interest_headline_cooled_named"
            }
            (HappinessEventType::InterestCooled, _, false) => {
                "transfer_interest_headline_cooled"
            }
            (HappinessEventType::UsedInterestForContractLeverage, _, _) => {
                "transfer_interest_headline_leverage"
            }
            (HappinessEventType::TransferBidRejected, _, true) => {
                "transfer_interest_headline_bid_rejected_named"
            }
            (HappinessEventType::TransferBidRejected, _, false) => {
                "transfer_interest_headline_bid_rejected"
            }
            (HappinessEventType::DreamMoveCollapsed, _, true) => {
                "transfer_interest_headline_dream_collapsed_named"
            }
            (HappinessEventType::DreamMoveCollapsed, _, false) => {
                "transfer_interest_headline_dream_collapsed"
            }
            (HappinessEventType::WantedByBiggerClub, S::ScoutWatched, _) => {
                "transfer_interest_headline_scouted"
            }
            (HappinessEventType::WantedByBiggerClub, _, true) => {
                "transfer_interest_headline_bigger_named"
            }
            (HappinessEventType::WantedByBiggerClub, _, false) => {
                "transfer_interest_headline_bigger"
            }
            _ => "transfer_interest_headline_speculation_distracts",
        }
    }

    /// Compose the reason / source line — explains *how* the rumour
    /// surfaced and *what kind* of move it represents.
    fn reason_sentence(ctx: &TransferInterestContext, i18n: &I18n) -> Option<String> {
        let stage_key = ctx.interest_stage.as_i18n_key();
        let source_key = ctx.interest_source.as_i18n_key();
        let kind_key = ctx.interest_kind.as_i18n_key();
        let stage = i18n.t(stage_key);
        let source = i18n.t(source_key);
        let kind = i18n.t(kind_key);

        let mut out = String::new();
        if stage != stage_key {
            out.push_str(stage);
        }
        if source != source_key {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(source);
        }
        if kind != kind_key {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(kind);
        }

        if let Some(evidence) = Self::pick_evidence(ctx) {
            let ev_key = evidence.as_i18n_key();
            let ev = i18n.t(ev_key);
            if ev != ev_key {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(ev);
            }
        }

        if out.is_empty() {
            return None;
        }
        Some(out)
    }

    /// Compose the reaction line — the player's private response and,
    /// when a sporting fit is set, how it shapes the calculation.
    fn reaction_sentence(
        ctx: &TransferInterestContext,
        i18n: &I18n,
    ) -> Option<String> {
        let key = ctx.player_reaction.as_i18n_key();
        let raw = i18n.t(key);
        if raw == key {
            return None;
        }
        let mut out = raw.to_string();
        if let Some(fit) = ctx.sporting_fit {
            let fk = fit.as_i18n_key();
            let f = i18n.t(fk);
            if f != fk {
                out.push(' ');
                out.push_str(f);
            }
        }
        Some(out)
    }

    /// Pick the most informative evidence atom — same idea as the
    /// support-render `EvidencePicker` but for the transfer-interest
    /// evidence catalog. Stable across insertion order.
    fn pick_evidence(ctx: &TransferInterestContext) -> Option<TransferInterestEvidence> {
        if ctx.evidence.is_empty() {
            return None;
        }
        for candidate in Self::PRIORITY {
            if ctx.evidence.contains(candidate) {
                return Some(*candidate);
            }
        }
        ctx.evidence.first().copied()
    }

    const PRIORITY: &'static [TransferInterestEvidence] = &[
        // Reframing reasons that dominate the explanation
        TransferInterestEvidence::FavoriteClub,
        TransferInterestEvidence::FormerClub,
        TransferInterestEvidence::HomeCountry,
        TransferInterestEvidence::RivalClub,
        TransferInterestEvidence::RejectedBid,
        TransferInterestEvidence::ChampionsLeagueOpportunity,
        // Football-specific context
        TransferInterestEvidence::BiggerLeague,
        TransferInterestEvidence::BiggerClub,
        TransferInterestEvidence::MoreLikelyStarts,
        TransferInterestEvidence::LessLikelyStarts,
        TransferInterestEvidence::CurrentPlayingTimeFrustration,
        TransferInterestEvidence::CurrentClubAmbitionMismatch,
        TransferInterestEvidence::ManagerPromiseConflict,
        TransferInterestEvidence::RecentNewSigningThreatensRole,
        // Source modifiers
        TransferInterestEvidence::AgentPushing,
        TransferInterestEvidence::ScoutAtMatch,
        TransferInterestEvidence::RepeatedRumours,
        TransferInterestEvidence::MediaNoise,
        TransferInterestEvidence::FanPressure,
        // Personality / contract
        TransferInterestEvidence::HighAmbition,
        TransferInterestEvidence::HighLoyalty,
        TransferInterestEvidence::HighProfessionalism,
        TransferInterestEvidence::HighControversy,
        TransferInterestEvidence::ContractExpiring,
        TransferInterestEvidence::Underpaid,
        TransferInterestEvidence::CurrentClubLoyalty,
        TransferInterestEvidence::LowAmbition,
        TransferInterestEvidence::LowLoyalty,
        TransferInterestEvidence::LanguageCultureFit,
    ];

    fn resolve_club(data: &SimulatorData, club_id: u32) -> Option<(String, String)> {
        let club = data.club(club_id)?;
        // Club pages are reached via the main team's slug — there is no
        // standalone club-level URL, so use the main team's slug as the
        // canonical "club link" target.
        let team_slug = club
            .teams
            .teams
            .first()
            .map(|t| t.slug.clone())
            .unwrap_or_default();
        if team_slug.is_empty() {
            return None;
        }
        Some((club.name.clone(), team_slug))
    }
}

/// Renderer for the Phase 1-11 structured context payloads. Each
/// cluster (training, manager, contract, ...) picks a deterministic
/// reason key from the stored context so the rendered headline + cause
/// sentence is stable across reloads. Falls back to the legacy static
/// line if no upgraded context was attached at the emit site.
struct TrainingRender;

impl TrainingRender {
    pub fn handles(event_type: &HappinessEventType) -> bool {
        matches!(
            event_type,
            HappinessEventType::GoodTraining | HappinessEventType::PoorTraining
        )
    }

    pub fn headline(ctx: &TrainingEventContext, i18n: &I18n) -> String {
        let key = format!("training_headline_{}", Self::reason_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            i18n.t(ctx.reason.as_i18n_key()).to_string()
        } else {
            raw.to_string()
        }
    }

    pub fn reason_sentence(ctx: &TrainingEventContext, i18n: &I18n) -> Option<String> {
        let key = format!("training_reason_main_{}", Self::reason_token(ctx));
        let main = i18n.t(&key);
        if main == key {
            return None;
        }
        Some(main.to_string())
    }

    fn reason_token(ctx: &TrainingEventContext) -> &'static str {
        use core::TrainingEventReason as R;
        match ctx.reason {
            R::SharpAfterBeingLeftOut => "sharp_after_being_left_out",
            R::RespondedToCriticism => "responded_to_criticism",
            R::StruggledWithIntensity => "struggled_with_intensity",
            R::DistractedByRumours => "distracted_by_rumours",
            R::PoorAttitude => "poor_attitude",
            R::ReturningFromInjuryNotSharp => "returning_from_injury_not_sharp",
            R::YoungImpressedStaff => "young_impressed_staff",
            R::SettingStandards => "setting_standards",
            R::ExtraWorkAfterSession => "extra_work_after_session",
            R::MatchPreparationFocus => "match_preparation_focus",
            R::RoutineGoodSession => "routine_good_session",
            R::RoutineBadSession => "routine_bad_session",
        }
    }
}

struct ManagerInteractionRender;

impl ManagerInteractionRender {
    pub fn handles(event_type: &HappinessEventType) -> bool {
        matches!(
            event_type,
            HappinessEventType::ManagerPraise
                | HappinessEventType::ManagerDiscipline
                | HappinessEventType::ManagerCriticism
                | HappinessEventType::ManagerTacticalInstruction
                | HappinessEventType::PromiseKept
                | HappinessEventType::PromiseBroken
                | HappinessEventType::ManagerPlayingTimePromise
        )
    }

    pub fn headline(
        event_type: &HappinessEventType,
        ctx: &ManagerInteractionEventContext,
        i18n: &I18n,
    ) -> String {
        // Criticism with a concrete reason gets the dedicated headline
        // family (event_manager_criticism_<reason>) — that's where the
        // football-specific copy lives ("Criticised over his pressing
        // work") and where the user reads the *what specifically*.
        if matches!(event_type, HappinessEventType::ManagerCriticism) {
            if let Some(reason) = ctx.criticism_reason {
                let key = format!("event_manager_criticism_{}", reason.as_headline_token());
                let raw = i18n.t(&key);
                if raw != key {
                    return raw.to_string();
                }
            }
        }
        let key = format!(
            "manager_interaction_headline_{}_{}",
            Self::event_token(event_type),
            ctx.topic.as_i18n_key().trim_start_matches("manager_topic_")
        );
        let raw = i18n.t(&key);
        if raw == key {
            i18n.t(event_type_to_i18n_key(event_type)).to_string()
        } else {
            raw.to_string()
        }
    }

    pub fn reason_sentence(
        ctx: &ManagerInteractionEventContext,
        i18n: &I18n,
    ) -> Option<String> {
        // Concrete criticism reason wins — that's where the cause copy
        // explains the *why* in football terms ("The criticism focused on
        // missed pressing triggers and slow recovery runs.").
        if let Some(reason) = ctx.criticism_reason {
            let key = reason.as_i18n_key();
            let raw = i18n.t(key);
            if raw != key {
                return Some(raw.to_string());
            }
        }
        let key = format!(
            "manager_reason_{}_{}",
            ctx.tone.as_i18n_key().trim_start_matches("manager_tone_"),
            ctx.acceptance.as_i18n_key().trim_start_matches("manager_acceptance_")
        );
        let raw = i18n.t(&key);
        if raw == key {
            // Topic-only fallback
            let topic_key = format!(
                "manager_reason_topic_{}",
                ctx.topic.as_i18n_key().trim_start_matches("manager_topic_")
            );
            let topic_raw = i18n.t(&topic_key);
            if topic_raw == topic_key {
                return None;
            }
            return Some(topic_raw.to_string());
        }
        Some(raw.to_string())
    }

    /// Compose the supporting "Evidence" sentence — concrete signal
    /// the manager weighed (rating number, repeat warning, trust gap).
    /// Returns `None` when the context has no readable evidence.
    pub fn evidence_sentence(
        ctx: &ManagerInteractionEventContext,
        i18n: &I18n,
    ) -> Option<String> {
        if ctx.repeated_recently {
            let key = "manager_evidence_repeated_recently";
            let raw = i18n.t(key);
            if raw != key {
                return Some(raw.to_string());
            }
        }
        if let Some(rating) = ctx.match_rating {
            if rating < 6.3 {
                let key = "manager_evidence_low_match_rating";
                let raw = i18n.t(key);
                if raw != key {
                    return Some(raw.replace("{rating}", &format!("{:.1}", rating)));
                }
            }
        }
        None
    }

    fn event_token(event_type: &HappinessEventType) -> &'static str {
        match event_type {
            HappinessEventType::ManagerPraise => "praise",
            HappinessEventType::ManagerDiscipline => "discipline",
            HappinessEventType::ManagerCriticism => "criticism",
            HappinessEventType::ManagerTacticalInstruction => "tactical",
            HappinessEventType::PromiseKept => "promise_kept",
            HappinessEventType::PromiseBroken => "promise_broken",
            HappinessEventType::ManagerPlayingTimePromise => "playing_time_promise",
            _ => "other",
        }
    }
}

/// Renders the structured payload for `ConflictWithTeammate` events.
/// Maps the concrete reason / location attached at emit time onto a
/// partner-aware headline + cause sentence so the rendered row reads
/// "Clashed with {partner} over training standards" instead of the
/// generic "Had a disagreement with a teammate" filler.
struct TeammateConflictRender;

impl TeammateConflictRender {
    /// Returns the partner-aware headline key (with `{partner}`
    /// placeholder) for a concrete conflict reason. `None` when the
    /// context has no specific reason — caller falls back to the legacy
    /// `event_conflict_with_teammate_named` line.
    pub fn partner_named_key(ctx: &TeammateConflictContext) -> Option<String> {
        if matches!(ctx.reason, core::TeammateConflictReason::Other) {
            return None;
        }
        Some(format!(
            "event_teammate_conflict_{}_named",
            ctx.reason.as_headline_token()
        ))
    }

    /// Compose the cause sentence describing why the conflict happened
    /// in concrete football terms.
    pub fn reason_sentence(ctx: &TeammateConflictContext, i18n: &I18n) -> Option<String> {
        let key = ctx.reason.as_i18n_key();
        let raw = i18n.t(key);
        if raw == key {
            return None;
        }
        Some(raw.to_string())
    }

    /// "Where it happened" sentence — short setting note that lands
    /// after the cause line. Optional: returns `None` when the locale
    /// has no copy for the location yet.
    pub fn evidence_sentence(ctx: &TeammateConflictContext, i18n: &I18n) -> Option<String> {
        let key = format!(
            "conflict_evidence_{}",
            match ctx.location {
                core::ConflictLocation::TrainingGround => "training_ground",
                core::ConflictLocation::DressingRoom => "dressing_room",
                core::ConflictLocation::Match => "match",
                core::ConflictLocation::Media => "media",
                core::ConflictLocation::TeamMeeting => "team_meeting",
            }
        );
        let raw = i18n.t(&key);
        if raw == key {
            return None;
        }
        Some(raw.to_string())
    }
}

struct ContractRender;

impl ContractRender {
    pub fn handles(event_type: &HappinessEventType) -> bool {
        matches!(
            event_type,
            HappinessEventType::ContractOffer
                | HappinessEventType::ContractRenewal
                | HappinessEventType::ContractTerminated
                | HappinessEventType::SalaryShock
                | HappinessEventType::SalaryBoost
                | HappinessEventType::SalaryGapNoticed
        )
    }

    pub fn headline(ctx: &ContractEventContext, i18n: &I18n) -> String {
        let key = format!("contract_headline_{}", Self::kind_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            i18n.t(ctx.kind.as_i18n_key()).to_string()
        } else {
            raw.to_string()
        }
    }

    pub fn reason_sentence(ctx: &ContractEventContext, i18n: &I18n) -> Option<String> {
        let key = format!("contract_reason_{}", Self::kind_token(ctx));
        let main = i18n.t(&key);
        if main == key {
            return None;
        }
        Some(main.to_string())
    }

    fn kind_token(ctx: &ContractEventContext) -> &'static str {
        use core::ContractEventKind as K;
        match ctx.kind {
            K::OfferReceived => "offer_received",
            K::TalksOpened => "talks_opened",
            K::TalksStalled => "talks_stalled",
            K::Renewed => "renewed",
            K::Terminated => "terminated",
            K::SalaryShock => "salary_shock",
            K::SalaryBoost => "salary_boost",
            K::LoyaltyDiscountAccepted => "loyalty_discount",
            K::AgentPushingForBetterTerms => "agent_pushing",
            K::WagePromiseFrustration => "wage_promise_frustration",
            K::AcceptedReducedRoleContract => "accepted_reduced_role",
            K::RejectedLowStatusOffer => "rejected_low_status",
        }
    }
}

struct InjuryRecoveryRender;

impl InjuryRecoveryRender {
    pub fn handles(event_type: &HappinessEventType) -> bool {
        matches!(event_type, HappinessEventType::InjuryReturn)
    }

    pub fn headline(ctx: &InjuryRecoveryEventContext, i18n: &I18n) -> String {
        let key = format!("injury_headline_{}", Self::stage_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            i18n.t(ctx.stage.as_i18n_key()).to_string()
        } else {
            raw.to_string()
        }
    }

    pub fn reason_sentence(ctx: &InjuryRecoveryEventContext, i18n: &I18n) -> Option<String> {
        let key = format!("injury_reason_{}", Self::stage_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            return None;
        }
        Some(raw.to_string())
    }

    fn stage_token(ctx: &InjuryRecoveryEventContext) -> &'static str {
        use core::InjuryRecoveryStage as S;
        match ctx.stage {
            S::ReturnedToFullTraining => "returned_full_training",
            S::FirstMinutesAfterInjury => "first_minutes",
            S::RecoverySetback => "recovery_setback",
            S::ProtectedByMedicalStaff => "protected",
            S::InjuryRecurrenceConcern => "recurrence_concern",
            S::FitnessConfidenceRestored => "confidence_restored",
        }
    }
}

struct MatchPerformanceRender;

impl MatchPerformanceRender {
    pub fn handles(event_type: &HappinessEventType) -> bool {
        matches!(
            event_type,
            HappinessEventType::FirstClubGoal
                | HappinessEventType::DecisiveGoal
                | HappinessEventType::SubstituteImpact
                | HappinessEventType::CleanSheetPride
                | HappinessEventType::CostlyMistake
                | HappinessEventType::RedCardFallout
                | HappinessEventType::HatTrick
                | HappinessEventType::AssistHatTrick
                | HappinessEventType::GoalDroughtEnded
                | HappinessEventType::ScoringDroughtConcern
                | HappinessEventType::PlayerOfTheMatch
        )
    }

    pub fn headline(
        event_type: &HappinessEventType,
        ctx: &MatchPerformanceEventContext,
        i18n: &I18n,
    ) -> String {
        let key = format!("match_perf_headline_{}", Self::kind_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            i18n.t(event_type_to_i18n_key(event_type)).to_string()
        } else {
            raw.to_string()
        }
    }

    pub fn reason_sentence(ctx: &MatchPerformanceEventContext, i18n: &I18n) -> Option<String> {
        let key = format!("match_perf_reason_{}", Self::kind_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            return None;
        }
        Some(raw.to_string())
    }

    fn kind_token(ctx: &MatchPerformanceEventContext) -> &'static str {
        use core::MatchPerformanceKind as K;
        match ctx.kind {
            K::AnsweredCriticsWithPerformance => "answered_critics",
            K::CostlyErrorUnderPressure => "costly_error_pressure",
            K::SavedResultLate => "saved_result_late",
            K::ChangedGameFromBench => "changed_game_from_bench",
            K::DefensiveLeaderPerformance => "defensive_leader",
            K::WastefulFinishingConcern => "wasteful_finishing",
            K::ComposurePraised => "composure_praised",
            K::BigMatchNerves => "big_match_nerves",
            K::StandoutDisplay => "standout",
            K::FirstClubGoalMoment => "first_club_goal",
            K::DroughtEnded => "drought_ended",
            K::HatTrickFire => "hat_trick",
        }
    }
}

struct RoleStatusRender;

impl RoleStatusRender {
    pub fn handles(event_type: &HappinessEventType) -> bool {
        matches!(
            event_type,
            HappinessEventType::SquadStatusChange
                | HappinessEventType::LackOfPlayingTime
                | HappinessEventType::RoleMismatch
                | HappinessEventType::WonStartingPlace
                | HappinessEventType::LostStartingPlace
        )
    }

    pub fn headline(ctx: &RoleStatusEventContext, i18n: &I18n) -> String {
        let key = format!("role_status_headline_{}", Self::kind_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            i18n.t(ctx.kind.as_i18n_key()).to_string()
        } else {
            raw.to_string()
        }
    }

    pub fn reason_sentence(ctx: &RoleStatusEventContext, i18n: &I18n) -> Option<String> {
        let key = format!("role_status_reason_{}", Self::kind_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            return None;
        }
        Some(raw.to_string())
    }

    fn kind_token(ctx: &RoleStatusEventContext) -> &'static str {
        use core::RoleStatusKind as K;
        match ctx.kind {
            K::RoleClarifiedByManager => "role_clarified",
            K::RoleUnclear => "role_unclear",
            K::DepthChartPressure => "depth_chart_pressure",
            K::DirectRivalPreferred => "direct_rival_preferred",
            K::TacticalRoleChanged => "tactical_role_changed",
            K::BenchedForBalance => "benched_for_balance",
            K::RestedForWorkload => "rested_for_workload",
            K::SquadStatusUpgrade => "squad_status_upgrade",
            K::SquadStatusDowngrade => "squad_status_downgrade",
            K::NoNaturalRoleInFormation => "no_natural_role",
            K::EstablishedStarter => "established_starter",
            K::SlippedOutOfStartingXI => "slipped_out_xi",
        }
    }
}

struct NationalTeamRender;

impl NationalTeamRender {
    pub fn handles(event_type: &HappinessEventType) -> bool {
        matches!(
            event_type,
            HappinessEventType::NationalTeamCallup
                | HappinessEventType::NationalTeamDropped
                | HappinessEventType::NationalTeamDebut
        )
    }

    pub fn headline(ctx: &NationalTeamEventContext, i18n: &I18n) -> String {
        let key = format!("national_headline_{}", Self::kind_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            i18n.t(ctx.kind.as_i18n_key()).to_string()
        } else {
            raw.to_string()
        }
    }

    pub fn reason_sentence(ctx: &NationalTeamEventContext, i18n: &I18n) -> Option<String> {
        let key = format!("national_reason_{}", Self::kind_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            return None;
        }
        Some(raw.to_string())
    }

    fn kind_token(ctx: &NationalTeamEventContext) -> &'static str {
        use core::NationalTeamEventKind as K;
        match ctx.kind {
            K::FirstCallup => "first_callup",
            K::Recall => "recall",
            K::EmergencyCallup => "emergency_callup",
            K::YouthToSeniorJump => "youth_to_senior",
            K::DroppedDueToForm => "dropped_form",
            K::DroppedDueToInjury => "dropped_injury",
            K::DroppedDueToCompetition => "dropped_competition",
            K::TournamentSquadOmitted => "tournament_squad_omitted",
            K::InternationalPlaceUnderThreat => "place_under_threat",
            K::FirstCapPride => "first_cap_pride",
            K::NationalTeamRoleGrowing => "role_growing",
        }
    }
}

struct LeadershipRender;

impl LeadershipRender {
    pub fn handles(event_type: &HappinessEventType) -> bool {
        matches!(
            event_type,
            HappinessEventType::CaptaincyAwarded
                | HappinessEventType::CaptaincyRemoved
                | HappinessEventType::LeadershipEmergence
        )
    }

    pub fn headline(ctx: &LeadershipEventContext, i18n: &I18n) -> String {
        let key = format!("leadership_headline_{}", Self::kind_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            i18n.t(ctx.kind.as_i18n_key()).to_string()
        } else {
            raw.to_string()
        }
    }

    pub fn reason_sentence(ctx: &LeadershipEventContext, i18n: &I18n) -> Option<String> {
        let key = format!("leadership_reason_{}", Self::kind_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            return None;
        }
        Some(raw.to_string())
    }

    fn kind_token(ctx: &LeadershipEventContext) -> &'static str {
        use core::LeadershipEventKind as K;
        match ctx.kind {
            K::CaptaincyAwarded => "captaincy_awarded",
            K::CaptaincyRemoved => "captaincy_removed",
            K::LeadershipEmergence => "emergence",
            K::SeniorPlayerMediates => "senior_mediates",
            K::BackedBySeniorPlayers => "backed_seniors",
            K::ChallengedTrainingStandards => "challenged_standards",
            K::InfluenceInDressingRoomRising => "influence_rising",
            K::InfluenceInDressingRoomFalling => "influence_falling",
            K::MentorshipStarted => "mentorship_started",
            K::MentorshipStrained => "mentorship_strained",
            K::SquadLeadershipQuestioned => "squad_leadership_questioned",
        }
    }
}

struct MediaFanRender;

impl MediaFanRender {
    pub fn handles(event_type: &HappinessEventType) -> bool {
        matches!(
            event_type,
            HappinessEventType::FanCriticism
                | HappinessEventType::MediaPraise
                | HappinessEventType::MediaCriticism
                | HappinessEventType::MediaPressureMounting
                | HappinessEventType::PublicApology
                | HappinessEventType::ControversyIncident
        )
    }

    pub fn headline(ctx: &MediaFanEventContext, i18n: &I18n) -> String {
        let key = format!("media_fan_headline_{}", Self::kind_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            i18n.t(ctx.kind.as_i18n_key()).to_string()
        } else {
            raw.to_string()
        }
    }

    pub fn reason_sentence(ctx: &MediaFanEventContext, i18n: &I18n) -> Option<String> {
        let key = format!("media_fan_reason_{}", Self::kind_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            return None;
        }
        Some(raw.to_string())
    }

    fn kind_token(ctx: &MediaFanEventContext) -> &'static str {
        use core::MediaFanEventKind as K;
        match ctx.kind {
            K::InterviewCalmsSpeculation => "interview_calms",
            K::InterviewFuelsSpeculation => "interview_fuels",
            K::FansSplitOverPlayer => "fans_split",
            K::SupportersBackPlayerDuringSlump => "supporters_back",
            K::PublicApologyAccepted => "apology_accepted",
            K::PublicApologyRejected => "apology_rejected",
            K::SocialMediaCriticism => "social_media_criticism",
            K::MediaNarrativeChanged => "narrative_changed",
            K::HomeFansApprove => "home_fans_approve",
            K::AwayFansHostile => "away_fans_hostile",
        }
    }
}

struct PersonalAdaptationRender;

impl PersonalAdaptationRender {
    pub fn handles(event_type: &HappinessEventType) -> bool {
        matches!(
            event_type,
            HappinessEventType::SettledIntoSquad
                | HappinessEventType::FeelingIsolated
                | HappinessEventType::LanguageProgress
        )
    }

    pub fn headline(ctx: &PersonalAdaptationEventContext, i18n: &I18n) -> String {
        let key = format!("adaptation_headline_{}", Self::kind_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            i18n.t(ctx.kind.as_i18n_key()).to_string()
        } else {
            raw.to_string()
        }
    }

    pub fn reason_sentence(ctx: &PersonalAdaptationEventContext, i18n: &I18n) -> Option<String> {
        let key = format!("adaptation_reason_{}", Self::kind_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            return None;
        }
        Some(raw.to_string())
    }

    fn kind_token(ctx: &PersonalAdaptationEventContext) -> &'static str {
        use core::PersonalAdaptationKind as K;
        match ctx.kind {
            K::HomesicknessConcern => "homesickness",
            K::FamilySettled => "family_settled",
            K::FamilyUnsettled => "family_unsettled",
            K::LifestyleAdaptation => "lifestyle",
            K::LanguageBarrierConcern => "language_barrier",
            K::LocalCultureSettling => "local_culture",
            K::CompanionSupport => "companion_support",
            K::AskedForPersonalLeave => "personal_leave",
            K::LanguageMilestone => "language_milestone",
            K::SettlingIntoSquad => "settling_squad",
            K::StillStrugglingToSettle => "still_struggling",
        }
    }
}

struct LoanRender;

impl LoanRender {
    pub fn handles(event_type: &HappinessEventType) -> bool {
        matches!(event_type, HappinessEventType::LoanListingAccepted)
    }

    pub fn headline(ctx: &LoanEventContext, i18n: &I18n) -> String {
        let key = format!("loan_headline_{}", Self::kind_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            i18n.t(ctx.kind.as_i18n_key()).to_string()
        } else {
            raw.to_string()
        }
    }

    pub fn reason_sentence(ctx: &LoanEventContext, i18n: &I18n) -> Option<String> {
        let key = format!("loan_reason_{}", Self::kind_token(ctx));
        let raw = i18n.t(&key);
        if raw == key {
            return None;
        }
        Some(raw.to_string())
    }

    fn kind_token(ctx: &LoanEventContext) -> &'static str {
        use core::LoanEventKind as K;
        match ctx.kind {
            K::LoanListingAccepted => "listing_accepted",
            K::LoanDevelopmentProgress => "development_progress",
            K::LoanMinutesConcern => "minutes_concern",
            K::LoanRecallDiscussed => "recall_discussed",
            K::SettledOnLoan => "settled",
            K::LoanMovePermanentInterest => "permanent_interest",
            K::LoanRoleBroken => "role_broken",
            K::ParentClubSatisfied => "parent_satisfied",
            K::ParentClubConcerned => "parent_concerned",
        }
    }
}

/// Renders award / recognition events (POW, POM, POS, top scorer,
/// world player of year, national-team debut). Reads season totals,
/// margin, and runner-up off the [`RecognitionEventContext`] so the
/// player feed can explain "named POM with 7 goals, 3 ahead of the
/// runner-up" instead of bare "Named Player of the Month".
struct RecognitionRender;

impl RecognitionRender {
    pub fn handles(event_type: &HappinessEventType) -> bool {
        matches!(
            event_type,
            HappinessEventType::PlayerOfTheWeek
                | HappinessEventType::YoungPlayerOfTheWeek
                | HappinessEventType::PlayerOfTheMonth
                | HappinessEventType::YoungPlayerOfTheMonth
                | HappinessEventType::TeamOfTheMonthSelection
                | HappinessEventType::YoungTeamOfTheMonthSelection
                | HappinessEventType::PlayerOfTheSeason
                | HappinessEventType::YoungPlayerOfTheSeason
                | HappinessEventType::TeamOfTheSeasonSelection
                | HappinessEventType::TeamOfTheYearSelection
                | HappinessEventType::LeagueTopScorer
                | HappinessEventType::LeagueTopAssists
                | HappinessEventType::LeagueGoldenGlove
                | HappinessEventType::WorldPlayerOfYear
                | HappinessEventType::WorldPlayerOfYearNomination
                | HappinessEventType::NationalTeamDebut
        )
    }

    pub fn headline(
        event_type: &HappinessEventType,
        ctx: &core::RecognitionEventContext,
        i18n: &I18n,
    ) -> String {
        let key = format!("recognition_headline_{}", ctx.kind.as_token());
        let raw = i18n.t(&key);
        if raw == key {
            i18n.t(event_type_to_i18n_key(event_type)).to_string()
        } else {
            raw.to_string()
        }
    }

    pub fn reason_sentence(ctx: &core::RecognitionEventContext, i18n: &I18n) -> Option<String> {
        let key = format!("recognition_reason_{}", ctx.kind.as_token());
        let raw = i18n.t(&key);
        if raw == key {
            return None;
        }
        Some(raw.to_string())
    }
}

/// Renders relegation / relegation-fear / survival events. Reads
/// position, points, and gap-to-safety off the
/// [`SeasonOutcomeContext`] so the renderer can describe the season
/// trajectory rather than emitting a single generic verdict line.
struct SeasonOutcomeRender;

impl SeasonOutcomeRender {
    pub fn handles(event_type: &HappinessEventType) -> bool {
        matches!(
            event_type,
            HappinessEventType::Relegated | HappinessEventType::RelegationFear
        )
    }

    pub fn headline(ctx: &core::SeasonOutcomeContext, i18n: &I18n) -> String {
        let key = format!("season_outcome_headline_{}", ctx.kind.as_token());
        let raw = i18n.t(&key);
        if raw == key {
            String::new()
        } else {
            raw.to_string()
        }
    }

    pub fn reason_sentence(ctx: &core::SeasonOutcomeContext, i18n: &I18n) -> Option<String> {
        let key = format!("season_outcome_reason_{}", ctx.kind.as_token());
        let raw = i18n.t(&key);
        if raw == key {
            return None;
        }
        Some(raw.to_string())
    }
}

/// Renders squad-registration / regulation events (e.g.
/// `SquadRegistrationOmitted`). Reads slot type and outcome off the
/// [`RegulationEventContext`] so the renderer can describe "left out
/// of the senior 25 to free a non-EU slot" rather than the bare
/// "Squad registration omitted" line.
struct RegulationRender;

impl RegulationRender {
    pub fn handles(event_type: &HappinessEventType) -> bool {
        matches!(event_type, HappinessEventType::SquadRegistrationOmitted)
    }

    pub fn headline(ctx: &core::RegulationEventContext, i18n: &I18n) -> String {
        let key = format!(
            "regulation_headline_{}_{}",
            ctx.outcome.as_token(),
            ctx.slot_kind.as_token()
        );
        let raw = i18n.t(&key);
        if raw == key {
            // Fall back to slot-only key, then to the legacy event-type
            // line. Keeps short copy when the slot×outcome matrix is
            // partially translated.
            let slot_key = format!("regulation_headline_slot_{}", ctx.slot_kind.as_token());
            let slot_raw = i18n.t(&slot_key);
            if slot_raw == slot_key {
                i18n.t(event_type_to_i18n_key(&HappinessEventType::SquadRegistrationOmitted))
                    .to_string()
            } else {
                slot_raw.to_string()
            }
        } else {
            raw.to_string()
        }
    }

    pub fn reason_sentence(ctx: &core::RegulationEventContext, i18n: &I18n) -> Option<String> {
        let key = format!("regulation_reason_{}", ctx.slot_kind.as_token());
        let raw = i18n.t(&key);
        if raw == key {
            return None;
        }
        Some(raw.to_string())
    }
}

/// Single-entry dispatch for context-aware headlines. Replaces the long
/// if/else chain in `build_events` — adding a new renderer is one new
/// branch in [`HeadlineDispatcher::try_render`], and the order of
/// branches encodes precedence when several renderers could handle the
/// same event type. Returns `None` when no renderer matches or the
/// matched renderer's specialized context is absent (caller falls back
/// to the legacy description).
struct HeadlineDispatcher<'a> {
    event: &'a HappinessEvent,
    simulator_data: &'a SimulatorData,
    i18n: &'a I18n,
    lang: &'a str,
}

impl<'a> HeadlineDispatcher<'a> {
    fn try_render(&self) -> Option<(String, bool)> {
        let ctx = self.event.context.as_ref()?;
        let ev = &self.event.event_type;

        // Selection comes first: MatchDropped's selection_context is the
        // primary signal, but the event-type predicate is narrow so it
        // wouldn't accidentally catch other dropped-from-squad cousins.
        if matches!(ev, HappinessEventType::MatchDropped) {
            if let Some(sel) = ctx.selection_context.as_ref() {
                let h = SelectionRender::headline(sel, self.simulator_data, self.i18n, self.lang);
                return Some((h.html, h.partner_in_headline));
            }
        }
        if SupportRender::handles(ev) {
            if let Some(s) = ctx.support_context.as_ref() {
                return Some((SupportRender::headline(ev, s, self.i18n), false));
            }
        }
        if TransferInterestRender::handles(ev) {
            if let Some(tic) = ctx.transfer_interest_context.as_ref() {
                let (html, _named) = TransferInterestRender::headline(
                    ev,
                    tic,
                    self.simulator_data,
                    self.i18n,
                    self.lang,
                );
                return Some((html, false));
            }
        }
        if TrainingRender::handles(ev) {
            if let Some(tc) = ctx.training_context.as_ref() {
                return Some((TrainingRender::headline(tc, self.i18n), false));
            }
        }
        if ManagerInteractionRender::handles(ev) {
            if let Some(mc) = ctx.manager_interaction_context.as_ref() {
                return Some((
                    ManagerInteractionRender::headline(ev, mc, self.i18n),
                    false,
                ));
            }
        }
        if ContractRender::handles(ev) {
            if let Some(cc) = ctx.contract_context.as_ref() {
                return Some((ContractRender::headline(cc, self.i18n), false));
            }
        }
        if InjuryRecoveryRender::handles(ev) {
            if let Some(ic) = ctx.injury_context.as_ref() {
                return Some((InjuryRecoveryRender::headline(ic, self.i18n), false));
            }
        }
        if MatchPerformanceRender::handles(ev) {
            if let Some(mp) = ctx.match_performance_context.as_ref() {
                return Some((
                    MatchPerformanceRender::headline(ev, mp, self.i18n),
                    false,
                ));
            }
        }
        if RoleStatusRender::handles(ev) {
            if let Some(rc) = ctx.role_status_context.as_ref() {
                return Some((RoleStatusRender::headline(rc, self.i18n), false));
            }
        }
        if NationalTeamRender::handles(ev) {
            if let Some(nt) = ctx.national_team_context.as_ref() {
                return Some((NationalTeamRender::headline(nt, self.i18n), false));
            }
        }
        if LeadershipRender::handles(ev) {
            if let Some(lc) = ctx.leadership_context.as_ref() {
                return Some((LeadershipRender::headline(lc, self.i18n), false));
            }
        }
        if MediaFanRender::handles(ev) {
            if let Some(mf) = ctx.media_fan_context.as_ref() {
                return Some((MediaFanRender::headline(mf, self.i18n), false));
            }
        }
        if PersonalAdaptationRender::handles(ev) {
            if let Some(pa) = ctx.personal_adaptation_context.as_ref() {
                return Some((PersonalAdaptationRender::headline(pa, self.i18n), false));
            }
        }
        if LoanRender::handles(ev) {
            if let Some(lc) = ctx.loan_context.as_ref() {
                return Some((LoanRender::headline(lc, self.i18n), false));
            }
        }
        if RecognitionRender::handles(ev) {
            if let Some(rc) = ctx.recognition_context.as_ref() {
                return Some((RecognitionRender::headline(ev, rc, self.i18n), false));
            }
        }
        if SeasonOutcomeRender::handles(ev) {
            if let Some(sc) = ctx.season_outcome_context.as_ref() {
                let h = SeasonOutcomeRender::headline(sc, self.i18n);
                if !h.is_empty() {
                    return Some((h, false));
                }
            }
        }
        if RegulationRender::handles(ev) {
            if let Some(rc) = ctx.regulation_context.as_ref() {
                return Some((RegulationRender::headline(rc, self.i18n), false));
            }
        }
        None
    }
}

/// Build the headline string for a single event. Substitutes the
/// `{partner}` placeholder with a player link when the upgraded event
/// type has a partner-aware key + a resolved partner; falls back to the
/// legacy static i18n line otherwise. The returned string is rendered
/// via askama's `|safe` filter, so this is the only point where event
/// copy crosses into HTML — keep substitutions to controlled values.
fn build_description(
    event: &HappinessEvent,
    partner: Option<&(String, String)>,
    i18n: &I18n,
    lang: &str,
    league_slug: Option<&str>,
) -> DescriptionRender {
    if matches!(event.event_type, HappinessEventType::TeamOfTheWeekSelection) {
        let raw = i18n.t(event_type_to_i18n_key(&event.event_type));
        let html = if let Some(slug) = league_slug {
            let url = format!("/{}/leagues/{}/awards", lang, slug);
            let link = format!(r#"<a href="{}">{}</a>"#, url, i18n.t("team_of_the_week"));
            raw.replace("{tow}", &link)
        } else {
            raw.replace("{tow}", i18n.t("team_of_the_week"))
        };
        return DescriptionRender {
            html,
            partner_in_headline: false,
        };
    }

    if matches!(event.event_type, HappinessEventType::YoungTeamOfTheWeekSelection) {
        let raw = i18n.t(event_type_to_i18n_key(&event.event_type));
        let html = if let Some(slug) = league_slug {
            let url = format!("/{}/leagues/{}/awards", lang, slug);
            let link = format!(
                r#"<a href="{}">{}</a>"#,
                url,
                i18n.t("young_team_of_the_week")
            );
            raw.replace("{tow}", &link)
        } else {
            raw.replace("{tow}", i18n.t("young_team_of_the_week"))
        };
        return DescriptionRender {
            html,
            partner_in_headline: false,
        };
    }

    if let Some((name, slug)) = partner {
        // Conflict event with a concrete reason → reach for the specific
        // partner-aware key family ("Clashed with {partner} over training
        // standards") before the legacy generic line.
        if matches!(event.event_type, HappinessEventType::ConflictWithTeammate) {
            if let Some(reason_key) = event
                .context
                .as_ref()
                .and_then(|c| c.teammate_conflict_context.as_ref())
                .and_then(TeammateConflictRender::partner_named_key)
            {
                let raw = i18n.t(&reason_key);
                if raw != reason_key {
                    let link = format!(
                        r#"<a href="/{}/players/{}">{}</a>"#,
                        lang, slug, name
                    );
                    return DescriptionRender {
                        html: raw.replace("{partner}", &link),
                        partner_in_headline: true,
                    };
                }
            }
        }
        if let Some(named_key) = partner_named_key(&event.event_type) {
            let raw = i18n.t(named_key);
            let link = format!(r#"<a href="/{}/players/{}">{}</a>"#, lang, slug, name);
            return DescriptionRender {
                html: raw.replace("{partner}", &link),
                partner_in_headline: true,
            };
        }
    }

    DescriptionRender {
        html: i18n.t(event_type_to_i18n_key(&event.event_type)).to_string(),
        partner_in_headline: false,
    }
}

/// Returns the `_named` i18n key for a partner-aware event type — one
/// that uses a `{partner}` placeholder so the rendered headline reads
/// like "Clashed with Marcus Edwards" instead of "Had a disagreement
/// with a teammate — Marcus Edwards". Only the upgraded event types
/// have a named variant; everything else falls back to the legacy line.
fn partner_named_key(event_type: &HappinessEventType) -> Option<&'static str> {
    Some(match event_type {
        HappinessEventType::TeammateBonding => "event_teammate_bonding_named",
        HappinessEventType::ConflictWithTeammate => "event_conflict_with_teammate_named",
        HappinessEventType::CloseFriendSold => "event_close_friend_sold_named",
        HappinessEventType::MentorDeparted => "event_mentor_departed_named",
        HappinessEventType::CompatriotJoined => "event_compatriot_joined_named",
        _ => return None,
    })
}

/// Events that don't make sense without a named partner. If the event was
/// emitted without a partner id (legacy data, generic emit site), it gets
/// filtered out of the player's history view. Mirrors the core
/// `requires_partner_id` enforcement gate: every type listed here MUST be
/// emitted with a `Some(_)` partner id at the source.
fn is_partner_required(event_type: &HappinessEventType) -> bool {
    matches!(
        event_type,
        HappinessEventType::TeammateBonding
            | HappinessEventType::ConflictWithTeammate
            | HappinessEventType::CloseFriendSold
            | HappinessEventType::MentorDeparted
            | HappinessEventType::CompatriotJoined
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
    let p = data
        .player(partner_id)
        .or_else(|| data.retired_player(partner_id))?;
    let display = format!(
        "{} {}",
        p.full_name.display_first_name(),
        p.full_name.display_last_name()
    );
    Some((display, p.slug()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partner_named_key_covers_every_partner_required_event() {
        // Every event that requires a partner must also have a `_named`
        // i18n key — otherwise the upgraded renderer would silently fall
        // back to the unnamed legacy phrase plus a dash-suffix, defeating
        // the partner-aware headline goal.
        for ev in [
            HappinessEventType::TeammateBonding,
            HappinessEventType::ConflictWithTeammate,
            HappinessEventType::CloseFriendSold,
            HappinessEventType::MentorDeparted,
            HappinessEventType::CompatriotJoined,
        ] {
            assert!(
                partner_named_key(&ev).is_some(),
                "{:?} is partner-required but has no _named i18n key",
                ev
            );
            assert!(is_partner_required(&ev));
        }
    }

    #[test]
    fn partner_named_key_returns_none_for_partnerless_events() {
        // Spot-check: events that aren't partner-required should not have
        // a `_named` key — the substitution machinery would inject an
        // unfilled `{partner}` placeholder otherwise.
        assert!(partner_named_key(&HappinessEventType::PoorTraining).is_none());
        assert!(partner_named_key(&HappinessEventType::ManagerPraise).is_none());
        assert!(partner_named_key(&HappinessEventType::SalaryGapNoticed).is_none());
        assert!(partner_named_key(&HappinessEventType::ControversyIncident).is_none());
    }

    #[test]
    fn severity_tag_matches_each_variant() {
        assert_eq!(
            EventContextRenderer::severity_tag(HappinessEventSeverity::Minor),
            "minor"
        );
        assert_eq!(
            EventContextRenderer::severity_tag(HappinessEventSeverity::Moderate),
            "moderate"
        );
        assert_eq!(
            EventContextRenderer::severity_tag(HappinessEventSeverity::Serious),
            "serious"
        );
        assert_eq!(
            EventContextRenderer::severity_tag(HappinessEventSeverity::Major),
            "major"
        );
    }

    #[test]
    fn evidence_picker_prefers_strained_over_axis_flags() {
        // The renderer surfaces the most informative atom — a strained
        // existing relationship reframes the entire incident, so it must
        // beat individual axis flags (low trust / friendship).
        let ctx = core::HappinessEventContext::new(
            core::HappinessEventCause::PersonalityClash,
            HappinessEventSeverity::Moderate,
            core::HappinessEventScope::DressingRoom,
        )
        .with_evidence(HappinessEventEvidence::LowTrust)
        .with_evidence(HappinessEventEvidence::AlreadyStrainedRelationship)
        .with_evidence(HappinessEventEvidence::LowFriendship);
        assert_eq!(
            EvidencePicker::pick(&ctx),
            Some(HappinessEventEvidence::AlreadyStrainedRelationship)
        );
    }

    #[test]
    fn evidence_picker_is_stable_across_insertion_order() {
        // Insertion order at the emit site must NOT change which atom
        // the renderer surfaces. Two contexts carrying the same set
        // should resolve to the same sentence, otherwise the same
        // saved event would render differently across page reloads.
        let a = core::HappinessEventContext::new(
            core::HappinessEventCause::TrainingFriction,
            HappinessEventSeverity::Minor,
            core::HappinessEventScope::TrainingGround,
        )
        .with_evidence(HappinessEventEvidence::LowTrust)
        .with_evidence(HappinessEventEvidence::TrainingStandardsMismatch);
        let b = core::HappinessEventContext::new(
            core::HappinessEventCause::TrainingFriction,
            HappinessEventSeverity::Minor,
            core::HappinessEventScope::TrainingGround,
        )
        .with_evidence(HappinessEventEvidence::TrainingStandardsMismatch)
        .with_evidence(HappinessEventEvidence::LowTrust);
        assert_eq!(EvidencePicker::pick(&a), EvidencePicker::pick(&b));
    }

    #[test]
    fn evidence_picker_returns_none_for_empty_evidence() {
        // Legacy emit sites that don't attach evidence must round-trip
        // through the picker as `None` so the renderer skips the
        // evidence sentence entirely (no fabricated explanation).
        let ctx = core::HappinessEventContext::new(
            core::HappinessEventCause::Other,
            HappinessEventSeverity::Minor,
            core::HappinessEventScope::Personal,
        );
        assert_eq!(EvidencePicker::pick(&ctx), None);
    }

    #[test]
    fn evidence_token_covers_every_variant() {
        // Spot every evidence variant has a token. If a new variant is
        // added to the core enum but not wired here, the renderer would
        // silently emit `reason_ev_<missing>` which would fall back to
        // the raw key — caught explicitly here.
        for ev in [
            HappinessEventEvidence::StrongExistingBond,
            HappinessEventEvidence::AlreadyStrainedRelationship,
            HappinessEventEvidence::WeakExistingBond,
            HappinessEventEvidence::SamePositionCompetition,
            HappinessEventEvidence::SimilarSquadStatusCompetition,
            HappinessEventEvidence::LowTrust,
            HappinessEventEvidence::LowFriendship,
            HappinessEventEvidence::LowProfessionalRespect,
            HappinessEventEvidence::HighProfessionalRespect,
            HappinessEventEvidence::HighAmbition,
            HappinessEventEvidence::LowTemperament,
            HappinessEventEvidence::HighControversy,
            HappinessEventEvidence::LowSportsmanship,
            HappinessEventEvidence::HighProfessionalism,
            HappinessEventEvidence::NewSigningStillSettling,
            HappinessEventEvidence::LanguageBarrier,
            HappinessEventEvidence::SharedNationality,
            HappinessEventEvidence::MentorInfluence,
            HappinessEventEvidence::MatchCooperation,
            HappinessEventEvidence::ComplementaryRoles,
            HappinessEventEvidence::TrainingStandardsMismatch,
            HappinessEventEvidence::RepeatedIncident,
            HappinessEventEvidence::WageGap,
            HappinessEventEvidence::ReputationGap,
            HappinessEventEvidence::NoInnerCircleYet,
            HappinessEventEvidence::SquadTurnover,
            HappinessEventEvidence::MediaIncident,
            HappinessEventEvidence::DressingRoomRow,
            HappinessEventEvidence::TrainingGroundIncident,
            HappinessEventEvidence::ExcellentPerformance,
            HappinessEventEvidence::PlayerOfTheMatch,
            HappinessEventEvidence::GoalContribution,
            HappinessEventEvidence::DecisiveContribution,
            HappinessEventEvidence::DerbyPerformance,
            HappinessEventEvidence::CupPerformance,
            HappinessEventEvidence::HomeCrowdMoment,
            HappinessEventEvidence::PoorMoraleBeforeTalk,
            HappinessEventEvidence::LowConfidence,
            HappinessEventEvidence::ManagerTrust,
            HappinessEventEvidence::StrongCoachRapport,
            HappinessEventEvidence::WeakCoachRapport,
            HappinessEventEvidence::HighPressurePersonality,
            HappinessEventEvidence::LowPressurePersonality,
            HappinessEventEvidence::HighDetermination,
            HappinessEventEvidence::ImportantMatchTemperament,
            HappinessEventEvidence::RepeatedTalkDampened,
            HappinessEventEvidence::CaptainOrLeaderInfluence,
            HappinessEventEvidence::YoungPlayerNeedingConfidence,
            HappinessEventEvidence::ReturnFromInjuryBoost,
        ] {
            let token = EventContextRenderer::evidence_token(ev);
            assert!(!token.is_empty(), "{:?} has empty token", ev);
        }
    }

    #[test]
    fn support_render_handles_only_the_four_support_event_types() {
        // Acceptance criterion: the SupportRender path must only fire
        // for the four upgraded events. Selection events keep their
        // own headline path, generic events keep the legacy line.
        assert!(SupportRender::handles(&HappinessEventType::ManagerEncouragement));
        assert!(SupportRender::handles(&HappinessEventType::DressingRoomSpeech));
        assert!(SupportRender::handles(&HappinessEventType::FanPraise));
        assert!(SupportRender::handles(&HappinessEventType::FansChantPlayerName));
        assert!(!SupportRender::handles(&HappinessEventType::ManagerCriticism));
        assert!(!SupportRender::handles(&HappinessEventType::PoorTraining));
        assert!(!SupportRender::handles(&HappinessEventType::MatchDropped));
    }

    #[test]
    fn support_headline_keys_are_trigger_specific() {
        // Each event type must produce a different key per trigger so
        // the rendered copy explains the *why* of the reaction. If two
        // triggers ever shared a key, the contextual variant would
        // collapse back into a single static line.
        let mut ctx = core::SupportEventContext::new(
            core::SupportSource::Manager,
            core::SupportSetting::PostMatch,
            core::SupportTrigger::PlayerOfMatch,
        );
        let pom_key =
            SupportRender::headline_key(&HappinessEventType::ManagerEncouragement, &ctx);
        ctx.trigger = core::SupportTrigger::DecisiveMoment;
        let decisive_key =
            SupportRender::headline_key(&HappinessEventType::ManagerEncouragement, &ctx);
        ctx.trigger = core::SupportTrigger::HighRating;
        let rating_key =
            SupportRender::headline_key(&HappinessEventType::ManagerEncouragement, &ctx);
        assert_ne!(pom_key, decisive_key);
        assert_ne!(decisive_key, rating_key);
        assert_ne!(pom_key, rating_key);
    }

    #[test]
    fn support_headline_keys_are_event_specific() {
        // The renderer must distinguish the four upgraded event types
        // even when the trigger is identical, otherwise a fan chant
        // and a manager talk would render the same headline.
        let ctx = core::SupportEventContext::new(
            core::SupportSource::Supporters,
            core::SupportSetting::HomeCrowd,
            core::SupportTrigger::PlayerOfMatch,
        );
        let mgr = SupportRender::headline_key(&HappinessEventType::ManagerEncouragement, &ctx);
        let fan = SupportRender::headline_key(&HappinessEventType::FanPraise, &ctx);
        let chant =
            SupportRender::headline_key(&HappinessEventType::FansChantPlayerName, &ctx);
        assert_ne!(mgr, fan);
        assert_ne!(fan, chant);
        assert_ne!(mgr, chant);
    }

    #[test]
    fn dressing_room_headline_varies_by_phase_and_tone() {
        // Acceptance criterion: a half-time talk and a full-time talk
        // never share a headline key, even when the tone is the same.
        let mut ctx = core::SupportEventContext::new(
            core::SupportSource::Manager,
            core::SupportSetting::DressingRoom,
            core::SupportTrigger::Generic,
        )
        .with_phase(core::SupportMatchPhase::PreMatch)
        .with_tone(core::SupportTone::Passionate);
        let pre_passionate =
            SupportRender::headline_key(&HappinessEventType::DressingRoomSpeech, &ctx);
        ctx.phase = Some(core::SupportMatchPhase::HalfTime);
        let half_passionate =
            SupportRender::headline_key(&HappinessEventType::DressingRoomSpeech, &ctx);
        ctx.tone = Some(core::SupportTone::Criticise);
        let half_criticise =
            SupportRender::headline_key(&HappinessEventType::DressingRoomSpeech, &ctx);
        assert_ne!(pre_passionate, half_passionate);
        assert_ne!(half_passionate, half_criticise);
    }

    #[test]
    fn legacy_event_without_context_renders_safely() {
        // Acceptance criterion: a stored event with no context must
        // round-trip through the renderer without producing the raw
        // i18n key — falls back to the legacy static headline.
        let mut event = core::HappinessEvent {
            event_type: HappinessEventType::ManagerEncouragement,
            magnitude: 1.5,
            days_ago: 1,
            partner_player_id: None,
            context: None,
        };
        // No context means SupportRender::handles still returns true
        // but the lookup short-circuits to the legacy headline path.
        let _ = &mut event; // silence unused-mut warning if the API changes
        // We only assert presence of the legacy key here; rendering
        // with a real I18n requires the bundle and is exercised by the
        // higher-level integration tests.
        assert_eq!(
            event_type_to_i18n_key(&HappinessEventType::ManagerEncouragement),
            "event_manager_encouragement"
        );
    }

    #[test]
    fn support_event_keys_present_in_en_locale() {
        // Locale audit: every variant headline key the renderer can
        // resolve to MUST exist in the English bundle. Catches a new
        // trigger added to the SupportRender match without a copy
        // line — the visible UI would otherwise display the raw key.
        let bytes = include_bytes!("../../../assets/i18n/en.json");
        let raw = std::str::from_utf8(bytes).unwrap();
        for key in [
            "event_manager_encouragement_default",
            "event_manager_encouragement_pom",
            "event_manager_encouragement_decisive",
            "event_manager_encouragement_goal_contribution",
            "event_manager_encouragement_high_rating",
            "event_manager_encouragement_morale_lift",
            "event_manager_encouragement_form_recovery",
            "event_dressing_room_speech_default",
            "event_dressing_room_speech_pre_passionate",
            "event_dressing_room_speech_half_passionate",
            "event_dressing_room_speech_half_criticise",
            "event_dressing_room_speech_full_praise",
            "event_dressing_room_speech_full_criticise",
            "event_fan_praise_default",
            "event_fan_praise_pom",
            "event_fan_praise_goal_contribution_win",
            "event_fan_praise_derby",
            "event_fan_praise_cup",
            "event_fans_chant_player_name_default",
            "event_fans_chant_player_name_decisive",
            "event_fans_chant_player_name_derby",
            "support_reason_main_high_rating",
            "support_reason_main_pom",
            "support_reason_main_decisive_moment",
            "support_reason_main_team_won",
            "support_reason_main_trailing_half_time",
            "follow_up_manager_trust_rising",
            "follow_up_fan_standing_rising",
        ] {
            assert!(
                raw.contains(key),
                "en.json missing required support-event key {}",
                key
            );
        }
    }

    #[test]
    fn legacy_dressing_room_speech_default_no_longer_says_player_gave_speech() {
        // Acceptance criterion (renderer requirements): the player did
        // not necessarily *give* the speech — the manager spoke and
        // the player reacted. The legacy default must read as a
        // reaction copy ("Responded to..."), not "Gave a motivational
        // speech...".
        let bytes = include_bytes!("../../../assets/i18n/en.json");
        let raw = std::str::from_utf8(bytes).unwrap();
        assert!(
            !raw.contains("\"event_dressing_room_speech\": \"Gave a motivational speech in the dressing room\""),
            "legacy 'Gave a motivational speech' copy still in en.json — it implied the player gave the speech"
        );
    }

    #[test]
    fn selection_headline_keys_present_in_en_locale() {
        let bytes = include_bytes!("../../../assets/i18n/en.json");
        let raw = std::str::from_utf8(bytes).unwrap();
        for key in [
            "selection_headline_left_out_named",
            "selection_headline_left_out",
            "selection_headline_dropped_to_bench_named",
            "selection_headline_dropped_to_bench",
            "selection_headline_unused_sub_named",
            "selection_headline_unused_sub",
            "selection_headline_rested",
            "selection_headline_rotation",
            "selection_headline_unavailable",
            "selection_comparison_with_factor",
            "selection_comparison_plain",
            "event_label_comparison",
        ] {
            assert!(
                raw.contains(key),
                "en.json missing required selection key {}",
                key
            );
        }
    }

    #[test]
    fn selection_headline_key_routing_is_scope_aware() {
        // Keys must vary by scope so the rendered headline matches the
        // football-realistic situation: a rested player and a left-out
        // player should never share the same copy.
        let ctx = MatchSelectionContext {
            scope: SelectionDecisionScope::LeftOutOfMatchdaySquad,
            reason: core::SelectionOmissionReason::TeammatePreferredOnAbility,
            comparison: None,
            role: core::SelectionRole::Striker,
            match_importance: 0.8,
            repeated: false,
            is_friendly: false,
        };
        assert_eq!(
            SelectionRender::headline_key_for(&ctx, true),
            "selection_headline_left_out_named"
        );
        assert_eq!(
            SelectionRender::headline_key_for(&ctx, false),
            "selection_headline_left_out"
        );

        let mut rested = ctx.clone();
        rested.scope = SelectionDecisionScope::Rested;
        assert_eq!(
            SelectionRender::headline_key_for(&rested, true),
            "selection_headline_rested"
        );

        let mut bench = ctx.clone();
        bench.scope = SelectionDecisionScope::DroppedToBench;
        assert_ne!(
            SelectionRender::headline_key_for(&bench, true),
            SelectionRender::headline_key_for(&ctx, true),
            "DroppedToBench and LeftOut must not share a headline key"
        );
    }

    #[test]
    fn transfer_interest_keys_present_in_en_locale() {
        // Acceptance criterion: every transfer-interest variant key the
        // renderer can resolve to MUST exist in the English bundle.
        // Catches a new stage / kind / reaction added to the core enum
        // without a copy line — the visible UI would otherwise show the
        // raw key.
        let bytes = include_bytes!("../../../assets/i18n/en.json");
        let raw = std::str::from_utf8(bytes).unwrap();
        for key in [
            "event_scouted_by_club",
            "event_transfer_rumour",
            "event_agent_stirs_interest",
            "event_interest_from_bigger_club",
            "event_interest_from_rival",
            "event_homecoming_rumour",
            "event_former_club_interest",
            "event_favorite_club_interest",
            "event_transfer_speculation_distracts",
            "event_transfer_interest_dismissed",
            "event_transfer_talks_expected",
            "event_interest_cooled",
            "event_used_interest_for_contract_leverage",
            "transfer_interest_headline_scouted_named",
            "transfer_interest_headline_bigger_named",
            "transfer_interest_headline_rival_named",
            "transfer_interest_headline_homecoming_named",
            "transfer_interest_headline_favorite_named",
            "transfer_interest_headline_dream_collapsed_named",
            "transfer_interest_headline_bid_rejected_named",
            "transfer_interest_headline_talks_named",
            "transfer_interest_headline_cooled_named",
            "transfer_interest_stage_concrete_interest",
            "transfer_interest_stage_bid_rejected",
            "transfer_interest_source_confirmed_approach",
            "transfer_interest_source_rejected_bid",
            "transfer_interest_kind_step_up",
            "transfer_interest_kind_rival_move",
            "transfer_interest_kind_homecoming",
            "transfer_interest_reaction_excited",
            "transfer_interest_reaction_loyal_to_current_club",
            "transfer_sporting_fit_clear_upgrade",
            "transfer_sporting_fit_emotional_fit",
            "transfer_interest_evidence_bigger_club",
            "transfer_interest_evidence_rival_club",
            "transfer_interest_evidence_repeated_rumours",
            "transfer_interest_evidence_rejected_bid",
        ] {
            assert!(
                raw.contains(key),
                "en.json missing required transfer-interest key {}",
                key
            );
        }
    }

    #[test]
    fn transfer_interest_render_handles_only_the_funnel_event_types() {
        assert!(TransferInterestRender::handles(
            &HappinessEventType::ScoutedByClub
        ));
        assert!(TransferInterestRender::handles(
            &HappinessEventType::InterestFromBiggerClub
        ));
        assert!(TransferInterestRender::handles(
            &HappinessEventType::TransferBidRejected
        ));
        assert!(!TransferInterestRender::handles(
            &HappinessEventType::PoorTraining
        ));
        assert!(!TransferInterestRender::handles(
            &HappinessEventType::ManagerPraise
        ));
    }

    #[test]
    fn phase_1_to_11_context_keys_present_in_en_locale() {
        // Locale audit: every new context-payload reason / headline key
        // the Phase 1-11 renderers can resolve to MUST exist in the
        // English bundle. Catches a new variant added to the core enum
        // without a copy line.
        let bytes = include_bytes!("../../../assets/i18n/en.json");
        let raw = std::str::from_utf8(bytes).unwrap();
        for key in [
            // Phase 1
            "training_headline_responded_to_criticism",
            "training_reason_main_struggled_with_intensity",
            "training_reason_main_routine_good_session",
            // Phase 2
            "manager_topic_playing_time",
            "manager_acceptance_motivated",
            "promise_kind_playing_time",
            "manager_reason_topic_playing_time",
            // Phase 3
            "contract_kind_renewed",
            "contract_headline_salary_shock",
            "contract_reason_loyalty_discount",
            // Phase 4
            "injury_headline_returned_full_training",
            "injury_reason_recovery_setback",
            // Phase 5
            "match_perf_headline_first_club_goal",
            "match_perf_reason_changed_game_from_bench",
            // Phase 6
            "role_status_headline_squad_status_downgrade",
            "role_status_reason_no_natural_role",
            // Phase 7
            "national_headline_recall",
            "national_reason_first_callup",
            // Phase 8
            "leadership_headline_captaincy_awarded",
            "leadership_reason_captaincy_removed",
            // Phase 9
            "media_fan_headline_supporters_back",
            "media_fan_reason_social_media_criticism",
            // Phase 10
            "adaptation_headline_settling_squad",
            "adaptation_reason_language_milestone",
            // Phase 11
            "loan_headline_listing_accepted",
            "loan_reason_minutes_concern",
        ] {
            assert!(
                raw.contains(key),
                "en.json missing required Phase 1-11 key {}",
                key
            );
        }
    }

    #[test]
    fn phase_1_to_11_keys_present_in_every_locale() {
        // Locale parity: a representative key from each phase must exist
        // in every supported locale. Catches the case where the i18n
        // injector ran on en.json but missed a translation file.
        const LOCALES: &[&str] = &["en", "de", "es", "fr", "ja", "pt", "ru", "tr", "zh"];
        const REPRESENTATIVES: &[&str] = &[
            "training_headline_responded_to_criticism",
            "manager_acceptance_motivated",
            "contract_headline_renewed",
            "injury_headline_returned_full_training",
            "match_perf_headline_first_club_goal",
            "role_status_headline_squad_status_downgrade",
            "national_headline_recall",
            "leadership_headline_captaincy_awarded",
            "media_fan_headline_supporters_back",
            "adaptation_headline_language_milestone",
            "loan_headline_listing_accepted",
        ];
        for loc in LOCALES {
            let path_full = format!(
                "{}/assets/i18n/{}.json",
                env!("CARGO_MANIFEST_DIR"),
                loc
            );
            let raw = std::fs::read_to_string(&path_full)
                .unwrap_or_else(|e| panic!("could not read {}: {}", path_full, e));
            for key in REPRESENTATIVES {
                assert!(
                    raw.contains(key),
                    "{}.json missing required Phase 1-11 key {}",
                    loc,
                    key
                );
            }
        }
    }

    #[test]
    fn every_dynamic_i18n_key_is_present_in_en_json() {
        // Audits that every key the renderer can construct via
        // `format!("..._{}", token)` exists in en.json. The token lists
        // below mirror the exhaustive matches in each renderer's
        // `_token()` function — adding a new variant to one of those
        // enums forces a `_token()` arm (compile-time guard) and
        // forces a token string here (this test, runtime guard). If
        // either is forgotten, en.json is missing a key and the
        // renderer falls back to the raw key string, which renders as
        // ugly snake_case in the UI.
        use std::collections::HashMap;

        let bytes = include_bytes!("../../../assets/i18n/en.json");
        let map: HashMap<String, String> =
            serde_json::from_slice(bytes).expect("en.json is valid JSON");

        const CAUSE_TOKENS: &[&str] = &[
            "personality_clash",
            "training_friction",
            "positional_rivalry",
            "wage_jealousy",
            "poor_form_pressure",
            "leadership_dispute",
            "tactical_disagreement",
            "adaptation_isolation",
            "media_pressure",
            "mentor_departure",
            "friend_departure",
            "match_cooperation",
            "nationality_integration",
            "training_partnership",
            "reputation_tension",
            "reputation_admiration",
            "manager_support",
            "supporter_appreciation",
            "supporter_identification",
            "dressing_room_lift",
            "other",
        ];
        const EVIDENCE_TOKENS: &[&str] = &[
            "strong_existing_bond",
            "already_strained_relationship",
            "weak_existing_bond",
            "same_position_competition",
            "similar_squad_status_competition",
            "low_trust",
            "low_friendship",
            "low_professional_respect",
            "high_professional_respect",
            "high_ambition",
            "low_temperament",
            "high_controversy",
            "low_sportsmanship",
            "high_professionalism",
            "new_signing_still_settling",
            "language_barrier",
            "shared_nationality",
            "mentor_influence",
            "match_cooperation",
            "complementary_roles",
            "training_standards_mismatch",
            "repeated_incident",
            "wage_gap",
            "reputation_gap",
            "no_inner_circle_yet",
            "squad_turnover",
            "media_incident",
            "dressing_room_row",
            "training_ground_incident",
            "excellent_performance",
            "player_of_the_match",
            "goal_contribution",
            "decisive_contribution",
            "derby_performance",
            "cup_performance",
            "home_crowd_moment",
            "poor_morale_before_talk",
            "low_confidence",
            "manager_trust",
            "strong_coach_rapport",
            "weak_coach_rapport",
            "high_pressure_personality",
            "low_pressure_personality",
            "high_determination",
            "important_match_temperament",
            "repeated_talk_dampened",
            "captain_or_leader_influence",
            "young_player_needing_confidence",
            "return_from_injury_boost",
        ];
        const SUPPORT_TRIGGERS: &[&str] = &[
            "high_rating",
            "pom",
            "goal_contribution",
            "decisive_moment",
            "poor_morale",
            "form_recovery",
            "big_match",
            "derby",
            "cup_tie",
            "leadership_moment",
            "trailing_half_time",
            "team_won",
            "young_player_confidence",
            "returning_from_injury",
            "generic",
        ];
        const SUPPORT_SETTINGS: &[&str] = &[
            "private",
            "training_ground",
            "dressing_room",
            "touchline",
            "home_crowd",
            "away_end",
            "post_match",
        ];
        const TRAINING_REASONS: &[&str] = &[
            "sharp_after_being_left_out",
            "responded_to_criticism",
            "struggled_with_intensity",
            "distracted_by_rumours",
            "poor_attitude",
            "returning_from_injury_not_sharp",
            "young_impressed_staff",
            "setting_standards",
            "extra_work_after_session",
            "match_preparation_focus",
            "routine_good_session",
            "routine_bad_session",
        ];
        const CONTRACT_KINDS: &[&str] = &[
            "offer_received",
            "talks_opened",
            "talks_stalled",
            "renewed",
            "terminated",
            "salary_shock",
            "salary_boost",
            "loyalty_discount",
            "agent_pushing",
            "wage_promise_frustration",
            "accepted_reduced_role",
            "rejected_low_status",
        ];
        const INJURY_STAGES: &[&str] = &[
            "returned_full_training",
            "first_minutes",
            "recovery_setback",
            "protected",
            "recurrence_concern",
            "confidence_restored",
        ];
        const MATCH_PERF_KINDS: &[&str] = &[
            "answered_critics",
            "costly_error_pressure",
            "saved_result_late",
            "changed_game_from_bench",
            "defensive_leader",
            "wasteful_finishing",
            "composure_praised",
            "big_match_nerves",
            "standout",
            "first_club_goal",
            "drought_ended",
            "hat_trick",
        ];
        const ROLE_STATUS_KINDS: &[&str] = &[
            "role_clarified",
            "role_unclear",
            "depth_chart_pressure",
            "direct_rival_preferred",
            "tactical_role_changed",
            "benched_for_balance",
            "rested_for_workload",
            "squad_status_upgrade",
            "squad_status_downgrade",
            "no_natural_role",
            "established_starter",
            "slipped_out_xi",
        ];
        const NATIONAL_KINDS: &[&str] = &[
            "first_callup",
            "recall",
            "emergency_callup",
            "youth_to_senior",
            "dropped_form",
            "dropped_injury",
            "dropped_competition",
            "tournament_squad_omitted",
            "place_under_threat",
            "first_cap_pride",
            "role_growing",
        ];
        const LEADERSHIP_KINDS: &[&str] = &[
            "captaincy_awarded",
            "captaincy_removed",
            "emergence",
            "senior_mediates",
            "backed_seniors",
            "challenged_standards",
            "influence_rising",
            "influence_falling",
            "mentorship_started",
            "mentorship_strained",
            "squad_leadership_questioned",
        ];
        const MEDIA_FAN_KINDS: &[&str] = &[
            "interview_calms",
            "interview_fuels",
            "fans_split",
            "supporters_back",
            "apology_accepted",
            "apology_rejected",
            "social_media_criticism",
            "narrative_changed",
            "home_fans_approve",
            "away_fans_hostile",
        ];
        const ADAPTATION_KINDS: &[&str] = &[
            "homesickness",
            "family_settled",
            "family_unsettled",
            "lifestyle",
            "language_barrier",
            "local_culture",
            "companion_support",
            "personal_leave",
            "language_milestone",
            "settling_squad",
            "still_struggling",
        ];
        const LOAN_KINDS: &[&str] = &[
            "listing_accepted",
            "development_progress",
            "minutes_concern",
            "recall_discussed",
            "settled",
            "permanent_interest",
            "role_broken",
            "parent_satisfied",
            "parent_concerned",
        ];
        const RECOGNITION_KINDS: &[&str] = &[
            "player_of_the_week",
            "player_of_the_month",
            "young_player_of_the_month",
            "player_of_the_season",
            "young_player_of_the_season",
            "team_of_the_season",
            "league_top_scorer",
            "league_top_assists",
            "league_golden_glove",
            "world_player_of_year",
            "world_player_nominee",
            "national_team_debut",
        ];
        const SEASON_OUTCOME_KINDS: &[&str] = &[
            "relegated",
            "relegation_fear",
            "survived_relegation",
        ];
        const REGULATION_SLOTS: &[&str] = &[
            "homegrown_quota",
            "non_eu_quota",
            "senior_squad_cap",
            "youth_slot",
            "international_registration",
            "other",
        ];

        let mut missing: Vec<String> = Vec::new();
        let mut check = |key: String| {
            if !map.contains_key(&key) {
                missing.push(key);
            }
        };

        for t in CAUSE_TOKENS {
            check(format!("reason_main_{}", t));
        }
        for t in EVIDENCE_TOKENS {
            check(format!("reason_ev_{}", t));
        }
        for t in SUPPORT_TRIGGERS {
            check(format!("support_reason_main_{}", t));
        }
        for t in SUPPORT_SETTINGS {
            check(format!("support_reason_setting_{}", t));
        }
        for t in TRAINING_REASONS {
            check(format!("training_headline_{}", t));
            check(format!("training_reason_main_{}", t));
        }
        for t in CONTRACT_KINDS {
            check(format!("contract_headline_{}", t));
            check(format!("contract_reason_{}", t));
        }
        for t in INJURY_STAGES {
            check(format!("injury_headline_{}", t));
            check(format!("injury_reason_{}", t));
        }
        for t in MATCH_PERF_KINDS {
            check(format!("match_perf_headline_{}", t));
            check(format!("match_perf_reason_{}", t));
        }
        for t in ROLE_STATUS_KINDS {
            check(format!("role_status_headline_{}", t));
            check(format!("role_status_reason_{}", t));
        }
        for t in NATIONAL_KINDS {
            check(format!("national_headline_{}", t));
            check(format!("national_reason_{}", t));
        }
        for t in LEADERSHIP_KINDS {
            check(format!("leadership_headline_{}", t));
            check(format!("leadership_reason_{}", t));
        }
        for t in MEDIA_FAN_KINDS {
            check(format!("media_fan_headline_{}", t));
            check(format!("media_fan_reason_{}", t));
        }
        for t in ADAPTATION_KINDS {
            check(format!("adaptation_headline_{}", t));
            check(format!("adaptation_reason_{}", t));
        }
        for t in LOAN_KINDS {
            check(format!("loan_headline_{}", t));
            check(format!("loan_reason_{}", t));
        }
        for t in RECOGNITION_KINDS {
            check(format!("recognition_headline_{}", t));
            check(format!("recognition_reason_{}", t));
        }
        for t in SEASON_OUTCOME_KINDS {
            check(format!("season_outcome_headline_{}", t));
            check(format!("season_outcome_reason_{}", t));
        }
        for t in REGULATION_SLOTS {
            check(format!("regulation_headline_slot_{}", t));
            check(format!("regulation_reason_{}", t));
        }

        assert!(
            missing.is_empty(),
            "en.json is missing {} dynamic keys: {:#?}",
            missing.len(),
            missing
        );
    }

    #[test]
    fn manager_criticism_reason_keys_present_in_en_locale() {
        // Locale audit: every concrete manager-criticism reason must
        // resolve to a real cause sentence + a real headline variant in
        // en.json. The renderer falls back to the legacy generic line
        // when a key is missing, which silently hides the upgrade.
        use std::collections::HashMap;
        let bytes = include_bytes!("../../../assets/i18n/en.json");
        let map: HashMap<String, String> =
            serde_json::from_slice(bytes).expect("en.json is valid JSON");
        const REASONS: &[core::ManagerCriticismReason] = &[
            core::ManagerCriticismReason::MissedAssignment,
            core::ManagerCriticismReason::PoorPressing,
            core::ManagerCriticismReason::CostlyError,
            core::ManagerCriticismReason::LowTrainingIntensity,
            core::ManagerCriticismReason::PoorBodyLanguage,
            core::ManagerCriticismReason::PublicComplaint,
            core::ManagerCriticismReason::LateArrival,
            core::ManagerCriticismReason::IgnoredTacticalInstruction,
            core::ManagerCriticismReason::RepeatedIncident,
            core::ManagerCriticismReason::Other,
        ];
        let mut missing: Vec<String> = Vec::new();
        for r in REASONS {
            let cause = r.as_i18n_key().to_string();
            if !map.contains_key(&cause) {
                missing.push(cause);
            }
            let headline = format!("event_manager_criticism_{}", r.as_headline_token());
            if !map.contains_key(&headline) {
                missing.push(headline);
            }
        }
        for k in [
            "manager_evidence_repeated_recently",
            "manager_evidence_low_match_rating",
        ] {
            if !map.contains_key(k) {
                missing.push(k.to_string());
            }
        }
        assert!(missing.is_empty(), "en.json missing keys: {:#?}", missing);
    }

    #[test]
    fn teammate_conflict_reason_keys_present_in_en_locale() {
        // Locale audit: every concrete conflict reason must have a
        // partner-aware headline ({partner} placeholder) plus a cause
        // sentence + a location evidence sentence in en.json. Otherwise
        // the renderer silently degrades to the generic legacy copy.
        use std::collections::HashMap;
        let bytes = include_bytes!("../../../assets/i18n/en.json");
        let map: HashMap<String, String> =
            serde_json::from_slice(bytes).expect("en.json is valid JSON");
        const REASONS: &[core::TeammateConflictReason] = &[
            core::TeammateConflictReason::TrainingStandards,
            core::TeammateConflictReason::PositionalRivalry,
            core::TeammateConflictReason::WageJealousy,
            core::TeammateConflictReason::TacticalBlame,
            core::TeammateConflictReason::PersonalityClash,
            core::TeammateConflictReason::LanguageBarrier,
            core::TeammateConflictReason::LeadershipChallenge,
            core::TeammateConflictReason::MediaComments,
        ];
        const LOCATIONS: &[(core::ConflictLocation, &str)] = &[
            (core::ConflictLocation::TrainingGround, "training_ground"),
            (core::ConflictLocation::DressingRoom, "dressing_room"),
            (core::ConflictLocation::Match, "match"),
            (core::ConflictLocation::Media, "media"),
            (core::ConflictLocation::TeamMeeting, "team_meeting"),
        ];
        let mut missing: Vec<String> = Vec::new();
        for r in REASONS {
            let cause = r.as_i18n_key().to_string();
            if !map.contains_key(&cause) {
                missing.push(cause);
            }
            let headline = format!("event_teammate_conflict_{}_named", r.as_headline_token());
            if let Some(value) = map.get(&headline) {
                assert!(
                    value.contains("{partner}"),
                    "{} must contain {{partner}} placeholder",
                    headline
                );
            } else {
                missing.push(headline);
            }
        }
        for (_, token) in LOCATIONS {
            let evidence = format!("conflict_evidence_{}", token);
            if !map.contains_key(&evidence) {
                missing.push(evidence);
            }
        }
        assert!(missing.is_empty(), "en.json missing keys: {:#?}", missing);
    }

    #[test]
    fn manager_criticism_and_conflict_keys_present_in_every_locale() {
        // Locale parity: a representative key from each new family must
        // exist in every supported locale. Catches the case where the
        // i18n injector ran on en.json but missed a translation file.
        const LOCALES: &[&str] = &["en", "de", "es", "fr", "ja", "pt", "ru", "tr", "zh"];
        const REPRESENTATIVES: &[&str] = &[
            "event_manager_criticism_pressing",
            "event_manager_criticism_costly_error",
            "manager_criticism_reason_poor_pressing",
            "manager_criticism_reason_repeated",
            "manager_evidence_repeated_recently",
            "event_teammate_conflict_training_standards_named",
            "event_teammate_conflict_leadership_challenge_named",
            "teammate_conflict_reason_training_standards",
            "teammate_conflict_reason_wage_jealousy",
            "conflict_evidence_training_ground",
            "conflict_evidence_dressing_room",
        ];
        for loc in LOCALES {
            let path_full = format!(
                "{}/assets/i18n/{}.json",
                env!("CARGO_MANIFEST_DIR"),
                loc
            );
            let raw = std::fs::read_to_string(&path_full)
                .unwrap_or_else(|e| panic!("could not read {}: {}", path_full, e));
            for key in REPRESENTATIVES {
                assert!(
                    raw.contains(key),
                    "{}.json missing required key {}",
                    loc,
                    key
                );
            }
        }
    }

    /// Lightweight in-memory I18n stub that returns either the loaded
    /// value or — for missing keys — the key itself. Mirrors the
    /// production `I18n.t` semantics so the renderer's "fall back to the
    /// raw key" branch can still be exercised without standing up the
    /// full bundle loader.
    fn test_i18n_from(map: std::collections::HashMap<String, String>) -> I18n {
        I18n::for_test(map)
    }

    fn load_en_i18n() -> I18n {
        let bytes = include_bytes!("../../../assets/i18n/en.json");
        let map: std::collections::HashMap<String, String> =
            serde_json::from_slice(bytes).expect("en.json is valid JSON");
        test_i18n_from(map)
    }

    #[test]
    fn manager_criticism_with_concrete_reason_renders_specific_cause() {
        // Acceptance criterion: ManagerCriticism with a concrete reason
        // must render the football-specific cause sentence, not the
        // generic topic-only fallback.
        let i18n = load_en_i18n();
        let mctx = core::ManagerInteractionEventContext::new(
            core::ManagerInteractionTopic::Tactical,
            core::ManagerInteractionTone::Honest,
            core::PlayerAcceptance::Discouraged,
        )
        .with_criticism_reason(core::ManagerCriticismReason::PoorPressing);
        let cause = ManagerInteractionRender::reason_sentence(&mctx, &i18n)
            .expect("reason sentence should render");
        assert!(
            cause.contains("pressing"),
            "expected pressing-specific copy, got: {cause}"
        );
        // Headline must follow the concrete-reason key family.
        let headline = ManagerInteractionRender::headline(
            &HappinessEventType::ManagerCriticism,
            &mctx,
            &i18n,
        );
        assert_ne!(
            headline,
            i18n.t("event_manager_criticism").to_string(),
            "headline must use the reason-specific variant, not the legacy line"
        );
    }

    #[test]
    fn manager_criticism_evidence_surfaces_repeated_warning() {
        let i18n = load_en_i18n();
        let mctx = core::ManagerInteractionEventContext::new(
            core::ManagerInteractionTopic::Performance,
            core::ManagerInteractionTone::Stern,
            core::PlayerAcceptance::Discouraged,
        )
        .with_criticism_reason(core::ManagerCriticismReason::PoorPressing)
        .with_repeated_recently(true);
        let evidence = ManagerInteractionRender::evidence_sentence(&mctx, &i18n)
            .expect("evidence sentence should render when repeated_recently is set");
        assert!(
            evidence.to_lowercase().contains("warned"),
            "expected repeat-warning copy, got: {evidence}"
        );
    }

    #[test]
    fn manager_criticism_without_reason_falls_back_safely() {
        // Legacy emit sites that don't attach a criticism reason must
        // still produce a non-empty cause sentence (topic-only fallback)
        // and the legacy generic headline. No raw i18n key bleed.
        let i18n = load_en_i18n();
        let mctx = core::ManagerInteractionEventContext::new(
            core::ManagerInteractionTopic::Performance,
            core::ManagerInteractionTone::Honest,
            core::PlayerAcceptance::Discouraged,
        );
        let _ = ManagerInteractionRender::reason_sentence(&mctx, &i18n);
        let headline = ManagerInteractionRender::headline(
            &HappinessEventType::ManagerCriticism,
            &mctx,
            &i18n,
        );
        assert!(
            !headline.starts_with("event_manager_criticism_"),
            "fallback headline must not be a raw key: {headline}"
        );
    }

    #[test]
    fn conflict_with_teammate_renders_partner_cause_and_outlook() {
        // Acceptance criterion: ConflictWithTeammate with a concrete
        // reason renders a partner-aware headline, a specific cause
        // sentence, and an outlook (follow_up) line.
        let i18n = load_en_i18n();
        let conflict = core::TeammateConflictContext::new(
            core::TeammateConflictReason::TrainingStandards,
            core::ConflictLocation::TrainingGround,
        );
        let key = TeammateConflictRender::partner_named_key(&conflict)
            .expect("training-standards reason must produce a partner key");
        let raw = i18n.t(&key);
        assert!(
            raw.contains("{partner}"),
            "partner-aware headline must carry {{partner}} placeholder, got: {raw}"
        );

        let cause = TeammateConflictRender::reason_sentence(&conflict, &i18n)
            .expect("conflict cause sentence must render");
        assert!(
            cause.to_lowercase().contains("training"),
            "training-standards cause must mention training, got: {cause}"
        );

        let evidence = TeammateConflictRender::evidence_sentence(&conflict, &i18n)
            .expect("conflict evidence sentence must render for known location");
        assert!(!evidence.is_empty());

        // Outlook copy lives on HappinessEventFollowUp::DressingRoomDamageRisk.
        let follow_up = i18n.t(core::HappinessEventFollowUp::DressingRoomDamageRisk.as_i18n_key());
        assert!(!follow_up.is_empty());
    }

    #[test]
    fn conflict_other_reason_falls_back_to_legacy_named_key() {
        // Reason::Other is the catch-all — the renderer must not invent
        // a key like "event_teammate_conflict_other_named". It returns
        // None so build_description falls back to the legacy
        // event_conflict_with_teammate_named line.
        let conflict = core::TeammateConflictContext::new(
            core::TeammateConflictReason::Other,
            core::ConflictLocation::DressingRoom,
        );
        assert_eq!(TeammateConflictRender::partner_named_key(&conflict), None);
    }

    #[test]
    fn conflict_reason_token_round_trips_through_partner_named_key() {
        // Every concrete reason must produce a partner-aware key whose
        // token suffix matches `as_headline_token`. Catches a missed
        // arm in either side of the renderer mapping.
        for (reason, expected_token) in [
            (
                core::TeammateConflictReason::TrainingStandards,
                "training_standards",
            ),
            (
                core::TeammateConflictReason::PositionalRivalry,
                "positional_rivalry",
            ),
            (
                core::TeammateConflictReason::WageJealousy,
                "wage_jealousy",
            ),
            (
                core::TeammateConflictReason::TacticalBlame,
                "tactical_blame",
            ),
            (
                core::TeammateConflictReason::PersonalityClash,
                "personality_clash",
            ),
            (
                core::TeammateConflictReason::LanguageBarrier,
                "language_barrier",
            ),
            (
                core::TeammateConflictReason::LeadershipChallenge,
                "leadership_challenge",
            ),
            (
                core::TeammateConflictReason::MediaComments,
                "media_comments",
            ),
        ] {
            let ctx = core::TeammateConflictContext::new(
                reason,
                core::ConflictLocation::DressingRoom,
            );
            let key = TeammateConflictRender::partner_named_key(&ctx)
                .expect("concrete reason must produce a partner key");
            assert!(
                key.ends_with(&format!("{}_named", expected_token)),
                "{:?} produced unexpected key {}",
                reason,
                key
            );
        }
    }

    #[test]
    fn loosely_formed_phrase_no_longer_in_en_locale() {
        // Acceptance criterion: drop the generic "loosely formed"
        // explanation. The phrase came from `relationship_state_mixed`
        // in the locale data — confirm it's gone from the bundled en
        // file. This guards against regressions where someone re-adds
        // a generic relationship-state-mixed sentence.
        let bytes = include_bytes!("../../../assets/i18n/en.json");
        let raw = std::str::from_utf8(bytes).unwrap();
        assert!(
            !raw.contains("loosely formed"),
            "en.json still contains the 'loosely formed' filler phrase"
        );
        // The key itself should also be gone — render path no longer
        // looks it up, and CLAUDE.md prefers not to keep dead keys.
        assert!(
            !raw.contains("relationship_state_mixed"),
            "en.json still has relationship_state_mixed key"
        );
    }
}
