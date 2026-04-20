use axum::{Json, extract::State};
use chrono::Utc;
use serde::Serialize;

use crate::{response::ApiResponse, state::AppState};

#[derive(Debug, Serialize)]
pub struct HealthPayload {
    pub app_name: String,
    pub version: String,
    pub now: String,
    pub db_driver: String,
}

pub async fn health_check(State(state): State<AppState>) -> Json<ApiResponse<HealthPayload>> {
    let db_driver = state
        .rb
        .driver_type()
        .unwrap_or("mysql")
        .to_string();

    Json(ApiResponse::success(HealthPayload {
        app_name: state.config.app.name,
        version: state.config.app.version,
        now: Utc::now().to_rfc3339(),
        db_driver,
    }))
}
