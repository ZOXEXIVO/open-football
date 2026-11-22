pub mod ai_report;
pub mod routes;

use crate::leagues::address::LeagueAddress;
use crate::player::PlayerStatusDto;
use crate::shared::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::shared::potential_stars::{PotentialStarsView, StarRating};
use crate::teams::newspaper::NewspaperCounter;
use crate::teams::{TeamLookup, TeamNeighborhood};
use crate::views::{self, MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::ContractType;
use core::Player;
use core::PlayerPositionType;
use core::PlayerStatusType;
use core::utils::{DateUtils, FormattingUtils};
use core::{SimulatorData, Team, TeamType};
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
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub i18n: I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_flag: Option<SubTitleFlag>,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    /// Numeric club id — the AI team-report button posts this to the agent.
    pub club_id: u32,
    /// Gates the AI report button: true only on the Main team page when an
    /// LLM contract is configured (hidden on B / reserve / youth squads).
    pub ai_enabled: bool,
    pub active_tab: &'static str,
    pub show_finances_tab: bool,
    pub show_academy_tab: bool,
    /// Printed items waiting on the newspaper tab, for the tabbar badge.
    pub newspaper_count: usize,
    pub players: Vec<TeamPlayer>,
}

pub struct TeamPlayer {
    pub id: u32,
    pub slug: String,
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
    pub is_force_match_selection: bool,
    pub country_slug: String,
    pub country_code: String,
    pub country_name: String,
    pub conditions: u8,
    pub current_ability: StarRating,
    pub potential_ability: StarRating,
    pub age: u8,
    pub played: u16,
    pub played_subs: u16,
    pub goals: u16,
    pub average_rating: String,
    pub is_captain: bool,
    pub is_vice_captain: bool,
    #[allow(dead_code)]
    pub status: PlayerStatusDto,
}

pub struct TeamGetPage;

