pub mod routes;

use crate::leagues::address::LeagueAddress;
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::shared::potential_stars::{PotentialStarsView, StarRating};
use crate::teams::newspaper::NewspaperCounter;
use crate::teams::{TeamLookup, TeamNeighborhood};
use crate::views::{self, MenuSection, SubTitleFlag};
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
    pub sub_title_flag: Option<SubTitleFlag>,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub active_tab: &'static str,
    pub show_finances_tab: bool,
    pub show_academy_tab: bool,
    /// Printed items waiting on the newspaper tab, for the tabbar badge.
    pub newspaper_count: usize,
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
    pub current_ability: StarRating,
    pub potential_ability: StarRating,
    /// Assessed-ceiling sort key on the full 1..200 scale (finer than
    /// the 0..5 star band above). Staff belief, never the hidden PA.
    pub potential_sort: u8,
    pub conditions: u8,
    /// i18n key for the phase — translated by the template.
    pub phase_key: &'static str,
    pub phase_sort: u8,
    pub readiness: i16,
    pub risk_low_condition: bool,
    pub risk_jaded: bool,
    pub risk_injury_prone: bool,
}

pub struct TeamAcademyPage;

impl TeamAcademyPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<TeamAcademyRequest>,
    ) -> ApiResult<impl IntoResponse> {
        let guard = state.data.read().await;

        let simulator_data = guard
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let i18n = state.i18n.for_lang(&route_params.lang);

        let team = TeamLookup::by_slug(simulator_data, &route_params.team_slug)?;

        // Academy tab only available for Main and U18 teams
        if team.team_type != core::TeamType::Main && team.team_type != core::TeamType::U18 {
            return Err(ApiError::NotFound(
                "Academy not available for this team type".to_string(),
            ));
        }

        let club = simulator_data.club(team.club_id).ok_or_else(|| {
            ApiError::InternalError(format!("Club with ID {} not found", team.club_id))
        })?;

        Self::template(simulator_data, team, club, i18n, &route_params)
    }

    fn template(
        data: &SimulatorData,
        team: &core::Team,
        club: &core::Club,
        i18n: I18n,
        route_params: &TeamAcademyRequest,
    ) -> Result<TeamAcademyTemplate, ApiError> {
        let now = data.date.date();
        let scorer = AcademyReadinessScorer::new(
            club.academy.pathway_reputation,
            &club.academy.pathway_policy,
        );
        let players = Self::players(data, team, club, now, &scorer);
        let counts = AcademyPipelineCounts::of(&players, club, now);

        let league = team.league_id.and_then(|id| data.league(id));
        let current_path = format!("/{}/teams/{}/academy", route_params.lang, team.slug);
        let neighborhood = TeamNeighborhood::for_club(team.club_id, data, &i18n)?;
        let menu_sections = neighborhood.menu_sections(
            data,
            team.club_id,
            &i18n,
            &route_params.lang,
            &current_path,
        );

        let title = team.name.clone();
        let league_title = league
            .map(|l| views::Menus::league_display_name(l, &i18n, data))
            .unwrap_or_default();
        let identity_key = AcademyLabels::identity(club.academy.development_identity);
        let recruitment_priority_keys = club
            .academy
            .recruitment_priorities
            .iter()
            .map(|g| AcademyLabels::position_group(*g))
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
                .map(|l| LeagueAddress::of(data, l).url(&route_params.lang))
                .unwrap_or_default(),
            sub_title_flag: None,
            header_color: club.colors.background.clone(),
            foreground_color: club.colors.foreground.clone(),
            menu_sections,
            team_slug: team.slug.clone(),
            active_tab: "academy",
            show_finances_tab: team.team_type.is_own_team(),
            show_academy_tab: true,
            newspaper_count: NewspaperCounter::count(data, team),
            academy_level: club.academy.level(),
            academy_tier: AcademyTier::from_level(club.academy.level()).value(),
            pathway_reputation: club.academy.pathway_reputation,
            identity_key,
            recruitment_priority_keys,
            players,
            foundation_count: counts.foundation,
            development_count: counts.development,
            professional_count: counts.professional,
            ready_for_youth_count: counts.ready_for_youth,
            at_risk_count: counts.at_risk,
            readiness_threshold: club.academy.pathway_policy.readiness_threshold,
        })
    }

    fn players(
        data: &SimulatorData,
        team: &core::Team,
        club: &core::Club,
        now: chrono::NaiveDate,
        scorer: &AcademyReadinessScorer,
    ) -> Vec<AcademyPlayer> {
        let head_coach = team.staffs.head_coach();
        let mut players: Vec<AcademyPlayer> = club
            .academy
            .players
            .players
            .iter()
            .filter_map(|p| {
                let country = data.country(p.country_id)?;
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
                    potential_ability: PotentialStarsView::potential_by_staff(
                        p, head_coach, false, now,
                    ),
                    potential_sort: PotentialStarsView::potential_value_by_staff(
                        p, head_coach, false, now,
                    ),
                    conditions: (100f32 * (p.player_attributes.condition as f32 / 10000.0)) as u8,
                    phase_key: AcademyLabels::phase(phase),
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
                .then_with(|| b.potential_sort.cmp(&a.potential_sort))
        });

        players
    }
}

