pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::ContractType;
use core::Person;
use core::Player;
use core::PlayerPositionType;
use core::PlayerSquadStatus;
use core::PlayerStatusType;
use core::SimulatorData;
use core::Team;
use core::transfers::TransferType;
use core::utils::FormattingUtils;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PlayerGetRequest {
    pub team_slug: String,
    pub player_id: u32,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/get/index.html")]
pub struct PlayerGetTemplate {
    pub css_version: &'static str,
    pub title: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub player: PlayerViewModel,
}

pub struct PlayerViewModel {
    pub id: u32,
    pub first_name: String,
    pub last_name: String,
    pub position: String,
    pub contract: Option<PlayerContractDto>,
    pub birth_date: String,
    pub age: u8,
    pub team_slug: String,
    pub team_name: String,
    pub country_slug: String,
    pub country_code: String,
    pub country_name: String,
    pub skills: PlayerSkillsDto,
    pub conditions: u8,
    pub current_ability: u8,
    pub potential_ability: u8,
    pub value: String,
    pub preferred_foot: String,
    pub player_attributes: PlayerAttributesDto,
    pub statistics: PlayerStatistics,
    #[allow(dead_code)]
    pub status: PlayerStatusDto,
    pub position_map: PositionMapDto,
    pub loan_status: Option<PlayerLoanDto>,
}

pub struct PlayerLoanDto {
    pub is_loan_in: bool,
    pub club_name: String,
    pub club_slug: String,
}

pub struct PositionMapDto {
    pub gk: bool,
    pub sw: bool,
    pub dl: bool,
    pub dcl: bool,
    pub dc: bool,
    pub dcr: bool,
    pub dr: bool,
    pub dm: bool,
    pub wl: bool,
    pub wr: bool,
    pub ml: bool,
    pub mcl: bool,
    pub mc: bool,
    pub mcr: bool,
    pub mr: bool,
    pub aml: bool,
    pub amc: bool,
    pub amr: bool,
    pub fl: bool,
    pub fc: bool,
    pub fr: bool,
    pub st: bool,
    pub primary: String,
}

pub struct PlayerStatistics {
    pub played: u16,
    pub played_subs: u16,
    pub goals: u16,
    pub assists: u16,
    pub penalties: u16,
    pub player_of_the_match: u8,
    pub yellow_cards: u8,
    pub red_cards: u8,
    pub shots_on_target: f32,
    pub tackling: f32,
    pub passes: u8,
    pub average_rating: f32,
}

pub struct PlayerContractDto {
    pub salary: u32,
    pub expiration: String,
    pub squad_status: String,
}

pub struct PlayerSkillsDto {
    pub technical: TechnicalDto,
    pub mental: MentalDto,
    pub physical: PhysicalDto,
}

pub struct TechnicalDto {
    pub corners: u8,
    pub crossing: u8,
    pub finishing: u8,
    pub first_touch: u8,
    pub free_kick_taking: u8,
    pub heading: u8,
    pub long_shots: u8,
    pub long_throws: u8,
    pub marking: u8,
    pub passing: u8,
    pub penalty_taking: u8,
    pub tackling: u8,
    pub technique: u8,
}

pub struct MentalDto {
    pub aggression: u8,
    pub anticipation: u8,
    pub composure: u8,
    pub concentration: u8,
    pub decisions: u8,
    pub determination: u8,
    pub flair: u8,
    pub leadership: u8,
    pub off_the_ball: u8,
    pub positioning: u8,
    pub teamwork: u8,
    pub vision: u8,
    pub work_rate: u8,
}

pub struct PhysicalDto {
    pub acceleration: u8,
    pub agility: u8,
    pub jumping_reach: u8,
    pub natural_fitness: u8,
    pub pace: u8,
    pub stamina: u8,
    pub strength: u8,
}

pub struct PlayerAttributesDto {
    pub international_apps: u16,
    pub international_goals: u16,
    pub under_21_international_apps: u16,
    pub under_21_international_goals: u16,
}

#[allow(dead_code)]
pub struct PlayerStatusDto {
    pub statuses: Vec<PlayerStatusType>,
}

impl PlayerStatusDto {
    pub fn new(statuses: Vec<PlayerStatusType>) -> Self {
        PlayerStatusDto { statuses }
    }

    #[allow(dead_code)]
    pub fn is_wanted(&self) -> bool {
        self.statuses.iter().any(|s| *s == PlayerStatusType::Wnt)
    }
}

pub async fn player_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<PlayerGetRequest>,
) -> ApiResult<impl IntoResponse> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let indexes = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?;

    let team_id = indexes
        .slug_indexes
        .get_team_by_slug(&route_params.team_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Team '{}' not found", route_params.team_slug))
        })?;

    let team: &Team = simulator_data
        .team(team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))?;

    let player: &Player = team
        .players
        .players()
        .iter()
        .find(|p| p.id == route_params.player_id)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Player with ID {} not found in team",
                route_params.player_id
            ))
        })?;

    let country = simulator_data
        .country(player.country_id)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Country with ID {} not found", player.country_id))
        })?;

    let now = simulator_data.date.date();

    let neighbor_teams: Vec<(String, String)> = get_neighbor_teams(team.club_id, simulator_data)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    let contract = player.contract.as_ref().map(|c| PlayerContractDto {
        salary: c.salary / 1000,
        expiration: c.expiration.format("%d.%m.%Y").to_string(),
        squad_status: format_squad_status(&c.squad_status),
    });

    let title = format!("{} {}", player.full_name.first_name, player.full_name.last_name);

    let loan_status = get_loan_status(player, team, simulator_data);

    let player_vm = PlayerViewModel {
        id: player.id,
        first_name: player.full_name.first_name.clone(),
        last_name: player.full_name.last_name.clone(),
        position: player.position().get_short_name().to_string(),
        contract,
        birth_date: player.birth_date.format("%d.%m.%Y").to_string(),
        age: player.age(simulator_data.date.date()),
        team_slug: team.slug.clone(),
        team_name: team.name.clone(),
        country_slug: country.slug.clone(),
        country_code: country.code.clone(),
        country_name: country.name.clone(),
        skills: get_skills(player),
        conditions: get_conditions(player),
        current_ability: get_current_ability_stars(player),
        potential_ability: get_potential_ability_stars(player),
        value: FormattingUtils::format_money(player.value(now)),
        preferred_foot: player.preferred_foot_str().to_string(),
        player_attributes: get_attributes(player),
        statistics: get_statistics(player),
        status: PlayerStatusDto::new(player.statuses.get()),
        position_map: get_position_map(player),
        loan_status,
    };

    Ok(PlayerGetTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        title,
        sub_title: team.name.clone(),
        sub_title_link: format!("/teams/{}", &team.slug),
        header_color: simulator_data.club(team.club_id).map(|c| c.colors.primary.clone()).unwrap_or_default(),
        foreground_color: simulator_data.club(team.club_id).map(|c| c.colors.secondary.clone()).unwrap_or_default(),
        menu_sections: views::player_menu(&neighbor_refs, &team.slug, &format!("/teams/{}", &team.slug)),
        player: player_vm,
    })
}

