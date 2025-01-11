use crate::GameAppData;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use core::player::Player;
use core::player::PlayerStatusType;
use core::utils::FormattingUtils;
use core::SimulatorData;
use core::Team;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use core::Person;

#[derive(Deserialize)]
pub struct PlayerGetRequest {
    pub team_slug: String,
    pub player_id: u32,
}

#[derive(Serialize)]
pub struct PlayerGetViewModel<'p> {
    pub id: u32,
    pub first_name: &'p str,
    pub last_name: &'p str,
    pub middle_name: Option<&'p str>,
    pub position: &'p str,
    pub contract: Option<PlayerContractDto>,
    pub birth_date: String,
    pub age: u8,
    pub team_slug: &'p str,
    pub team_name: &'p str,
    pub country_slug: &'p str,
    pub country_code: &'p str,
    pub country_name: &'p str,
    pub skills: PlayerSkillsDto,
    pub conditions: u8,
    pub current_ability: u8,
    pub potential_ability: u8,

    pub value: &'p str,

    pub preferred_foot: &'p str,

    pub player_attributes: PlayerAttributesDto,

    pub neighbor_teams: Vec<ClubTeam<'p>>,

    pub statistics: PlayerStatistics,

    pub status: PlayerStatusDto,
}

#[derive(Serialize)]
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

#[derive(Serialize)]
pub struct ClubTeam<'c> {
    pub name: &'c str,
    pub slug: &'c str,
    pub reputation: u16,
}

#[derive(Serialize)]
pub struct PlayerContractDto {
    pub salary: u32,
    pub expiration: String,
    pub squad_status: String,
}

#[derive(Serialize)]
pub struct PlayerSkillsDto {
    pub technical: TechnicalDto,
    pub mental: MentalDto,
    pub physical: PhysicalDto,
}

#[derive(Serialize)]
pub struct TechnicalDto {
    pub corners: u8,
    pub crossing: u8,
    pub dribbling: u8,
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

#[derive(Serialize)]
pub struct MentalDto {
    pub aggression: u8,
    pub anticipation: u8,
    pub bravery: u8,
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

#[derive(Serialize)]
pub struct PhysicalDto {
    pub acceleration: u8,
    pub agility: u8,
    pub balance: u8,
    pub jumping_reach: u8,
    pub natural_fitness: u8,
    pub pace: u8,
    pub stamina: u8,
    pub strength: u8,

    pub match_readiness: u8,
}

#[derive(Serialize)]
pub struct PlayerAttributesDto {
    pub international_apps: u16,
    pub international_goals: u16,

    pub under_21_international_apps: u16,
    pub under_21_international_goals: u16,
}

#[derive(Serialize)]
pub struct PlayerStatusDto {
    pub statuses: Vec<PlayerStatusType>,
}

impl PlayerStatusDto {
    pub fn new(statuses: Vec<PlayerStatusType>) -> Self {
        PlayerStatusDto { statuses }
    }

    pub fn is_wanted(&self) -> bool {
        self.statuses.iter().contains(&PlayerStatusType::Wnt)
    }
}

pub async fn player_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<PlayerGetRequest>,
) -> Response {
    let guard = state.data.read().await;

    let simulator_data = guard.as_ref().unwrap();

    let team_id = simulator_data
        .indexes
        .as_ref()
        .unwrap()
        .slug_indexes
        .get_team_by_slug(&route_params.team_slug)
        .unwrap();

    let team: &Team = simulator_data.team(team_id).unwrap();

    let player: &Player = team
        .players
        .players()
        .iter()
        .find(|p| p.id == route_params.player_id)
        .unwrap();

    let country = simulator_data.country(player.country_id).unwrap();

    let now = simulator_data.date.date();

    let mut model = PlayerGetViewModel {
        id: player.id,
        first_name: &player.full_name.first_name,
        last_name: &player.full_name.last_name,
        middle_name: player.full_name.middle_name.as_deref(),
        position: player.position().get_short_name(),
        contract: None,
        birth_date: player.birth_date.format("%d.%m.%Y").to_string(),
        age: player.age(simulator_data.date.date()),
        team_slug: &team.slug,
        team_name: &team.name,
        country_slug: &country.slug,
        country_code: &country.code,
        country_name: &country.name,
        skills: get_skills(player),
        conditions: get_conditions(player),
        current_ability: get_current_ability_stars(player),
        potential_ability: get_potential_ability_stars(player),
        value: &FormattingUtils::format_money(player.value(now)),
        preferred_foot: player.preferred_foot_str(),
        player_attributes: get_attributes(player),
        neighbor_teams: get_neighbor_teams(team.club_id, simulator_data),
        statistics: get_statistics(player),
        status: PlayerStatusDto::new(player.statuses.get()),
    };

    if let Some(contract) = &player.contract {
        model.contract = Some(PlayerContractDto {
            salary: (contract.salary / 1000),
            expiration: contract.expiration.format("%d.%m.%Y").to_string(),
            squad_status: String::from("First team player"),
        });
    }

    Json(model).into_response()
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
            dribbling: player.skills.technical.dribbling.floor() as u8,
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
            bravery: player.skills.mental.bravery.floor() as u8,
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
            balance: player.skills.physical.balance.floor() as u8,
            jumping_reach: player.skills.physical.jumping.floor() as u8,
            natural_fitness: player.skills.physical.natural_fitness.floor() as u8,
            pace: player.skills.physical.pace.floor() as u8,
            stamina: player.skills.physical.stamina.floor() as u8,
            strength: player.skills.physical.strength.floor() as u8,
            match_readiness: player.skills.physical.match_readiness.floor() as u8,
        },
    }
}

fn get_neighbor_teams(club_id: u32, data: &SimulatorData) -> Vec<ClubTeam> {
    let club = data.club(club_id).unwrap();

    let mut teams: Vec<ClubTeam> = club
        .teams
        .teams
        .iter()
        .map(|team| ClubTeam {
            name: &team.name,
            slug: &team.slug,
            reputation: team.reputation.world,
        })
        .collect();

    teams.sort_by(|a, b| b.reputation.cmp(&a.reputation));

    teams
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
