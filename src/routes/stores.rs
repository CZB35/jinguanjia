use anyhow::Context;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    auth,
    db,
    error::AppError,
    models::{
        customer::CustomerEntity,
        store::StoreEntity,
        task::TaskEntity,
        user_store::UserStoreEntity,
    },
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct StoreListQuery {
    #[serde(default, alias = "name")]
    pub store_name: Option<String>,
    pub code: Option<String>,
    pub contact_mobile: Option<String>,
    pub status: Option<String>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct StoreItem {
    pub id: i64,
    pub name: String,
    pub store_name: String,
    pub code: Option<String>,
    pub contact_name: Option<String>,
    pub contact_mobile: Option<String>,
    pub province: Option<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub region_text: String,
    pub address: Option<String>,
    pub remark: Option<String>,
    pub business_hours: Option<String>,
    pub status: String,
    pub technician_count: usize,
    pub customer_count: usize,
    pub completed_task_count: usize,
    pub completion_rate: f64,
}

#[derive(Debug, Serialize)]
pub struct StoreListResponse {
    pub items: Vec<StoreItem>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, Deserialize)]
pub struct CreateStoreRequest {
    #[serde(alias = "name")]
    pub store_name: String,
    pub code: Option<String>,
    pub province: Option<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub address: Option<String>,
    pub remark: Option<String>,
    pub business_hours: Option<String>,
    pub contact_name: Option<String>,
    pub contact_mobile: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateStoreRequest {
    #[serde(default, alias = "name")]
    pub store_name: Option<String>,
    pub code: Option<String>,
    pub province: Option<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub address: Option<String>,
    pub remark: Option<String>,
    pub business_hours: Option<String>,
    pub contact_name: Option<String>,
    pub contact_mobile: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateStoreStatusRequest {
    pub status: String,
}

pub async fn list_stores(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<StoreListQuery>,
) -> Result<Json<ApiResponse<StoreListResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    let dealer_store_ids = dealer_scope_store_ids(&state, &auth_user).await?;

    let status_filter = normalize_status_filter(query.status.as_deref())?;
    let mut stores: Vec<StoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "store", value! {})
        .await
        .context("failed to query stores")
        .map_err(AppError::Internal)?;
    stores.retain(|item| {
        if let Some(store_ids) = &dealer_store_ids
            && item.id.map(|id| !store_ids.contains(&id)).unwrap_or(true)
        {
            return false;
        }
        if let Some(store_name) = &query.store_name
            && !item.store_name.as_deref().unwrap_or_default().contains(store_name)
        {
            return false;
        }
        if let Some(code) = &query.code
            && !item.code.as_deref().unwrap_or_default().contains(code)
        {
            return false;
        }
        if let Some(contact_mobile) = &query.contact_mobile
            && !item.contact_mobile.as_deref().unwrap_or_default().contains(contact_mobile)
        {
            return false;
        }
        if let Some(status) = &status_filter
            && canonical_store_status(item.status.as_deref()) != *status
        {
            return false;
        }
        true
    });

    let mut items = Vec::with_capacity(stores.len());
    for item in stores {
        items.push(build_store_item(&state, item).await?);
    }

    let total = items.len();
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).max(1);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let items = if start >= total {
        Vec::new()
    } else {
        items.into_iter().skip(start).take(page_size).collect()
    };

    Ok(Json(ApiResponse::success(StoreListResponse {
        items,
        total,
        page,
        page_size,
    })))
}

