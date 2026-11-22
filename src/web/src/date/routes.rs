use crate::GameAppData;
use crate::date::CurrentDateModel;
use axum::Router;
use axum::routing::get;

pub struct CurrentDateRoutes;

impl CurrentDateRoutes {
    pub fn routes() -> Router<GameAppData> {
        Router::new().route("/api/date", get(CurrentDateModel::action))
    }
}
