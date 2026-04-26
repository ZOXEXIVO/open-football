pub mod routes;

use crate::common::default_handler::{CSS_VERSION, COMPUTER_NAME};
use crate::common::slug::{resolve_player_page, PlayerPage};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::utils::FormattingUtils;
use core::Player;
use core::PlayerStatusType;
use core::SimulatorData;
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
    pub is_on_watchlist: bool,
    pub personality: PersonalityDto,
    pub morale: MoraleDto,
    pub happiness_factors: Vec<HappinessFactorDto>,
    pub concerns: Vec<String>,
    pub behaviour: String,
    pub manager_relationship: Option<ManagerRelationshipDto>,
    pub favorite_clubs: Vec<FavoriteClubDto>,
    pub player_info: PlayerInfoDto,
    pub reputation: ReputationDto,
}

pub struct FavoriteClubDto {
    pub name: String,
    pub slug: String,
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

pub struct MoraleDto {
    pub value: u8,
    pub label: String,
}

pub struct HappinessFactorDto {
    pub name: String,
    pub value: i8,
    pub label: String,
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
        PlayerPage::Found { player, team, canonical_slug } => (player, team, canonical_slug),
        PlayerPage::Redirect(r) => return Ok(r),
    };

    let (neighbor_teams, country_leagues) = if let Some(team) = team_opt {
        get_neighbor_teams(team.club_id, simulator_data, &i18n)?
    } else {
        (Vec::new(), Vec::new())
    };
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
    let league_refs: Vec<(&str, &str)> = country_leagues.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    let title = format!("{} {}", player.full_name.display_first_name(), player.full_name.display_last_name());

    let personality = get_personality(player);
    let morale = get_morale(player, &i18n);
    let happiness_factors = get_happiness_factors(player, &i18n);
    let concerns = get_concerns(player, &i18n);
    let behaviour = i18n.t(&format!("behaviour_{}", player.behaviour.as_str().to_lowercase())).to_string();

    let manager_relationship = team_opt
        .map(|team| {
            let head_coach = team.staffs.head_coach();
            get_manager_relationship(player, head_coach, &i18n)
        })
        .flatten();

    let favorite_clubs: Vec<FavoriteClubDto> = player.favorite_clubs.iter()
        .filter_map(|&club_id| {
            simulator_data.club(club_id).map(|club| {
                let slug = club.teams.teams.iter()
                    .find(|t| t.team_type == core::TeamType::Main)
                    .map(|t| t.slug.clone())
                    .unwrap_or_default();
                FavoriteClubDto {
                    name: club.name.clone(),
                    slug,
                }
            })
        })
        .collect();

    let player_info = get_player_info(player, &i18n);
    let reputation = get_reputation(player, &i18n);

    Ok(PlayerPersonalTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
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
        sub_title_link: team_opt.map(|t| format!("/{}/teams/{}", &route_params.lang, &t.slug)).unwrap_or_default(),
        sub_title_country_code: String::new(),
        header_color: team_opt.and_then(|t| simulator_data.club(t.club_id).map(|c| c.colors.background.clone())).unwrap_or_else(|| "#808080".to_string()),
        foreground_color: team_opt.and_then(|t| simulator_data.club(t.club_id).map(|c| c.colors.foreground.clone())).unwrap_or_else(|| "#ffffff".to_string()),
        menu_sections: if let Some(team) = team_opt {
            let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
            let current_path = format!("/{}/teams/{}", &route_params.lang, &team.slug);
            let mp = views::MenuParams { i18n: &i18n, lang: &route_params.lang, current_path: &current_path, country_name: cn, country_slug: cs };
            views::team_menu(&mp, &neighbor_refs, &team.slug, &league_refs, team.team_type == core::TeamType::Main)
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
        is_on_watchlist: simulator_data.watchlist.contains(&player.id),
        personality,
        morale,
        happiness_factors,
        concerns,
        behaviour,
        manager_relationship,
        favorite_clubs,
        player_info,
        reputation,
    }.into_response())
}