pub async fn create_store(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateStoreRequest>,
) -> Result<Json<ApiResponse<StoreItem>>, AppError> {
    ensure_store_manage_access(&state, &headers)?;

    let store_name = payload.store_name.trim();
    if store_name.is_empty() {
        return Err(AppError::BadRequest("请输入门店名称".to_string()));
    }
    let code = normalize_optional_text(payload.code.as_deref());
    let status = normalize_store_status(payload.status.as_deref()).unwrap_or_else(|_| "enabled".to_string());

    ensure_store_name_unique(&state, store_name, None).await?;
    ensure_store_code_unique(&state, code.as_deref(), None).await?;

    StoreEntity::insert(
        state.rb.as_ref(),
        &StoreEntity {
            id: None,
            store_name: Some(store_name.to_string()),
            code: code.clone(),
            contact_name: normalize_optional_owned(payload.contact_name),
            contact_mobile: normalize_optional_owned(payload.contact_mobile),
            province: normalize_optional_owned(payload.province),
            city: normalize_optional_owned(payload.city),
            district: normalize_optional_owned(payload.district),
            address: normalize_optional_owned(payload.address),
            remark: normalize_optional_owned(payload.remark),
            business_hours: normalize_optional_owned(payload.business_hours),
            status: Some(status),
            created_at: None,
            updated_at: None,
        },
    )
    .await
    .context("failed to create store")
    .map_err(AppError::Internal)?;

    let store = db::select_one_by_map_compat::<StoreEntity>(
        state.rb.as_ref(),
        "store",
        value! { "store_name": store_name },
    )
    .await
    .context("failed to fetch created store")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::Internal(anyhow::anyhow!("store create succeeded but fetch failed")))?;

    Ok(Json(ApiResponse::success(build_store_item(&state, store).await?)))
}

pub async fn get_store_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(store_id): Path<i64>,
) -> Result<Json<ApiResponse<StoreItem>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    ensure_store_access(&state, &auth_user, store_id).await?;

    let store = find_store(state.rb.as_ref(), store_id).await?;
    Ok(Json(ApiResponse::success(build_store_item(&state, store).await?)))
}

pub async fn update_store_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(store_id): Path<i64>,
    Json(payload): Json<UpdateStoreRequest>,
) -> Result<Json<ApiResponse<StoreItem>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    ensure_store_access(&state, &auth_user, store_id).await?;

    let existing = find_store(state.rb.as_ref(), store_id).await?;
    let next_store_name = payload
        .store_name
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
        .or_else(|| existing.store_name.clone())
        .ok_or_else(|| AppError::BadRequest("请输入门店名称".to_string()))?;
    let next_code = payload
        .code
        .as_deref()
        .map(|value| normalize_optional_text(Some(value)))
        .unwrap_or_else(|| existing.code.clone());
    let next_status = match payload.status.as_deref() {
        Some(status) => Some(normalize_store_status(Some(status))?),
        None => existing.status.clone(),
    };

    ensure_store_name_unique(&state, &next_store_name, Some(store_id)).await?;
    ensure_store_code_unique(&state, next_code.as_deref(), Some(store_id)).await?;

    let updated = StoreEntity {
        store_name: Some(next_store_name),
        code: next_code,
        province: payload
            .province
            .map(|value| normalize_optional_text(Some(value.as_str())))
            .unwrap_or_else(|| existing.province.clone()),
        city: payload
            .city
            .map(|value| normalize_optional_text(Some(value.as_str())))
            .unwrap_or_else(|| existing.city.clone()),
        district: payload
            .district
            .map(|value| normalize_optional_text(Some(value.as_str())))
            .unwrap_or_else(|| existing.district.clone()),
        address: payload
            .address
            .map(|value| normalize_optional_text(Some(value.as_str())))
            .unwrap_or_else(|| existing.address.clone()),
        remark: payload
            .remark
            .map(|value| normalize_optional_text(Some(value.as_str())))
            .unwrap_or_else(|| existing.remark.clone()),
        business_hours: payload
            .business_hours
            .map(|value| normalize_optional_text(Some(value.as_str())))
            .unwrap_or_else(|| existing.business_hours.clone()),
        contact_name: payload
            .contact_name
            .map(|value| normalize_optional_text(Some(value.as_str())))
            .unwrap_or_else(|| existing.contact_name.clone()),
        contact_mobile: payload
            .contact_mobile
            .map(|value| normalize_optional_text(Some(value.as_str())))
            .unwrap_or_else(|| existing.contact_mobile.clone()),
        status: next_status,
        ..existing.clone()
    };

    StoreEntity::update_by_map(
        state.rb.as_ref(),
        &updated,
        value! { "id": store_id, "column": ["store_name", "code", "province", "city", "district", "address", "remark", "business_hours", "contact_name", "contact_mobile", "status"] },
    )
    .await
    .context("failed to update store detail")
    .map_err(AppError::Internal)?;

    get_store_detail(State(state), headers, Path(store_id)).await
}

