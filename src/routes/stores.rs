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
    models::{store::StoreEntity, task::TaskEntity, user_store::UserStoreEntity},
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct StoreListQuery {
    pub store_name: Option<String>,
    pub contact_mobile: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StoreItem {
    pub id: i64,
    pub store_name: String,
    pub contact_name: Option<String>,
    pub contact_mobile: Option<String>,
    pub address: Option<String>,
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
}

#[derive(Debug, Deserialize)]
pub struct CreateStoreRequest {
    pub store_name: String,
    pub address: Option<String>,
    pub business_hours: Option<String>,
    pub contact_name: Option<String>,
    pub contact_mobile: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateStoreRequest {
    pub store_name: Option<String>,
    pub address: Option<String>,
    pub business_hours: Option<String>,
    pub contact_name: Option<String>,
    pub contact_mobile: Option<String>,
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

    let mut stores: Vec<StoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "store", value! {})
        .await
        .context("failed to query stores")
        .map_err(AppError::Internal)?;
    stores.retain(|item| {
        if let Some(store_name) = &query.store_name
            && !item.store_name.as_deref().unwrap_or_default().contains(store_name)
        {
            return false;
        }
        if let Some(contact_mobile) = &query.contact_mobile
            && !item.contact_mobile.as_deref().unwrap_or_default().contains(contact_mobile)
        {
            return false;
        }
        if let Some(status) = &query.status
            && item.status.as_deref() != Some(status.as_str())
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

    Ok(Json(ApiResponse::success(StoreListResponse { items, total })))
}

pub async fn create_store(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateStoreRequest>,
) -> Result<Json<ApiResponse<StoreItem>>, AppError> {
    ensure_store_manage_access(&state, &headers)?;
    if payload.store_name.trim().is_empty() {
        return Err(AppError::BadRequest("请输入门店名称".to_string()));
    }

    let existing: Vec<StoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "store",
        value! { "store_name": payload.store_name.as_str() },
    )
    .await
    .context("failed to query existing store by name")
    .map_err(AppError::Internal)?;
    if !existing.is_empty() {
        return Err(AppError::Conflict("门店名称已存在".to_string()));
    }

    StoreEntity::insert(
        state.rb.as_ref(),
        &StoreEntity {
            id: None,
            store_name: Some(payload.store_name.clone()),
            contact_name: payload.contact_name.clone(),
            contact_mobile: payload.contact_mobile.clone(),
            province: None,
            city: None,
            district: None,
            address: payload.address.clone(),
            business_hours: payload.business_hours.clone(),
            status: Some("enabled".to_string()),
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
        value! { "store_name": payload.store_name.as_str() },
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

    let existing = find_store(state.rb.as_ref(), store_id).await?;
    let updated = StoreEntity {
        store_name: payload.store_name.or(existing.store_name.clone()),
        address: payload.address.or(existing.address.clone()),
        business_hours: payload.business_hours.or(existing.business_hours.clone()),
        contact_name: payload.contact_name.or(existing.contact_name.clone()),
        contact_mobile: payload.contact_mobile.or(existing.contact_mobile.clone()),
        ..existing.clone()
    };

    StoreEntity::update_by_map(
        state.rb.as_ref(),
        &updated,
        value! { "id": store_id, "column": ["store_name", "address", "business_hours", "contact_name", "contact_mobile"] },
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
    if payload.status != "enabled" && payload.status != "disabled" {
        return Err(AppError::BadRequest("status只能为enabled或disabled".to_string()));
    }

    StoreEntity::update_by_map(
        state.rb.as_ref(),
        &StoreEntity {
            status: Some(payload.status),
            ..Default::default()
        },
        value! { "id": store_id },
    )
    .await
    .context("failed to update store status")
    .map_err(AppError::Internal)?;

    get_store_detail(State(state), headers, Path(store_id)).await
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

    Ok(StoreItem {
        id: store_id,
        store_name: store.store_name.unwrap_or_default(),
        contact_name: store.contact_name,
        contact_mobile: store.contact_mobile,
        address: store.address,
        business_hours: store.business_hours,
        status: store.status.unwrap_or_else(|| "enabled".to_string()),
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

fn ensure_store_manage_access(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let auth_user = auth::authenticate(state, headers)?;
    auth_user.require_any_role(&["hq_reviewer", "super_admin"])
}
