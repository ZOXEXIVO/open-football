use crate::{ApiError, ApiResult, GameAppData};
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct CountryListViewModel<'c> {
    pub name: &'c str,
    pub countries: Vec<CountryDto<'c>>,
}

#[derive(Serialize)]
pub struct CountryDto<'c> {
    pub slug: &'c str,
    pub code: &'c str,
    pub name: &'c str,
    pub leagues: Vec<LeagueDto<'c>>,
}

#[derive(Serialize)]
pub struct LeagueDto<'l> {
    pub slug: &'l str,
    pub name: &'l str,
}

pub async fn country_list_action(State(state): State<GameAppData>) -> ApiResult<Response> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let mut model = Vec::with_capacity(simulator_data.continents.len());

    for continent in &simulator_data.continents {
        let item = CountryListViewModel {
            name: &continent.name,
            countries: continent
                .countries
                .iter()
                .filter(|c| c.leagues.leagues.len() > 0)
                .map(|country| CountryDto {
                    slug: &country.slug,
                    code: &country.code,
                    name: &country.name,
                    leagues: country
                        .leagues
                        .leagues
                        .iter()
                        .map(|l| LeagueDto {
                            slug: &l.slug,
                            name: &l.name,
                        })
                        .collect(),
                })
                .collect(),
        };

        model.push(item);
    }

    Ok(Json(model).into_response())
}
