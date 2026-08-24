use anyhow::Context;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    auth, db, error::AppError, models::customer_source::CustomerSourceEntity,
    response::ApiResponse, state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct CustomerSourceListQuery {
    pub page: Option<usize>,
    pub page_size: Option<usize>,
    pub name: Option<String>,
    pub code: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCustomerSourceRequest {
    pub name: String,
    pub code: String,
    #[serde(default, alias = "sort_no")]
    pub sort: Option<i32>,
    pub remark: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCustomerSourceRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    #[serde(default, alias = "sort_no")]
    pub sort: Option<i32>,
    pub remark: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CustomerSourceItem {
    pub id: i64,
    pub name: String,
    pub code: String,
    pub sort: i32,
    pub remark: Option<String>,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct CustomerSourceListResponse {
    pub items: Vec<CustomerSourceItem>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

pub async fn list_customer_sources(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CustomerSourceListQuery>,
) -> Result<Json<ApiResponse<CustomerSourceListResponse>>, AppError> {
    ensure_manage_access(&state, &headers)?;

    let status_filter = normalize_status_filter(query.status.as_deref())?;
    let mut items: Vec<CustomerSourceEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "customer_source", value! {})
            .await
            .context("failed to query customer sources")
            .map_err(AppError::Internal)?;

    items.retain(|item| {
        if let Some(name) = &query.name
            && !item.name.as_deref().unwrap_or_default().contains(name)
        {
            return false;
        }
        if let Some(code) = &query.code
            && !item.code.as_deref().unwrap_or_default().contains(code)
        {
            return false;
        }
        if let Some(status) = &status_filter
            && canonical_status(item.status.as_deref()) != *status
        {
            return false;
        }
        true
    });
    items.sort_by(|left, right| {
        right.created_at.cmp(&left.created_at).then_with(|| {
            right
                .id
                .unwrap_or_default()
                .cmp(&left.id.unwrap_or_default())
        })
    });

    let total = items.len();
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).max(1);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let items = if start >= total {
        Vec::new()
    } else {
        items
            .into_iter()
            .skip(start)
            .take(page_size)
            .map(map_customer_source_item)
            .collect()
    };

    Ok(Json(ApiResponse::success(CustomerSourceListResponse {
        items,
        total,
        page,
        page_size,
    })))
}

pub async fn get_customer_source_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(source_id): Path<i64>,
) -> Result<Json<ApiResponse<CustomerSourceItem>>, AppError> {
    ensure_manage_access(&state, &headers)?;
    let item = find_customer_source(&state, source_id).await?;
    Ok(Json(ApiResponse::success(map_customer_source_item(item))))
}

pub async fn create_customer_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateCustomerSourceRequest>,
) -> Result<Json<ApiResponse<CustomerSourceItem>>, AppError> {
    ensure_manage_access(&state, &headers)?;

    let name = normalize_required_text(&payload.name, "name不能为空")?;
    let code = normalize_required_text(&payload.code, "code不能为空")?;
    let sort_no = payload.sort.unwrap_or(0);
    let status = normalize_status(payload.status.as_deref())?;

    ensure_customer_source_name_unique(&state, &name, None).await?;
    ensure_customer_source_code_unique(&state, &code, None).await?;

    CustomerSourceEntity::insert(
        state.rb.as_ref(),
        &CustomerSourceEntity {
            id: None,
            name: Some(name.clone()),
            code: Some(code.clone()),
            sort_no: Some(sort_no),
            remark: normalize_optional_owned(payload.remark),
            status: Some(status),
            created_at: None,
            updated_at: None,
        },
    )
    .await
    .context("failed to create customer source")
    .map_err(AppError::Internal)?;

    let item = db::select_one_by_map_compat::<CustomerSourceEntity>(
        state.rb.as_ref(),
        "customer_source",
        value! { "code": code.as_str() },
    )
    .await
    .context("failed to fetch created customer source")
    .map_err(AppError::Internal)?
    .ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!(
            "customer source create succeeded but fetch failed"
        ))
    })?;

    Ok(Json(ApiResponse::success(map_customer_source_item(item))))
}

