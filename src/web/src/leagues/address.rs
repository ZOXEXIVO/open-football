use crate::{ApiError, ApiResult, GameAppData};
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use core::league::League;
use core::{Country, SimulatorData};
use serde::Deserialize;

/// Where a league's pages live: `/{lang}/leagues/{country_slug}/{league_slug}`.
/// A league slug is unique on its own, so the country segment is checked
/// rather than trusted: a league reached under the wrong country, or by its
/// pre-country URL, moves permanently to its own address.
pub struct LeagueAddress<'a> {
    country_slug: &'a str,
    league_slug: &'a str,
}

pub enum LeaguePage<'a> {
    Found(&'a League, &'a Country),
    Moved(Response),
}

impl<'a> LeagueAddress<'a> {
    pub fn new(country_slug: &'a str, league_slug: &'a str) -> Self {
        LeagueAddress {
            country_slug,
            league_slug,
        }
    }

    pub fn of(data: &'a SimulatorData, league: &'a League) -> Self {
        let country = data
            .country(league.country_id)
            .expect("every league is run by a country");
        Self::new(&country.slug, &league.slug)
    }

    /// For records that kept only the slug. `None` once no league answers
    /// to it.
    pub fn by_slug(data: &'a SimulatorData, league_slug: &str) -> Option<Self> {
        Self::find(data, league_slug).map(|league| Self::of(data, league))
    }

    /// The address without its language prefix, for templates that add it.
    pub fn path(&self) -> String {
        format!("/leagues/{}/{}", self.country_slug, self.league_slug)
    }

    pub fn url(&self, lang: &str) -> String {
        format!("/{}{}", lang, self.path())
    }

    /// `tab` is the requesting page's suffix (`""` for the overview), so a
    /// move lands on the same tab.
    pub fn resolve(
        data: &'a SimulatorData,
        lang: &str,
        country_slug: &str,
        league_slug: &str,
        tab: &str,
    ) -> ApiResult<LeaguePage<'a>> {
        let league = Self::find(data, league_slug)
            .ok_or_else(|| ApiError::NotFound(format!("League '{}' not found", league_slug)))?;
        let country = data.country(league.country_id).ok_or_else(|| {
            ApiError::NotFound(format!("Country with ID {} not found", league.country_id))
        })?;

        if country.slug != country_slug {
            let address = Self::new(&country.slug, &league.slug);
            return Ok(LeaguePage::Moved(address.moved(lang, tab)));
        }

        Ok(LeaguePage::Found(league, country))
    }

    fn find(data: &'a SimulatorData, league_slug: &str) -> Option<&'a League> {
        data.indexes
            .as_ref()?
            .slug_indexes
            .get_league_by_slug(league_slug)
            .and_then(|id| data.league(id))
    }

    /// Built by hand because `Redirect::permanent` is a 308, and 301 is the
    /// status browsers and crawlers cache as a permanent move.
    fn moved(&self, lang: &str, rest: &str) -> Response {
        let url = format!("{}{}", self.url(lang), rest);
        (StatusCode::MOVED_PERMANENTLY, [(header::LOCATION, url)]).into_response()
    }
}

#[derive(Deserialize)]
pub struct LeagueMovedRequest {
    pub lang: String,
    pub league_slug: String,
}

/// Pre-country league URLs: `/{lang}/leagues/{league_slug}` and its tabs.
pub async fn league_moved_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<LeagueMovedRequest>,
    uri: Uri,
) -> ApiResult<Response> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let address =
        LeagueAddress::by_slug(simulator_data, &route_params.league_slug).ok_or_else(|| {
            ApiError::NotFound(format!("League '{}' not found", route_params.league_slug))
        })?;

    // Whatever followed the league segment — the tab and the query — carries
    // over to the new address.
    let tab = uri
        .path()
        .splitn(5, '/')
        .nth(4)
        .map(|tab| format!("/{}", tab))
        .unwrap_or_default();
    let query = uri
        .query()
        .map(|query| format!("?{}", query))
        .unwrap_or_default();

    Ok(address.moved(&route_params.lang, &format!("{}{}", tab, query)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_league_lives_under_its_country() {
        let address = LeagueAddress::new("france", "ligue-1");

        assert_eq!(address.url("en"), "/en/leagues/france/ligue-1");
        assert_eq!(address.path(), "/leagues/france/ligue-1");
    }

    /// 301, not the 308 `Redirect::permanent` answers: it is the status
    /// search engines carry a page's standing across.
    #[test]
    fn a_move_is_a_301_onto_the_same_tab() {
        let response =
            LeagueAddress::new("france", "ligue-1").moved("en", "/transfers?season=2025");

        assert_eq!(response.status(), StatusCode::MOVED_PERMANENTLY);
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .expect("a redirect must carry a Location"),
            "/en/leagues/france/ligue-1/transfers?season=2025"
        );
    }

    /// The legacy tab routes and the country routes share a depth; the
    /// router rejects an ambiguous pair at registration, not at request.
    #[test]
    fn the_site_router_accepts_the_legacy_and_country_routes_together() {
        let _ = crate::routes::ServerRoutes::create();
    }
}
