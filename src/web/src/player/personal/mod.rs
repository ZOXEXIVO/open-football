pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::slug::{PlayerPage, resolve_player_page};
use crate::player::events::PlayerEventsCounter;
use crate::player::newspaper::PlayerNewsCounter;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use chrono::NaiveDate;
use core::Player;
use core::PlayerPreferredFoot;
use core::PlayerSquadStatus;
use core::PlayerStatusType;
use core::SimulatorData;
use core::StaffPosition;
use core::TeamType;
use core::club::player::mind;
use core::utils::FormattingUtils;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PlayerPersonalRequest {
    pub lang: String,
    pub player_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/personal/index.html")]
pub struct PlayerPersonalTemplate {
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
    pub events_count: usize,
    pub interested_clubs_count: usize,
    pub awards_count: u32,
    pub news_count: usize,
    pub personality: PersonalityDto,
    pub morale: MoraleDto,
    pub happiness_factors: Vec<HappinessFactorDto>,
    pub concerns: Vec<String>,
    pub behaviour: String,
    pub manager_relationship: Option<ManagerRelationshipDto>,
    pub favorite_clubs: Vec<FavoriteClubDto>,
    pub player_info: PlayerInfoDto,
    pub reputation: ReputationDto,
    /// What he wants, and what he remembers about this club. `None` for
    /// a mind with nothing in it yet — a fresh save, or a player whose
    /// side has not ticked since he arrived.
    pub mind: Option<MindDto>,
    /// Whether the club column of the morale panel has anything to
    /// say — a manager he has a relationship with, or memories of
    /// this club. Both are optional; the column is skipped when
    /// neither is there.
    pub has_club_ties: bool,
}

pub struct FavoriteClubDto {
    pub name: String,
    pub slug: String,
}

pub struct PersonalityDto {
    /// Centre of the radar, where every axis starts.
    pub cx: f32,
    pub cy: f32,
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

pub struct MoraleDto {
    pub value: u8,
    pub label: String,
}

pub struct HappinessFactorDto {
    pub name: String,
    pub value: i8,
    pub label: String,
    /// Bar length in percent of the whole ledger track, 0..=50 — see
    /// `HappinessLedger::bar`.
    pub bar: u8,
}

pub struct ManagerRelationshipDto {
    pub manager_name: String,
    pub level: i8,
    pub label: String,
    pub trust: u8,
    pub respect: u8,
}

pub struct ReputationDto {
    pub current: u8,
    pub current_label: String,
    pub home: u8,
    pub home_label: String,
    pub world: u8,
    pub world_label: String,
}

pub struct PlayerInfoDto {
    pub birth_date: String,
    pub preferred_foot: String,
    pub leadership: u8,
    pub determination: u8,
    pub work_rate: u8,
    pub condition: u8,
    pub fitness: u8,
    pub squad_status: String,
    pub salary: String,
    pub contract_expiry: String,
    pub international_apps: u16,
    pub international_goals: u16,
    pub languages: Vec<PlayerLanguageDto>,
}

pub struct PlayerLanguageDto {
    pub name: String,
    pub proficiency: u8,
    pub level: String,
    pub is_native: bool,
}

pub async fn player_personal_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<PlayerPersonalRequest>,
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
        "/personal",
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

    let personality = get_personality(player);
    let morale = get_morale(player, &i18n);
    let happiness_factors = get_happiness_factors(player, &i18n);
    let concerns = get_concerns(player, &i18n);
    let behaviour = i18n
        .t(&format!(
            "behaviour_{}",
            player.behaviour.as_str().to_lowercase()
        ))
        .to_string();

    let manager_relationship = team_opt
        .and_then(|team| {
            team.staffs.manager().or_else(|| {
                team.staffs
                    .find_by_position(StaffPosition::AssistantManager)
            })
        })
        .and_then(|staff| get_manager_relationship(player, staff, &i18n));

    let favorite_clubs: Vec<FavoriteClubDto> = player
        .favorite_clubs
        .iter()
        .filter_map(|&club_id| {
            simulator_data.club(club_id).map(|club| {
                let slug = club
                    .teams
                    .teams
                    .iter()
                    .find(|t| t.team_type == TeamType::Main)
                    .map(|t| t.slug.clone())
                    .unwrap_or_default();
                FavoriteClubDto {
                    name: club.name.clone(),
                    slug,
                }
            })
        })
        .collect();

    let mind = get_mind(
        player,
        team_opt.map(|t| t.club_id).unwrap_or(0),
        simulator_data.date.date(),
        &i18n,
    );

    let has_club_ties = manager_relationship.is_some()
        || mind.as_ref().is_some_and(|m| !m.memories.is_empty());

    let player_info = get_player_info(player, &i18n);
    let reputation = get_reputation(player, &i18n);

    Ok(PlayerPersonalTemplate {
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
            views::team_menu(&mp, &neighbor_refs, &league_refs)
        } else {
            Vec::new()
        },
        i18n,
        lang: route_params.lang.clone(),
        active_tab: "personal",
        player_id: player.id,
        player_slug: canonical,
        club_id: team_opt.map(|t| t.club_id).unwrap_or(0),
        is_on_loan: player.is_on_loan(),
        is_injured: player.player_attributes.is_injured,
        is_unhappy: player.statuses.get().contains(&PlayerStatusType::Unh),
        is_force_match_selection: player.is_force_match_selection,
        is_on_watchlist: simulator_data.watchlist.contains(&player.id),
        events_count: PlayerEventsCounter::count(player),
        interested_clubs_count: simulator_data.clubs_interested_in_player(player.id).len(),
        awards_count: player.awards_count.total(),
        news_count: PlayerNewsCounter::count(simulator_data, player),
        personality,
        morale,
        happiness_factors,
        concerns,
        behaviour,
        manager_relationship,
        favorite_clubs,
        player_info,
        reputation,
        mind,
        has_club_ties,
    }
    .into_response())
}