fn get_attributes(player: &Player) -> PlayerAttributesDto {
    PlayerAttributesDto {
        international_apps: player.player_attributes.international_apps,
        international_goals: player.player_attributes.international_goals,
        under_21_international_apps: player.player_attributes.under_21_international_apps,
        under_21_international_goals: player.player_attributes.under_21_international_goals,
    }
}

fn get_skills(player: &Player) -> PlayerSkillsDto {
    PlayerSkillsDto {
        technical: TechnicalDto {
            corners: player.skills.technical.corners.floor() as u8,
            crossing: player.skills.technical.crossing.floor() as u8,
            finishing: player.skills.technical.finishing.floor() as u8,
            first_touch: player.skills.technical.first_touch.floor() as u8,
            free_kick_taking: player.skills.technical.free_kicks.floor() as u8,
            heading: player.skills.technical.heading.floor() as u8,
            long_shots: player.skills.technical.long_shots.floor() as u8,
            long_throws: player.skills.technical.long_throws.floor() as u8,
            marking: player.skills.technical.marking.floor() as u8,
            passing: player.skills.technical.passing.floor() as u8,
            penalty_taking: player.skills.technical.penalty_taking.floor() as u8,
            tackling: player.skills.technical.tackling.floor() as u8,
            technique: player.skills.technical.technique.floor() as u8,
        },
        mental: MentalDto {
            aggression: player.skills.mental.aggression.floor() as u8,
            anticipation: player.skills.mental.anticipation.floor() as u8,
            composure: player.skills.mental.composure.floor() as u8,
            concentration: player.skills.mental.concentration.floor() as u8,
            decisions: player.skills.mental.decisions.floor() as u8,
            determination: player.skills.mental.determination.floor() as u8,
            flair: player.skills.mental.flair.floor() as u8,
            leadership: player.skills.mental.leadership.floor() as u8,
            off_the_ball: player.skills.mental.off_the_ball.floor() as u8,
            positioning: player.skills.mental.positioning.floor() as u8,
            teamwork: player.skills.mental.teamwork.floor() as u8,
            vision: player.skills.mental.vision.floor() as u8,
            work_rate: player.skills.mental.work_rate.floor() as u8,
        },
        physical: PhysicalDto {
            acceleration: player.skills.physical.acceleration.floor() as u8,
            agility: player.skills.physical.agility.floor() as u8,
            jumping_reach: player.skills.physical.jumping.floor() as u8,
            natural_fitness: player.skills.physical.natural_fitness.floor() as u8,
            pace: player.skills.physical.pace.floor() as u8,
            stamina: player.skills.physical.stamina.floor() as u8,
            strength: player.skills.physical.strength.floor() as u8,
        },
    }
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
) -> Result<Vec<(String, String)>, ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let mut teams: Vec<(String, String, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| (team.team_type.to_string(), team.slug.clone(), team.reputation.world))
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    Ok(teams
        .into_iter()
        .map(|(name, slug, _)| (name, slug))
        .collect())
}

