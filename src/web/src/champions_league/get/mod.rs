pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::continent::competitions::CompetitionStage;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct ChampionsLeagueGetRequest {
    pub lang: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "champions_league/get/index.html")]
pub struct ChampionsLeagueGetTemplate {
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
    pub current_stage: String,
    pub groups: Vec<ClGroupDto>,
    pub knockout_ties: Vec<KnockoutTieDto>,
}

pub struct ClGroupDto {
    pub name: String,
    pub rows: Vec<ClGroupRowDto>,
}

pub struct ClGroupRowDto {
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

pub async fn champions_league_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<ChampionsLeagueGetRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;
    let simulator_data = guard.as_ref().unwrap();

    // Find European continent's CL data
    let cl = simulator_data.continents.iter()
        .find(|c| c.name == "Europe")
        .map(|c| &c.continental_competitions.champions_league)
        .filter(|cl| !cl.groups.is_empty() || !matches!(cl.current_stage, CompetitionStage::NotStarted));

    let mut groups = Vec::new();
    let mut knockout_ties = Vec::new();
    let mut current_stage = "Not Started";

    if let Some(cl) = cl {
        current_stage = stage_label(&cl.current_stage);

        // Build group DTOs
        for (idx, group) in cl.groups.iter().enumerate() {
            let letter = (b'A' + idx as u8) as char;
            let rows = group.rows.iter().map(|row| {
                let (name, slug) = club_display(simulator_data, row.team_id);
                ClGroupRowDto {
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
            }).collect();

            groups.push(ClGroupDto {
                name: format!("Group {}", letter),
                rows,
            });
        }

        // Build knockout DTOs
        for tie in &cl.knockout_round {
            let (home_name, home_slug) = club_display(simulator_data, tie.home_team);
            let (away_name, away_slug) = club_display(simulator_data, tie.away_team);
            let winner_name = tie.winner.map(|w| club_display(simulator_data, w).0);

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

    let current_path = format!("/{}/champions-league", &route_params.lang);

    Ok(ChampionsLeagueGetTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        computer_name: &crate::common::default_handler::COMPUTER_NAME,
        title: i18n.t("champions_league").to_string(),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: "UEFA".to_string(),
        sub_title_link: String::new(),
        sub_title_country_code: String::new(),
        header_color: "#1a3668".to_string(),
        foreground_color: "#ffffff".to_string(),
        menu_sections: views::champions_league_menu(&i18n, &route_params.lang, &current_path),
        lang: route_params.lang,
        i18n,
        current_stage: current_stage.to_string(),
        groups,
        knockout_ties,
    })
}
