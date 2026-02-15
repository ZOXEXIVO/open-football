use crate::countries::country_routes;
use crate::date::current_date_routes;
use crate::game::game_routes;
use crate::leagues::league_routes;
use crate::player::player_routes;
use crate::r#match::routes::match_routes;
use crate::teams::team_routes;
use crate::GameAppData;
use axum::Router;
use tower_http::services::ServeDir;

pub struct ServerRoutes;

impl ServerRoutes {
    pub fn create() -> Router<GameAppData> {
        #[cfg(debug_assertions)]
        let static_dir = "src/server/static";

        #[cfg(not(debug_assertions))]
        let static_dir = "static";

        Router::<GameAppData>::new()
            .merge(country_routes())
            .merge(game_routes())
            .merge(league_routes())
            .merge(team_routes())
            .merge(player_routes())
            .merge(match_routes())
            .merge(current_date_routes())
            .nest_service("/static", ServeDir::new(static_dir))
    }
}