pub async fn update_store_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(store_id): Path<i64>,
    Json(payload): Json<UpdateStoreStatusRequest>,
) -> Result<Json<ApiResponse<StoreItem>>, AppError> {
    ensure_store_manage_access(&state, &headers)?;
    let status = normalize_store_status(Some(payload.status.as_str()))?;

    StoreEntity::update_by_map(
        state.rb.as_ref(),
        &StoreEntity {
            status: Some(status),
            ..Default::default()
        },
        value! { "id": store_id },
    )
    .await
    .context("failed to update store status")
    .map_err(AppError::Internal)?;

    get_store_detail(State(state), headers, Path(store_id)).await
}

pub async fn delete_store(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(store_id): Path<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    ensure_store_manage_access(&state, &headers)?;
    let _ = find_store(state.rb.as_ref(), store_id).await?;

    let task_count = db::select_by_map_compat::<TaskEntity>(state.rb.as_ref(), "task", value! { "store_id": store_id })
        .await
        .context("failed to query store tasks before delete")
        .map_err(AppError::Internal)?
        .len();
    let user_relation_count = db::select_by_map_compat::<UserStoreEntity>(state.rb.as_ref(), "user_store", value! { "store_id": store_id })
        .await
        .context("failed to query store user relations before delete")
        .map_err(AppError::Internal)?
        .len();
    let primary_customer_count = db::select_by_map_compat::<CustomerEntity>(
        state.rb.as_ref(),
        "customer",
        value! { "primary_store_id": store_id },
    )
    .await
    .context("failed to query store customers before delete")
    .map_err(AppError::Internal)?
    .len();

    if task_count > 0 || user_relation_count > 0 || primary_customer_count > 0 {
        return Err(AppError::Conflict(format!(
            "门店已被引用，无法删除（任务{}条，用户门店关系{}条，客户主门店{}条）",
            task_count, user_relation_count, primary_customer_count
        )));
    }

    StoreEntity::delete_by_map(state.rb.as_ref(), value! { "id": store_id })
        .await
        .context("failed to delete store")
        .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(serde_json::json!({ "ok": true }))))
}

async fn build_store_item(state: &AppState, store: StoreEntity) -> Result<StoreItem, AppError> {
    let store_id = store.id.unwrap_or_default();
    let store_tasks: Vec<TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! { "store_id": store_id })
        .await
        .context("failed to query tasks for store item")
        .map_err(AppError::Internal)?;
    let completed_task_count = store_tasks
        .iter()
        .filter(|item| item.task_status.as_deref() == Some("completed"))
        .count();
    let customer_count = store_tasks
        .iter()
        .filter_map(|item| item.customer_id)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let technician_count = db::select_by_map_compat::<UserStoreEntity>(state.rb.as_ref(), "user_store", value! { "store_id": store_id })
        .await
        .context("failed to query store technician count")
        .map_err(AppError::Internal)?
        .len();
    let store_name = store.store_name.unwrap_or_default();
    let region_text = join_region(store.province.as_deref(), store.city.as_deref(), store.district.as_deref());

    Ok(StoreItem {
        id: store_id,
        name: store_name.clone(),
        store_name,
        code: store.code,
        contact_name: store.contact_name,
        contact_mobile: store.contact_mobile,
        province: store.province,
        city: store.city,
        district: store.district,
        region_text,
        address: store.address,
        remark: store.remark,
        business_hours: store.business_hours,
        status: canonical_store_status(store.status.as_deref()),
        technician_count,
        customer_count,
        completed_task_count,
        completion_rate: if store_tasks.is_empty() {
            0.0
        } else {
            (completed_task_count as f64 / store_tasks.len() as f64) * 100.0
        },
    })
}