fn get_personality(player: &Player) -> PersonalityDto {
    let attrs = &player.attributes;

    PersonalityRadar::plot([
        attrs.adaptability.round().clamp(1.0, 20.0) as u8,
        attrs.ambition.round().clamp(1.0, 20.0) as u8,
        attrs.controversy.round().clamp(1.0, 20.0) as u8,
        attrs.loyalty.round().clamp(1.0, 20.0) as u8,
        attrs.pressure.round().clamp(1.0, 20.0) as u8,
        attrs.professionalism.round().clamp(1.0, 20.0) as u8,
        attrs.sportsmanship.round().clamp(1.0, 20.0) as u8,
        attrs.temperament.round().clamp(1.0, 20.0) as u8,
    ])
}

/// Lays the eight hidden attributes out on the radar the personality
/// panel draws.
struct PersonalityRadar;

impl PersonalityRadar {
    const NAMES: [&'static str; 8] = [
        "adaptability",
        "ambition",
        "controversy",
        "loyalty",
        "pressure",
        "professionalism",
        "sportsmanship",
        "temperament",
    ];

    /// Centred in a 440x280 viewBox. The polygon is drawn at nearly the
    /// full height; the extra width is what keeps the longest label,
    /// "Sportsmanship", inside the left edge with its value after it.
    const CX: f32 = 220.0;
    const CY: f32 = 140.0;
    const MAX_R: f32 = 96.0;
    const LABEL_R: f32 = 114.0;

    fn angle_at(i: usize) -> f32 {
        std::f32::consts::PI * 2.0 * (i as f32) / (Self::NAMES.len() as f32)
            - std::f32::consts::FRAC_PI_2
    }

