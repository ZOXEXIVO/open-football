pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::utils::{FormattingUtils};
use core::{CoachingStyle, SimulatorData, Staff, StaffEventType, StaffLicenseType, StaffPosition};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct StaffPersonalRequest {
    pub lang: String,
    pub staff_id: u32,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "staff/personal/index.html")]
pub struct StaffPersonalTemplate {
    pub css_version: &'static str,
    pub hostname: &'static str,
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
    pub staff_id: u32,
    pub personality: PersonalityDto,
    pub staff_info: StaffInfoDto,
    pub performance: PerformanceDto,
    pub recent_events: Vec<StaffRecentEventDto>,
}

pub struct PersonalityDto {
    pub radar_points: String,
    pub radar_grid_4: String,
    pub radar_grid_3: String,
    pub radar_grid_2: String,
    pub radar_grid_1: String,
    pub radar_axes: Vec<RadarAxisDto>,
    pub radar_items: Vec<RadarLabelDto>,
}

pub struct RadarAxisDto {
    pub x2: f32,
    pub y2: f32,
}

pub struct RadarLabelDto {
    pub name: String,
    pub value: u8,
    pub x: f32,
    pub y: f32,
    pub anchor: String,
}

pub struct StaffInfoDto {
    pub behaviour: String,
    pub coaching_style: String,
    pub license: String,
    pub fatigue: u8,
    pub job_satisfaction: u8,
    pub determination: u8,
    pub man_management: u8,
    pub motivating: u8,
    pub discipline: u8,
    pub salary: String,
    pub contract_expiry: String,
    pub role: String,
}

pub struct PerformanceDto {
    pub training_effectiveness: u8,
    pub player_development: u8,
    pub injury_prevention: u8,
    pub tactical_implementation: u8,
}

pub struct StaffRecentEventDto {
    pub description: String,
    pub is_positive: bool,
    pub days_ago: u16,
}

pub async fn staff_personal_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<StaffPersonalRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let (staff, team) = simulator_data
        .staff_with_team(route_params.staff_id)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Staff with ID {} not found", route_params.staff_id))
        })?;

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

    let title = format!(
        "{} {}",
        staff.full_name.first_name, staff.full_name.last_name
    );

    let role_key = staff
        .contract
        .as_ref()
        .map(|c| position_to_i18n_key(&c.position).to_string())
        .unwrap_or_else(|| "staff_free".to_string());

    let personality = get_personality(staff);
    let staff_info = get_staff_info(staff, &i18n);
    let performance = get_performance(staff);
    let recent_events = get_recent_events(staff, &i18n);

    Ok(StaffPersonalTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        hostname: &crate::common::default_handler::HOSTNAME,
        title,
        sub_title_prefix: i18n.t(&role_key).to_string(),
        sub_title_suffix: if team.team_type == core::TeamType::Main {
            String::new()
        } else {
            i18n.t(team.team_type.as_i18n_key()).to_string()
        },
        sub_title: team.name.clone(),
        sub_title_link: format!("/{}/teams/{}", &route_params.lang, &team.slug),
        sub_title_country_code: String::new(),
        header_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.background.clone())
            .unwrap_or_default(),
        foreground_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.foreground.clone())
            .unwrap_or_default(),
        menu_sections: {
            let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
            let current_path = format!("/{}/teams/{}", &route_params.lang, &team.slug);
            let mp = views::MenuParams { i18n: &i18n, lang: &route_params.lang, current_path: &current_path, country_name: cn, country_slug: cs };
            views::team_menu(&mp, &neighbor_refs, &team.slug, &league_refs, team.team_type == core::TeamType::Main)
        },
        i18n,
        lang: route_params.lang.clone(),
        staff_id: staff.id,
        personality,
        staff_info,
        performance,
        recent_events,
    })
}

