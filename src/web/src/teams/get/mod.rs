pub mod routes;

use crate::player::PlayerStatusDto;
use crate::common::default_handler::{CSS_VERSION, COMPUTER_NAME};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::NaiveDate;
use core::utils::{DateUtils, FormattingUtils};
use core::ContractType;
use core::Player;
use core::PlayerPositionType;
use core::PlayerStatusType;
use core::{SimulatorData, Team};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamGetRequest {
    pub lang: String,
    pub team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/get/index.html")]
pub struct TeamGetTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub i18n: I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub active_tab: &'static str,
    pub show_finances_tab: bool,
    pub show_academy_tab: bool,
    pub players: Vec<TeamPlayer>,
    pub watchlist_ids: Vec<u32>,
}

pub struct TeamPlayer {
    pub id: u32,
    pub last_name: String,
    pub first_name: String,
    pub behaviour: String,
    pub position: String,
    pub position_sort: PlayerPositionType,
    pub value: String,
    pub injured: bool,
    pub unhappy: bool,
    pub transfer_listed: bool,
    pub loan_listed: bool,
    pub is_wanted: bool,
    pub is_loan: bool,
    pub is_loaned_out: bool,
    pub is_youth: bool,
    pub country_slug: String,
    pub country_code: String,
    pub country_name: String,
    pub conditions: u8,
    pub current_ability: u8,
    pub potential_ability: u8,
    pub age: u8,
    pub played: u16,
    pub played_subs: u16,
    pub goals: u16,
    pub average_rating: String,
    pub has_recent_decision: bool,
    #[allow(dead_code)]
    pub status: PlayerStatusDto,
}

