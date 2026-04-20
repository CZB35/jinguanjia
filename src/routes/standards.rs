use anyhow::Context;
use axum::{
    Json,
    extract::{Path, State},
};
use rbs::value;
use serde::Serialize;

use crate::{
    db,
    error::AppError,
    models::service_standard::ServiceStandardEntity,
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Serialize)]
pub struct StandardResponse {
    pub service_type: String,
    pub title: String,
    pub content: Option<String>,
    pub example_file_url: Option<String>,
    pub version_no: Option<String>,
}

pub async fn get_standard_by_service_type(
    State(state): State<AppState>,
    Path(service_type): Path<String>,
) -> Result<Json<ApiResponse<StandardResponse>>, AppError> {
    let standard: ServiceStandardEntity = db::select_one_by_map_compat(
        state.rb.as_ref(),
        "service_standard",
        value! { "service_type": service_type.as_str(), "status": "enabled" },
    )
    .await
    .context("failed to query service standard")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::NotFound("服务标准不存在".to_string()))?;

    Ok(Json(ApiResponse::success(StandardResponse {
        service_type: standard.service_type.unwrap_or_default(),
        title: standard.title.unwrap_or_default(),
        content: standard.content,
        example_file_url: standard.example_file_url,
        version_no: standard.version_no,
    })))
}