fn get_personality(staff: &Staff) -> PersonalityDto {
    let attrs = &staff.attributes;

    let values: [u8; 8] = [
        attrs.adaptability.round().clamp(1.0, 20.0) as u8,
        attrs.ambition.round().clamp(1.0, 20.0) as u8,
        attrs.controversy.round().clamp(1.0, 20.0) as u8,
        attrs.loyalty.round().clamp(1.0, 20.0) as u8,
        attrs.pressure.round().clamp(1.0, 20.0) as u8,
        attrs.professionalism.round().clamp(1.0, 20.0) as u8,
        attrs.sportsmanship.round().clamp(1.0, 20.0) as u8,
        attrs.temperament.round().clamp(1.0, 20.0) as u8,
    ];
    let names = [
        "adaptability",
        "ambition",
        "controversy",
        "loyalty",
        "pressure",
        "professionalism",
        "sportsmanship",
        "temperament",
    ];

    let cx: f32 = 200.0;
    let cy: f32 = 140.0;
    let max_r: f32 = 75.0;
    let label_r: f32 = 90.0;
    let n = values.len();

    let angle_at = |i: usize| -> f32 {
        std::f32::consts::PI * 2.0 * (i as f32) / (n as f32) - std::f32::consts::FRAC_PI_2
    };

    let grid_polygon = |radius: f32| -> String {
        (0..n)
            .map(|i| {
                let a = angle_at(i);
                format!("{:.1},{:.1}", cx + radius * a.cos(), cy + radius * a.sin())
            })
            .collect::<Vec<_>>()
            .join(" ")
    };

    let mut data_points = Vec::new();
    let mut radar_axes = Vec::new();
    let mut radar_items = Vec::new();

    for i in 0..n {
        let angle = angle_at(i);
        let ratio = values[i] as f32 / 20.0;
        data_points.push(format!(
            "{:.1},{:.1}",
            cx + max_r * ratio * angle.cos(),
            cy + max_r * ratio * angle.sin()
        ));

        radar_axes.push(RadarAxisDto {
            x2: cx + max_r * angle.cos(),
            y2: cy + max_r * angle.sin(),
        });

        let lx = cx + label_r * angle.cos();
        let ly = cy + label_r * angle.sin();
        let anchor = if angle.cos().abs() < 0.01 {
            "middle"
        } else if angle.cos() > 0.0 {
            "start"
        } else {
            "end"
        };

        radar_items.push(RadarLabelDto {
            name: names[i].to_string(),
            value: values[i],
            x: lx,
            y: ly,
            anchor: anchor.to_string(),
        });
    }

    PersonalityDto {
        radar_points: data_points.join(" "),
        radar_grid_4: grid_polygon(max_r),
        radar_grid_3: grid_polygon(max_r * 0.75),
        radar_grid_2: grid_polygon(max_r * 0.5),
        radar_grid_1: grid_polygon(max_r * 0.25),
        radar_axes,
        radar_items,
    }
}

fn get_staff_info(staff: &Staff, i18n: &crate::I18n) -> StaffInfoDto {
    let behaviour = i18n
        .t(&format!(
            "behaviour_{}",
            staff.behaviour.as_str().to_lowercase()
        ))
        .to_string();

    let coaching_style = match staff.coaching_style {
        CoachingStyle::Authoritarian => i18n.t("style_authoritarian"),
        CoachingStyle::Democratic => i18n.t("style_democratic"),
        CoachingStyle::LaissezFaire => i18n.t("style_laissez_faire"),
        CoachingStyle::Transformational => i18n.t("style_transformational"),
        CoachingStyle::Tactical => i18n.t("style_tactical"),
    }
    .to_string();

    let license = match staff.license {
        StaffLicenseType::ContinentalPro => i18n.t("license_continental_pro"),
        StaffLicenseType::ContinentalA => i18n.t("license_continental_a"),
        StaffLicenseType::ContinentalB => i18n.t("license_continental_b"),
        StaffLicenseType::ContinentalC => i18n.t("license_continental_c"),
        StaffLicenseType::NationalA => i18n.t("license_national_a"),
        StaffLicenseType::NationalB => i18n.t("license_national_b"),
        StaffLicenseType::NationalC => i18n.t("license_national_c"),
    }
    .to_string();

    let fatigue = staff.fatigue.round().clamp(0.0, 100.0) as u8;
    let job_satisfaction = staff.job_satisfaction.round().clamp(0.0, 100.0) as u8;

    let mental = &staff.staff_attributes.mental;
    let determination = mental.determination;
    let man_management = mental.man_management;
    let motivating = mental.motivating;
    let discipline = mental.discipline;

    let (salary, contract_expiry, role) = if let Some(contract) = &staff.contract {
        let wage = format!(
            "{} {}",
            FormattingUtils::format_money(contract.salary as f64),
            i18n.t("per_week")
        );
        let expiry = contract.expired.format("%d.%m.%Y").to_string();
        let role = i18n.t(position_to_i18n_key(&contract.position)).to_string();
        (wage, expiry, role)
    } else {
        (String::new(), String::new(), String::new())
    };

    StaffInfoDto {
        behaviour,
        coaching_style,
        license,
        fatigue,
        job_satisfaction,
        determination,
        man_management,
        motivating,
        discipline,
        salary,
        contract_expiry,
        role,
    }
}

