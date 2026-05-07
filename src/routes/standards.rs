use anyhow::Context;
use axum::{
    Json,
    extract::{Path, State},
};
use rbs::value;
use serde::Serialize;

use crate::{
    db, error::AppError,
    models::{service_standard::ServiceStandardEntity, system_config::SystemConfigEntity},
    response::ApiResponse, state::AppState,
};

#[derive(Debug, Serialize)]
pub struct StandardResponse {
    pub service_type: String,
    pub title: String,
    pub content: Option<String>,
    pub example_file_url: Option<String>,
    pub warm_tip_content: Option<String>,
    pub version_no: Option<String>,
}

pub async fn get_standard_by_service_type(
    State(state): State<AppState>,
    Path(service_type): Path<String>,
) -> Result<Json<ApiResponse<StandardResponse>>, AppError> {
    if let Some(project_setting) = load_project_setting(&state, &service_type).await? {
        return Ok(Json(ApiResponse::success(StandardResponse {
            service_type: project_setting.service_type,
            title: project_setting.title,
            content: Some(project_setting.requirement_content),
            example_file_url: project_setting.banner_image_url,
            warm_tip_content: Some(project_setting.warm_tip_content),
            version_no: Some("project_setting".to_string()),
        })));
    }

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
        warm_tip_content: None,
        version_no: standard.version_no,
    })))
}

#[derive(Debug)]
struct ProjectSettingPayload {
    service_type: String,
    title: String,
    banner_image_url: Option<String>,
    requirement_content: String,
    warm_tip_content: String,
}

async fn load_project_setting(
    state: &AppState,
    service_type: &str,
) -> Result<Option<ProjectSettingPayload>, AppError> {
    let config_key = format!("project_setting:{}", normalize_service_type(service_type));
    let config = db::select_one_by_map_compat::<SystemConfigEntity>(
        state.rb.as_ref(),
        "system_config",
        value! { "config_key": config_key.as_str() },
    )
    .await
    .context("failed to query project setting for standard")
    .map_err(AppError::Internal)?;
    let Some(config) = config else {
        return Ok(None);
    };
    let Some(raw) = config.config_value else {
        return Ok(None);
    };
    let parsed = serde_json::from_str::<serde_json::Value>(&raw)
        .context("failed to parse project setting config")
        .map_err(AppError::Internal)?;
    if parsed
        .get("status")
        .and_then(|item| item.as_str())
        .unwrap_or("enabled")
        != "enabled"
    {
        return Ok(None);
    }

    Ok(Some(ProjectSettingPayload {
        service_type: normalize_service_type(service_type).to_string(),
        title: parsed
            .get("title")
            .and_then(|item| item.as_str())
            .unwrap_or_else(|| default_title(service_type))
            .to_string(),
        banner_image_url: parsed
            .get("banner_image_url")
            .and_then(|item| item.as_str())
            .map(ToString::to_string),
        requirement_content: parsed
            .get("requirement_content")
            .and_then(|item| item.as_str())
            .unwrap_or_default()
            .to_string(),
        warm_tip_content: parsed
            .get("warm_tip_content")
            .and_then(|item| item.as_str())
            .unwrap_or_default()
            .to_string(),
    }))
}

fn normalize_service_type(service_type: &str) -> &str {
    match service_type.trim() {
        "量尺" | "量尺服务" => "measure",
        "安装" | "安装服务" => "install",
        "熨烫" | "熨烫服务" => "ironing",
        "除尘除螨" | "除尘除螨服务" => "cleaning",
        other => other,
    }
}

fn default_title(service_type: &str) -> &str {
    match normalize_service_type(service_type) {
        "measure" => "量尺服务",
        "install" => "安装服务",
        "ironing" => "熨烫服务",
        "cleaning" => "除尘除螨服务",
        _ => "检核照片标准",
    }
}