impl TeamGetPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<TeamGetRequest>,
    ) -> ApiResult<impl IntoResponse> {
        let guard = state.data.read().await;

        let simulator_data = guard
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let i18n = state.i18n.for_lang(&route_params.lang);
        let team: &Team = TeamLookup::by_slug(simulator_data, &route_params.team_slug)?;

        // The AI team report is a club-level feature surfaced once, on the Main
        // team page only — not on B / reserve / youth (U18…) squads.
        let ai_enabled = team.team_type == TeamType::Main && state.ai.is_configured().await;

        Self::template(simulator_data, team, i18n, ai_enabled, &route_params)
    }

    fn template(
        data: &SimulatorData,
        team: &Team,
        i18n: I18n,
        ai_enabled: bool,
        route_params: &TeamGetRequest,
    ) -> Result<TeamGetTemplate, ApiError> {
        let league = team.league_id.and_then(|id| data.league(id));
        let league_rep = league.map(|l| l.reputation).unwrap_or(0);
        let club_rep = team.reputation.market_value_score();
        let now = data.date.date();

        // Potential stars are this coach's own read of each player's
        // ceiling — the viewer has no club, so every roster shows what its
        // coach believes, mistakes included.
        let head_coach = team.staffs.head_coach();
        let players = Self::players(data, team, now, league_rep, club_rep, head_coach);

        let current_path = format!("/{}/teams/{}", route_params.lang, team.slug);
        let neighborhood = TeamNeighborhood::for_club(team.club_id, data, &i18n)?;
        let menu_sections = neighborhood.menu_sections(
            data,
            team.club_id,
            &i18n,
            &route_params.lang,
            &current_path,
        );
        let title = team.name.clone();
        let league_title = league
            .map(|l| views::Menus::league_display_name(l, &i18n, data))
            .unwrap_or_default();

        Ok(TeamGetTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            i18n,
            lang: route_params.lang.clone(),
            title,
            sub_title_prefix: String::new(),
            sub_title_suffix: String::new(),
            sub_title: league_title,
            sub_title_link: league
                .map(|l| LeagueAddress::of(data, l).url(&route_params.lang))
                .unwrap_or_default(),
            sub_title_flag: None,
            header_color: data
                .club(team.club_id)
                .map(|c| c.colors.background.clone())
                .unwrap_or_default(),
            foreground_color: data
                .club(team.club_id)
                .map(|c| c.colors.foreground.clone())
                .unwrap_or_default(),
            menu_sections,
            team_slug: team.slug.clone(),
            club_id: team.club_id,
            ai_enabled,
            active_tab: "squad",
            show_finances_tab: team.team_type.is_own_team(),
            show_academy_tab: team.team_type == TeamType::Main || team.team_type == TeamType::U18,
            newspaper_count: NewspaperCounter::count(data, team),
            players,
        })
    }

    fn players(
        data: &SimulatorData,
        team: &Team,
        now: chrono::NaiveDate,
        league_rep: u16,
        club_rep: u16,
        head_coach: &core::Staff,
    ) -> Vec<TeamPlayer> {
        let mut players = Self::squad_players(data, team, now, league_rep, club_rep, head_coach);
        players.extend(Self::loaned_out_players(
            data, team, now, league_rep, club_rep, head_coach,
        ));

        players.sort_by(|a, b| {
            a.position_sort
                .partial_cmp(&b.position_sort)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        players
    }

    fn squad_players(
        data: &SimulatorData,
        team: &Team,
        now: chrono::NaiveDate,
        league_rep: u16,
        club_rep: u16,
        head_coach: &core::Staff,
    ) -> Vec<TeamPlayer> {
        let captain_id = team.captain_id;
        let vice_captain_id = team.vice_captain_id;

        team.players()
            .iter()
            .filter(|p| !p.statuses.get().contains(&PlayerStatusType::Ret))
            .map(|p| {
                let (country_slug, country_code, country_name) =
                    Self::country_info(data, p.country_id);
                let position = p.positions.display_positions_compact();
                let is_loan = p.is_on_loan();
                let is_youth = p
                    .contract
                    .as_ref()
                    .map(|c| c.contract_type == ContractType::Youth)
                    .unwrap_or(false);

                // Apps, goals and rating all read from one all-competition
                // (league + cup + friendly) season sample so the columns agree.
                let season_stats = p.current_season_all_statistics();

                TeamPlayer {
                    id: p.id,
                    slug: p.slug(),
                    first_name: p.full_name.display_first_name().to_string(),
                    position_sort: p.position(),
                    position,
                    behaviour: p.behaviour.as_str().to_string(),
                    injured: p.player_attributes.is_injured,
                    unhappy: !p.happiness.is_happy(),
                    transfer_listed: p.statuses.get().contains(&PlayerStatusType::Lst),
                    loan_listed: p.statuses.get().contains(&PlayerStatusType::Loa),
                    is_wanted: !data.clubs_interested_in_player(p.id).is_empty(),
                    is_loan,
                    is_loaned_out: false,
                    is_youth,
                    is_force_match_selection: p.is_force_match_selection,
                    country_slug,
                    country_code,
                    country_name,
                    last_name: p.full_name.display_last_name().to_string(),
                    conditions: TeamPlayer::condition_percent(p),
                    value: FormattingUtils::format_money(p.value(now, league_rep, club_rep)),
                    current_ability: PotentialStarsView::current(p),
                    potential_ability: PotentialStarsView::potential_by_staff(
                        p,
                        head_coach,
                        team.team_type == TeamType::Main,
                        now,
                    ),
                    age: DateUtils::age(p.birth_date, now),
                    played: season_stats.played,
                    played_subs: season_stats.played_subs,
                    goals: season_stats.goals,
                    average_rating: season_stats.display_average_rating(),
                    is_captain: captain_id == Some(p.id),
                    is_vice_captain: vice_captain_id == Some(p.id),
                    status: PlayerStatusDto::new(p.statuses.get()),
                }
            })
            .collect()
    }

    // Find loaned-out players by scanning all clubs for players
    // whose contract_loan has loan_from_team_id == this team
    fn loaned_out_players(
        data: &SimulatorData,
        team: &Team,
        now: chrono::NaiveDate,
        league_rep: u16,
        club_rep: u16,
        head_coach: &core::Staff,
    ) -> Vec<TeamPlayer> {
        let team_id = team.id;
        let mut players = Vec::new();

        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for other_team in &club.teams.teams {
                        for player in &other_team.players.players {
                            let is_loaned_from_this_team = player
                                .contract_loan
                                .as_ref()
                                .map(|c| c.loan_from_team_id == Some(team_id))
                                .unwrap_or(false);

                            if !is_loaned_from_this_team {
                                continue;
                            }

                            players.push(Self::loaned_out_player(
                                data, player, now, league_rep, club_rep, head_coach,
                            ));
                        }
                    }
                }
            }
        }

        players
    }

    fn loaned_out_player(
        data: &SimulatorData,
        player: &core::Player,
        now: chrono::NaiveDate,
        league_rep: u16,
        club_rep: u16,
        head_coach: &core::Staff,
    ) -> TeamPlayer {
        let (country_slug, country_code, country_name) =
            Self::country_info(data, player.country_id);
        let position = player.positions.display_positions_compact();

        // Same all-competition season sample as the squad block above, so
        // loaned-out rows are consistent too.
        let season_stats = player.current_season_all_statistics();

        TeamPlayer {
            id: player.id,
            slug: player.slug(),
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
            is_force_match_selection: player.is_force_match_selection,
            country_slug,
            country_code,
            country_name,
            last_name: player.full_name.display_last_name().to_string(),
            conditions: TeamPlayer::condition_percent(player),
            value: FormattingUtils::format_money(player.value(now, league_rep, club_rep)),
            // Loaned-out: the parent's coach assesses a player who trains
            // elsewhere — reduced visibility, not a daily read.
            current_ability: PotentialStarsView::current(player),
            potential_ability: PotentialStarsView::potential_by_staff(
                player, head_coach, false, now,
            ),
            age: DateUtils::age(player.birth_date, now),
            played: season_stats.played,
            played_subs: season_stats.played_subs,
            goals: season_stats.goals,
            average_rating: season_stats.display_average_rating(),
            is_captain: false,
            is_vice_captain: false,
            status: PlayerStatusDto::new(player.statuses.get()),
        }
    }

    fn country_info(data: &SimulatorData, country_id: u32) -> (String, String, String) {
        data.country(country_id)
            .map(|c| (c.slug.clone(), c.code.clone(), c.name.clone()))
            .or_else(|| {
                data.country_info
                    .get(&country_id)
                    .map(|i| (i.slug.clone(), i.code.clone(), i.name.clone()))
            })
            .unwrap_or_default()
    }
}

impl TeamPlayer {
    fn condition_percent(player: &Player) -> u8 {
        (100f32 * ((player.player_attributes.condition as f32) / 10000.0)) as u8
    }
}
