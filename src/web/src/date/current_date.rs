use crate::GameAppData;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::Serialize;

#[derive(Serialize)]
pub struct CurrentDateModel {
    pub date: String,
    pub time: String,
}

pub async fn current_date_action(State(state): State<GameAppData>) -> Response {
    let data = state.data.read().await;

    let date = match data.as_ref() {
        None => Utc::now().naive_utc(),
        Some(data) => data.date,
    };

    let model = CurrentDateModel {
        date: date.format("%d %b %Y").to_string(),
        time: date.format("%a %R").to_string(),
    };

    Json(model).into_response()
}
