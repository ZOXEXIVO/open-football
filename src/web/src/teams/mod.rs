pub mod academy;
pub mod finances;
pub mod get;
pub mod newspaper;
pub mod relations;
pub mod schedule;
pub mod scouting;
pub mod staff;
pub mod stats;
pub mod tactics;
pub mod transfers;

use crate::views::{self, MenuSection};
use crate::{ApiError, GameAppData, I18n};
use axum::Router;
use core::{SimulatorData, Team};

pub struct TeamRoutes;

impl TeamRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new()
            .merge(get::routes::TeamGetRoutes::router())
            .merge(newspaper::routes::TeamNewspaperRoutes::router())
            .merge(staff::routes::TeamStaffRoutes::router())
            .merge(tactics::routes::TeamTacticsRoutes::router())
            .merge(finances::routes::TeamFinancesRoutes::router())
            .merge(relations::routes::TeamRelationsRoutes::router())
            .merge(scouting::routes::TeamScoutingRoutes::router())
            .merge(academy::routes::TeamAcademyRoutes::router())
            .merge(schedule::routes::TeamScheduleRoutes::router())
            .merge(stats::routes::TeamStatsRoutes::router())
            .merge(transfers::routes::TeamTransfersRoutes::router())
    }
}

/// Kept as a callable alias for the site router while the route modules are
/// migrated to their owning types.
#[allow(non_upper_case_globals)]
pub const team_routes: fn() -> Router<GameAppData> = TeamRoutes::router;

pub struct TeamLookup;

impl TeamLookup {
    pub fn by_slug<'a>(data: &'a SimulatorData, slug: &str) -> Result<&'a Team, ApiError> {
        let indexes = data
            .indexes
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?;

        let team_id = indexes
            .slug_indexes
            .get_team_by_slug(slug)
            .ok_or_else(|| ApiError::NotFound(format!("Team '{}' not found", slug)))?;

        data.team(team_id)
            .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))
    }
}

/// The club's other squads and its country's leagues, as the team left menu
/// needs them.
pub(crate) struct TeamNeighborhood {
    pub(crate) teams: Vec<(String, String)>,
    pub(crate) leagues: Vec<(String, String)>,
}

impl TeamNeighborhood {
    pub(crate) fn for_club(
        club_id: u32,
        data: &SimulatorData,
        i18n: &I18n,
    ) -> Result<Self, ApiError> {
        let club = data.club(club_id).ok_or_else(|| {
            ApiError::InternalError(format!("Club with ID {} not found", club_id))
        })?;

        let teams = views::Menus::neighbor_teams(club, i18n);

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

        Ok(TeamNeighborhood {
            teams,
            leagues: country_leagues
                .into_iter()
                .map(|(_, name, slug)| (name, slug))
                .collect(),
        })
    }

    pub(crate) fn menu_sections(
        &self,
        data: &SimulatorData,
        club_id: u32,
        i18n: &I18n,
        lang: &str,
        current_path: &str,
    ) -> Vec<MenuSection> {
        let neighbor_refs: Vec<(&str, &str)> = self
            .teams
            .iter()
            .map(|(name, slug)| (name.as_str(), slug.as_str()))
            .collect();
        let league_refs: Vec<(&str, &str)> = self
            .leagues
            .iter()
            .map(|(name, slug)| (name.as_str(), slug.as_str()))
            .collect();

        let (country_name, country_slug) = views::Menus::club_country_info(data, club_id);
        let menu_params = views::MenuParams {
            i18n,
            lang,
            current_path,
            country_name,
            country_slug,
        };

        views::Menus::team_menu(&menu_params, &neighbor_refs, &league_refs)
    }
}
