use anyhow::Context;
use axum::{
    Json,
    extract::{Path, State},
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
pub struct UpdateStoreRequest {
    pub store_name: Option<String>,
    pub address: Option<String>,
    pub business_hours: Option<String>,
    pub contact_name: Option<String>,
    pub contact_mobile: Option<String>,
}

pub async fn list_stores(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<StoreListResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let stores: Vec<StoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "store", value! {})
        .await
        .context("failed to query stores")
        .map_err(AppError::Internal)?;
    let items = stores
        .into_iter()
        .map(|item| StoreItem {
            id: item.id.unwrap_or_default(),
            store_name: item.store_name.unwrap_or_default(),
            contact_name: item.contact_name,
            contact_mobile: item.contact_mobile,
            address: item.address,
            business_hours: item.business_hours,
            status: item.status.unwrap_or_else(|| "enabled".to_string()),
            technician_count: 0,
            customer_count: 0,
            completed_task_count: 0,
            completion_rate: 0.0,
        })
        .collect::<Vec<_>>();
    let total = items.len();

    Ok(Json(ApiResponse::success(StoreListResponse { items, total })))
}

pub async fn get_store_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(store_id): Path<i64>,
) -> Result<Json<ApiResponse<StoreItem>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let store: StoreEntity = db::select_one_by_map_compat(state.rb.as_ref(), "store", value! { "id": store_id })
        .await
        .context("failed to query store by id")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("门店不存在".to_string()))?;

    Ok(Json(ApiResponse::success(StoreItem {
        id: store.id.unwrap_or_default(),
        store_name: store.store_name.unwrap_or_default(),
        contact_name: store.contact_name,
        contact_mobile: store.contact_mobile,
        address: store.address,
        business_hours: store.business_hours,
        status: store.status.unwrap_or_else(|| "enabled".to_string()),
        technician_count: db::select_by_map_compat::<UserStoreEntity>(state.rb.as_ref(), "user_store", value! { "store_id": store_id })
            .await
            .context("failed to query store technician count")
            .map_err(AppError::Internal)?
            .len(),
        customer_count: db::select_by_map_compat::<TaskEntity>(state.rb.as_ref(), "task", value! { "store_id": store_id })
            .await
            .context("failed to query store customer count")
            .map_err(AppError::Internal)?
            .into_iter()
            .filter_map(|item| item.customer_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        completed_task_count: db::select_by_map_compat::<TaskEntity>(state.rb.as_ref(), "task", value! { "store_id": store_id, "task_status": "completed" })
            .await
            .context("failed to query store completed task count")
            .map_err(AppError::Internal)?
            .len(),
        completion_rate: {
            let total = db::select_by_map_compat::<TaskEntity>(state.rb.as_ref(), "task", value! { "store_id": store_id })
                .await
                .context("failed to query store total task count")
                .map_err(AppError::Internal)?
                .len();
            let completed = db::select_by_map_compat::<TaskEntity>(state.rb.as_ref(), "task", value! { "store_id": store_id, "task_status": "completed" })
                .await
                .context("failed to query store completion rate")
                .map_err(AppError::Internal)?
                .len();
            if total == 0 { 0.0 } else { (completed as f64 / total as f64) * 100.0 }
        },
    })))
}

pub async fn update_store_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(store_id): Path<i64>,
    Json(payload): Json<UpdateStoreRequest>,
) -> Result<Json<ApiResponse<StoreItem>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let existing: StoreEntity = db::select_one_by_map_compat(state.rb.as_ref(), "store", value! { "id": store_id })
        .await
        .context("failed to query store before update")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("门店不存在".to_string()))?;

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
