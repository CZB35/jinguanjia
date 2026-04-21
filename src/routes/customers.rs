use anyhow::{Context, anyhow};
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
    models::{customer::CustomerEntity, customer_address::CustomerAddressEntity, task::TaskEntity, task_service_item::TaskServiceItemEntity, user::UserEntity},
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct CustomerListQuery {
    pub mobile: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CustomerSearchByMobileQuery {
    pub mobile: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateCustomerRequest {
    pub name: String,
    pub mobile: String,
    pub source: Option<String>,
    pub remark: Option<String>,
    pub address: Option<AddressPayload>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCustomerRequest {
    pub name: Option<String>,
    pub source: Option<String>,
    pub remark: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AddressPayload {
    pub province: Option<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct CustomerItem {
    pub id: i64,
    pub name: String,
    pub mobile: String,
    pub source: Option<String>,
    pub address_count: usize,
    pub default_address: Option<String>,
    pub order_count: usize,
    pub service_count: usize,
    pub latest_service_date: Option<String>,
    pub latest_service_status: Option<String>,
    pub store_names: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CustomerListResponse {
    pub items: Vec<CustomerItem>,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct CustomerDetailResponse {
    pub id: i64,
    pub name: String,
    pub mobile: String,
    pub source: Option<String>,
    pub remark: Option<String>,
    pub addresses: Vec<AddressPayload>,
    pub task_count: usize,
    pub service_count: usize,
    pub latest_service_date: Option<String>,
    pub latest_service_status: Option<String>,
    pub store_names: Vec<String>,
    pub default_address: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CustomerServiceHistoryItem {
    pub task_id: i64,
    pub task_no: String,
    pub service_type: String,
    pub task_status: String,
    pub service_items: Vec<CustomerServiceItemPayload>,
    pub technician_name: String,
    pub service_date: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CustomerServiceItemPayload {
    pub service_type: String,
    pub service_status: String,
}

pub async fn list_customers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CustomerListQuery>,
) -> Result<Json<ApiResponse<CustomerListResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let mut customers: Vec<CustomerEntity> = db::select_by_map_compat(state.rb.as_ref(), "customer", value! {})
        .await
        .context("failed to query customers")
        .map_err(AppError::Internal)?;

    if auth_user.has_role("dealer") && !auth_user.has_role("hq_reviewer") && !auth_user.has_role("super_admin") {
        let store_ids = load_user_store_ids(&state, auth_user.user_id).await?;
        let scoped_tasks = if store_ids.is_empty() {
            Vec::new()
        } else {
            db::select_by_map_compat::<TaskEntity>(state.rb.as_ref(), "task", value! { "store_id": store_ids })
                .await
                .context("failed to query dealer scoped customer tasks")
                .map_err(AppError::Internal)?
        };
        let visible_customer_ids = scoped_tasks
            .into_iter()
            .filter_map(|item| item.customer_id)
            .collect::<std::collections::BTreeSet<_>>();
        customers.retain(|customer| customer.id.map(|id| visible_customer_ids.contains(&id)).unwrap_or(false));
    }

    customers.retain(|customer| {
        if let Some(mobile) = &query.mobile
            && !customer.mobile.as_deref().unwrap_or_default().contains(mobile)
        {
            return false;
        }
        if let Some(name) = &query.name
            && !customer.name.as_deref().unwrap_or_default().contains(name)
        {
            return false;
        }
        true
    });

    let mut items = Vec::with_capacity(customers.len());
    for customer in customers {
        let customer_id = customer.id.unwrap_or_default();
        let addresses: Vec<CustomerAddressEntity> = db::select_by_map_compat(state.rb.as_ref(), "customer_address", value! { "customer_id": customer_id })
            .await
            .context("failed to query customer addresses")
            .map_err(AppError::Internal)?;
        let tasks: Vec<TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! { "customer_id": customer_id })
            .await
            .context("failed to query customer tasks for list")
            .map_err(AppError::Internal)?;
        let latest_service_date = tasks
            .iter()
            .filter_map(|item| item.appointment_time)
            .max()
            .map(|dt| dt.format("%Y-%m-%d").to_string());
        let latest_task = tasks.iter().max_by_key(|item| item.appointment_time);
        let store_ids = tasks.iter().filter_map(|item| item.store_id).collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>();
        let store_names = if store_ids.is_empty() {
            Vec::new()
        } else {
            db::select_by_map_compat::<crate::models::store::StoreEntity>(state.rb.as_ref(), "store", value! { "id": store_ids })
                .await
                .context("failed to query customer store names")
                .map_err(AppError::Internal)?
                .into_iter()
                .filter_map(|item| item.store_name)
                .collect::<Vec<_>>()
        };
        items.push(CustomerItem {
            id: customer_id,
            name: customer.name.unwrap_or_default(),
            mobile: customer.mobile.unwrap_or_default(),
            source: customer.source,
            address_count: addresses.len(),
            default_address: addresses.iter().find(|item| item.is_default == Some(1)).and_then(|item| item.address.clone()).or_else(|| addresses.first().and_then(|item| item.address.clone())),
            order_count: tasks.len(),
            service_count: tasks.len(),
            latest_service_date,
            latest_service_status: latest_task.and_then(|item| item.task_status.clone()),
            store_names,
        });
    }

    let total = items.len();
    Ok(Json(ApiResponse::success(CustomerListResponse { items, total })))
}

pub async fn create_customer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateCustomerRequest>,
) -> Result<Json<ApiResponse<CustomerDetailResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    if payload.name.trim().is_empty() {
        return Err(AppError::BadRequest("请输入客户姓名".to_string()));
    }
    if payload.mobile.trim().is_empty() {
        return Err(AppError::BadRequest("请输入客户手机号".to_string()));
    }

    let existing: Vec<CustomerEntity> = db::select_by_map_compat(state.rb.as_ref(), "customer", value! { "mobile": payload.mobile.as_str() })
        .await
        .context("failed to query existing customer")
        .map_err(AppError::Internal)?;
    if !existing.is_empty() {
        return Err(AppError::Conflict("该手机号客户已存在".to_string()));
    }

    CustomerEntity::insert(
        state.rb.as_ref(),
        &CustomerEntity {
            id: None,
            name: Some(payload.name.clone()),
            mobile: Some(payload.mobile.clone()),
            source: payload.source.clone().or_else(|| Some("manual".to_string())),
            remark: payload.remark.clone(),
            created_by: Some(auth_user.user_id),
            created_at: None,
            updated_at: None,
        },
    )
    .await
    .context("failed to create customer")
    .map_err(AppError::Internal)?;

    let customer: CustomerEntity = db::select_one_by_map_compat(state.rb.as_ref(), "customer", value! { "mobile": payload.mobile.as_str() })
        .await
        .context("failed to fetch created customer")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow!("customer create succeeded but fetch failed")))?;

    if let Some(address) = payload.address {
        CustomerAddressEntity::insert(
            state.rb.as_ref(),
            &CustomerAddressEntity {
                id: None,
                customer_id: customer.id,
                province: address.province,
                city: address.city,
                district: address.district,
                address: Some(address.detail),
                is_default: Some(1),
                created_at: None,
            },
        )
        .await
        .context("failed to create customer address")
        .map_err(AppError::Internal)?;
    }

    get_customer_detail_inner(&state, customer.id.unwrap_or_default()).await
}

pub async fn update_customer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(customer_id): Path<i64>,
    Json(payload): Json<UpdateCustomerRequest>,
) -> Result<Json<ApiResponse<CustomerDetailResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let _ = find_customer(&state, customer_id).await?;

    CustomerEntity::update_by_map(
        state.rb.as_ref(),
        &CustomerEntity {
            name: payload.name.clone(),
            source: payload.source.clone(),
            remark: payload.remark.clone(),
            ..Default::default()
        },
        value! { "id": customer_id },
    )
    .await
    .context("failed to update customer")
    .map_err(AppError::Internal)?;

    get_customer_detail_inner(&state, customer_id).await
}

