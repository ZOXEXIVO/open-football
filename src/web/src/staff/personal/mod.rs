pub mod routes;

use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::staff::get::{StaffMenus, StaffRole};
use crate::views::{self, MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::utils::FormattingUtils;
use core::{CoachingStyle, SimulatorData, Staff, StaffEventType, StaffLicenseType};
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
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_flag: Option<SubTitleFlag>,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: I18n,
    pub lang: String,
    pub staff_id: u32,
    pub personality: PersonalityDto,
    pub staff_info: StaffInfoDto,
    pub performance: PerformanceDto,
    pub recent_events: Vec<StaffRecentEventDto>,
    /// Players this scout is currently monitoring. Empty for non-scout
    /// roles (the template renders nothing in that case).
    pub monitoring: Vec<StaffMonitoringDto>,
}

pub struct StaffMonitoringDto {
    pub player_id: u32,
    pub player_name: String,
    pub target_club_name: String,
    pub target_team_slug: String,
    pub status_key: String,
    pub last_observed: String,
    pub confidence_pct: u8,
    pub times_watched: u16,
    pub matches_watched: u16,
    pub assessed_ability: u8,
    pub assessed_potential: u8,
    /// i18n key for the latest meeting vote. Empty if the scout
    /// hasn't been to a meeting on this player yet.
    pub latest_vote_key: String,
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
    /// Pre-formatted "time-ago" label. Renders as the localised "now"
    /// for events emitted today, "{n}d ago" otherwise.
    pub time_ago_label: String,
}
pub struct StaffPersonalPage<'a> {
    data: &'a SimulatorData,
    i18n: I18n,
    lang: String,
    staff: &'a Staff,
    team: &'a core::Team,
}

impl<'a> StaffPersonalPage<'a> {
    pub async fn action(
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

        let page = StaffPersonalPage {
            data: simulator_data,
            i18n,
            lang: route_params.lang,
            staff,
            team,
        };
        page.render()
    }

    fn render(self) -> ApiResult<StaffPersonalTemplate> {
        let (neighbor_teams, country_leagues) =
            StaffMenus::of(self.team.club_id, self.data, &self.i18n)?;

        let title = format!(
            "{} {}",
            self.staff.full_name.first_name, self.staff.full_name.last_name
        );

        let role_key = self
            .staff
            .contract
            .as_ref()
            .map(|c| StaffRole::as_i18n_key(&c.position).to_string())
            .unwrap_or_else(|| "staff_free".to_string());

        Ok(StaffPersonalTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            title,
            sub_title_prefix: self.i18n.t(&role_key).to_string(),
            sub_title_suffix: if self.team.team_type == core::TeamType::Main {
                String::new()
            } else {
                self.i18n.t(self.team.team_type.as_i18n_key()).to_string()
            },
            sub_title: self.team.name.clone(),
            sub_title_link: format!("/{}/teams/{}", self.lang, self.team.slug),
            sub_title_flag: None,
            header_color: self
                .data
                .club(self.team.club_id)
                .map(|c| c.colors.background.clone())
                .unwrap_or_default(),
            foreground_color: self
                .data
                .club(self.team.club_id)
                .map(|c| c.colors.foreground.clone())
                .unwrap_or_default(),
            menu_sections: self.menu_sections(&neighbor_teams, &country_leagues),
            staff_id: self.staff.id,
            personality: PersonalityDto::of(self.staff),
            staff_info: StaffInfoDto::of(self.staff, &self.i18n),
            performance: PerformanceDto::of(self.staff),
            recent_events: StaffRecentEventDto::rows(self.staff, &self.i18n),
            monitoring: StaffMonitoringDto::rows(self.staff, self.data),
            i18n: self.i18n,
            lang: self.lang,
        })
    }

    fn menu_sections(
        &self,
        neighbor_teams: &[(String, String)],
        country_leagues: &[(String, String)],
    ) -> Vec<MenuSection> {
        let neighbor_refs: Vec<(&str, &str)> = neighbor_teams
            .iter()
            .map(|(n, s)| (n.as_str(), s.as_str()))
            .collect();
        let league_refs: Vec<(&str, &str)> = country_leagues
            .iter()
            .map(|(n, s)| (n.as_str(), s.as_str()))
            .collect();
        let (cn, cs) = views::Menus::club_country_info(self.data, self.team.club_id);
        let current_path = format!("/{}/teams/{}", self.lang, self.team.slug);
        let mp = views::MenuParams {
            i18n: &self.i18n,
            lang: &self.lang,
            current_path: &current_path,
            country_name: cn,
            country_slug: cs,
        };
        views::Menus::team_menu(&mp, &neighbor_refs, &league_refs)
    }
}

