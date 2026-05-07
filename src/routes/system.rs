use anyhow::Context;
use axum::{Json, extract::{Query, State}, http::HeaderMap};
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    auth, db,
    error::AppError,
    models::{
        audit_log::AuditLogEntity, system_config::SystemConfigEntity,
        system_version::SystemVersionEntity,
    },
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

#[derive(Debug, Deserialize)]
pub struct ProjectSettingsQuery {
    pub service_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SaveProjectSettingsRequest {
    pub items: Vec<ProjectSettingUpsertItem>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectSettingUpsertItem {
    pub service_type: String,
    pub title: Option<String>,
    pub banner_image_url: Option<String>,
    pub requirement_content: Option<String>,
    pub warm_tip_content: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProjectSettingItem {
    pub service_type: String,
    pub title: String,
    pub banner_image_url: Option<String>,
    pub requirement_content: String,
    pub warm_tip_content: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct ProjectSettingsResponse {
    pub items: Vec<ProjectSettingItem>,
}

pub async fn get_configs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<ConfigListResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let configs: Vec<SystemConfigEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "system_config", value! {})
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
        let normalized_value =
            normalize_json_config_value(&item.config_value).map_err(AppError::Internal)?;
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

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn get_version(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<VersionResponse>>, AppError> {
    let versions: Vec<SystemVersionEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "system_version", value! {})
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
    let versions: Vec<SystemVersionEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "system_version", value! {})
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
    let logs: Vec<AuditLogEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "audit_log", value! {})
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
            created_at: item
                .created_at
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string()),
        })
        .collect::<Vec<_>>();
    let total = items.len();

    Ok(Json(ApiResponse::success(AuditLogListResponse {
        items,
        total,
    })))
}

pub async fn get_operation_log_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(log_id): axum::extract::Path<i64>,
) -> Result<Json<ApiResponse<AuditLogItem>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let log: AuditLogEntity =
        db::select_one_by_map_compat(state.rb.as_ref(), "audit_log", value! { "id": log_id })
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
        created_at: log
            .created_at
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string()),
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
    save_agreement(
        &state,
        &headers,
        "privacy_policy",
        "privacy",
        payload.content,
    )
    .await
}

pub async fn get_project_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ProjectSettingsQuery>,
) -> Result<Json<ApiResponse<ProjectSettingsResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let items = load_project_settings(&state).await?;
    let filtered = if let Some(service_type) = query
        .service_type
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        items
            .into_iter()
            .filter(|item| item.service_type == service_type)
            .collect()
    } else {
        items
    };
    Ok(Json(ApiResponse::success(ProjectSettingsResponse {
        items: filtered,
    })))
}

pub async fn save_project_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SaveProjectSettingsRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    ensure_admin_access(&state, &headers)?;

    for item in payload.items {
        let service_type = normalize_service_type(&item.service_type)?;
        let title = normalize_optional_text(item.title.as_deref())
            .unwrap_or_else(|| default_project_setting_title(&service_type).to_string());
        let config_key = format!("project_setting:{service_type}");
        let config_value = serde_json::json!({
            "service_type": service_type,
            "title": title,
            "banner_image_url": normalize_optional_text(item.banner_image_url.as_deref()),
            "requirement_content": normalize_optional_text(item.requirement_content.as_deref()).unwrap_or_default(),
            "warm_tip_content": normalize_optional_text(item.warm_tip_content.as_deref()).unwrap_or_default(),
            "status": normalize_config_status(item.status.as_deref())?,
        });
        let normalized_value = normalize_json_config_value(&config_value.to_string())
            .map_err(AppError::Internal)?;

        let existing = db::select_one_by_map_compat::<SystemConfigEntity>(
            state.rb.as_ref(),
            "system_config",
            value! { "config_key": config_key.as_str() },
        )
        .await
        .context("failed to query project setting before save")
        .map_err(AppError::Internal)?;

        if let Some(existing_item) = existing {
            let updated = SystemConfigEntity {
                config_value: Some(normalized_value),
                remark: Some("项目设置".to_string()),
                ..existing_item.clone()
            };
            SystemConfigEntity::update_by_map(
                state.rb.as_ref(),
                &updated,
                value! { "id": existing_item.id.unwrap_or_default(), "column": ["config_value", "remark"] },
            )
            .await
            .context("failed to update project setting")
            .map_err(AppError::Internal)?;
        } else {
            SystemConfigEntity::insert(
                state.rb.as_ref(),
                &SystemConfigEntity {
                    id: None,
                    config_key: Some(config_key),
                    config_value: Some(normalized_value),
                    remark: Some("项目设置".to_string()),
                    created_at: None,
                    updated_at: None,
                },
            )
            .await
            .context("failed to insert project setting")
            .map_err(AppError::Internal)?;
        }
    }

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

