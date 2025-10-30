use crate::{ApiError, ApiResult, GameAppData};
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use core::Country;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct CountryGetRequest {
    country_slug: String,
}

#[derive(Serialize)]
pub struct CountryGetViewModel<'c> {
    pub slug: &'c str,
    pub name: &'c str,
    pub code: &'c str,
    pub continent_name: &'c str,
    pub leagues: Vec<LeagueDto<'c>>,
}

#[derive(Serialize)]
pub struct LeagueDto<'l> {
    pub slug: &'l str,
    pub name: &'l str,
}

pub async fn country_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<CountryGetRequest>,
) -> ApiResult<Response> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let indexes = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?;

    let country_id = indexes
        .slug_indexes
        .get_country_by_slug(&route_params.country_slug)
        .ok_or_else(|| ApiError::NotFound(format!("Country '{}' not found", route_params.country_slug)))?;

    let country: &Country = simulator_data
        .continents
        .iter()
        .flat_map(|c| &c.countries)
        .find(|country| country.id == country_id)
        .ok_or_else(|| ApiError::NotFound(format!("Country with ID {} not found in continents", country_id)))?;

    let continent = simulator_data
        .continent(country.continent_id)
        .ok_or_else(|| ApiError::NotFound(format!("Continent with ID {} not found", country.continent_id)))?;

    let model = CountryGetViewModel {
        slug: &country.slug,
        name: &country.name,
        code: &country.code,
        continent_name: &continent.name,
        leagues: country
            .leagues
            .leagues
            .iter()
            .map(|l| LeagueDto {
                slug: &l.slug,
                name: &l.name,
            })
            .collect(),
    };

    Ok(Json(model).into_response())
}
