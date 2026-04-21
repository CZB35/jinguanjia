use anyhow::Context;
use axum::{
    Json,
    extract::State,
    http::HeaderMap,
};
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    auth,
    db,
    error::AppError,
    models::{audit_log::AuditLogEntity, system_config::SystemConfigEntity, system_version::SystemVersionEntity},
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct UpdateConfigsRequest {
    pub items: Vec<ConfigUpsertItem>,
}

#[derive(Debug, Deserialize)]
pub struct ConfigUpsertItem {
    pub config_key: String,
    pub config_value: String,
    pub remark: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConfigItem {
    pub config_key: String,
    pub config_value: String,
    pub remark: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConfigListResponse {
    pub items: Vec<ConfigItem>,
}

#[derive(Debug, Serialize)]
pub struct VersionResponse {
    pub version_no: String,
    pub version_title: String,
    pub changelog: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuditLogItem {
    pub id: i64,
    pub biz_type: String,
    pub biz_id: i64,
    pub action: String,
    pub operator_id: i64,
    pub operator_role: String,
    pub remark: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuditLogListResponse {
    pub items: Vec<AuditLogItem>,
    pub total: usize,
}

#[derive(Debug, Deserialize)]
pub struct SaveAgreementRequest {
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct AgreementResponse {
    pub agreement_type: String,
    pub content: String,
}

pub async fn get_configs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<ConfigListResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let configs: Vec<SystemConfigEntity> = db::select_by_map_compat(state.rb.as_ref(), "system_config", value! {})
        .await
        .context("failed to query system configs")
        .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(ConfigListResponse {
        items: configs
            .into_iter()
            .map(|item| ConfigItem {
                config_key: item.config_key.unwrap_or_default(),
                config_value: item.config_value.unwrap_or_default(),
                remark: item.remark,
            })
            .collect(),
    })))
}

pub async fn update_configs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UpdateConfigsRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    ensure_admin_access(&state, &headers)?;

    for item in payload.items {
        let normalized_value = normalize_json_config_value(&item.config_value)
            .map_err(AppError::Internal)?;
        let existing: Vec<SystemConfigEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "system_config",
            value! { "config_key": item.config_key.as_str() },
        )
        .await
        .context("failed to query config before upsert")
        .map_err(AppError::Internal)?;

        if let Some(existing_item) = existing.into_iter().next() {
            let updated = SystemConfigEntity {
                config_value: Some(normalized_value.clone()),
                remark: item.remark.clone(),
                ..existing_item.clone()
            };
            SystemConfigEntity::update_by_map(
                state.rb.as_ref(),
                &updated,
                value! { "id": existing_item.id.unwrap_or_default(), "column": ["config_value", "remark"] },
            )
            .await
            .context("failed to update system config")
            .map_err(AppError::Internal)?;
        } else {
            SystemConfigEntity::insert(
                state.rb.as_ref(),
                &SystemConfigEntity {
                    id: None,
                    config_key: Some(item.config_key),
                    config_value: Some(normalized_value),
                    remark: item.remark,
                    created_at: None,
                    updated_at: None,
                },
            )
            .await
            .context("failed to insert system config")
            .map_err(AppError::Internal)?;
        }
    }

    Ok(Json(ApiResponse::success(serde_json::json!({ "ok": true }))))
}

pub async fn get_version(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<VersionResponse>>, AppError> {
    let versions: Vec<SystemVersionEntity> = db::select_by_map_compat(state.rb.as_ref(), "system_version", value! {})
        .await
        .context("failed to query system versions")
        .map_err(AppError::Internal)?;

    let latest = versions
        .into_iter()
        .max_by_key(|item| item.id.unwrap_or_default())
        .unwrap_or(SystemVersionEntity {
            version_no: Some("0.1.0".to_string()),
            version_title: Some("初始版本".to_string()),
            changelog: Some("系统初始化".to_string()),
            ..Default::default()
        });

    Ok(Json(ApiResponse::success(VersionResponse {
        version_no: latest.version_no.unwrap_or_default(),
        version_title: latest.version_title.unwrap_or_default(),
        changelog: latest.changelog,
    })))
}

pub async fn get_changelog(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<Vec<VersionResponse>>>, AppError> {
    let versions: Vec<SystemVersionEntity> = db::select_by_map_compat(state.rb.as_ref(), "system_version", value! {})
        .await
        .context("failed to query changelog")
        .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(
        versions
            .into_iter()
            .map(|item| VersionResponse {
                version_no: item.version_no.unwrap_or_default(),
                version_title: item.version_title.unwrap_or_default(),
                changelog: item.changelog,
            })
            .collect(),
    )))
}