fn get_statistics(player: &Player) -> PlayerStatistics {
    PlayerStatistics {
        played: player.statistics.played,
        played_subs: player.statistics.played_subs,
        goals: player.statistics.goals,
        assists: player.statistics.assists,
        penalties: player.statistics.penalties,
        player_of_the_match: player.statistics.player_of_the_match,
        yellow_cards: player.statistics.yellow_cards,
        red_cards: player.statistics.red_cards,
        shots_on_target: player.statistics.shots_on_target,
        tackling: player.statistics.tackling,
        passes: player.statistics.passes,
        average_rating: player.statistics.average_rating,
    }
}

pub fn get_conditions(player: &Player) -> u8 {
    (100f32 * ((player.player_attributes.condition as f32) / 10000.0)) as u8
}

pub fn get_current_ability_stars(player: &Player) -> u8 {
    (5.0f32 * ((player.player_attributes.current_ability as f32) / 200.0)) as u8
}

pub fn get_potential_ability_stars(player: &Player) -> u8 {
    (5.0f32 * ((player.player_attributes.potential_ability as f32) / 200.0)) as u8
}

fn format_squad_status(status: &PlayerSquadStatus) -> String {
    match status {
        PlayerSquadStatus::KeyPlayer => "Key Player",
        PlayerSquadStatus::FirstTeamRegular => "First Team Regular",
        PlayerSquadStatus::FirstTeamSquadRotation => "Squad Rotation",
        PlayerSquadStatus::MainBackupPlayer => "Backup Player",
        PlayerSquadStatus::HotProspectForTheFuture => "Hot Prospect",
        PlayerSquadStatus::DecentYoungster => "Decent Youngster",
        PlayerSquadStatus::NotNeeded => "Not Needed",
        PlayerSquadStatus::NotYetSet | PlayerSquadStatus::Invalid | PlayerSquadStatus::SquadStatusCount => "N/A",
    }
    .to_string()
}

