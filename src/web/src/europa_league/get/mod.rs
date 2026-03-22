pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::continent::competitions::CompetitionStage;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct EuropaLeagueGetRequest {
    pub lang: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "europa_league/get/index.html")]
pub struct EuropaLeagueGetTemplate {
    pub css_version: &'static str,
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
    pub current_stage: String,
    pub participants: Vec<ParticipantDto>,
    pub matches: Vec<MatchDto>,
}

pub struct ParticipantDto {
    pub club_name: String,
    pub club_slug: String,
}

pub struct MatchDto {
    pub home_name: String,
    pub home_slug: String,
    pub away_name: String,
    pub away_slug: String,
    pub date: String,
    pub stage: String,
}

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
        let slug = club.teams.teams.first()
            .map(|t| t.slug.clone())
            .unwrap_or_default();
        (club.name.clone(), slug)
    } else {
        ("Unknown".to_string(), String::new())
    }
}

pub async fn europa_league_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<EuropaLeagueGetRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;
    let simulator_data = guard.as_ref().unwrap();

    let el = simulator_data.continents.iter()
        .map(|c| &c.continental_competitions.europa_league)
        .find(|el| !el.participating_clubs.is_empty() || !matches!(el.current_stage, CompetitionStage::NotStarted));

    let mut participants = Vec::new();
    let mut matches = Vec::new();
    let mut current_stage = "Not Started";

    if let Some(el) = el {
        current_stage = stage_label(&el.current_stage);

        for &club_id in &el.participating_clubs {
            let (name, slug) = club_display(simulator_data, club_id);
            participants.push(ParticipantDto { club_name: name, club_slug: slug });
        }

        for m in &el.matches {
            let (home_name, home_slug) = club_display(simulator_data, m.home_team);
            let (away_name, away_slug) = club_display(simulator_data, m.away_team);
            matches.push(MatchDto {
                home_name,
                home_slug,
                away_name,
                away_slug,
                date: m.date.format("%d.%m.%Y").to_string(),
                stage: stage_label(&m.stage).to_string(),
            });
        }
    }

    let current_path = format!("/{}/europa-league", &route_params.lang);

    Ok(EuropaLeagueGetTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        title: i18n.t("europa_league").to_string(),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: "UEFA".to_string(),
        sub_title_link: String::new(),
        sub_title_country_code: String::new(),
        header_color: "#f37021".to_string(),
        foreground_color: "#ffffff".to_string(),
        menu_sections: views::europa_league_menu(&i18n, &route_params.lang, &current_path),
        lang: route_params.lang,
        i18n,
        current_stage: current_stage.to_string(),
        participants,
        matches,
    })
}