fn get_performance(staff: &Staff) -> PerformanceDto {
    let perf = &staff.recent_performance;
    PerformanceDto {
        training_effectiveness: (perf.training_effectiveness * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8,
        player_development: (perf.player_development_rate * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8,
        injury_prevention: (perf.injury_prevention_rate * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8,
        tactical_implementation: (perf.tactical_implementation * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8,
    }
}

fn get_recent_events(staff: &Staff, i18n: &crate::I18n) -> Vec<StaffRecentEventDto> {
    let mut events: Vec<_> = staff
        .recent_events
        .iter()
        .take(8)
        .map(|e| {
            let (key, positive) = match &e.event_type {
                StaffEventType::TrainingConducted => ("staff_event_training", true),
                StaffEventType::MatchObserved => ("staff_event_match_observed", true),
                StaffEventType::PlayerScouted => ("staff_event_player_scouted", true),
                StaffEventType::PositiveInteraction => ("staff_event_positive_interaction", true),
                StaffEventType::Conflict => ("staff_event_conflict", false),
                StaffEventType::MentorshipStarted => ("staff_event_mentorship", true),
                StaffEventType::TrustBuilt => ("staff_event_trust_built", true),
                StaffEventType::PerformanceExcellent => ("staff_event_excellent_performance", true),
                StaffEventType::PerformanceDeclined => ("staff_event_performance_declined", false),
                StaffEventType::LicenseUpgrade => ("staff_event_license_upgrade", true),
                StaffEventType::ProfessionalDevelopment => ("staff_event_professional_dev", true),
                StaffEventType::Birthday => ("staff_event_birthday", true),
                StaffEventType::HighFatigue => ("staff_event_high_fatigue", false),
                StaffEventType::ContractNegotiation => ("staff_event_contract_negotiation", false),
            };
            StaffRecentEventDto {
                description: i18n.t(key).to_string(),
                is_positive: positive,
                days_ago: e.days_ago,
            }
        })
        .collect();

    events.sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
    events
}

fn position_to_i18n_key(position: &StaffPosition) -> &'static str {
    match position {
        StaffPosition::Manager => "staff_manager",
        StaffPosition::AssistantManager => "staff_assistant_manager",
        StaffPosition::CaretakerManager => "staff_caretaker_manager",
        StaffPosition::Coach => "staff_coach",
        StaffPosition::FirstTeamCoach => "staff_first_team_coach",
        StaffPosition::FitnessCoach => "staff_fitness_coach",
        StaffPosition::GoalkeeperCoach => "staff_goalkeeper_coach",
        StaffPosition::YouthCoach => "staff_youth_coach",
        StaffPosition::U21Manager => "staff_u21_manager",
        StaffPosition::U19Manager => "staff_u19_manager",
        StaffPosition::Scout => "staff_scout",
        StaffPosition::ChiefScout => "staff_chief_scout",
        StaffPosition::Physio => "staff_physio",
        StaffPosition::HeadOfPhysio => "staff_head_of_physio",
        StaffPosition::Chairman => "staff_chairman",
        StaffPosition::Director => "staff_director",
        StaffPosition::ManagingDirector => "staff_managing_director",
        StaffPosition::DirectorOfFootball => "staff_director_of_football",
        StaffPosition::GeneralManager => "staff_general_manager",
        StaffPosition::HeadOfYouthDevelopment => "staff_head_of_youth_dev",
        StaffPosition::MediaPundit => "staff_media_pundit",
        StaffPosition::Free => "staff_free",
    }
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &crate::I18n,
) -> Result<(Vec<(String, String)>, Vec<(String, String)>), ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let club_name = &club.name;

    let mut teams: Vec<(String, String, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| {
            (
                format!("{}  |  {}", club_name, i18n.t(team.team_type.as_i18n_key())),
                team.slug.clone(),
                team.reputation.world,
            )
        })
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

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
        teams
            .into_iter()
            .map(|(name, slug, _)| (name, slug))
            .collect(),
        country_leagues
            .into_iter()
            .map(|(_, name, slug)| (name, slug))
            .collect(),
    ))
}