struct AcademyPipelineCounts {
    foundation: usize,
    development: usize,
    professional: usize,
    ready_for_youth: usize,
    at_risk: usize,
}

impl AcademyPipelineCounts {
    fn of(players: &[AcademyPlayer], club: &core::Club, now: chrono::NaiveDate) -> Self {
        // "Ready for youth" counts players *eligible to enter the youth-team
        // pathway* this season (old enough, fit, not exhausted) — not just
        // high-CA prospects. Quality only ranks them; it doesn't gate the
        // count. Sourced from the academy's canonical eligibility method.
        let ready_for_youth = club.academy.graduation_candidates(now).len();
        let mut foundation = 0usize;
        let mut development = 0usize;
        let mut professional = 0usize;
        let mut at_risk = 0usize;
        for p in players {
            match p.phase_sort {
                0 => foundation += 1,
                1 => development += 1,
                _ => professional += 1,
            }
            if p.risk_low_condition || p.risk_jaded || p.risk_injury_prone {
                at_risk += 1;
            }
        }

        Self {
            foundation,
            development,
            professional,
            ready_for_youth,
            at_risk,
        }
    }
}

struct AcademyLabels;

impl AcademyLabels {
    fn identity(identity: AcademyDevelopmentIdentity) -> &'static str {
        match identity {
            AcademyDevelopmentIdentity::Balanced => "academy_identity_balanced",
            AcademyDevelopmentIdentity::TechnicalSchool => "academy_identity_technical",
            AcademyDevelopmentIdentity::TacticalSchool => "academy_identity_tactical",
            AcademyDevelopmentIdentity::AthleticDevelopment => "academy_identity_athletic",
            AcademyDevelopmentIdentity::PlayerTrading => "academy_identity_player_trading",
        }
    }

    fn position_group(group: core::PlayerFieldPositionGroup) -> &'static str {
        match group {
            core::PlayerFieldPositionGroup::Goalkeeper => "position_group_gk",
            core::PlayerFieldPositionGroup::Defender => "position_group_df",
            core::PlayerFieldPositionGroup::Midfielder => "position_group_mf",
            core::PlayerFieldPositionGroup::Forward => "position_group_fw",
        }
    }

    fn phase(phase: AcademyPlayerPhase) -> &'static str {
        match phase {
            AcademyPlayerPhase::Foundation => "academy_phase_foundation",
            AcademyPlayerPhase::Development => "academy_phase_development",
            AcademyPlayerPhase::Professional => "academy_phase_professional",
        }
    }
}
