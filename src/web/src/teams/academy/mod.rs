pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::potential_stars::PotentialStarsView;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::PlayerPositionType;
use core::SimulatorData;
use core::club::academy::{
    AcademyDevelopmentIdentity, AcademyPlayerPhase, AcademyReadinessScorer, AcademyTier,
};
use core::utils::DateUtils;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamAcademyRequest {
    lang: String,
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/academy/index.html")]
#[allow(dead_code)]
pub struct TeamAcademyTemplate {
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
    pub academy_level: u8,
    pub academy_tier: u8,
    pub pathway_reputation: u8,
    /// i18n key for the academy identity label — translated by the template.
    pub identity_key: &'static str,
    /// i18n keys for the recruitment-priority position groups —
    /// translated by the template.
    pub recruitment_priority_keys: Vec<&'static str>,
    pub players: Vec<AcademyPlayer>,
    /// Headline pipeline counts for the academy header.
    pub foundation_count: usize,
    pub development_count: usize,
    pub professional_count: usize,
    pub ready_for_youth_count: usize,
    pub at_risk_count: usize,
    /// 0..100 readiness threshold used to colour-band the per-player bar.
    pub readiness_threshold: i16,
}

pub struct AcademyPlayer {
    pub _id: u32,
    pub first_name: String,
    pub last_name: String,
    pub position: String,
    #[allow(dead_code)]
    pub position_sort: PlayerPositionType,
    pub country_slug: String,
    pub country_code: String,
    pub country_name: String,
    pub age: u8,
    pub current_ability: u8,
    pub potential_ability: u8,
    pub potential_ability_raw: u8,
    pub conditions: u8,
    /// i18n key for the phase — translated by the template.
    pub phase_key: &'static str,
    pub phase_sort: u8,
    pub readiness: i16,
    pub risk_low_condition: bool,
    pub risk_jaded: bool,
    pub risk_injury_prone: bool,
}

pub async fn team_academy_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamAcademyRequest>,
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

    // Academy tab only available for Main and U18 teams
    if team.team_type != core::TeamType::Main && team.team_type != core::TeamType::U18 {
        return Err(ApiError::NotFound(
            "Academy not available for this team type".to_string(),
        ));
    }

    let league = team.league_id.and_then(|id| simulator_data.league(id));
    let now = simulator_data.date.date();

    let club = simulator_data.club(team.club_id).ok_or_else(|| {
        ApiError::InternalError(format!("Club with ID {} not found", team.club_id))
    })?;

    let head_coach = team.staffs.head_coach();

    let scorer = AcademyReadinessScorer::new(
        club.academy.pathway_reputation,
        &club.academy.pathway_policy,
    );

    // Get academy players directly from club academy
    let mut players: Vec<AcademyPlayer> = club
        .academy
        .players
        .players
        .iter()
        .filter_map(|p| {
            let country = simulator_data.country(p.country_id)?;
            let position = p.positions.display_positions_compact();
            let age = DateUtils::age(p.birth_date, now);
            let phase = AcademyPlayerPhase::from_age(age);
            let readiness = scorer.score(p, now);

            Some(AcademyPlayer {
                _id: p.id,
                first_name: p.full_name.display_first_name().to_string(),
                last_name: p.full_name.display_last_name().to_string(),
                position,
                position_sort: p.position(),
                country_slug: country.slug.clone(),
                country_code: country.code.clone(),
                country_name: country.name.clone(),
                age,
                current_ability: PotentialStarsView::current(p),
                potential_ability: PotentialStarsView::potential_by_staff(p, head_coach),
                potential_ability_raw: p.player_attributes.potential_ability,
                conditions: (100f32 * (p.player_attributes.condition as f32 / 10000.0)) as u8,
                phase_key: phase_i18n_key(phase),
                phase_sort: phase.index(),
                readiness,
                risk_low_condition: p.player_attributes.condition < 5500,
                risk_jaded: p.player_attributes.jadedness > 5500,
                risk_injury_prone: p.player_attributes.injury_proneness >= 17,
            })
        })
        .collect();

    // Default sort: phase ASC, readiness DESC, raw PA DESC.
    players.sort_by(|a, b| {
        a.phase_sort
            .cmp(&b.phase_sort)
            .then_with(|| b.readiness.cmp(&a.readiness))
            .then_with(|| b.potential_ability_raw.cmp(&a.potential_ability_raw))
    });

    let readiness_threshold = club.academy.pathway_policy.readiness_threshold;
    let mut foundation_count = 0usize;
    let mut development_count = 0usize;
    let mut professional_count = 0usize;
    let mut ready_for_youth_count = 0usize;
    let mut at_risk_count = 0usize;
    for p in &players {
        match p.phase_sort {
            0 => foundation_count += 1,
            1 => development_count += 1,
            _ => professional_count += 1,
        }
        if p.readiness >= readiness_threshold {
            ready_for_youth_count += 1;
        }
        if p.risk_low_condition || p.risk_jaded || p.risk_injury_prone {
            at_risk_count += 1;
        }
    }

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
    let current_path = format!("/{}/teams/{}/academy", &route_params.lang, &team.slug);
    let menu_params = views::MenuParams {
        i18n: &i18n,
        lang: &route_params.lang,
        current_path: &current_path,
        country_name: cn,
        country_slug: cs,
    };
    let menu_sections =
        views::team_menu(&menu_params, &neighbor_refs, &league_refs);

    let title = team.name.clone();
    let league_title = league
        .map(|l| views::league_display_name(l, &i18n, simulator_data))
        .unwrap_or_default();

    let identity_key = identity_i18n_key(club.academy.development_identity);
    let recruitment_priority_keys = club
        .academy
        .recruitment_priorities
        .iter()
        .map(|g| position_group_i18n_key(*g))
        .collect();

    Ok(TeamAcademyTemplate {
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
        header_color: club.colors.background.clone(),
        foreground_color: club.colors.foreground.clone(),
        menu_sections,
        team_slug: team.slug.clone(),
        active_tab: "academy",
        show_finances_tab: team.team_type.is_own_team(),
        show_academy_tab: true,
        academy_level: club.academy.level(),
        academy_tier: AcademyTier::from_level(club.academy.level()).value(),
        pathway_reputation: club.academy.pathway_reputation,
        identity_key,
        recruitment_priority_keys,
        players,
        foundation_count,
        development_count,
        professional_count,
        ready_for_youth_count,
        at_risk_count,
        readiness_threshold,
    })
}

