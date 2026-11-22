pub mod free_agents;
pub mod get;
pub mod list;
pub mod schedule;
pub mod squad;
pub mod staff;

use crate::views::{self, MenuSection};
use crate::{ApiError, GameAppData, I18n};
use axum::Router;
use axum::extract::{Path, Request};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use core::continent::Continent;
use core::shared::fullname::FullName;
use core::{Country, SimulatorData};
use std::collections::HashMap;

pub struct CountryRoutes;

impl CountryRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new()
            .merge(list::routes::CountryListRoutes::router())
            .merge(get::routes::CountryGetRoutes::router())
            .merge(squad::routes::CountrySquadRoutes::router())
            .merge(staff::routes::CountryStaffRoutes::router())
            .merge(schedule::routes::CountryScheduleRoutes::router())
            .merge(free_agents::routes::CountryFreeAgentsRoutes::router())
            .route_layer(middleware::from_fn(CountrySlug::canonical))
    }
}

/// Kept as a callable alias for the site router while the route modules are
/// migrated to their owning types.
#[allow(non_upper_case_globals)]
pub const country_routes: fn() -> Router<GameAppData> = CountryRoutes::router;

pub(crate) struct CountryLookup;

impl CountryLookup {
    pub(crate) fn by_slug<'a>(
        data: &'a SimulatorData,
        slug: &str,
    ) -> Result<(&'a Country, &'a Continent), ApiError> {
        let indexes = data
            .indexes
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?;

        let country_id = indexes
            .slug_indexes
            .get_country_by_slug(slug)
            .ok_or_else(|| ApiError::NotFound(format!("Country '{}' not found", slug)))?;

        let country: &Country = data
            .continents
            .iter()
            .flat_map(|c| &c.countries)
            .find(|country| country.id == country_id)
            .ok_or_else(|| {
                ApiError::NotFound(format!(
                    "Country with ID {} not found in continents",
                    country_id
                ))
            })?;

        let continent = data.continent(country.continent_id).ok_or_else(|| {
            ApiError::NotFound(format!(
                "Continent with ID {} not found",
                country.continent_id
            ))
        })?;

        Ok((country, continent))
    }
}

pub(crate) struct CountryMenu;

impl CountryMenu {
    pub(crate) fn sections(
        country: &Country,
        i18n: &I18n,
        lang: &str,
        country_slug: &str,
        current_path: &str,
    ) -> Vec<MenuSection> {
        let leagues: Vec<(&str, &str)> = country
            .leagues
            .leagues
            .iter()
            .filter(|l| !l.friendly)
            .map(|l| (l.name.as_str(), l.slug.as_str()))
            .collect();
        let cup = country
            .domestic_cup
            .as_ref()
            .map(|c| (c.league.name.as_str(), c.league.slug.as_str()));
        let playoffs: Vec<(&str, &str)> = country
            .playoffs
            .iter()
            .map(|p| (p.league.name.as_str(), p.league.slug.as_str()))
            .collect();
        let menu_params = views::MenuParams {
            i18n,
            lang,
            current_path,
            country_name: &country.name,
            country_slug,
        };

        views::Menus::country_menu(&menu_params, &leagues, cup, &playoffs, country.continent_id)
    }
}

/// Country slugs are folded at load ("south africa" → "south-africa"), so a
/// `{country_slug}` the fold would still change comes from a pre-fold URL.
struct CountrySlug;

impl CountrySlug {
    /// Moves such a URL permanently to the folded slug, tab and query intact.
    /// Built by hand because `Redirect::permanent` is a 308, and 301 is the
    /// status browsers and crawlers cache as a permanent move.
    async fn canonical(
        Path(params): Path<HashMap<String, String>>,
        request: Request,
        next: Next,
    ) -> Response {
        let Some(slug) = params.get("country_slug") else {
            return next.run(request).await;
        };
        let folded = FullName::slug_from_display(slug);
        if folded == *slug {
            return next.run(request).await;
        }

        Self::moved(request.uri().path(), request.uri().query(), &folded)
    }

    /// Every country route is `/{lang}/countries/{country_slug}…`, so the
    /// slug is the fourth `/`-separated piece of the path.
    fn moved(path: &str, query: Option<&str>, folded: &str) -> Response {
        let mut segments: Vec<&str> = path.split('/').collect();
        segments[3] = folded;
        let mut url = segments.join("/");
        if let Some(query) = query {
            url.push('?');
            url.push_str(query);
        }
        (StatusCode::MOVED_PERMANENTLY, [(header::LOCATION, url)]).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pre_fold_country_url_moves_to_the_folded_slug_on_the_same_tab() {
        let response = CountrySlug::moved(
            "/en/countries/south%20africa/u21/schedule",
            Some("season=2025"),
            "south-africa",
        );

        assert_eq!(response.status(), StatusCode::MOVED_PERMANENTLY);
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .expect("a redirect must carry a Location"),
            "/en/countries/south-africa/u21/schedule?season=2025"
        );
    }
}