fn get_personality(player: &Player) -> PersonalityDto {
    let attrs = &player.attributes;

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
    let names = ["adaptability", "ambition", "controversy", "loyalty", "pressure", "professionalism", "sportsmanship", "temperament"];

    // Centered in a 400x280 viewBox with enough room for labels
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
        data_points.push(format!("{:.1},{:.1}", cx + max_r * ratio * angle.cos(), cy + max_r * ratio * angle.sin()));

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

fn get_player_info(player: &Player, i18n: &I18n) -> PlayerInfoDto {
    use core::{PlayerPreferredFoot, PlayerSquadStatus};

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
        let wage = format!("{} {}", FormattingUtils::format_money(contract.salary as f64), i18n.t("per_year"));
        let expiry = contract.expiration.format("%d.%m.%Y").to_string();
        (status.to_string(), wage, expiry)
    } else {
        (String::new(), String::new(), String::new())
    };

    let languages: Vec<PlayerLanguageDto> = player.languages.iter()
        .filter(|l| l.proficiency >= 5 || l.is_native)
        .map(|l| PlayerLanguageDto {
            name: i18n.t(l.language.i18n_key()).to_string(),
            proficiency: l.proficiency,
            level: i18n.t(l.level_key()).to_string(),
            is_native: l.is_native,
        })
        .collect();

    PlayerInfoDto {
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

fn get_happiness_factors(player: &Player, i18n: &I18n) -> Vec<HappinessFactorDto> {
    let f = &player.happiness.factors;
    // Core seven factors (existing) plus the six derived "life in the
    // team" factors. Surface them all so the user can answer "why is
    // Messi unhappy at this club?" without guessing.
    let factors = [
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
    ];

    factors
        .iter()
        .filter(|(_, val)| val.abs() > 0.5)
        .map(|(key, val)| {
            let label = if *val > 5.0 {
                i18n.t("factor_very_happy")
            } else if *val > 1.0 {
                i18n.t("factor_happy")
            } else if *val > -1.0 {
                i18n.t("factor_neutral")
            } else if *val > -5.0 {
                i18n.t("factor_unhappy")
            } else {
                i18n.t("factor_very_unhappy")
            };
            HappinessFactorDto {
                name: i18n.t(key).to_string(),
                value: val.round().clamp(-10.0, 10.0) as i8,
                label: label.to_string(),
            }
        })
        .collect()
}

fn get_concerns(player: &Player, i18n: &I18n) -> Vec<String> {
    use core::PlayerStatusType;

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
    if f.playing_time < -3.0 && !concerns.iter().any(|c| c.contains(&i18n.t("concern_unhappy").to_string())) {
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

fn get_manager_relationship(player: &Player, head_coach: &core::Staff, i18n: &I18n) -> Option<ManagerRelationshipDto> {
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
    }.to_string()
}

fn get_reputation(player: &Player, i18n: &I18n) -> ReputationDto {
    let pa = &player.player_attributes;
    // Scale 0-10000 to 0-100 for progress bar percentage
    let current_pct = (pa.current_reputation as f32 / 100.0).round().clamp(0.0, 100.0) as u8;
    let home_pct = (pa.home_reputation as f32 / 100.0).round().clamp(0.0, 100.0) as u8;
    let world_pct = (pa.world_reputation as f32 / 100.0).round().clamp(0.0, 100.0) as u8;

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
            country.leagues.leagues.iter()
                .filter(|l| !l.friendly)
                .map(|l| (l.id, l.name.clone(), l.slug.clone()))
                .collect()
        })
        .unwrap_or_default();
    country_leagues.sort_by_key(|(id, _, _)| *id);

    Ok((
        teams,
        country_leagues.into_iter().map(|(_, name, slug)| (name, slug)).collect(),
    ))
}