pub async fn team_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamGetRequest>,
) -> ApiResult<impl IntoResponse> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let i18n = state.i18n.for_lang(&route_params.lang);

    let indexes = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?;

    let team_id = indexes
        .slug_indexes
        .get_team_by_slug(&route_params.team_slug)
        .ok_or_else(|| ApiError::NotFound(format!("Team '{}' not found", route_params.team_slug)))?;

    let team: &Team = simulator_data
        .team(team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))?;

    let league = team.league_id.and_then(|id| simulator_data.league(id));
    let league_rep = league.map(|l| l.reputation).unwrap_or(0);
    let club_rep = team.reputation.world;

    let now = simulator_data.date.date();

    let _club_id = team.club_id;

    let head_coach = team.staffs.head_coach();
    let staff_judging = head_coach.staff_attributes.knowledge.judging_player_potential;
    let staff_id = head_coach.id;

    let mut players: Vec<TeamPlayer> = team
        .players()
        .iter()
        .filter(|p| !p.statuses.get().contains(&PlayerStatusType::Ret))
        .map(|p| {
            let (country_slug, country_code, country_name) = simulator_data.country(p.country_id)
                .map(|c| (c.slug.clone(), c.code.clone(), c.name.clone()))
                .or_else(|| simulator_data.country_info.get(&p.country_id)
                    .map(|i| (i.slug.clone(), i.code.clone(), i.name.clone())))
                .unwrap_or_default();
            let position = p.positions.display_positions_compact();

            let is_loan = p.is_on_loan();

            let is_youth = p.contract.as_ref()
                .map(|c| c.contract_type == ContractType::Youth)
                .unwrap_or(false);

            let has_recent_decision = has_decision_within_days(p, now, 30);

            TeamPlayer {
                id: p.id,
                first_name: p.full_name.display_first_name().to_string(),
                position_sort: p.position(),
                position,
                behaviour: p.behaviour.as_str().to_string(),
                injured: p.player_attributes.is_injured,
                unhappy: !p.happiness.is_happy(),
                transfer_listed: p.statuses.get().contains(&PlayerStatusType::Lst),
                loan_listed: p.statuses.get().contains(&PlayerStatusType::Loa),
                is_wanted: !simulator_data.clubs_interested_in_player(p.id).is_empty(),
                is_loan,
                is_loaned_out: false,
                is_youth,
                country_slug,
                country_code,
                country_name,
                last_name: p.full_name.display_last_name().to_string(),
                conditions: get_conditions(p),
                value: FormattingUtils::format_money(p.value(now, league_rep, club_rep)),
                current_ability: get_current_ability_stars(p),
                potential_ability: get_potential_ability_stars_by_staff(p, staff_judging, staff_id),
                age: DateUtils::age(p.birth_date, now),
                played: p.statistics.played + p.friendly_statistics.played + p.cup_statistics.played,
                played_subs: p.statistics.played_subs + p.friendly_statistics.played_subs + p.cup_statistics.played_subs,
                goals: p.statistics.goals + p.friendly_statistics.goals + p.cup_statistics.goals,
                average_rating: p.statistics.combined_rating_str(&p.friendly_statistics),
                has_recent_decision,
                status: PlayerStatusDto::new(p.statuses.get()),
            }
        })
        .collect();

    // Find loaned-out players by scanning all clubs for players
    // whose contract_loan has loan_from_team_id == this team
    let team_id = team.id;
    for continent in &simulator_data.continents {
        for country in &continent.countries {
            for club in &country.clubs {
                for team_iter in &club.teams.teams {
                    for player in &team_iter.players.players {
                        let is_loaned_from_this_team = player.contract_loan.as_ref().map(|c| {
                            c.loan_from_team_id == Some(team_id)
                        }).unwrap_or(false);

                        if !is_loaned_from_this_team { continue; }

                        let (country_slug, country_code, country_name) = simulator_data.country(player.country_id)
                            .map(|c| (c.slug.clone(), c.code.clone(), c.name.clone()))
                            .or_else(|| simulator_data.country_info.get(&player.country_id)
                                .map(|i| (i.slug.clone(), i.code.clone(), i.name.clone())))
                            .unwrap_or_default();
                        let position = player.positions.display_positions_compact();

                        players.push(TeamPlayer {
                            id: player.id,
                            first_name: player.full_name.display_first_name().to_string(),
                            position_sort: player.position(),
                            position,
                            behaviour: player.behaviour.as_str().to_string(),
                            injured: player.player_attributes.is_injured,
                            unhappy: !player.happiness.is_happy(),
                            transfer_listed: false,
                            loan_listed: false,
                            is_wanted: false,
                            is_loan: false,
                            is_loaned_out: true,
                            is_youth: false,
                            country_slug,
                            country_code,
                            country_name,
                            last_name: player.full_name.display_last_name().to_string(),
                            conditions: get_conditions(player),
                            value: FormattingUtils::format_money(player.value(now, league_rep, club_rep)),
                            current_ability: get_current_ability_stars(player),
                            potential_ability: get_potential_ability_stars_by_staff(player, staff_judging, staff_id),
                            age: DateUtils::age(player.birth_date, now),
                            played: player.statistics.played + player.friendly_statistics.played + player.cup_statistics.played,
                            played_subs: player.statistics.played_subs + player.friendly_statistics.played_subs + player.cup_statistics.played_subs,
                            goals: player.statistics.goals + player.friendly_statistics.goals + player.cup_statistics.goals,
                            average_rating: player.statistics.combined_rating_str(&player.friendly_statistics),
                            has_recent_decision: has_decision_within_days(player, now, 7),
                            status: PlayerStatusDto::new(player.statuses.get()),
                        });
                    }
                }
            }
        }
    }

    players.sort_by(|a, b| {
        a.position_sort
            .partial_cmp(&b.position_sort)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let (neighbor_teams, country_leagues) = get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
    let league_refs: Vec<(&str, &str)> = country_leagues.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
    let current_path = format!("/{}/teams/{}", &route_params.lang, &team.slug);
    let menu_params = views::MenuParams { i18n: &i18n, lang: &route_params.lang, current_path: &current_path, country_name: cn, country_slug: cs };
    let menu_sections = views::team_menu(&menu_params, &neighbor_refs, &team.slug, &league_refs, team.team_type == core::TeamType::Main);
    let title = team.name.clone();

    let watchlist_ids = simulator_data.watchlist.clone();
    let league_title = league.map(|l| views::league_display_name(l, &i18n, simulator_data)).unwrap_or_default();

    Ok(TeamGetTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        i18n,
        lang: route_params.lang.clone(),
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: league_title,
        sub_title_link: league.map(|l| format!("/{}/leagues/{}", &route_params.lang, &l.slug)).unwrap_or_default(),
        sub_title_country_code: String::new(),
        header_color: simulator_data.club(team.club_id).map(|c| c.colors.background.clone()).unwrap_or_default(),
        foreground_color: simulator_data.club(team.club_id).map(|c| c.colors.foreground.clone()).unwrap_or_default(),
        menu_sections,
        team_slug: team.slug.clone(),
        active_tab: "squad",
        show_finances_tab: team.team_type == core::TeamType::Main || team.team_type == core::TeamType::B,
        show_academy_tab: team.team_type == core::TeamType::Main || team.team_type == core::TeamType::U18,
        players,
        watchlist_ids,
    })
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &I18n,
) -> Result<(Vec<(String, String)>, Vec<(String, String)>), ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let club_name = &club.name;

    let mut teams: Vec<(String, String, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| {
            (format!("{}  |  {}", club_name, i18n.t(team.team_type.as_i18n_key())), team.slug.clone(), team.reputation.world)
        })
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

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
        teams.into_iter().map(|(name, slug, _)| (name, slug)).collect(),
        country_leagues.into_iter().map(|(_, name, slug)| (name, slug)).collect(),
    ))
}

pub fn get_conditions(player: &Player) -> u8 {
    (100f32 * ((player.player_attributes.condition as f32) / 10000.0)) as u8
}

pub fn get_current_ability_stars(player: &Player) -> u8 {
    (5.0f32 * ((player.player_attributes.current_ability as f32) / 200.0)).round() as u8
}

/// Potential ability stars as seen through staff's judging ability.
/// Higher `judging_potential` (1-20) means more accurate assessment.
pub fn get_potential_ability_stars_by_staff(player: &Player, staff_judging: u8, staff_id: u32) -> u8 {
    let raw_stars = 5.0 * (player.player_attributes.potential_ability as f32 / 200.0);
    let accuracy = (staff_judging as f32 / 20.0).clamp(0.0, 1.0);
    let noise_scale = (1.0 - accuracy) * 1.5;

    // Deterministic noise per staff+player pair
    let hash = staff_id
        .wrapping_mul(2654435761)
        .wrapping_add(player.id.wrapping_mul(2246822519));
    let hash = hash ^ (hash >> 16);
    let hash = hash.wrapping_mul(0x45d9f3b);
    let hash = hash ^ (hash >> 16);
    let noise = (hash & 0xFFFF) as f32 / 32768.0 - 1.0;

    (raw_stars + noise * noise_scale).round().clamp(0.0, 5.0) as u8
}

fn has_decision_within_days(player: &Player, now: NaiveDate, days: i64) -> bool {
    player.decision_history.items.iter().any(|d| {
        (now - d.date).num_days() <= days
    })
}