pub async fn get_customer_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(customer_id): Path<i64>,
) -> Result<Json<ApiResponse<CustomerDetailResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    get_customer_detail_inner(&state, customer_id).await
}

pub async fn get_customer_service_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(customer_id): Path<i64>,
) -> Result<Json<ApiResponse<Vec<CustomerServiceHistoryItem>>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let _ = find_customer(&state, customer_id).await?;
    let tasks: Vec<TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! { "customer_id": customer_id })
        .await
        .context("failed to query customer service history")
        .map_err(AppError::Internal)?;
    let mut items = Vec::new();
    for item in tasks {
        let task_id = item.id.unwrap_or_default();
        let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(state.rb.as_ref(), "task_service_item", value! { "task_id": task_id })
            .await
            .context("failed to query customer service history task service items")
            .map_err(AppError::Internal)?;
        let technician_name = if let Some(technician_id) = item.current_technician_id {
            load_user_name(&state, technician_id).await.unwrap_or_default()
        } else {
            String::new()
        };
        items.push(CustomerServiceHistoryItem {
            task_id,
            task_no: item.task_no.unwrap_or_default(),
            service_type: item.service_type.unwrap_or_default(),
            task_status: item.task_status.unwrap_or_default(),
            service_items: service_items
                .into_iter()
                .map(|service_item| CustomerServiceItemPayload {
                    service_type: service_item.service_type.unwrap_or_default(),
                    service_status: service_item.service_status.unwrap_or_default(),
                })
                .collect(),
            technician_name,
            service_date: item.appointment_time.map(|dt| dt.format("%Y-%m-%d").to_string()),
        });
    }
    Ok(Json(ApiResponse::success(items)))
}