fn identity_i18n_key(identity: AcademyDevelopmentIdentity) -> &'static str {
    match identity {
        AcademyDevelopmentIdentity::Balanced => "academy_identity_balanced",
        AcademyDevelopmentIdentity::TechnicalSchool => "academy_identity_technical",
        AcademyDevelopmentIdentity::TacticalSchool => "academy_identity_tactical",
        AcademyDevelopmentIdentity::AthleticDevelopment => "academy_identity_athletic",
        AcademyDevelopmentIdentity::PlayerTrading => "academy_identity_player_trading",
    }
}

fn position_group_i18n_key(group: core::PlayerFieldPositionGroup) -> &'static str {
    match group {
        core::PlayerFieldPositionGroup::Goalkeeper => "position_group_gk",
        core::PlayerFieldPositionGroup::Defender => "position_group_df",
        core::PlayerFieldPositionGroup::Midfielder => "position_group_mf",
        core::PlayerFieldPositionGroup::Forward => "position_group_fw",
    }
}

fn phase_i18n_key(phase: AcademyPlayerPhase) -> &'static str {
    match phase {
        AcademyPlayerPhase::Foundation => "academy_phase_foundation",
        AcademyPlayerPhase::Development => "academy_phase_development",
        AcademyPlayerPhase::Professional => "academy_phase_professional",
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