fn get_loan_status(player: &Player, team: &Team, data: &SimulatorData) -> Option<PlayerLoanDto> {
    let is_loan_contract = player.contract.as_ref()
        .map(|c| c.contract_type == ContractType::Loan)
        .unwrap_or(false);

    let club_id = team.club_id;

    if let Some(country) = data.country_by_club(club_id) {
        // Check if player is loaned IN (contract is Loan type, or transfer record says so)
        let loan_in_record = country.transfer_market.transfer_history.iter().find(|t| {
            t.player_id == player.id
                && t.to_club_id == club_id
                && matches!(&t.transfer_type, TransferType::Loan(_))
        });

        if is_loan_contract || loan_in_record.is_some() {
            if let Some(record) = loan_in_record {
                let club_slug = data.club(record.from_club_id)
                    .and_then(|c| c.teams.teams.first())
                    .map(|t| t.slug.clone())
                    .unwrap_or_default();

                return Some(PlayerLoanDto {
                    is_loan_in: true,
                    club_name: record.from_team_name.clone(),
                    club_slug,
                });
            }
        }

        // Check if player is loaned OUT from this club
        let loan_out_record = country.transfer_market.transfer_history.iter().find(|t| {
            t.player_id == player.id
                && t.from_club_id == club_id
                && matches!(&t.transfer_type, TransferType::Loan(_))
        });

        if let Some(record) = loan_out_record {
            let club_slug = data.club(record.to_club_id)
                .and_then(|c| c.teams.teams.first())
                .map(|t| t.slug.clone())
                .unwrap_or_default();

            return Some(PlayerLoanDto {
                is_loan_in: false,
                club_name: record.to_team_name.clone(),
                club_slug,
            });
        }
    }

    None
}

fn get_position_map(player: &Player) -> PositionMapDto {
    let active = player.positions();
    let primary = player.position().get_short_name().to_string();

    PositionMapDto {
        gk: active.contains(&PlayerPositionType::Goalkeeper),
        sw: active.contains(&PlayerPositionType::Sweeper),
        dl: active.contains(&PlayerPositionType::DefenderLeft),
        dcl: active.contains(&PlayerPositionType::DefenderCenterLeft),
        dc: active.contains(&PlayerPositionType::DefenderCenter),
        dcr: active.contains(&PlayerPositionType::DefenderCenterRight),
        dr: active.contains(&PlayerPositionType::DefenderRight),
        dm: active.contains(&PlayerPositionType::DefensiveMidfielder),
        wl: active.contains(&PlayerPositionType::WingbackLeft),
        wr: active.contains(&PlayerPositionType::WingbackRight),
        ml: active.contains(&PlayerPositionType::MidfielderLeft),
        mcl: active.contains(&PlayerPositionType::MidfielderCenterLeft),
        mc: active.contains(&PlayerPositionType::MidfielderCenter),
        mcr: active.contains(&PlayerPositionType::MidfielderCenterRight),
        mr: active.contains(&PlayerPositionType::MidfielderRight),
        aml: active.contains(&PlayerPositionType::AttackingMidfielderLeft),
        amc: active.contains(&PlayerPositionType::AttackingMidfielderCenter),
        amr: active.contains(&PlayerPositionType::AttackingMidfielderRight),
        fl: active.contains(&PlayerPositionType::ForwardLeft),
        fc: active.contains(&PlayerPositionType::ForwardCenter),
        fr: active.contains(&PlayerPositionType::ForwardRight),
        st: active.contains(&PlayerPositionType::Striker),
        primary,
    }
}