pub async fn update_customer_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(source_id): Path<i64>,
    Json(payload): Json<UpdateCustomerSourceRequest>,
) -> Result<Json<ApiResponse<CustomerSourceItem>>, AppError> {
    ensure_manage_access(&state, &headers)?;
    let existing = find_customer_source(&state, source_id).await?;

    let name = match payload.name.as_deref() {
        Some(value) => normalize_required_text(value, "name不能为空")?,
        None => existing.name.clone().unwrap_or_default(),
    };
    let code = match payload.code.as_deref() {
        Some(value) => normalize_required_text(value, "code不能为空")?,
        None => existing.code.clone().unwrap_or_default(),
    };
    let status = match payload.status.as_deref() {
        Some(value) => Some(normalize_status(Some(value))?),
        None => existing.status.clone(),
    };

    ensure_customer_source_name_unique(&state, &name, Some(source_id)).await?;
    ensure_customer_source_code_unique(&state, &code, Some(source_id)).await?;

    CustomerSourceEntity::update_by_map(
        state.rb.as_ref(),
        &CustomerSourceEntity {
            name: Some(name),
            code: Some(code),
            sort_no: payload.sort.or(existing.sort_no),
            remark: payload
                .remark
                .map(|value| normalize_optional_text(Some(value.as_str())))
                .unwrap_or_else(|| existing.remark.clone()),
            status,
            ..Default::default()
        },
        value! { "id": source_id, "column": ["name", "code", "sort_no", "remark", "status"] },
    )
    .await
    .context("failed to update customer source")
    .map_err(AppError::Internal)?;

    get_customer_source_detail(State(state), headers, Path(source_id)).await
}

pub async fn delete_customer_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(source_id): Path<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    ensure_manage_access(&state, &headers)?;
    let _ = find_customer_source(&state, source_id).await?;

    CustomerSourceEntity::delete_by_map(state.rb.as_ref(), value! { "id": source_id })
        .await
        .context("failed to delete customer source")
        .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

async fn find_customer_source(
    state: &AppState,
    source_id: i64,
) -> Result<CustomerSourceEntity, AppError> {
    db::select_one_by_map_compat(
        state.rb.as_ref(),
        "customer_source",
        value! { "id": source_id },
    )
    .await
    .context("failed to query customer source by id")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::NotFound("客户来源不存在".to_string()))
}

async fn ensure_customer_source_name_unique(
    state: &AppState,
    name: &str,
    source_id: Option<i64>,
) -> Result<(), AppError> {
    let existing: Vec<CustomerSourceEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "customer_source",
        value! { "name": name },
    )
    .await
    .context("failed to query customer source by name")
    .map_err(AppError::Internal)?;

    if existing.into_iter().any(|item| item.id != source_id) {
        return Err(AppError::Conflict("客户来源名称已存在".to_string()));
    }

    Ok(())
}

async fn ensure_customer_source_code_unique(
    state: &AppState,
    code: &str,
    source_id: Option<i64>,
) -> Result<(), AppError> {
    let existing: Vec<CustomerSourceEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "customer_source",
        value! { "code": code },
    )
    .await
    .context("failed to query customer source by code")
    .map_err(AppError::Internal)?;

    if existing.into_iter().any(|item| item.id != source_id) {
        return Err(AppError::Conflict("客户来源编码已存在".to_string()));
    }

    Ok(())
}

fn ensure_manage_access(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let auth_user = auth::authenticate(state, headers)?;
    auth_user.require_any_role(&["dealer", "super_admin", "hq_reviewer"])
}

fn map_customer_source_item(item: CustomerSourceEntity) -> CustomerSourceItem {
    CustomerSourceItem {
        id: item.id.unwrap_or_default(),
        name: item.name.unwrap_or_default(),
        code: item.code.unwrap_or_default(),
        sort: item.sort_no.unwrap_or_default(),
        remark: item.remark,
        status: canonical_status(item.status.as_deref()),
    }
}

fn normalize_required_text(value: &str, message: &str) -> Result<String, AppError> {
    normalize_optional_text(Some(value)).ok_or_else(|| AppError::BadRequest(message.to_string()))
}

fn normalize_optional_owned(value: Option<String>) -> Option<String> {
    value.and_then(|item| normalize_optional_text(Some(item.as_str())))
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
}

fn normalize_status(raw: Option<&str>) -> Result<String, AppError> {
    match raw
        .unwrap_or("enabled")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "enabled" | "active" => Ok("enabled".to_string()),
        "disabled" | "inactive" => Ok("disabled".to_string()),
        _ => Err(AppError::BadRequest(
            "status只能为enabled/disabled或active/inactive".to_string(),
        )),
    }
}

fn normalize_status_filter(raw: Option<&str>) -> Result<Option<String>, AppError> {
    match raw {
        Some(value) => Ok(Some(normalize_status(Some(value))?)),
        None => Ok(None),
    }
}

fn canonical_status(raw: Option<&str>) -> String {
    normalize_status(raw).unwrap_or_else(|_| "enabled".to_string())
}
