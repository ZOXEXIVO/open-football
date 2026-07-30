pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection};
use crate::{GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use serde::Deserialize;

pub fn about_routes() -> axum::Router<GameAppData> {
    routes::routes()
}

#[derive(Deserialize)]
pub struct AboutPageRequest {
    pub lang: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "about/index.html")]
pub struct AboutPageTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub version: &'static str,
    pub i18n: I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
}

/// The one page that renders without touching the simulator: it explains
/// the project rather than reporting on the world, so it stays readable
/// even before a world exists.
pub async fn about_page_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<AboutPageRequest>,
) -> impl IntoResponse {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let current_path = format!("/{}/about", &route_params.lang);
    let menu_sections = views::about_menu(&i18n, &route_params.lang, &current_path);
    let title = i18n.t("about").to_string();

    AboutPageTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        version: env!("CARGO_PKG_VERSION"),
        i18n,
        lang: route_params.lang.clone(),
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: String::new(),
        sub_title_link: String::new(),
        sub_title_country_code: String::new(),
        header_color: String::new(),
        foreground_color: String::new(),
        menu_sections,
    }
}
