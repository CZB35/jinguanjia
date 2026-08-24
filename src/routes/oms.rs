use anyhow::Context;
use axum::{Json, extract::State, http::HeaderMap};
use chrono::Utc;
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    auth, db,
    error::AppError,
    models::{oms_config::OmsConfigEntity, oms_sync_log::OmsSyncLogEntity, task::TaskEntity},
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct UpdateOmsConfigRequest {
    pub base_url: String,
    pub app_key: String,
    pub app_secret: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct OmsConfigResponse {
    pub id: i64,
    pub base_url: String,
    pub app_key: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct OmsSyncLogItem {
    pub id: i64,
    pub biz_type: String,
    pub biz_id: i64,
    pub sync_direction: String,
    pub sync_status: String,
    pub error_message: Option<String>,
}

pub async fn get_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<OmsConfigResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let config = load_or_default_config(&state).await?;
    Ok(Json(ApiResponse::success(OmsConfigResponse {
        id: config.id.unwrap_or_default(),
        base_url: config.base_url.unwrap_or_default(),
        app_key: config.app_key.unwrap_or_default(),
        status: config.status.unwrap_or_else(|| "enabled".to_string()),
    })))
}

pub async fn update_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UpdateOmsConfigRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let existing: Vec<OmsConfigEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "oms_config", value! {})
            .await
            .context("failed to query oms config")
            .map_err(AppError::Internal)?;
    if let Some(item) = existing.into_iter().next() {
        OmsConfigEntity::update_by_map(
            state.rb.as_ref(),
            &OmsConfigEntity {
                base_url: Some(payload.base_url),
                app_key: Some(payload.app_key),
                app_secret: Some(payload.app_secret),
                status: Some(payload.status),
                ..Default::default()
            },
            value! { "id": item.id.unwrap_or_default() },
        )
        .await
        .context("failed to update oms config")
        .map_err(AppError::Internal)?;
    } else {
        OmsConfigEntity::insert(
            state.rb.as_ref(),
            &OmsConfigEntity {
                id: None,
                base_url: Some(payload.base_url),
                app_key: Some(payload.app_key),
                app_secret: Some(payload.app_secret),
                status: Some(payload.status),
                created_at: None,
                updated_at: None,
            },
        )
        .await
        .context("failed to insert oms config")
        .map_err(AppError::Internal)?;
    }

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn sync_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["super_admin", "hq_reviewer"])?;

    let tasks: Vec<TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! {})
        .await
        .context("failed to query tasks for oms sync")
        .map_err(AppError::Internal)?;
    for task in tasks
        .into_iter()
        .filter(|task| task.deleted_at.is_none())
        .take(20)
    {
        OmsSyncLogEntity::insert(
            state.rb.as_ref(),
            &OmsSyncLogEntity {
                id: None,
                biz_type: Some("task".to_string()),
                biz_id: task.id,
                sync_direction: Some("out".to_string()),
                request_payload: Some(serde_json::to_string(&task).unwrap_or_default()),
                response_payload: Some("{\"ok\":true}".to_string()),
                sync_status: Some("success".to_string()),
                error_message: None,
                created_at: Some(Utc::now().naive_utc()),
            },
        )
        .await
        .context("failed to insert oms sync log")
        .map_err(AppError::Internal)?;
    }

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn sync_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<Vec<OmsSyncLogItem>>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let logs: Vec<OmsSyncLogEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "oms_sync_log", value! {})
            .await
            .context("failed to query oms sync logs")
            .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(
        logs.into_iter()
            .map(|item| OmsSyncLogItem {
                id: item.id.unwrap_or_default(),
                biz_type: item.biz_type.unwrap_or_default(),
                biz_id: item.biz_id.unwrap_or_default(),
                sync_direction: item.sync_direction.unwrap_or_default(),
                sync_status: item.sync_status.unwrap_or_default(),
                error_message: item.error_message,
            })
            .collect(),
    )))
}

async fn load_or_default_config(state: &AppState) -> Result<OmsConfigEntity, AppError> {
    let config =
        db::select_one_by_map_compat::<OmsConfigEntity>(state.rb.as_ref(), "oms_config", value! {})
            .await
            .context("failed to query oms config")
            .map_err(AppError::Internal)?
            .unwrap_or(OmsConfigEntity {
                base_url: Some(String::new()),
                app_key: Some(String::new()),
                status: Some("enabled".to_string()),
                ..Default::default()
            });
    Ok(config)
}

fn ensure_admin_access(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let auth_user = auth::authenticate(state, headers)?;
    auth_user.require_any_role(&["super_admin", "hq_reviewer"])
}
