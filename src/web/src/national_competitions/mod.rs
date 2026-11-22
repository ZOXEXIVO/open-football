pub mod get;

use crate::GameAppData;
use axum::Router;

pub struct NationalCompetitionsRoutes;

impl NationalCompetitionsRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().merge(get::routes::NationalCompetitionsGetRoutes::router())
    }
}

#[allow(non_upper_case_globals)]
pub const national_competitions_routes: fn() -> Router<GameAppData> =
    NationalCompetitionsRoutes::router;