    fn grid_polygon(radius: f32) -> String {
        (0..Self::NAMES.len())
            .map(|i| {
                let a = Self::angle_at(i);
                format!(
                    "{:.1},{:.1}",
                    Self::CX + radius * a.cos(),
                    Self::CY + radius * a.sin()
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn plot(values: [u8; 8]) -> PersonalityDto {
        let mut data_points = Vec::new();
        let mut radar_axes = Vec::new();
        let mut radar_items = Vec::new();

        for (i, name) in Self::NAMES.iter().enumerate() {
            let angle = Self::angle_at(i);
            let ratio = values[i] as f32 / 20.0;
            data_points.push(format!(
                "{:.1},{:.1}",
                Self::CX + Self::MAX_R * ratio * angle.cos(),
                Self::CY + Self::MAX_R * ratio * angle.sin()
            ));

            radar_axes.push(RadarAxisDto {
                x2: Self::CX + Self::MAX_R * angle.cos(),
                y2: Self::CY + Self::MAX_R * angle.sin(),
            });

            let anchor = if angle.cos().abs() < 0.01 {
                "middle"
            } else if angle.cos() > 0.0 {
                "start"
            } else {
                "end"
            };

            radar_items.push(RadarLabelDto {
                name: name.to_string(),
                value: values[i],
                x: Self::CX + Self::LABEL_R * angle.cos(),
                y: Self::CY + Self::LABEL_R * angle.sin(),
                anchor: anchor.to_string(),
            });
        }

        PersonalityDto {
            cx: Self::CX,
            cy: Self::CY,
            radar_points: data_points.join(" "),
            radar_grid_4: Self::grid_polygon(Self::MAX_R),
            radar_grid_3: Self::grid_polygon(Self::MAX_R * 0.75),
            radar_grid_2: Self::grid_polygon(Self::MAX_R * 0.5),
            radar_grid_1: Self::grid_polygon(Self::MAX_R * 0.25),
            radar_axes,
            radar_items,
        }
    }
}

fn get_player_info(player: &Player, i18n: &I18n) -> PlayerInfoDto {
    let preferred_foot = match player.preferred_foot {
        PlayerPreferredFoot::Left => i18n.t("foot_left"),
        PlayerPreferredFoot::Right => i18n.t("foot_right"),
        PlayerPreferredFoot::Both => i18n.t("foot_both"),
    };

    let mental = &player.skills.mental;
    let leadership = mental.leadership.round().clamp(1.0, 20.0) as u8;
    let determination = mental.determination.round().clamp(1.0, 20.0) as u8;
    let work_rate = mental.work_rate.round().clamp(1.0, 20.0) as u8;

    let pa = &player.player_attributes;
    let condition = (pa.condition as f32 / 100.0).round().clamp(0.0, 100.0) as u8;
    let fitness = (pa.fitness as f32 / 100.0).round().clamp(0.0, 100.0) as u8;

    let (squad_status, salary, contract_expiry) = if let Some(contract) = &player.contract {
        let status = match contract.squad_status {
            PlayerSquadStatus::KeyPlayer => i18n.t("squad_key_player"),
            PlayerSquadStatus::FirstTeamRegular => i18n.t("squad_first_team_regular"),
            PlayerSquadStatus::FirstTeamSquadRotation => i18n.t("squad_rotation"),
            PlayerSquadStatus::MainBackupPlayer => i18n.t("squad_backup_player"),
            PlayerSquadStatus::HotProspectForTheFuture => i18n.t("squad_hot_prospect"),
            PlayerSquadStatus::DecentYoungster => i18n.t("squad_decent_youngster"),
            PlayerSquadStatus::NotNeeded => i18n.t("squad_not_needed"),
            _ => "",
        };
        let wage = format!(
            "{} {}",
            FormattingUtils::format_money(contract.salary as f64),
            i18n.t("per_year")
        );
        let expiry = i18n.format_date(contract.expiration);
        (status.to_string(), wage, expiry)
    } else {
        (String::new(), String::new(), String::new())
    };

    let languages: Vec<PlayerLanguageDto> = player
        .languages
        .iter()
        .filter(|l| l.proficiency >= 5 || l.is_native)
        .map(|l| PlayerLanguageDto {
            name: i18n.t(l.language.i18n_key()).to_string(),
            proficiency: l.proficiency,
            level: i18n.t(l.level_key()).to_string(),
            is_native: l.is_native,
        })
        .collect();

    PlayerInfoDto {
        birth_date: i18n.format_date(player.birth_date),
        preferred_foot: preferred_foot.to_string(),
        leadership,
        determination,
        work_rate,
        condition,
        fitness,
        squad_status,
        salary,
        contract_expiry,
        international_apps: pa.international_apps,
        international_goals: pa.international_goals,
        languages,
    }
}

fn get_morale(player: &Player, i18n: &I18n) -> MoraleDto {
    let m = player.happiness.morale;
    let label = if m >= 80.0 {
        i18n.t("morale_superb")
    } else if m >= 65.0 {
        i18n.t("morale_good")
    } else if m >= 45.0 {
        i18n.t("morale_okay")
    } else if m >= 25.0 {
        i18n.t("morale_poor")
    } else {
        i18n.t("morale_very_poor")
    };
    MoraleDto {
        value: m.round().clamp(0.0, 100.0) as u8,
        label: label.to_string(),
    }
}

/// Sentiment bucket for a single happiness *factor*. A factor is one
/// per-axis enrichment, not the player's overall verdict — so a lone axis
/// sitting at -5 is a "Major concern", never "Very Unhappy". The player's
/// overall mood is the morale label; the factors only explain *why*. Keeping
/// the factor labels in the concern/positive register (rather than the
/// happy/unhappy register the morale line uses) stops a single moderate axis
/// from reading as a dressing-room crisis when total morale is fine.
struct FactorSentiment;

impl FactorSentiment {
    /// i18n key for the label describing a factor of the given value.
    fn i18n_key(value: f32) -> &'static str {
        if value > 5.0 {
            "factor_strong_positive"
        } else if value > 1.0 {
            "factor_positive"
        } else if value >= -1.0 {
            "factor_neutral"
        } else if value > -5.0 {
            "factor_concern"
        } else {
            "factor_major_concern"
        }
    }
}

fn get_happiness_factors(player: &Player, i18n: &I18n) -> Vec<HappinessFactorDto> {
    let f = &player.happiness.factors;
    // Core seven factors (existing) plus the six derived "life in the
    // team" factors. Surface them all so the user can answer "why is
    // Messi unhappy at this club?" without guessing.
    HappinessLedger::rows(
        &[
            ("factor_playing_time", f.playing_time),
            ("factor_salary", f.salary_satisfaction),
            ("factor_manager", f.manager_relationship),
            ("factor_ambition_fit", f.ambition_fit),
            ("factor_injury", f.injury_frustration),
            ("factor_role_clarity", f.role_clarity),
            ("factor_coach_credibility", f.coach_credibility),
            ("factor_dressing_room_status", f.dressing_room_status),
            ("factor_club_fit", f.club_fit),
            ("factor_pressure_load", f.pressure_load),
            ("factor_promise_trust", f.promise_trust),
        ],
        i18n,
    )
}

/// The happiness factors as the morale panel lists them: a ledger of
/// what is pulling him each way, worst first.
struct HappinessLedger;

impl HappinessLedger {
    /// A factor this close to zero says nothing and is left off.
    const SILENT: f32 = 0.5;

    fn rows(factors: &[(&str, f32)], i18n: &I18n) -> Vec<HappinessFactorDto> {
        let mut factors = factors.to_vec();
        // Worst first: the panel exists to answer "why is he unhappy",
        // so the reader meets the cause before the consolation.
        factors.sort_by(|a, b| a.1.total_cmp(&b.1));

        factors
            .iter()
            .filter(|(_, val)| val.abs() > Self::SILENT)
            .map(|(key, val)| HappinessFactorDto {
                name: i18n.t(key).to_string(),
                value: val.round().clamp(-10.0, 10.0) as i8,
                label: i18n.t(FactorSentiment::i18n_key(*val)).to_string(),
                bar: Self::bar(*val),
            })
            .collect()
    }

    /// Bar length in percent of the whole track. Half the track is a
    /// full-strength factor, so a bar never crosses the centre line.
    fn bar(value: f32) -> u8 {
        (value.abs() * 5.0).round().clamp(0.0, 50.0) as u8
    }
}

fn get_concerns(player: &Player, i18n: &I18n) -> Vec<String> {
    let statuses = player.statuses.get();
    let mut concerns = Vec::new();

    for status in &statuses {
        let key = match status {
            PlayerStatusType::Unh => Some("concern_unhappy"),
            PlayerStatusType::Req => Some("concern_transfer_request"),
            PlayerStatusType::Rst => Some("concern_needs_rest"),
            PlayerStatusType::Fut => Some("concern_future"),
            PlayerStatusType::Abs => Some("concern_absent"),
            PlayerStatusType::Slt => Some("concern_slight_concerns"),
            PlayerStatusType::Frt => Some("concern_wants_free_transfer"),
            _ => None,
        };
        if let Some(k) = key {
            concerns.push(i18n.t(k).to_string());
        }
    }

    // Add happiness-derived concerns
    let f = &player.happiness.factors;
    if f.playing_time < -3.0
        && !concerns
            .iter()
            .any(|c| c.contains(&i18n.t("concern_unhappy").to_string()))
    {
        concerns.push(i18n.t("concern_lacking_playing_time").to_string());
    }
    if f.salary_satisfaction < -3.0 {
        concerns.push(i18n.t("concern_unhappy_with_salary").to_string());
    }
    if f.ambition_fit < -3.0 {
        concerns.push(i18n.t("concern_ambition_not_met").to_string());
    }
    if f.injury_frustration < -3.0 {
        concerns.push(i18n.t("concern_frustrated_by_injuries").to_string());
    }

    concerns
}

fn get_manager_relationship(
    player: &Player,
    head_coach: &core::Staff,
    i18n: &I18n,
) -> Option<ManagerRelationshipDto> {
    let rel = player.relations.get_staff(head_coach.id)?;
    let level = rel.level.round().clamp(-100.0, 100.0) as i8;
    let label = if level > 50 {
        i18n.t("rel_excellent")
    } else if level > 20 {
        i18n.t("rel_good")
    } else if level > -20 {
        i18n.t("rel_neutral")
    } else if level > -50 {
        i18n.t("rel_poor")
    } else {
        i18n.t("rel_very_poor")
    };

    Some(ManagerRelationshipDto {
        manager_name: format!(
            "{} {}",
            head_coach.full_name.display_first_name(),
            head_coach.full_name.display_last_name()
        ),
        level,
        label: label.to_string(),
        trust: (rel.trust_in_abilities.round().clamp(0.0, 100.0)) as u8,
        respect: (rel.authority_respect.round().clamp(0.0, 100.0)) as u8,
    })
}

fn reputation_label(value: i16, i18n: &I18n) -> String {
    if value >= 8000 {
        i18n.t("rep_world_class")
    } else if value >= 6000 {
        i18n.t("rep_continental")
    } else if value >= 4000 {
        i18n.t("rep_national")
    } else if value >= 2000 {
        i18n.t("rep_regional")
    } else if value >= 500 {
        i18n.t("rep_local")
    } else {
        i18n.t("rep_unknown")
    }
    .to_string()
}

fn get_reputation(player: &Player, i18n: &I18n) -> ReputationDto {
    let pa = &player.player_attributes;
    // Scale 0-10000 to 0-100 for progress bar percentage
    let current_pct = (pa.current_reputation as f32 / 100.0)
        .round()
        .clamp(0.0, 100.0) as u8;
    let home_pct = (pa.home_reputation as f32 / 100.0)
        .round()
        .clamp(0.0, 100.0) as u8;
    let world_pct = (pa.world_reputation as f32 / 100.0)
        .round()
        .clamp(0.0, 100.0) as u8;

    ReputationDto {
        current: current_pct,
        current_label: reputation_label(pa.current_reputation, i18n),
        home: home_pct,
        home_label: reputation_label(pa.home_reputation, i18n),
        world: world_pct,
        world_label: reputation_label(pa.world_reputation, i18n),
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

#[cfg(test)]
mod factor_sentiment_tests {
    use super::FactorSentiment;

    // A single moderate negative factor must read as a "Concern", not as a
    // "Very Unhappy"-style verdict — that register is reserved for overall
    // morale, never one per-axis enrichment.
    #[test]
    fn moderate_negative_factor_is_a_concern_not_a_verdict() {
        assert_eq!(FactorSentiment::i18n_key(-3.0), "factor_concern");
        assert_eq!(FactorSentiment::i18n_key(-4.0), "factor_concern");
        // The factor labels never use the morale "very unhappy" register.
        for v in [-3.0_f32, -4.0, -4.9] {
            assert_ne!(FactorSentiment::i18n_key(v), "factor_very_unhappy");
        }
    }

    #[test]
    fn severe_negative_factor_is_a_major_concern() {
        assert_eq!(FactorSentiment::i18n_key(-5.0), "factor_major_concern");
        assert_eq!(FactorSentiment::i18n_key(-9.0), "factor_major_concern");
    }

    #[test]
    fn neutral_band_around_zero() {
        assert_eq!(FactorSentiment::i18n_key(0.0), "factor_neutral");
        assert_eq!(FactorSentiment::i18n_key(-1.0), "factor_neutral");
        assert_eq!(FactorSentiment::i18n_key(1.0), "factor_neutral");
    }

    #[test]
    fn positive_bands() {
        assert_eq!(FactorSentiment::i18n_key(3.0), "factor_positive");
        assert_eq!(FactorSentiment::i18n_key(6.0), "factor_strong_positive");
    }
}

/// One thing a player is currently after, as the profile page shows it.
pub struct MindWantDto {
    /// What he wants.
    pub name: String,
    /// How far along the ladder it has climbed — "shapes every decision",
    /// "has said so", "demanding it".
    pub status: String,
    /// True while nobody has heard him say it. `Latent` and `Active` are
    /// designed to be the silent rungs, and showing them as silent is
    /// what makes the panel worth reading: a manager can see a want
    /// forming a season before it becomes a transfer request.
    pub unspoken: bool,
    /// A date he has privately given it, when he has given one.
    pub deadline: Option<String>,
    /// Something is stopping him acting on it at all.
    pub blocked: Option<String>,
    /// 0..100 — how hard it presses, for the bar.
    pub pressure: u8,
}

/// One conviction he holds about this club, as a sentence.
pub struct MindMemoryDto {
    pub text: String,
    /// True for the ones he is glad about.
    pub warm: bool,
}

/// What a player wants, and what he remembers about the place he is at.
///
/// The two halves of `PlayerMind` that are worth a reader's time: the
/// goal stack, and a club-cued look at memory. Deliberately built with
/// `PlayerMind::inspect` rather than `recall` — reading a man's memory
/// on a web page must not rehearse it, or a player who happens to be
/// popular would never forget anything.
pub struct MindDto {
    pub wants: Vec<MindWantDto>,
    pub memories: Vec<MindMemoryDto>,
    /// −100..100, how he feels about this club overall.
    pub sentiment: i8,
    pub sentiment_label: String,
}

fn get_mind(player: &Player, club_id: u32, today: NaiveDate, i18n: &I18n) -> Option<MindDto> {
    let ctx = player.mind_context(today, Some(club_id).filter(|id| *id != 0));

    let mut wants: Vec<MindWantDto> = player
        .mind
        .goals()
        .live()
        .filter(|goal| goal.kind != mind::GoalKind::None)
        .map(|goal| MindWantDto {
            name: i18n.t(goal.kind.as_i18n_key()).to_string(),
            status: i18n.t(goal.status.as_i18n_key()).to_string(),
            unspoken: matches!(
                goal.status,
                mind::GoalStatus::Latent | mind::GoalStatus::Active
            ),
            deadline: (goal.deadline > 0).then(|| {
                i18n.t("mind_deadline").replace(
                    "{date}",
                    &mind::MindClock::date(goal.deadline)
                        .format("%d.%m.%Y")
                        .to_string(),
                )
            }),
            blocked: goal
                .blocked_by
                .is_blocked()
                .then(|| i18n.t(goal.blocked_by.as_i18n_key()).to_string()),
            pressure: (goal.pressure() * 100.0).clamp(0.0, 100.0) as u8,
        })
        .collect();
    // Loudest first — the want that is actually driving him leads.
    wants.sort_by(|a, b| b.pressure.cmp(&a.pressure));

    // Club 0 is not a club. Cueing on it would match every episode
    // recorded while he had no club at all and present them as things he
    // remembers about *this* place, which for a free agent is the whole
    // set. He has no "here" to remember.
    let recalled = if club_id == 0 {
        Default::default()
    } else {
        player.mind.inspect(mind::RecallCue::Club(club_id), &ctx)
    };
    let memories: Vec<MindMemoryDto> = recalled
        .facts
        .iter()
        .filter(|fact| fact.claim != mind::FactClaim::None)
        .take(6)
        .map(|fact| MindMemoryDto {
            text: i18n.t(fact.claim.as_i18n_key()).to_string(),
            warm: fact.claim.valence() >= 0.0,
        })
        .collect();

    if wants.is_empty() && memories.is_empty() {
        return None;
    }

    let sentiment = recalled.sentiment();
    let sentiment_label = if sentiment > 0.35 {
        "mind_sentiment_fond"
    } else if sentiment > 0.1 {
        "mind_sentiment_warm"
    } else if sentiment < -0.35 {
        "mind_sentiment_bitter"
    } else if sentiment < -0.1 {
        "mind_sentiment_cool"
    } else {
        "mind_sentiment_neutral"
    };

    Some(MindDto {
        wants,
        memories,
        sentiment: (sentiment * 100.0).clamp(-100.0, 100.0) as i8,
        sentiment_label: i18n.t(sentiment_label).to_string(),
    })
}

#[cfg(test)]
mod page_tests {
    use super::*;
    use std::collections::HashMap;

    /// Matías Daniele's page as it stood in June 2034: every block of
    /// the layout populated, including the branches a settled player
    /// never shows.
    struct Fixture;

    impl Fixture {
        fn i18n() -> I18n {
            let raw = std::fs::read_to_string("assets/i18n/en.json").expect("en bundle");
            let map: HashMap<String, String> =
                serde_json::from_str(&raw).expect("en bundle is a flat map");
            I18n::for_test(map)
        }

        fn want(name: &str, status: &str, pressure: u8, unspoken: bool) -> MindWantDto {
            MindWantDto {
                name: name.to_string(),
                status: status.to_string(),
                unspoken,
                deadline: None,
                blocked: None,
                pressure,
            }
        }

        fn template() -> PlayerPersonalTemplate {
            let i18n = Self::i18n();
            let happiness_factors = HappinessLedger::rows(
                &[
                    ("factor_playing_time", -7.0),
                    ("factor_salary", 7.0),
                    ("factor_manager", -3.0),
                    ("factor_ambition_fit", 0.2),
                    ("factor_injury", 0.0),
                    ("factor_role_clarity", 3.0),
                    ("factor_coach_credibility", -2.0),
                    ("factor_dressing_room_status", -3.0),
                    ("factor_club_fit", 2.0),
                    ("factor_pressure_load", -0.3),
                    ("factor_promise_trust", -6.0),
                ],
                &i18n,
            );
            let mut wants = vec![
                Self::want("To be paid what he is worth", "Demanding it", 72, false),
                Self::want("To go home", "Has said so", 58, false),
                Self::want("A settled future", "Shapes every decision", 47, true),
                Self::want("To learn the language", "Has said so", 41, false),
                Self::want("Out, anywhere", "Shapes every decision", 22, true),
                Self::want("First-team football", "Shapes every decision", 18, true),
                Self::want("His place back", "Beginning to feel it", 6, true),
            ];
            wants[0].blocked = Some("He has only just arrived".to_string());
            wants[1].deadline = Some("Has given it until 01.09.2034".to_string());

            PlayerPersonalTemplate {
                css_version: "test",
                computer_name: "test",
                cpu_brand: "test",
                cores_count: 1,
                title: "Matías Daniele".to_string(),
                sub_title_prefix: "GK".to_string(),
                sub_title_suffix: String::new(),
                sub_title: "AC Milan".to_string(),
                sub_title_link: "/en/teams/ac-milan".to_string(),
                sub_title_country_code: String::new(),
                header_color: "#c8102e".to_string(),
                foreground_color: "#ffffff".to_string(),
                menu_sections: Vec::new(),
                lang: "en".to_string(),
                active_tab: "personal",
                player_id: 2000200423,
                player_slug: "2000200423-matias-daniele".to_string(),
                club_id: 1,
                is_on_loan: false,
                is_injured: false,
                is_unhappy: true,
                is_force_match_selection: false,
                is_on_watchlist: false,
                events_count: 133,
                interested_clubs_count: 2,
                awards_count: 1,
                news_count: 8,
                personality: PersonalityRadar::plot([2, 3, 19, 18, 19, 6, 3, 2]),
                morale: MoraleDto {
                    value: 14,
                    label: i18n.t("morale_very_poor").to_string(),
                },
                happiness_factors,
                concerns: vec![
                    i18n.t("concern_unhappy").to_string(),
                    i18n.t("concern_transfer_request").to_string(),
                ],
                behaviour: i18n.t("behaviour_good").to_string(),
                manager_relationship: Some(ManagerRelationshipDto {
                    manager_name: "Riccardo Greco".to_string(),
                    level: 4,
                    label: i18n.t("rel_neutral").to_string(),
                    trust: 100,
                    respect: 40,
                }),
                favorite_clubs: vec![FavoriteClubDto {
                    name: "Belgrano".to_string(),
                    slug: "belgrano".to_string(),
                }],
                player_info: PlayerInfoDto {
                    birth_date: "2 Jan 2004".to_string(),
                    preferred_foot: i18n.t("foot_right").to_string(),
                    leadership: 10,
                    determination: 11,
                    work_rate: 7,
                    condition: 91,
                    fitness: 100,
                    squad_status: i18n.t("squad_backup_player").to_string(),
                    salary: "3.1M per year".to_string(),
                    contract_expiry: "16 Jun 2035".to_string(),
                    international_apps: 12,
                    international_goals: 3,
                    languages: vec![
                        PlayerLanguageDto {
                            name: "Spanish".to_string(),
                            proficiency: 100,
                            level: i18n.t("lang_level_native").to_string(),
                            is_native: true,
                        },
                        PlayerLanguageDto {
                            name: "Italian".to_string(),
                            proficiency: 32,
                            level: i18n.t("lang_level_basic").to_string(),
                            is_native: false,
                        },
                    ],
                },
                reputation: ReputationDto {
                    current: 45,
                    current_label: i18n.t("rep_national").to_string(),
                    home: 47,
                    home_label: i18n.t("rep_national").to_string(),
                    world: 18,
                    world_label: i18n.t("rep_regional").to_string(),
                },
                mind: Some(MindDto {
                    wants,
                    memories: vec![MindMemoryDto {
                        text: "He won everything here".to_string(),
                        warm: true,
                    }],
                    sentiment: 40,
                    sentiment_label: i18n.t("mind_sentiment_fond").to_string(),
                }),
                has_club_ties: true,
                i18n,
            }
        }
    }

    /// The ledger is read worst first, and a factor that says nothing
    /// is not on it.
    #[test]
    fn ledger_lists_worst_first_and_drops_the_silent() {
        let i18n = Fixture::i18n();
        let rows = HappinessLedger::rows(
            &[
                ("factor_salary", 7.0),
                ("factor_injury", 0.2),
                ("factor_playing_time", -7.0),
                ("factor_manager", -3.0),
            ],
            &i18n,
        );

        let values: Vec<i8> = rows.iter().map(|r| r.value).collect();
        assert_eq!(values, vec![-7, -3, 7]);
        assert_eq!(rows[0].name, "Playing Time");
    }

    /// Half the track is a full-strength factor; nothing crosses the
    /// centre line however far the value is clamped from.
    #[test]
    fn a_full_strength_factor_reaches_the_centre_line_and_no_further() {
        assert_eq!(HappinessLedger::bar(7.0), 35);
        assert_eq!(HappinessLedger::bar(-10.0), 50);
        assert_eq!(HappinessLedger::bar(-14.0), 50);
        assert_eq!(HappinessLedger::bar(0.0), 0);
    }

    /// Every block of the page renders from the fixture — the flags,
    /// the ledger, the wants with their tag, and the club column —
    /// and the ledger opens on the worst factor.
    #[test]
    fn every_block_of_the_page_renders() {
        let html = Fixture::template().render().expect("render");

        for marker in [
            "fm-mh-flag\"",
            "fm-mh-ledger-row",
            "fm-mh-want is-unspoken",
            "fm-mh-want-tag",
            "fm-mh-want-note is-blocked",
            "fm-mh-manager-name",
            "fm-mh-memories",
            "fm-pp-rep-tile",
            "fm-pp-trait-val td_10",
            "fm-radar-val",
        ] {
            assert!(html.contains(marker), "missing {marker}");
        }

        let ledger = html.find("fm-mh-ledger-row").expect("ledger");
        let after = &html[ledger..];
        assert!(after.find("Playing Time").unwrap() < after.find("Salary Satisfaction").unwrap());
    }

    /// Writes a self-contained copy of the page so the layout can be
    /// looked at in a browser without starting the server:
    ///
    /// ```text
    /// PERSONAL_PREVIEW_DIR=<dir> cargo test -p web --lib player_personal_preview -- --ignored
    /// ```
    #[test]
    #[ignore]
    fn player_personal_preview() {
        let Ok(dir) = std::env::var("PERSONAL_PREVIEW_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).expect("preview dir");

        let page = Fixture::template().render().expect("render");

        // The rendered page links assets by absolute URL, which a
        // `file://` load cannot resolve — inline the two stylesheets the
        // layout actually depends on and drop the rest.
        let bootstrap =
            std::fs::read_to_string("assets/static/css/bootstrap.min.css").expect("bootstrap");
        let style = std::fs::read_to_string("assets/static/css/style.css").expect("stylesheet");

        let mut html = page;
        for link in [
            "<link href=\"/static/css/bootstrap.min.css\" rel=\"stylesheet\">",
            "<link href=\"/static/css/flags.css\" rel=\"stylesheet\">",
            "<link href=\"/static/css/font.min.css\" rel=\"stylesheet\">",
        ] {
            html = html.replace(link, "");
        }
        if let Some(start) = html.find("<link href=\"/static/css/styles.min.css") {
            if let Some(end) = html[start..].find('>') {
                html.replace_range(start..start + end + 1, "");
            }
        }
        html = html.replace(
            "</head>",
            &format!("<style>{bootstrap}</style><style>{style}</style></head>"),
        );

        std::fs::write(
            std::path::Path::new(&dir).join("player-personal.html"),
            html,
        )
        .expect("write preview");
    }
}
