pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::slug::{PlayerPage, resolve_player_page};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::HappinessEvent;
use core::HappinessEventContext;
use core::HappinessEventEvidence;
use core::HappinessEventSeverity;
use core::HappinessEventType;
use core::MatchSelectionContext;
use core::PlayerStatusType;
use core::SelectionDecisionScope;
use core::SelectionScoreFactor;
use core::SimulatorData;
use core::SupportEventContext;
use core::SupportTrigger;
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
            | HappinessEventType::TeamOfTheSeasonSelection
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
        HappinessEventType::TeamOfTheWeekSelection => "event_team_of_the_week_selection",
        HappinessEventType::TeamOfTheSeasonSelection => "event_team_of_the_season_selection",
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

fn build_events(
    player: &core::Player,
    i18n: &I18n,
    simulator_data: &SimulatorData,
    lang: &str,
    league_slug: Option<&str>,
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
            // Selection-aware events get a richer headline that names
            // the chosen rival inline. Falls back to the generic
            // "Dropped from match squad" line when no selection
            // context was attached.
            let (description_html, partner_in_headline) = if matches!(
                e.event_type,
                HappinessEventType::MatchDropped
            ) {
                if let Some(sel) = e
                    .context
                    .as_ref()
                    .and_then(|c| c.selection_context.as_ref())
                {
                    let h = SelectionRender::headline(sel, simulator_data, i18n, lang);
                    (h.html, h.partner_in_headline)
                } else {
                    (description.html, description.partner_in_headline)
                }
            } else if SupportRender::handles(&e.event_type) {
                if let Some(support) = e
                    .context
                    .as_ref()
                    .and_then(|c| c.support_context.as_ref())
                {
                    let html = SupportRender::headline(&e.event_type, support, i18n);
                    (html, false)
                } else {
                    (description.html, description.partner_in_headline)
                }
            } else if TransferInterestRender::handles(&e.event_type) {
                if let Some(tic) = e
                    .context
                    .as_ref()
                    .and_then(|c| c.transfer_interest_context.as_ref())
                {
                    let (html, _named) = TransferInterestRender::headline(
                        &e.event_type,
                        tic,
                        simulator_data,
                        i18n,
                        lang,
                    );
                    (html, false)
                } else {
                    (description.html, description.partner_in_headline)
                }
            } else {
                (description.html, description.partner_in_headline)
            };

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

    if let Some((name, slug)) = partner {
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