pub async fn get_operation_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<AuditLogListResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let logs: Vec<AuditLogEntity> = db::select_by_map_compat(state.rb.as_ref(), "audit_log", value! {})
        .await
        .context("failed to query operation logs")
        .map_err(AppError::Internal)?;

    let items = logs
        .into_iter()
        .map(|item| AuditLogItem {
            id: item.id.unwrap_or_default(),
            biz_type: item.biz_type.unwrap_or_default(),
            biz_id: item.biz_id.unwrap_or_default(),
            action: item.action.unwrap_or_default(),
            operator_id: item.operator_id.unwrap_or_default(),
            operator_role: item.operator_role.unwrap_or_default(),
            remark: item.remark,
            created_at: item.created_at.map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string()),
        })
        .collect::<Vec<_>>();
    let total = items.len();

    Ok(Json(ApiResponse::success(AuditLogListResponse { items, total })))
}

pub async fn get_operation_log_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(log_id): axum::extract::Path<i64>,
) -> Result<Json<ApiResponse<AuditLogItem>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let log: AuditLogEntity = db::select_one_by_map_compat(state.rb.as_ref(), "audit_log", value! { "id": log_id })
        .await
        .context("failed to query operation log detail")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("日志不存在".to_string()))?;

    Ok(Json(ApiResponse::success(AuditLogItem {
        id: log.id.unwrap_or_default(),
        biz_type: log.biz_type.unwrap_or_default(),
        biz_id: log.biz_id.unwrap_or_default(),
        action: log.action.unwrap_or_default(),
        operator_id: log.operator_id.unwrap_or_default(),
        operator_role: log.operator_role.unwrap_or_default(),
        remark: log.remark,
        created_at: log.created_at.map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string()),
    })))
}

pub async fn get_user_agreement(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<AgreementResponse>>, AppError> {
    get_agreement(&state, "user_agreement", "user").await
}

pub async fn save_user_agreement(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SaveAgreementRequest>,
) -> Result<Json<ApiResponse<AgreementResponse>>, AppError> {
    save_agreement(&state, &headers, "user_agreement", "user", payload.content).await
}

pub async fn get_privacy_policy(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<AgreementResponse>>, AppError> {
    get_agreement(&state, "privacy_policy", "privacy").await
}

pub async fn save_privacy_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SaveAgreementRequest>,
) -> Result<Json<ApiResponse<AgreementResponse>>, AppError> {
    save_agreement(&state, &headers, "privacy_policy", "privacy", payload.content).await
}

fn ensure_admin_access(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let auth_user = auth::authenticate(state, headers)?;
    auth_user.require_any_role(&["super_admin", "hq_reviewer"])
}

fn normalize_json_config_value(input: &str) -> anyhow::Result<String> {
    if serde_json::from_str::<serde_json::Value>(input).is_ok() {
        return Ok(input.to_string());
    }
    Ok(serde_json::to_string(input)?)
}

async fn get_agreement(
    state: &AppState,
    config_key: &str,
    agreement_type: &str,
) -> Result<Json<ApiResponse<AgreementResponse>>, AppError> {
    let item = db::select_one_by_map_compat::<SystemConfigEntity>(
        state.rb.as_ref(),
        "system_config",
        value! { "config_key": config_key },
    )
    .await
    .context("failed to query agreement config")
    .map_err(AppError::Internal)?;

    let content = item
        .and_then(|cfg| cfg.config_value)
        .map(|raw| parse_json_string_value(&raw))
        .unwrap_or_default();

    Ok(Json(ApiResponse::success(AgreementResponse {
        agreement_type: agreement_type.to_string(),
        content,
    })))
}

async fn save_agreement(
    state: &AppState,
    headers: &HeaderMap,
    config_key: &str,
    agreement_type: &str,
    content: String,
) -> Result<Json<ApiResponse<AgreementResponse>>, AppError> {
    ensure_admin_access(state, headers)?;
    let normalized_value = normalize_json_config_value(&content).map_err(AppError::Internal)?;

    let existing = db::select_one_by_map_compat::<SystemConfigEntity>(
        state.rb.as_ref(),
        "system_config",
        value! { "config_key": config_key },
    )
    .await
    .context("failed to query agreement before save")
    .map_err(AppError::Internal)?;

    if let Some(existing_item) = existing {
        let updated = SystemConfigEntity {
            config_value: Some(normalized_value),
            remark: Some("协议文本".to_string()),
            ..existing_item.clone()
        };
        SystemConfigEntity::update_by_map(
            state.rb.as_ref(),
            &updated,
            value! { "id": existing_item.id.unwrap_or_default(), "column": ["config_value", "remark"] },
        )
        .await
        .context("failed to update agreement config")
        .map_err(AppError::Internal)?;
    } else {
        SystemConfigEntity::insert(
            state.rb.as_ref(),
            &SystemConfigEntity {
                id: None,
                config_key: Some(config_key.to_string()),
                config_value: Some(normalized_value),
                remark: Some("协议文本".to_string()),
                created_at: None,
                updated_at: None,
            },
        )
        .await
        .context("failed to insert agreement config")
        .map_err(AppError::Internal)?;
    }

    Ok(Json(ApiResponse::success(AgreementResponse {
        agreement_type: agreement_type.to_string(),
        content,
    })))
}

fn parse_json_string_value(raw: &str) -> String {
    serde_json::from_str::<String>(raw).unwrap_or_else(|_| raw.to_string())
}
