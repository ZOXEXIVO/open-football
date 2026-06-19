pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection, NationalCompetitionLink};
use crate::{ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::continent::national::{
    CompetitionPhase, CompetitionScope, NationalCompetitionConfig, NationalTeamCompetition,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct NationalCompetitionsGetRequest {
    pub lang: String,
}

#[derive(Deserialize)]
pub struct NationalCompetitionGetRequest {
    pub lang: String,
    pub slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "national_competitions/get/index.html")]
pub struct NationalCompetitionsGetTemplate {
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
    pub competitions: Vec<CompetitionDto>,
}

pub struct CompetitionDto {
    pub name: String,
    /// Governing body derived from scope/continent (FIFA, UEFA, CAF, ...).
    pub confederation: &'static str,
    pub phase: String,
    /// i18n key for the team-level label ("senior" / "u21").
    pub level_key: &'static str,
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

/// Governing body for a competition: FIFA for global tournaments, otherwise
/// the confederation that owns the continent it is contested in.
fn confederation_label(scope: &CompetitionScope, continent_id: Option<u32>) -> &'static str {
    match scope {
        CompetitionScope::Global => "FIFA",
        CompetitionScope::Continental => match continent_id {
            Some(0) => "CAF",
            Some(1) => "UEFA",
            Some(2) => "CONCACAF",
            Some(3) => "CONMEBOL",
            Some(4) => "AFC",
            Some(5) => "OFC",
            _ => "International",
        },
    }
}

/// Distinct confederations present among the displayed competitions, in a
/// stable display order, joined for the page subtitle (e.g. "FIFA · UEFA · CAF").
fn confederations_subtitle(competitions: &[CompetitionDto]) -> String {
    const ORDER: [&str; 7] = [
        "FIFA",
        "UEFA",
        "CONMEBOL",
        "CAF",
        "CONCACAF",
        "AFC",
        "OFC",
    ];
    let present: Vec<&str> = ORDER
        .iter()
        .copied()
        .filter(|conf| competitions.iter().any(|c| c.confederation == *conf))
        .collect();
    if present.is_empty() {
        "International".to_string()
    } else {
        present.join(" · ")
    }
}

/// URL-safe slug from a competition name ("FIFA World Cup" → "fifa-world-cup").
fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

fn country_display(simulator_data: &core::SimulatorData, country_id: u32) -> (String, String) {
    for continent in &simulator_data.continents {
        if let Some(country) = continent.countries.iter().find(|c| c.id == country_id) {
            return (country.name.clone(), country.slug.clone());
        }
    }
    ("Unknown".to_string(), String::new())
}

/// Build the view model for one competition instance (qualifying or
/// tournament). Tournament groups supersede qualifying groups once the
/// finals are drawn; the global World Cup is assembled straight into
/// tournament groups (its qualifying lives per-continent), so prefer them.
fn build_competition_dto(
    simulator_data: &core::SimulatorData,
    comp: &NationalTeamCompetition,
) -> CompetitionDto {
    let group_source = if comp.tournament_groups.is_empty() {
        &comp.qualifying_groups
    } else {
        &comp.tournament_groups
    };

    let groups: Vec<GroupDto> = group_source
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

    CompetitionDto {
        name: format!("{} {}", comp.config.name, comp.cycle_year),
        confederation: confederation_label(&comp.config.scope, comp.config.continent_id),
        phase: phase_label(&comp.phase).to_string(),
        level_key: comp.config.team_level.as_i18n_key(),
        groups,
        knockout,
    }
}

/// Distinct competition configs across the global and continental
/// trackers, deduped by id and ordered by id (World Cup first). Drives
/// the left-menu list — present whether or not a cycle is active.
fn menu_competition_configs(simulator_data: &core::SimulatorData) -> Vec<&NationalCompetitionConfig> {
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut configs: Vec<&NationalCompetitionConfig> = Vec::new();

    for cfg in &simulator_data.global_competitions.configs {
        if seen.insert(cfg.id) {
            configs.push(cfg);
        }
    }
    for continent in &simulator_data.continents {
        for cfg in &continent.national_team_competitions.competition_configs {
            if seen.insert(cfg.id) {
                configs.push(cfg);
            }
        }
    }

    configs.sort_by_key(|c| c.id);
    configs
}

pub async fn national_competitions_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<NationalCompetitionsGetRequest>,
) -> ApiResult<impl IntoResponse> {
    render(state, route_params.lang, None).await
}

pub async fn national_competition_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<NationalCompetitionGetRequest>,
) -> ApiResult<impl IntoResponse> {
    render(state, route_params.lang, Some(route_params.slug)).await
}

async fn render(
    state: GameAppData,
    lang: String,
    selected_slug: Option<String>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&lang);
    let guard = state.data.read().await;
    let simulator_data = guard.as_ref().unwrap();

    let configs = menu_competition_configs(simulator_data);

    // Resolve the selected competition (per-competition page) by slug.
    let selected = selected_slug
        .as_ref()
        .and_then(|slug| configs.iter().find(|c| slugify(&c.name) == *slug).copied());
    let selected_id = selected.map(|c| c.id);

    let include = |comp: &NationalTeamCompetition| {
        selected_id.is_none_or(|id| comp.config.id == id)
            && !(comp.phase == CompetitionPhase::Completed && comp.champion.is_none())
    };

    // Live instances: per-continent qualifiers / continental tournaments,
    // plus the global World Cup finals (assembled at the world level).
    let mut competitions: Vec<CompetitionDto> = Vec::new();
    for continent in &simulator_data.continents {
        for comp in &continent.national_team_competitions.competitions {
            if include(comp) {
                competitions.push(build_competition_dto(simulator_data, comp));
            }
        }
    }
    for comp in &simulator_data.global_competitions.tournaments {
        if include(comp) {
            competitions.push(build_competition_dto(simulator_data, comp));
        }
    }

    let menu_links: Vec<NationalCompetitionLink> = configs
        .iter()
        .map(|c| NationalCompetitionLink {
            slug: slugify(&c.name),
            title: c.name.clone(),
        })
        .collect();

    let (current_path, title) = match selected {
        Some(c) => (
            format!("/{}/national-competitions/{}", lang, slugify(&c.name)),
            c.name.clone(),
        ),
        None => (
            format!("/{}/national-competitions", lang),
            i18n.t("national_competitions").to_string(),
        ),
    };
    let sub_title = confederations_subtitle(&competitions);
    let menu_sections = views::national_competitions_menu(&i18n, &lang, &current_path, &menu_links);

    Ok(NationalCompetitionsGetTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title,
        sub_title_link: String::new(),
        sub_title_country_code: String::new(),
        header_color: "#326295".to_string(),
        foreground_color: "#ffffff".to_string(),
        menu_sections,
        lang,
        i18n,
        competitions,
    })
}
