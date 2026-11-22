use crate::GameAppData;
use crate::about::AboutPageTemplate;
use axum::Router;
use axum::routing::get;

pub struct AboutRoutes;

impl AboutRoutes {
    pub fn routes() -> Router<GameAppData> {
        Router::new().route("/{lang}/about", get(AboutPageTemplate::action))
    }
}