fn ensure_admin_access(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let auth_user = auth::authenticate(state, headers)?;
    auth_user.require_any_role(&["super_admin", "hq_reviewer"])
}

async fn load_project_settings(state: &AppState) -> Result<Vec<ProjectSettingItem>, AppError> {
    let configs: Vec<SystemConfigEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "system_config",
        value! {},
    )
    .await
    .context("failed to query project settings")
    .map_err(AppError::Internal)?;

    let mut items = configs
        .into_iter()
        .filter_map(|item| {
            let config_key = item.config_key?;
            let service_type = config_key.strip_prefix("project_setting:")?.to_string();
            let value = item.config_value?;
            let parsed = serde_json::from_str::<serde_json::Value>(&value).ok()?;
            Some(ProjectSettingItem {
                service_type: service_type.clone(),
                title: parsed
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| default_project_setting_title(&service_type).to_string()),
                banner_image_url: parsed
                    .get("banner_image_url")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string),
                requirement_content: parsed
                    .get("requirement_content")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                warm_tip_content: parsed
                    .get("warm_tip_content")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                status: parsed
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("enabled")
                    .to_string(),
            })
        })
        .collect::<Vec<_>>();

    for service_type in ["measure", "install", "ironing", "cleaning"] {
        if !items.iter().any(|item| item.service_type == service_type) {
            items.push(ProjectSettingItem {
                service_type: service_type.to_string(),
                title: default_project_setting_title(service_type).to_string(),
                banner_image_url: None,
                requirement_content: String::new(),
                warm_tip_content: String::new(),
                status: "enabled".to_string(),
            });
        }
    }
    items.sort_by_key(|item| project_setting_sort_no(&item.service_type));
    Ok(items)
}

fn normalize_service_type(value: &str) -> Result<String, AppError> {
    match value.trim() {
        "measure" | "量尺" | "量尺服务" => Ok("measure".to_string()),
        "install" | "安装" | "安装服务" => Ok("install".to_string()),
        "ironing" | "熨烫" | "熨烫服务" => Ok("ironing".to_string()),
        "cleaning" | "除尘除螨" | "除尘除螨服务" => Ok("cleaning".to_string()),
        _ => Err(AppError::BadRequest("service_type不支持".to_string())),
    }
}

fn normalize_config_status(value: Option<&str>) -> Result<String, AppError> {
    match value.unwrap_or("enabled").trim() {
        "enabled" | "active" | "启用" => Ok("enabled".to_string()),
        "disabled" | "inactive" | "禁用" => Ok("disabled".to_string()),
        _ => Err(AppError::BadRequest(
            "status只能为启用/禁用或enabled/disabled或active/inactive".to_string(),
        )),
    }
}

fn default_project_setting_title(service_type: &str) -> &'static str {
    match service_type {
        "measure" => "量尺服务",
        "install" => "安装服务",
        "ironing" => "熨烫服务",
        "cleaning" => "除尘除螨服务",
        _ => "项目设置",
    }
}

fn project_setting_sort_no(service_type: &str) -> i32 {
    match service_type {
        "measure" => 1,
        "install" => 2,
        "ironing" => 3,
        "cleaning" => 4,
        _ => 99,
    }
}

fn normalize_json_config_value(input: &str) -> anyhow::Result<String> {
    if serde_json::from_str::<serde_json::Value>(input).is_ok() {
        return Ok(input.to_string());
    }
    Ok(serde_json::to_string(input)?)
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
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
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| default_agreement_content(config_key).to_string());

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

fn default_agreement_content(config_key: &str) -> &'static str {
    match config_key {
        "user_agreement" => {
            "欢迎使用金管家服务。\n\n1. 用户应保证注册信息真实、准确、完整。\n2. 用户应妥善保管账号与密码，并对账号下的操作承担责任。\n3. 平台有权在法律法规允许范围内提供服务支持与运营管理。\n4. 用户使用过程中不得发布违法违规内容，不得干扰平台正常运行。\n5. 如因业务需要调整服务内容，平台将通过合理方式进行通知。"
        }
        "privacy_policy" => {
            "我们重视您的个人信息与隐私保护。\n\n1. 为提供服务，系统可能收集手机号、姓名、门店、任务记录等必要信息。\n2. 收集的信息仅用于身份识别、业务处理、服务通知与运营保障。\n3. 未经用户授权或法律法规要求，不会向无关第三方泄露个人信息。\n4. 我们将采取合理安全措施保护您的数据，防止丢失、泄露或被非法使用。\n5. 用户可通过平台提供的方式查询、更正相关信息。"
        }
        _ => "",
    }
}