pub async fn search_customer_by_mobile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CustomerSearchByMobileQuery>,
) -> Result<Json<ApiResponse<Vec<CustomerItem>>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let customers: Vec<CustomerEntity> = db::select_by_map_compat(state.rb.as_ref(), "customer", value! { "mobile": query.mobile.as_str() })
        .await
        .context("failed to search customer by mobile")
        .map_err(AppError::Internal)?;
    let mut result = Vec::new();
    for item in customers {
        let customer_id = item.id.unwrap_or_default();
        let addresses: Vec<CustomerAddressEntity> = db::select_by_map_compat(state.rb.as_ref(), "customer_address", value! { "customer_id": customer_id })
            .await
            .context("failed to query customer addresses for search")
            .map_err(AppError::Internal)?;
        let tasks: Vec<TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! { "customer_id": customer_id })
            .await
            .context("failed to query customer tasks for search")
            .map_err(AppError::Internal)?;
        let store_ids = tasks.iter().filter_map(|task| task.store_id).collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>();
        let store_names = if store_ids.is_empty() {
            Vec::new()
        } else {
            db::select_by_map_compat::<crate::models::store::StoreEntity>(state.rb.as_ref(), "store", value! { "id": store_ids })
                .await
                .context("failed to query customer store names for search")
                .map_err(AppError::Internal)?
                .into_iter()
                .filter_map(|item| item.store_name)
                .collect::<Vec<_>>()
        };
        result.push(CustomerItem {
            id: customer_id,
            name: item.name.unwrap_or_default(),
            mobile: item.mobile.unwrap_or_default(),
            source: item.source,
            address_count: addresses.len(),
            default_address: addresses.iter().find(|addr| addr.is_default == Some(1)).and_then(|addr| addr.address.clone()).or_else(|| addresses.first().and_then(|addr| addr.address.clone())),
            order_count: tasks.len(),
            service_count: tasks.len(),
            latest_service_date: tasks.iter().filter_map(|task| task.appointment_time).max().map(|dt| dt.format("%Y-%m-%d").to_string()),
            latest_service_status: tasks.iter().max_by_key(|task| task.appointment_time).and_then(|task| task.task_status.clone()),
            store_names,
        });
    }
    Ok(Json(ApiResponse::success(result)))
}

async fn get_customer_detail_inner(
    state: &AppState,
    customer_id: i64,
) -> Result<Json<ApiResponse<CustomerDetailResponse>>, AppError> {
    let customer = find_customer(state, customer_id).await?;
    let addresses: Vec<CustomerAddressEntity> = db::select_by_map_compat(state.rb.as_ref(), "customer_address", value! { "customer_id": customer_id })
        .await
        .context("failed to query customer addresses")
        .map_err(AppError::Internal)?;
    let tasks: Vec<TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! { "customer_id": customer_id })
        .await
        .context("failed to query customer tasks")
        .map_err(AppError::Internal)?;
    let latest_service_date = tasks
        .iter()
        .filter_map(|item| item.appointment_time)
        .max()
        .map(|dt| dt.format("%Y-%m-%d").to_string());
    let latest_service_status = tasks.iter().max_by_key(|item| item.appointment_time).and_then(|item| item.task_status.clone());
    let store_ids = tasks.iter().filter_map(|item| item.store_id).collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>();
    let store_names = if store_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<crate::models::store::StoreEntity>(state.rb.as_ref(), "store", value! { "id": store_ids })
            .await
            .context("failed to query customer detail store names")
            .map_err(AppError::Internal)?
            .into_iter()
            .filter_map(|item| item.store_name)
            .collect::<Vec<_>>()
    };
    let default_address = addresses.iter().find(|item| item.is_default == Some(1)).and_then(|item| item.address.clone()).or_else(|| addresses.first().and_then(|item| item.address.clone()));

    Ok(Json(ApiResponse::success(CustomerDetailResponse {
        id: customer.id.unwrap_or_default(),
        name: customer.name.unwrap_or_default(),
        mobile: customer.mobile.unwrap_or_default(),
        source: customer.source,
        remark: customer.remark,
        addresses: addresses
            .into_iter()
            .map(|item| AddressPayload {
                province: item.province,
                city: item.city,
                district: item.district,
                detail: item.address.unwrap_or_default(),
            })
            .collect(),
        task_count: tasks.len(),
        service_count: tasks.len(),
        latest_service_date,
        latest_service_status,
        store_names,
        default_address,
    })))
}

async fn find_customer(state: &AppState, customer_id: i64) -> Result<CustomerEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "customer", value! { "id": customer_id })
        .await
        .context("failed to query customer by id")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("客户不存在".to_string()))
}

async fn load_user_name(state: &AppState, user_id: i64) -> Result<String, AppError> {
    let user = db::select_one_by_map_compat::<UserEntity>(state.rb.as_ref(), "user", value! { "id": user_id })
        .await
        .context("failed to query user name for customer history")
        .map_err(AppError::Internal)?;
    Ok(user.and_then(|item| item.name).unwrap_or_default())
}

async fn load_user_store_ids(state: &AppState, user_id: i64) -> Result<Vec<i64>, AppError> {
    let stores: Vec<crate::models::user_store::UserStoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_store",
        value! { "user_id": user_id },
    )
    .await
    .context("failed to query user stores for customer scope")
    .map_err(AppError::Internal)?;
    Ok(stores.into_iter().filter_map(|item| item.store_id).collect())
}
