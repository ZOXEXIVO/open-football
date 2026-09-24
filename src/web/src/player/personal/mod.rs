pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::slug::{PlayerPage, resolve_player_page};
use crate::player::events::PlayerEventsCounter;
use crate::player::newspaper::PlayerNewsCounter;
use crate::views::{self, MenuSection, NeighborMenus};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use chrono::NaiveDate;
use core::Person;
use core::Player;
use core::PlayerSquadStatus;
use core::PlayerStatusType;
use core::SimulatorData;
use core::StaffPosition;
use core::TeamType;
use core::club::player::mind;
use core::utils::FormattingUtils;
use serde::Deserialize;
use std::cmp::Reverse;

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
    /// The two halves of the happiness ledger, each sorted loudest first.
    /// Split rather than diverging: a reader should not have to decode a
    /// centre line to learn which way a factor pulls.
    pub weighing: Vec<HappinessFactorDto>,
    pub lifting: Vec<HappinessFactorDto>,
    pub concerns: Vec<String>,
    pub behaviour: String,
    pub manager_relationship: Option<ManagerRelationshipDto>,
    pub favorite_clubs: Vec<FavoriteClubDto>,
    pub player_info: PlayerInfoDto,
    pub reputation: ReputationDto,
    /// The arc he is living out — the thing that orders the wants
    /// below. `None` for a man who has decided nothing, which is most
    /// footballers most weeks.
    pub career_plan: Option<CareerPlanDto>,
    /// What he is after, loudest first.
    pub wants: Vec<MindWantDto>,
    /// What he remembers about this club. `None` for a player who has
    /// nothing to remember of the place yet.
    pub mind: Option<MindDto>,
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
    /// Colour class shared by the word and the bar.
    pub tone: &'static str,
    /// The ledger balance said as a sentence — the one part of the panel
    /// that reads without decoding anything.
    pub summary: String,
    /// The five named bands, in order, so the bar carries its own legend.
    pub bands: Vec<MoraleBandDto>,
}

pub struct MoraleBandDto {
    pub label: String,
    /// Share of the track this band covers, in grid `fr` units.
    pub weight: u8,
    pub active: bool,
}

pub struct HappinessFactorDto {
    pub name: String,
    pub value: i8,
    pub label: String,
    /// Bar length in percent of the row track, 0..=100 — see
    /// `HappinessLedger::bar`.
    pub bar: u8,
}

pub struct ManagerRelationshipDto {
    pub manager_name: String,
    /// `None` until the two of them have worked together long enough for a
    /// relation to exist. The manager is still named — an absent column
    /// tells the reader less than a stated absence.
    pub bond: Option<ManagerBondDto>,
}

pub struct ManagerBondDto {
    pub label: String,
    pub tone: &'static str,
    pub trust: u8,
    pub respect: u8,
}

pub struct ReputationDto {
    pub rows: Vec<ReputationRowDto>,
}

pub struct ReputationRowDto {
    pub name: String,
    /// How much of the six-tier track he has filled, in percent — see
    /// `ReputationLadder::fill`.
    pub fill: u8,
    pub label: String,
}

pub struct PlayerInfoDto {
    pub age: u8,
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
    pub level: String,
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

    let today = simulator_data.date.date();

    let personality = get_personality(player);
    let (weighing, lifting) = get_happiness_factors(player, &i18n);
    let morale = MoraleScale::read(player.happiness.morale, &weighing, &lifting, &i18n);
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
        .map(|staff| ManagerBond::of(player, staff, &i18n));

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

    let (wants, mind) = PlayerMindView::of(
        player,
        team_opt.map(|t| t.club_id).unwrap_or(0),
        today,
        &i18n,
    );
    let career_plan = CareerPlanCard::of(player, today, &i18n);