impl StaffMonitoringDto {
    fn rows(staff: &Staff, data: &SimulatorData) -> Vec<StaffMonitoringDto> {
        // Only meaningful for scouting roles. Non-scouts render an empty
        // list in the template (which the template hides).
        let is_scouting_role = staff
            .contract
            .as_ref()
            .map(|c| c.position.is_scouting())
            .unwrap_or(false);
        if !is_scouting_role {
            return Vec::new();
        }
        data.staff_monitoring_workload(staff.id)
            .into_iter()
            .map(|row| StaffMonitoringDto {
                player_id: row.player_id,
                player_name: row.player_name,
                target_club_name: row.target_club_name,
                target_team_slug: row.target_team_slug,
                status_key: format!("monitoring_status_{}", row.status),
                last_observed: row
                    .last_observed
                    .map(|d| d.format("%d.%m.%Y").to_string())
                    .unwrap_or_default(),
                confidence_pct: (row.confidence * 100.0).round().clamp(0.0, 100.0) as u8,
                times_watched: row.times_watched,
                matches_watched: row.matches_watched,
                assessed_ability: row.assessed_ability,
                assessed_potential: row.assessed_potential,
                latest_vote_key: row
                    .latest_vote
                    .map(|v| format!("scout_vote_{}", v))
                    .unwrap_or_default(),
            })
            .collect()
    }
}

impl PersonalityDto {
    fn of(staff: &Staff) -> PersonalityDto {
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
}

impl StaffInfoDto {
    fn of(staff: &Staff, i18n: &I18n) -> StaffInfoDto {
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
            let role = i18n
                .t(StaffRole::as_i18n_key(&contract.position))
                .to_string();
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
}

impl PerformanceDto {
    fn of(staff: &Staff) -> PerformanceDto {
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
}

impl StaffRecentEventDto {
    fn rows(staff: &Staff, i18n: &I18n) -> Vec<StaffRecentEventDto> {
        let mut events: Vec<_> = staff
            .recent_events
            .iter()
            .take(8)
            .map(|e| {
                let (key, positive) = match &e.event_type {
                    StaffEventType::TrainingConducted => ("staff_event_training", true),
                    StaffEventType::MatchObserved => ("staff_event_match_observed", true),
                    StaffEventType::PlayerScouted => ("staff_event_player_scouted", true),
                    StaffEventType::PositiveInteraction => {
                        ("staff_event_positive_interaction", true)
                    }
                    StaffEventType::Conflict => ("staff_event_conflict", false),
                    StaffEventType::MentorshipStarted => ("staff_event_mentorship", true),
                    StaffEventType::TrustBuilt => ("staff_event_trust_built", true),
                    StaffEventType::PerformanceExcellent => {
                        ("staff_event_excellent_performance", true)
                    }
                    StaffEventType::PerformanceDeclined => {
                        ("staff_event_performance_declined", false)
                    }
                    StaffEventType::LicenseUpgrade => ("staff_event_license_upgrade", true),
                    StaffEventType::ProfessionalDevelopment => {
                        ("staff_event_professional_dev", true)
                    }
                    StaffEventType::Birthday => ("staff_event_birthday", true),
                    StaffEventType::HighFatigue => ("staff_event_high_fatigue", false),
                    StaffEventType::ContractNegotiation => {
                        ("staff_event_contract_negotiation", false)
                    }
                    StaffEventType::RecruitmentMeeting => ("staff_event_recruitment_meeting", true),
                    StaffEventType::TargetRecommended => ("staff_event_target_recommended", true),
                    StaffEventType::TargetRejected => ("staff_event_target_rejected", false),
                    StaffEventType::BoardPresentation => ("staff_event_board_presentation", true),
                };
                let time_ago_label = if e.days_ago == 0 {
                    i18n.t("now").to_string()
                } else {
                    format!("{}{}", e.days_ago, i18n.t("days_ago_short"))
                };
                StaffRecentEventDto {
                    description: i18n.t(key).to_string(),
                    is_positive: positive,
                    days_ago: e.days_ago,
                    time_ago_label,
                }
            })
            .collect();

        events.sort_by_key(|a| a.days_ago);
        events
    }
}