async fn find_store(rb: &rbatis::RBatis, store_id: i64) -> Result<StoreEntity, AppError> {
    db::select_one_by_map_compat(rb, "store", value! { "id": store_id })
        .await
        .context("failed to query store by id")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("门店不存在".to_string()))
}

async fn ensure_store_name_unique(state: &AppState, store_name: &str, store_id: Option<i64>) -> Result<(), AppError> {
    let existing: Vec<StoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "store",
        value! { "store_name": store_name },
    )
    .await
    .context("failed to query existing store by name")
    .map_err(AppError::Internal)?;

    if existing
        .into_iter()
        .any(|item| item.id != store_id)
    {
        return Err(AppError::Conflict("门店名称已存在".to_string()));
    }

    Ok(())
}

async fn ensure_store_code_unique(state: &AppState, code: Option<&str>, store_id: Option<i64>) -> Result<(), AppError> {
    let Some(code) = code else {
        return Ok(());
    };

    let existing: Vec<StoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "store",
        value! { "code": code },
    )
    .await
    .context("failed to query existing store by code")
    .map_err(AppError::Internal)?;

    if existing.into_iter().any(|item| item.id != store_id) {
        return Err(AppError::Conflict("门店编码已存在".to_string()));
    }

    Ok(())
}

fn ensure_store_manage_access(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let auth_user = auth::authenticate(state, headers)?;
    auth_user.require_any_role(&["hq_reviewer", "super_admin"])
}

async fn ensure_store_access(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    store_id: i64,
) -> Result<(), AppError> {
    if !auth_user.has_role("dealer") || auth_user.has_role("hq_reviewer") || auth_user.has_role("super_admin") {
        return Ok(());
    }

    let store_ids = load_user_store_ids(state, auth_user.user_id).await?;
    if store_ids.contains(&store_id) {
        return Ok(());
    }

    Err(AppError::Forbidden("您只能查看或维护所属门店".to_string()))
}

async fn dealer_scope_store_ids(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
) -> Result<Option<Vec<i64>>, AppError> {
    if !auth_user.has_role("dealer") || auth_user.has_role("hq_reviewer") || auth_user.has_role("super_admin") {
        return Ok(None);
    }

    Ok(Some(load_user_store_ids(state, auth_user.user_id).await?))
}

async fn load_user_store_ids(state: &AppState, user_id: i64) -> Result<Vec<i64>, AppError> {
    let stores: Vec<UserStoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_store",
        value! { "user_id": user_id },
    )
    .await
    .context("failed to query user store relation")
    .map_err(AppError::Internal)?;

    Ok(stores.into_iter().filter_map(|item| item.store_id).collect())
}

fn normalize_store_status(raw: Option<&str>) -> Result<String, AppError> {
    let Some(raw) = raw else {
        return Ok("enabled".to_string());
    };

    match raw.trim().to_ascii_lowercase().as_str() {
        "enabled" | "active" => Ok("enabled".to_string()),
        "disabled" | "inactive" => Ok("disabled".to_string()),
        _ => Err(AppError::BadRequest("status只能为enabled/disabled或active/inactive".to_string())),
    }
}

fn normalize_status_filter(raw: Option<&str>) -> Result<Option<String>, AppError> {
    match raw {
        Some(value) => Ok(Some(normalize_store_status(Some(value))?)),
        None => Ok(None),
    }
}

fn canonical_store_status(raw: Option<&str>) -> String {
    normalize_store_status(raw).unwrap_or_else(|_| "enabled".to_string())
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

fn join_region(province: Option<&str>, city: Option<&str>, district: Option<&str>) -> String {
    [province, city, district]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}