    let player_info = get_player_info(player, today, &i18n);
    let reputation = ReputationLadder::rows(player, &i18n);

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
            .map(|t| format!("/{}/teams/{}", route_params.lang, t.slug))
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
            let current_path = format!("/{}/teams/{}", route_params.lang, team.slug);
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
        weighing,
        lifting,
        concerns,
        behaviour,
        manager_relationship,
        favorite_clubs,
        player_info,
        reputation,
        career_plan,
        wants,
        mind,
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

fn get_player_info(player: &Player, today: NaiveDate, i18n: &I18n) -> PlayerInfoDto {
    let preferred_foot = i18n.t(player.preferred_foot.as_i18n_key());

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
            level: i18n.t(l.level_key()).to_string(),
        })
        .collect();

    PlayerInfoDto {
        age: player.age(today),
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

/// The morale bar and the word above it, read off one table of bands.
///
/// The bands are the widths the word actually changes at, so printing
/// their names under the track turns the bar into its own legend — a
/// reader can see where "Okay" ends instead of being told a number.
struct MoraleScale;

impl MoraleScale {
    /// `(floor, width, label key, colour)`, lowest first. The widths are
    /// the gaps between the floors, so they sum to the whole track.
    const BANDS: [(f32, u8, &'static str, &'static str); 5] = [
        (0.0, 25, "morale_very_poor", "is-poor"),
        (25.0, 20, "morale_poor", "is-poor"),
        (45.0, 20, "morale_okay", "is-okay"),
        (65.0, 15, "morale_good", "is-good"),
        (80.0, 20, "morale_superb", "is-good"),
    ];

    fn band_of(morale: f32) -> usize {
        Self::BANDS
            .iter()
            .rposition(|(floor, ..)| morale >= *floor)
            .unwrap_or(0)
    }

    fn read(
        morale: f32,
        weighing: &[HappinessFactorDto],
        lifting: &[HappinessFactorDto],
        i18n: &I18n,
    ) -> MoraleDto {
        let active = Self::band_of(morale);
        let (_, _, label, tone) = Self::BANDS[active];

        MoraleDto {
            value: morale.round().clamp(0.0, 100.0) as u8,
            label: i18n.t(label).to_string(),
            tone,
            summary: MoraleVerdict::summary(weighing, lifting, i18n),
            bands: Self::BANDS
                .iter()
                .enumerate()
                .map(|(i, (_, weight, key, _))| MoraleBandDto {
                    label: i18n.t(key).to_string(),
                    weight: *weight,
                    active: i == active,
                })
                .collect(),
        }
    }
}

/// Says in one sentence which way the ledger is pulling.
///
/// Deliberately about the balance rather than about any one factor: the
/// columns underneath already name the factors, and a sentence built by
/// splicing a factor name into a template breaks in every language that
/// declines nouns.
struct MoraleVerdict;

impl MoraleVerdict {
    /// Above this share of the total pull, one side is doing the talking.
    const DOMINANT: f32 = 0.65;

    fn summary(
        weighing: &[HappinessFactorDto],
        lifting: &[HappinessFactorDto],
        i18n: &I18n,
    ) -> String {
        let down: i32 = weighing.iter().map(|f| f.value.unsigned_abs() as i32).sum();
        let up: i32 = lifting.iter().map(|f| f.value as i32).sum();

        let key = match (down, up) {
            (0, 0) => "morale_summary_none",
            (0, _) => "morale_summary_all_good",
            (_, 0) => "morale_summary_all_bad",
            _ => {
                let share = up as f32 / (up + down) as f32;
                if share > Self::DOMINANT {
                    "morale_summary_mostly_good"
                } else if share < 1.0 - Self::DOMINANT {
                    "morale_summary_mostly_bad"
                } else {
                    "morale_summary_mixed"
                }
            }
        };

        i18n.t(key).to_string()
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

fn get_happiness_factors(
    player: &Player,
    i18n: &I18n,
) -> (Vec<HappinessFactorDto>, Vec<HappinessFactorDto>) {
    let f = &player.happiness.factors;
    // Core seven factors (existing) plus the six derived "life in the
    // team" factors. Surface them all so the user can answer "why is
    // Messi unhappy at this club?" without guessing.
    HappinessLedger::split(
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

/// The happiness factors as the morale panel lists them: two columns,
/// what is dragging him down and what is holding him up, each strongest
/// first.
struct HappinessLedger;

impl HappinessLedger {
    /// A factor this close to zero says nothing and is left off. It is
    /// `FactorSentiment`'s neutral band rather than a threshold of its
    /// own — a row reading "Neutral" under "Weighing on him" is the sign
    /// the two had drifted apart.
    const SILENT: f32 = 1.0;

    fn split(
        factors: &[(&str, f32)],
        i18n: &I18n,
    ) -> (Vec<HappinessFactorDto>, Vec<HappinessFactorDto>) {
        let mut weighing: Vec<(&str, f32)> = factors
            .iter()
            .copied()
            .filter(|(_, val)| *val < -Self::SILENT)
            .collect();
        weighing.sort_by(|a, b| a.1.total_cmp(&b.1));

        let mut lifting: Vec<(&str, f32)> = factors
            .iter()
            .copied()
            .filter(|(_, val)| *val > Self::SILENT)
            .collect();
        lifting.sort_by(|a, b| b.1.total_cmp(&a.1));

        (Self::rows(&weighing, i18n), Self::rows(&lifting, i18n))
    }

    fn rows(factors: &[(&str, f32)], i18n: &I18n) -> Vec<HappinessFactorDto> {
        factors
            .iter()
            .map(|(key, val)| HappinessFactorDto {
                name: i18n.t(key).to_string(),
                value: val.round().clamp(-10.0, 10.0) as i8,
                label: i18n.t(FactorSentiment::i18n_key(*val)).to_string(),
                bar: Self::bar(*val),
            })
            .collect()
    }

    /// Bar length in percent of the row track. The columns are already
    /// signed by which one a factor is in, so the whole track is
    /// available to show strength.
    fn bar(value: f32) -> u8 {
        (value.abs() * 10.0).round().clamp(0.0, 100.0) as u8
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

/// Where the player stands with the man picking the side.
struct ManagerBond;

impl ManagerBond {
    fn of(player: &Player, head_coach: &core::Staff, i18n: &I18n) -> ManagerRelationshipDto {
        ManagerRelationshipDto {
            manager_name: format!(
                "{} {}",
                head_coach.full_name.display_first_name(),
                head_coach.full_name.display_last_name()
            ),
            bond: player.relations.get_staff(head_coach.id).map(|rel| {
                let (label, tone) = if rel.level > 50.0 {
                    ("rel_excellent", "is-good")
                } else if rel.level > 20.0 {
                    ("rel_good", "is-good")
                } else if rel.level > -20.0 {
                    ("rel_neutral", "")
                } else if rel.level > -50.0 {
                    ("rel_poor", "is-poor")
                } else {
                    ("rel_very_poor", "is-poor")
                };

                ManagerBondDto {
                    label: i18n.t(label).to_string(),
                    tone,
                    trust: (rel.trust_in_abilities.round().clamp(0.0, 100.0)) as u8,
                    respect: (rel.authority_respect.round().clamp(0.0, 100.0)) as u8,
                }
            }),
        }
    }
}

/// How big a name he is, on a ladder of six tiers.
///
/// Three reputations on the same six rungs: a reader can compare them by
/// counting, which a percentage of an invisible 0–10000 scale never
/// allowed.
struct ReputationLadder;

impl ReputationLadder {
    /// `(floor, label key)`, highest tier first.
    const TIERS: [(i16, &'static str); 6] = [
        (8000, "rep_world_class"),
        (6000, "rep_continental"),
        (4000, "rep_national"),
        (2000, "rep_regional"),
        (500, "rep_local"),
        (i16::MIN, "rep_unknown"),
    ];

    fn rows(player: &Player, i18n: &I18n) -> ReputationDto {
        let pa = &player.player_attributes;
        ReputationDto {
            rows: [
                ("rep_current", pa.current_reputation),
                ("rep_home", pa.home_reputation),
                ("rep_world", pa.world_reputation),
            ]
            .into_iter()
            .map(|(name, value)| Self::row(name, value, i18n))
            .collect(),
        }
    }

    fn row(name_key: &str, value: i16, i18n: &I18n) -> ReputationRowDto {
        let tier = Self::TIERS
            .iter()
            .position(|(floor, _)| value >= *floor)
            .unwrap_or(Self::TIERS.len() - 1);
        let reached = Self::TIERS.len() - tier;

        ReputationRowDto {
            name: i18n.t(name_key).to_string(),
            fill: Self::fill(reached),
            label: i18n.t(Self::TIERS[tier].1).to_string(),
        }
    }

    /// The track is cut into one slot per tier, so a fill has to land on
    /// a seam rather than a hair past it. The seams in `.fm-pp-rep-track`
    /// are drawn at these same rounded percentages.
    fn fill(reached: usize) -> u8 {
        (reached as f32 / Self::TIERS.len() as f32 * 100.0).round() as u8
    }
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &I18n,
) -> Result<NeighborMenus, ApiError> {
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

/// The arc a player is living out, as the profile page shows it.
///
/// Read next to the wants rather than instead of them: the wants are
/// what he is after this month, and this is the shape they are in
/// service of. A reader who can see both can tell a boy asking for a
/// loan because he is sulking from one asking because he means to come
/// back and take the shirt.
pub struct CareerPlanDto {
    /// "Prove himself on loan", "Claim his place", …
    pub arc: String,
    /// How far along it he is, and whether anybody has heard him say it.
    pub stage: String,
    pub unspoken: bool,
    /// The date he privately gave it, when the deadline is close enough
    /// to be worth showing.
    pub deadline: Option<String>,
    /// What he will accept for the next move, in plain words.
    pub band_floor: String,
    /// What the club made of his last spell away, when there was one.
    pub last_verdict: Option<String>,
    /// Where the club has him on its own pathway — the other side of the
    /// same conversation.
    pub pathway: String,
    /// What the board signed the cheque FOR, when it signed one. Absent
    /// for everybody the club did not buy.
    pub purpose: Option<String>,
}

/// Builds it.
struct CareerPlanCard;

impl CareerPlanCard {
    /// Below this the deadline is too far off to be news.
    const DEADLINE_SHOWN_FROM: f32 = 0.25;

    fn of(player: &Player, today: NaiveDate, i18n: &I18n) -> Option<CareerPlanDto> {
        let plan = player.mind.career.plan?;
        let day = mind::MindClock::day(today);
        Some(CareerPlanDto {
            arc: i18n.t(plan.arc.as_i18n_key()).to_string(),
            stage: i18n.t(plan.stage.as_i18n_key()).to_string(),
            unspoken: !plan.stage.is_asking(),
            deadline: (plan.deadline_pressure(day) >= Self::DEADLINE_SHOWN_FROM).then(|| {
                i18n.t("mind_deadline").replace(
                    "{date}",
                    &mind::MindClock::date(plan.review_on)
                        .format("%d.%m.%Y")
                        .to_string(),
                )
            }),
            band_floor: i18n.t(Self::band_key(plan.band_floor)).to_string(),
            last_verdict: player
                .plan
                .as_ref()
                .and_then(|p| p.last_verdict)
                .map(|verdict| i18n.t(verdict.as_i18n_key()).to_string()),
            pathway: i18n.t(player.pathway_stage().as_i18n_key()).to_string(),
            purpose: player
                .mandate()
                .filter(|mandate| mandate.is_purchase())
                .map(|mandate| i18n.t(mandate.purpose.as_i18n_key()).to_string()),
        })
    }

    /// The lowest level he will drop to, said the way a reader thinks
    /// about it rather than as a number.
    fn band_key(floor: f32) -> &'static str {
        if floor >= 0.85 {
            "career_band_floor_upward"
        } else if floor >= 0.5 {
            "career_band_floor_same_level"
        } else if floor >= 0.0 {
            "career_band_floor_one_level"
        } else {
            "career_band_floor_anywhere"
        }
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
    /// 0..100 — how hard it presses. Orders the list; the status
    /// phrase beside each want is what the reader sees of it.
    pub pressure: u8,
}

/// One conviction he holds about this club, as a sentence.
pub struct MindMemoryDto {
    pub text: String,
    /// True for the ones he is glad about.
    pub warm: bool,
}

/// What a player remembers about the place he is at.
pub struct MindDto {
    pub memories: Vec<MindMemoryDto>,
    /// −100..100, how he feels about this club overall.
    pub sentiment: i8,
    pub sentiment_label: String,
}

/// The two halves of `PlayerMind` that are worth a reader's time: the
/// goal stack, and a club-cued look at memory.
///
/// Deliberately built with `PlayerMind::inspect` rather than `recall` —
/// reading a man's memory on a web page must not rehearse it, or a
/// player who happens to be popular would never forget anything.
struct PlayerMindView;

impl PlayerMindView {
    fn of(
        player: &Player,
        club_id: u32,
        today: NaiveDate,
        i18n: &I18n,
    ) -> (Vec<MindWantDto>, Option<MindDto>) {
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
        wants.sort_by_key(|w| Reverse(w.pressure));

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

        let mind = (!memories.is_empty()).then(|| MindDto {
            memories,
            sentiment: (sentiment * 100.0).clamp(-100.0, 100.0) as i8,
            sentiment_label: i18n.t(sentiment_label).to_string(),
        });

        (wants, mind)
    }
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
            let (weighing, lifting) = HappinessLedger::split(
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
                morale: MoraleScale::read(14.0, &weighing, &lifting, &i18n),
                weighing,
                lifting,
                concerns: vec![
                    i18n.t("concern_unhappy").to_string(),
                    i18n.t("concern_transfer_request").to_string(),
                ],
                behaviour: i18n.t("behaviour_good").to_string(),
                career_plan: Some(CareerPlanDto {
                    arc: i18n.t("career_arc_step_down_to_play").to_string(),
                    stage: i18n.t("plan_stage_asking").to_string(),
                    unspoken: false,
                    deadline: Some("Has given it until 01.09.2034".to_string()),
                    band_floor: i18n.t("career_band_floor_one_level").to_string(),
                    last_verdict: Some(i18n.t("loan_verdict_steady").to_string()),
                    pathway: i18n.t("pathway_stage_reassess").to_string(),
                    purpose: None,
                }),
                manager_relationship: Some(ManagerRelationshipDto {
                    manager_name: "Riccardo Greco".to_string(),
                    bond: Some(ManagerBondDto {
                        label: i18n.t("rel_neutral").to_string(),
                        tone: "",
                        trust: 100,
                        respect: 40,
                    }),
                }),
                favorite_clubs: vec![FavoriteClubDto {
                    name: "Belgrano".to_string(),
                    slug: "belgrano".to_string(),
                }],
                player_info: PlayerInfoDto {
                    age: 30,
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
                            level: i18n.t("lang_level_native").to_string(),
                        },
                        PlayerLanguageDto {
                            name: "Italian".to_string(),
                            level: i18n.t("lang_level_basic").to_string(),
                        },
                    ],
                },
                reputation: ReputationDto {
                    rows: vec![
                        ReputationLadder::row("rep_current", 4500, &i18n),
                        ReputationLadder::row("rep_home", 4700, &i18n),
                        ReputationLadder::row("rep_world", 1800, &i18n),
                    ],
                },
                wants,
                mind: Some(MindDto {
                    memories: vec![MindMemoryDto {
                        text: "He won everything here".to_string(),
                        warm: true,
                    }],
                    sentiment: 40,
                    sentiment_label: i18n.t("mind_sentiment_fond").to_string(),
                }),
                i18n,
            }
        }
    }

    /// Nothing listed is labelled "Neutral": the cut-off for listing a
    /// factor is the same band `FactorSentiment` calls neutral.
    #[test]
    fn a_listed_factor_is_never_a_neutral_one() {
        let i18n = Fixture::i18n();
        let neutral = i18n.t("factor_neutral").to_string();

        for tenth in -20_i32..=20 {
            let value = tenth as f32 / 10.0;
            let (weighing, lifting) = HappinessLedger::split(&[("factor_salary", value)], &i18n);
            for row in weighing.iter().chain(lifting.iter()) {
                assert_ne!(row.label, neutral, "at {value}");
            }
        }
    }

    /// Each column is read strongest first, and a factor that says
    /// nothing is on neither.
    #[test]
    fn each_column_leads_with_its_strongest_and_drops_the_silent() {
        let i18n = Fixture::i18n();
        let (weighing, lifting) = HappinessLedger::split(
            &[
                ("factor_salary", 7.0),
                ("factor_injury", 0.8),
                ("factor_playing_time", -7.0),
                ("factor_manager", -3.0),
                ("factor_club_fit", 2.0),
            ],
            &i18n,
        );

        assert_eq!(
            weighing.iter().map(|r| r.value).collect::<Vec<_>>(),
            vec![-7, -3]
        );
        assert_eq!(
            lifting.iter().map(|r| r.value).collect::<Vec<_>>(),
            vec![7, 2]
        );
        assert_eq!(weighing[0].name, "Playing Time");
    }

    /// The column a factor is in carries its sign, so the whole track
    /// is available to show how strongly it pulls.
    #[test]
    fn a_full_strength_factor_fills_the_track_and_no_further() {
        assert_eq!(HappinessLedger::bar(7.0), 70);
        assert_eq!(HappinessLedger::bar(-10.0), 100);
        assert_eq!(HappinessLedger::bar(-14.0), 100);
        assert_eq!(HappinessLedger::bar(0.0), 0);
    }

    /// The bar carries its own legend: the band the morale word came
    /// from is the one lit under the track.
    #[test]
    fn the_scale_lights_the_band_the_word_came_from() {
        let i18n = Fixture::i18n();
        for (morale, word) in [
            (0.0, "Very Poor"),
            (24.9, "Very Poor"),
            (25.0, "Poor"),
            (50.0, "Okay"),
            (65.0, "Good"),
            (100.0, "Superb"),
        ] {
            let scale = MoraleScale::read(morale, &[], &[], &i18n);
            assert_eq!(scale.label, word, "at {morale}");
            let lit: Vec<&str> = scale
                .bands
                .iter()
                .filter(|b| b.active)
                .map(|b| b.label.as_str())
                .collect();
            assert_eq!(lit, vec![word], "at {morale}");
        }
    }

    /// The verdict reads the balance of the two columns, and says so
    /// even when both are empty — which is every player on day one.
    #[test]
    fn the_verdict_reads_the_balance_of_the_ledger() {
        let i18n = Fixture::i18n();
        let rows = |factors: &[(&str, f32)]| HappinessLedger::split(factors, &i18n);

        let (none_w, none_l) = rows(&[]);
        assert_eq!(
            MoraleVerdict::summary(&none_w, &none_l, &i18n),
            i18n.t("morale_summary_none")
        );

        let (w, l) = rows(&[("factor_salary", 6.0)]);
        assert_eq!(
            MoraleVerdict::summary(&w, &l, &i18n),
            i18n.t("morale_summary_all_good")
        );

        let (w, l) = rows(&[("factor_playing_time", -6.0)]);
        assert_eq!(
            MoraleVerdict::summary(&w, &l, &i18n),
            i18n.t("morale_summary_all_bad")
        );

        let (w, l) = rows(&[("factor_playing_time", -8.0), ("factor_salary", 1.5)]);
        assert_eq!(
            MoraleVerdict::summary(&w, &l, &i18n),
            i18n.t("morale_summary_mostly_bad")
        );

        let (w, l) = rows(&[("factor_playing_time", -4.0), ("factor_salary", 5.0)]);
        assert_eq!(
            MoraleVerdict::summary(&w, &l, &i18n),
            i18n.t("morale_summary_mixed")
        );
    }

    /// Three reputations, one six-slot meter: the fill stops on the seam
    /// of the tier the word names, and the top tier fills the track.
    #[test]
    fn the_reputation_meter_fills_to_the_tier_reached() {
        let i18n = Fixture::i18n();
        for (value, fill, word) in [
            (0_i16, 17, "Unknown"),
            (500, 33, "Local"),
            (2000, 50, "Regional"),
            (4500, 67, "National"),
            (6000, 83, "Continental"),
            (9000, 100, "World Class"),
        ] {
            let row = ReputationLadder::row("rep_world", value, &i18n);
            assert_eq!(row.fill, fill, "at {value}");
            assert_eq!(row.label, word);
        }
    }

    /// Every fill lands on a seam the stylesheet actually draws.
    #[test]
    fn every_fill_lands_on_a_drawn_seam() {
        let seams = [17, 33, 50, 67, 83, 100];
        for reached in 1..=6 {
            assert!(seams.contains(&ReputationLadder::fill(reached)));
        }
    }

    /// Every block of the page renders from the fixture — the flags,
    /// the named bands, both halves of the ledger, the wants with their
    /// tag, and the club column — and each ledger column opens on its
    /// strongest factor.
    #[test]
    fn every_block_of_the_page_renders() {
        let html = Fixture::template().render().expect("render");

        for marker in [
            "fm-mh-flag\"",
            "fm-mh-summary",
            "fm-mh-band is-active",
            "fm-mh-row-fill is-neg",
            "fm-mh-row-fill is-pos",
            "fm-mh-row-fill is-level",
            "fm-mh-want is-unspoken",
            "fm-mh-want-tag",
            "fm-mh-want-note is-blocked",
            "fm-mh-manager-name",
            "fm-mh-memories",
            "fm-pp-rep-fill",
            "fm-pp-trait-val td_10",
            "fm-radar-val",
        ] {
            assert!(html.contains(marker), "missing {marker}");
        }

        let at = |needle: &str| {
            html.find(needle)
                .unwrap_or_else(|| panic!("missing {needle}"))
        };

        assert!(at("Playing Time") < at("Promise Trust"));
        assert!(at("Salary Satisfaction") < at("Role Clarity"));
        assert!(at("Promise Trust") < at("Salary Satisfaction"));
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
        if let Some(start) = html.find("<link href=\"/static/css/styles.min.css")
            && let Some(end) = html[start..].find('>')
        {
            html.replace_range(start..start + end + 1, "");
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
