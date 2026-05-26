pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection};
use crate::{ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::continent::competitions::CompetitionStage;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct CopaLibertadoresGetRequest {
    pub lang: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "copa_libertadores/get/index.html")]
pub struct CopaLibertadoresGetTemplate {
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
    pub current_stage: String,
    pub groups: Vec<CopaGroupDto>,
    pub knockout_ties: Vec<KnockoutTieDto>,
}

impl CopaLibertadoresGetTemplate {
    fn stage_label(stage: &CompetitionStage) -> &'static str {
        match stage {
            CompetitionStage::NotStarted => "Not Started",
            CompetitionStage::Qualifying => "Qualifying",
            CompetitionStage::GroupStage => "Group Stage",
            CompetitionStage::RoundOf32 => "Round of 32",
            CompetitionStage::RoundOf16 => "Round of 16",
            CompetitionStage::QuarterFinals => "Quarter-Finals",
            CompetitionStage::SemiFinals => "Semi-Finals",
            CompetitionStage::Final => "Final",
        }
    }

    fn club_display(simulator_data: &core::SimulatorData, club_id: u32) -> (String, String) {
        if let Some(club) = simulator_data.club(club_id) {
            let slug = club
                .teams
                .teams
                .first()
                .map(|t| t.slug.clone())
                .unwrap_or_default();
            (club.name.clone(), slug)
        } else {
            ("Unknown".to_string(), String::new())
        }
    }
}

pub struct CopaGroupDto {
    pub name: String,
    pub rows: Vec<CopaGroupRowDto>,
}

pub struct CopaGroupRowDto {
    pub club_name: String,
    pub club_slug: String,
    pub played: u8,
    pub won: u8,
    pub drawn: u8,
    pub lost: u8,
    pub gf: u8,
    pub ga: u8,
    pub points: u8,
}

pub struct KnockoutTieDto {
    pub home_name: String,
    pub home_slug: String,
    pub away_name: String,
    pub away_slug: String,
    pub leg1_score: Option<(u8, u8)>,
    pub leg2_score: Option<(u8, u8)>,
    pub winner_name: Option<String>,
}

pub async fn copa_libertadores_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<CopaLibertadoresGetRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;
    let simulator_data = guard.as_ref().unwrap();

    // Find South America's Copa Libertadores data
    let copa = simulator_data
        .continents
        .iter()
        .find(|c| c.name == "South America")
        .map(|c| &c.continental_competitions.copa_libertadores)
        .filter(|copa| {
            !copa.groups.is_empty() || !matches!(copa.current_stage, CompetitionStage::NotStarted)
        });

    let mut groups = Vec::new();
    let mut knockout_ties = Vec::new();
    let mut current_stage = "Not Started";

    if let Some(copa) = copa {
        current_stage = CopaLibertadoresGetTemplate::stage_label(&copa.current_stage);

        // Build group DTOs
        for (idx, group) in copa.groups.iter().enumerate() {
            let letter = (b'A' + idx as u8) as char;
            let rows = group
                .rows
                .iter()
                .map(|row| {
                    let (name, slug) =
                        CopaLibertadoresGetTemplate::club_display(simulator_data, row.team_id);
                    CopaGroupRowDto {
                        club_name: name,
                        club_slug: slug,
                        played: row.played,
                        won: row.won,
                        drawn: row.drawn,
                        lost: row.lost,
                        gf: row.gf,
                        ga: row.ga,
                        points: row.points,
                    }
                })
                .collect();

            groups.push(CopaGroupDto {
                name: format!("Group {}", letter),
                rows,
            });
        }

        // Build knockout DTOs
        for tie in &copa.knockout_round {
            let (home_name, home_slug) =
                CopaLibertadoresGetTemplate::club_display(simulator_data, tie.home_team);
            let (away_name, away_slug) =
                CopaLibertadoresGetTemplate::club_display(simulator_data, tie.away_team);
            let winner_name = tie
                .winner
                .map(|w| CopaLibertadoresGetTemplate::club_display(simulator_data, w).0);

            knockout_ties.push(KnockoutTieDto {
                home_name,
                home_slug,
                away_name,
                away_slug,
                leg1_score: tie.leg1_score,
                leg2_score: tie.leg2_score,
                winner_name,
            });
        }
    }

    let current_path = format!("/{}/copa-libertadores", &route_params.lang);

    Ok(CopaLibertadoresGetTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        title: i18n.t("copa_libertadores").to_string(),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: "CONMEBOL".to_string(),
        sub_title_link: String::new(),
        sub_title_country_code: String::new(),
        header_color: "#7a1020".to_string(),
        foreground_color: "#ffffff".to_string(),
        menu_sections: views::copa_libertadores_menu(&i18n, &route_params.lang, &current_path),
        lang: route_params.lang,
        i18n,
        current_stage: current_stage.to_string(),
        groups,
        knockout_ties,
    })
}
