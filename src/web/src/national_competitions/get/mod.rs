pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CSS_VERSION};
use crate::views::{self, MenuSection};
use crate::{ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::continent::national::CompetitionPhase;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct NationalCompetitionsGetRequest {
    pub lang: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "national_competitions/get/index.html")]
pub struct NationalCompetitionsGetTemplate {
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
    pub competitions: Vec<CompetitionDto>,
}

pub struct CompetitionDto {
    pub name: String,
    pub phase: String,
    pub groups: Vec<GroupDto>,
    pub knockout: Vec<KnockoutDto>,
}

pub struct GroupDto {
    pub name: String,
    pub rows: Vec<GroupRowDto>,
}

pub struct GroupRowDto {
    pub country_name: String,
    pub country_slug: String,
    pub played: u8,
    pub won: u8,
    pub drawn: u8,
    pub lost: u8,
    pub gf: u8,
    pub ga: u8,
    pub points: u8,
}

pub struct KnockoutDto {
    pub round_name: String,
    pub fixtures: Vec<KnockoutFixtureDto>,
}

pub struct KnockoutFixtureDto {
    pub home_name: String,
    pub home_slug: String,
    pub away_name: String,
    pub away_slug: String,
    pub home_score: Option<u8>,
    pub away_score: Option<u8>,
    pub winner_name: Option<String>,
}

fn phase_label(phase: &CompetitionPhase) -> &'static str {
    match phase {
        CompetitionPhase::NotStarted => "Not Started",
        CompetitionPhase::Qualifying => "Qualifying",
        CompetitionPhase::QualifyingPlayoff => "Qualifying Playoff",
        CompetitionPhase::GroupStage => "Group Stage",
        CompetitionPhase::Knockout => "Knockout",
        CompetitionPhase::Completed => "Completed",
    }
}

fn country_display(simulator_data: &core::SimulatorData, country_id: u32) -> (String, String) {
    for continent in &simulator_data.continents {
        if let Some(country) = continent.countries.iter().find(|c| c.id == country_id) {
            return (country.name.clone(), country.slug.clone());
        }
    }
    ("Unknown".to_string(), String::new())
}

pub async fn national_competitions_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<NationalCompetitionsGetRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;
    let simulator_data = guard.as_ref().unwrap();

    let mut competitions = Vec::new();

    for continent in &simulator_data.continents {
        for comp in &continent.national_team_competitions.competitions {
            if comp.phase == CompetitionPhase::Completed && comp.champion.is_none() {
                continue;
            }

            // Build qualifying groups
            let groups: Vec<GroupDto> = comp
                .qualifying_groups
                .iter()
                .enumerate()
                .map(|(idx, group)| {
                    let letter = (b'A' + idx as u8) as char;
                    let rows = group
                        .standings
                        .iter()
                        .map(|standing| {
                            let (name, slug) = country_display(simulator_data, standing.country_id);
                            GroupRowDto {
                                country_name: name,
                                country_slug: slug,
                                played: standing.played,
                                won: standing.won,
                                drawn: standing.drawn,
                                lost: standing.lost,
                                gf: standing.goals_for as u8,
                                ga: standing.goals_against as u8,
                                points: standing.points,
                            }
                        })
                        .collect();

                    GroupDto {
                        name: format!("Group {}", letter),
                        rows,
                    }
                })
                .collect();

            // Build knockout brackets
            let knockout: Vec<KnockoutDto> = comp
                .knockout
                .iter()
                .map(|bracket| {
                    let round_name = match &bracket.round {
                        core::continent::national::KnockoutRound::RoundOf16 => "Round of 16",
                        core::continent::national::KnockoutRound::QuarterFinals => "Quarter-Finals",
                        core::continent::national::KnockoutRound::SemiFinals => "Semi-Finals",
                        core::continent::national::KnockoutRound::ThirdPlace => "Third Place",
                        core::continent::national::KnockoutRound::Final => "Final",
                    };

                    let fixtures = bracket
                        .fixtures
                        .iter()
                        .map(|fix| {
                            let (home_name, home_slug) =
                                country_display(simulator_data, fix.home_country_id);
                            let (away_name, away_slug) =
                                country_display(simulator_data, fix.away_country_id);
                            let winner_name = fix.result.as_ref().map(|r| {
                                let winner_id = r.winner(fix.home_country_id, fix.away_country_id);
                                country_display(simulator_data, winner_id).0
                            });

                            KnockoutFixtureDto {
                                home_name,
                                home_slug,
                                away_name,
                                away_slug,
                                home_score: fix.result.as_ref().map(|r| r.home_score),
                                away_score: fix.result.as_ref().map(|r| r.away_score),
                                winner_name,
                            }
                        })
                        .collect();

                    KnockoutDto {
                        round_name: round_name.to_string(),
                        fixtures,
                    }
                })
                .collect();

            competitions.push(CompetitionDto {
                name: format!("{} {}", comp.config.name, comp.cycle_year),
                phase: phase_label(&comp.phase).to_string(),
                groups,
                knockout,
            });
        }
    }

    let current_path = format!("/{}/national-competitions", &route_params.lang);

    Ok(NationalCompetitionsGetTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        title: i18n.t("national_competitions").to_string(),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: "FIFA / UEFA".to_string(),
        sub_title_link: String::new(),
        sub_title_country_code: String::new(),
        header_color: "#326295".to_string(),
        foreground_color: "#ffffff".to_string(),
        menu_sections: views::national_competitions_menu(&i18n, &route_params.lang, &current_path),
        lang: route_params.lang,
        i18n,
        competitions,
    })
}
