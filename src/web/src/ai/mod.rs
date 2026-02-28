pub mod providers;
pub mod registry;
pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use registry::AiProviderInfo;
use serde::Deserialize;

pub fn ai_routes() -> axum::Router<GameAppData> {
    routes::routes()
}

#[derive(Deserialize)]
pub struct AiPageRequest {
    pub lang: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "ai/index.html")]
pub struct AiPageTemplate {
    pub css_version: &'static str,
    pub i18n: crate::I18n,
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
    pub providers: Vec<AiProviderInfo>,
    pub total_requests: u64,
    pub total_completed: u64,
    pub provider_count: usize,
}

pub async fn ai_page_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<AiPageRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let current_path = format!("/{}/ai", &route_params.lang);

    let providers = state.ai_registry.list().await;
    let total_requests = state.ai_registry.total_request_count().await;
    let total_completed = state.ai_registry.total_completed_count().await;
    let provider_count = state.ai_registry.provider_count().await;

    let menu_sections = views::ai_menu(&i18n, &route_params.lang, &current_path);

    Ok(AiPageTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        i18n,
        lang: route_params.lang.clone(),
        title: "AI Management".to_string(),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: String::new(),
        sub_title_link: String::new(),
        sub_title_country_code: String::new(),
        header_color: String::new(),
        foreground_color: String::new(),
        menu_sections,
        providers,
        total_requests,
        total_completed,
        provider_count,
    })
}

#[derive(Deserialize)]
pub struct AddProviderRequest {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub model: String,
    pub batch_size: Option<usize>,
}

pub async fn ai_add_provider_action(
    State(state): State<GameAppData>,
    Json(body): Json<AddProviderRequest>,
) -> impl IntoResponse {
    let request = providers::OllamaRequest::new(&body.host, body.port, &body.model)
        .with_batch_size(body.batch_size.unwrap_or(1));

    state.ai_registry.add(
        &body.name,
        Box::new(request),
    ).await;

    StatusCode::OK
}

#[derive(Deserialize)]
pub struct RemoveProviderPath {
    pub provider_id: u64,
}

pub async fn ai_remove_provider_action(
    State(state): State<GameAppData>,
    Path(path): Path<RemoveProviderPath>,
) -> impl IntoResponse {
    let removed = state.ai_registry.remove(path.provider_id).await;
    if removed {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}
