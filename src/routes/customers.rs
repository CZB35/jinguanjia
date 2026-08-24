use anyhow::{Context, anyhow};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::Utc;
use rbs::value;
use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    audit, auth, db,
    error::AppError,
    identity,
    models::{
        customer::CustomerEntity, customer_address::CustomerAddressEntity,
        customer_source::CustomerSourceEntity, task::TaskEntity,
        task_service_item::TaskServiceItemEntity, user::UserEntity,
    },
    response::ApiResponse,
    routes::imports::{deserialize_vec_i64_from_mixed, normalize_delete_ids},
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct CustomerListQuery {
    #[serde(alias = "customer_mobile", alias = "customerMobile", alias = "phone")]
    pub mobile: Option<String>,
    #[serde(alias = "customerName")]
    pub name: Option<String>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct CustomerSearchByMobileQuery {
    #[serde(alias = "phone", alias = "contact_phone", alias = "keyword")]
    pub mobile: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateCustomerRequest {
    pub name: String,
    pub mobile: String,
    pub source: Option<String>,
    pub source_id: Option<i64>,
    pub customer_source_id: Option<i64>,
    pub remark: Option<String>,
    pub primary_store_id: Option<i64>,
    pub store_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_address_payload")]
    pub address: Option<AddressPayload>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCustomerRequest {
    pub name: Option<String>,
    pub mobile: Option<String>,
    pub source: Option<String>,
    pub source_id: Option<i64>,
    pub customer_source_id: Option<i64>,
    pub remark: Option<String>,
    pub primary_store_id: Option<i64>,
    pub store_id: Option<i64>,
    pub province: Option<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_address_payload")]
    pub address: Option<AddressPayload>,
}

#[derive(Debug, Deserialize)]
pub struct BatchDeleteCustomersRequest {
    #[serde(
        default,
        alias = "ids",
        alias = "customerIds",
        deserialize_with = "deserialize_vec_i64_from_mixed"
    )]
    pub customer_ids: Vec<i64>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AddressPayload {
    pub province: Option<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub detail: String,
}

fn deserialize_optional_address_payload<'de, D>(
    deserializer: D,
) -> Result<Option<AddressPayload>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };

    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(text) => Ok(Some(AddressPayload {
            province: None,
            city: None,
            district: None,
            detail: text.trim().to_string(),
        })),
        other => serde_json::from_value::<AddressPayload>(other)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

#[derive(Debug, Serialize)]
pub struct CustomerItem {
    pub id: i64,
    pub name: String,
    pub mobile: String,
    pub source: Option<String>,
    pub primary_store_id: Option<i64>,
    pub primary_store_name: Option<String>,
    pub store_ids: Vec<i64>,
    pub address_count: usize,
    pub default_address: Option<String>,
    pub province: Option<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub address: Option<String>,
    pub order_count: usize,
    pub service_count: usize,
    pub latest_service_date: Option<String>,
    pub latest_service_status: Option<String>,
    pub store_names: Vec<String>,
    pub service_history: Vec<CustomerServiceHistoryItem>,
    #[serde(skip_serializing)]
    pub created_at_sort: String,
}

#[derive(Debug, Serialize)]
pub struct CustomerListResponse {
    pub items: Vec<CustomerItem>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, Serialize)]
pub struct CustomerDetailResponse {
    pub id: i64,
    pub name: String,
    pub mobile: String,
    pub source: Option<String>,
    pub source_id: Option<i64>,
    pub customer_source_id: Option<i64>,
    pub remark: Option<String>,
    pub store_id: Option<i64>,
    pub primary_store_id: Option<i64>,
    pub primary_store_name: Option<String>,
    pub store_ids: Vec<i64>,
    pub province: Option<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub address: Option<String>,
    pub addresses: Vec<AddressPayload>,
    pub task_count: usize,
    pub service_count: usize,
    pub latest_service_date: Option<String>,
    pub latest_service_status: Option<String>,
    pub store_names: Vec<String>,
    pub default_address: Option<String>,
    pub service_history: Vec<CustomerServiceHistoryItem>,
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
    pub appointment_time: Option<String>,
    pub store_name: String,
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

    let mut customers: Vec<CustomerEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "customer", value! {})
            .await
            .context("failed to query customers")
            .map_err(AppError::Internal)?;

    if auth_user.has_role("dealer")
        && !auth_user.has_role("hq_reviewer")
        && !auth_user.has_role("super_admin")
    {
        let store_ids = load_user_store_ids(&state, auth_user.user_id).await?;
        let scoped_tasks = if store_ids.is_empty() {
            Vec::new()
        } else {
            active_tasks(
                db::select_by_map_compat::<TaskEntity>(
                    state.rb.as_ref(),
                    "task",
                    value! { "store_id": store_ids.clone() },
                )
                .await
                .context("failed to query dealer scoped customer tasks")
                .map_err(AppError::Internal)?,
            )
        };
        let visible_customer_ids = scoped_tasks
            .into_iter()
            .filter_map(|item| item.customer_id)
            .collect::<std::collections::BTreeSet<_>>();
        customers.retain(|customer| {
            customer
                .primary_store_id
                .map(|id| store_ids.contains(&id))
                .unwrap_or(false)
                || customer
                    .id
                    .map(|id| visible_customer_ids.contains(&id))
                    .unwrap_or(false)
        });
    }

    customers.retain(|customer| {
        if customer.deleted_at.is_some() {
            return false;
        }
        if let Some(mobile) = &query.mobile
            && !customer
                .mobile
                .as_deref()
                .unwrap_or_default()
                .contains(mobile)
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
        let addresses: Vec<CustomerAddressEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "customer_address",
            value! { "customer_id": customer_id },
        )
        .await
        .context("failed to query customer addresses")
        .map_err(AppError::Internal)?;
        let tasks = active_tasks(
            db::select_by_map_compat(
                state.rb.as_ref(),
                "task",
                value! { "customer_id": customer_id },
            )
            .await
            .context("failed to query customer tasks for list")
            .map_err(AppError::Internal)?,
        );
        let latest_service_date = tasks
            .iter()
            .filter_map(|item| item.appointment_time)
            .max()
            .map(|dt| dt.format("%Y-%m-%d").to_string());
        let latest_task = tasks.iter().max_by_key(|item| item.appointment_time);
        let store_ids = tasks
            .iter()
            .filter_map(|item| item.store_id)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let store_names = if store_ids.is_empty() {
            Vec::new()
        } else {
            db::select_by_map_compat::<crate::models::store::StoreEntity>(
                state.rb.as_ref(),
                "store",
                value! { "id": store_ids.clone() },
            )
            .await
            .context("failed to query customer store names")
            .map_err(AppError::Internal)?
            .into_iter()
            .filter_map(|item| item.store_name)
            .collect::<Vec<_>>()
        };
        let primary_store_id = customer
            .primary_store_id
            .or_else(|| store_ids.first().copied());
        let primary_store_name = load_store_name(&state, primary_store_id).await?;
        let default_address_entity = addresses
            .iter()
            .find(|item| item.is_default == Some(1))
            .cloned()
            .or_else(|| addresses.first().cloned());
        items.push(CustomerItem {
            id: customer_id,
            name: customer.name.unwrap_or_default(),
            mobile: customer.mobile.unwrap_or_default(),
            source: customer.source,
            primary_store_id,
            primary_store_name,
            store_ids: store_ids.clone(),
            address_count: addresses.len(),
            default_address: default_address_entity
                .as_ref()
                .and_then(|item| item.address.clone()),
            province: default_address_entity
                .as_ref()
                .and_then(|item| item.province.clone()),
            city: default_address_entity
                .as_ref()
                .and_then(|item| item.city.clone()),
            district: default_address_entity
                .as_ref()
                .and_then(|item| item.district.clone()),
            address: default_address_entity
                .as_ref()
                .and_then(|item| item.address.clone()),
            order_count: tasks.len(),
            service_count: tasks.len(),
            latest_service_date,
            latest_service_status: latest_task.and_then(|item| item.task_status.clone()),
            store_names,
            service_history: Vec::new(),
            created_at_sort: customer
                .created_at
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_default(),
        });
    }

    items.sort_by(|left, right| {
        right
            .created_at_sort
            .cmp(&left.created_at_sort)
            .then_with(|| right.id.cmp(&left.id))
    });

    let total = items.len();
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).max(1);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let items = items
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();
    Ok(Json(ApiResponse::success(CustomerListResponse {
        items,
        total,
        page,
        page_size,
    })))
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

    let existing: Vec<CustomerEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "customer",
        value! { "mobile": payload.mobile.as_str() },
    )
    .await
    .context("failed to query existing customer")
    .map_err(AppError::Internal)?;
    if !existing.is_empty() {
        return Err(AppError::Conflict("该手机号客户已存在".to_string()));
    }

    let primary_store_id = resolve_customer_store_id(
        &state,
        &auth_user,
        payload.primary_store_id.or(payload.store_id),
    )
    .await?;
    let source = resolve_customer_source_value(
        &state,
        payload.source,
        payload.source_id.or(payload.customer_source_id),
    )
    .await?;

    CustomerEntity::insert(
        state.rb.as_ref(),
        &CustomerEntity {
            id: None,
            name: Some(payload.name.clone()),
            mobile: Some(payload.mobile.clone()),
            source: source.or_else(|| Some("manual".to_string())),
            remark: payload.remark.clone(),
            primary_store_id,
            created_by: Some(auth_user.user_id),
            created_at: None,
            updated_at: None,
            deleted_at: None,
        },
    )
    .await
    .context("failed to create customer")
    .map_err(AppError::Internal)?;

    let customer: CustomerEntity = db::select_one_by_map_compat(
        state.rb.as_ref(),
        "customer",
        value! { "mobile": payload.mobile.as_str() },
    )
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

    let existing_customer = find_customer(&state, customer_id).await?;
    let next_mobile = match payload.mobile.as_deref() {
        Some(value) => {
            let value = value.trim();
            if value.is_empty() {
                return Err(AppError::BadRequest("请输入客户手机号".to_string()));
            }
            let exists: Vec<CustomerEntity> =
                db::select_by_map_compat(state.rb.as_ref(), "customer", value! { "mobile": value })
                    .await
                    .context("failed to query existing customer mobile during update")
                    .map_err(AppError::Internal)?;
            if exists.into_iter().any(|item| item.id != Some(customer_id)) {
                return Err(AppError::Conflict("该手机号客户已存在".to_string()));
            }
            Some(value.to_string())
        }
        None => existing_customer.mobile.clone(),
    };
    let requested_store_id = payload.primary_store_id.or(payload.store_id);
    let primary_store_id = if requested_store_id.is_some() {
        resolve_customer_store_id(&state, &auth_user, requested_store_id).await?
    } else {
        existing_customer.primary_store_id
    };
    let source = if payload.source.is_some()
        || payload.source_id.is_some()
        || payload.customer_source_id.is_some()
    {
        resolve_customer_source_value(
            &state,
            payload.source.clone(),
            payload.source_id.or(payload.customer_source_id),
        )
        .await?
    } else {
        existing_customer.source.clone()
    };
    let has_address_update = payload.address.is_some()
        || payload
            .province
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .is_some()
        || payload
            .city
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .is_some()
        || payload
            .district
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .is_some();

    CustomerEntity::update_by_map(
        state.rb.as_ref(),
        &CustomerEntity {
            name: payload.name.clone(),
            mobile: next_mobile,
            source,
            remark: payload.remark.clone(),
            primary_store_id,
            ..Default::default()
        },
        value! { "id": customer_id, "column": ["name", "mobile", "source", "remark", "primary_store_id"] },
    )
    .await
    .context("failed to update customer")
    .map_err(AppError::Internal)?;

    if has_address_update {
        let address_payload = payload.address.unwrap_or_else(|| AddressPayload {
            province: payload.province.clone(),
            city: payload.city.clone(),
            district: payload.district.clone(),
            detail: String::new(),
        });
        let detail = address_payload.detail.trim().to_string();
        if detail.is_empty() {
            return Err(AppError::BadRequest("请输入详细地址".to_string()));
        }

        let existing_addresses: Vec<CustomerAddressEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "customer_address",
            value! { "customer_id": customer_id },
        )
        .await
        .context("failed to query customer addresses during update")
        .map_err(AppError::Internal)?;

        if let Some(default_address) = existing_addresses
            .iter()
            .find(|item| item.is_default == Some(1))
            .or_else(|| existing_addresses.first())
        {
            CustomerAddressEntity::update_by_map(
                state.rb.as_ref(),
                &CustomerAddressEntity {
                    province: address_payload.province.clone(),
                    city: address_payload.city.clone(),
                    district: address_payload.district.clone(),
                    address: Some(detail),
                    is_default: Some(1),
                    ..Default::default()
                },
                value! { "id": default_address.id.unwrap_or_default(), "column": ["province", "city", "district", "address", "is_default"] },
            )
            .await
            .context("failed to update customer address")
            .map_err(AppError::Internal)?;
        } else {
            CustomerAddressEntity::insert(
                state.rb.as_ref(),
                &CustomerAddressEntity {
                    id: None,
                    customer_id: Some(customer_id),
                    province: address_payload.province.clone(),
                    city: address_payload.city.clone(),
                    district: address_payload.district.clone(),
                    address: Some(detail),
                    is_default: Some(1),
                    created_at: None,
                },
            )
            .await
            .context("failed to insert customer address during update")
            .map_err(AppError::Internal)?;
        }
    }

    get_customer_detail_inner(&state, customer_id).await
}

pub async fn delete_customer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(customer_id): Path<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let customer = find_customer(&state, customer_id).await?;
    ensure_customer_access(&state, &auth_user, &customer).await?;
    soft_delete_customer(
        &state,
        &auth_user,
        &customer,
        "delete_customer",
        "后台删除客户",
    )
    .await?;

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn batch_delete_customers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<BatchDeleteCustomersRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    let customer_ids = normalize_delete_ids(payload.customer_ids, "customer_ids")?;

    let mut customers = Vec::with_capacity(customer_ids.len());
    for customer_id in customer_ids {
        let customer = find_customer(&state, customer_id).await?;
        ensure_customer_access(&state, &auth_user, &customer).await?;
        customers.push(customer);
    }

    let deleted_count = customers.len();
    for customer in &customers {
        soft_delete_customer(
            &state,
            &auth_user,
            customer,
            "batch_delete_customer",
            "后台批量删除客户",
        )
        .await?;
    }

    Ok(Json(ApiResponse::success(serde_json::json!({
        "ok": true,
        "deleted_count": deleted_count
    }))))
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
    let tasks = active_tasks(
        db::select_by_map_compat(
            state.rb.as_ref(),
            "task",
            value! { "customer_id": customer_id },
        )
        .await
        .context("failed to query customer service history")
        .map_err(AppError::Internal)?,
    );
    let items = build_customer_service_history_items(&state, tasks).await?;
    Ok(Json(ApiResponse::success(items)))
}

pub async fn search_customer_by_mobile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CustomerSearchByMobileQuery>,
) -> Result<Json<ApiResponse<Vec<CustomerItem>>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    if query.mobile.trim().is_empty() {
        return Err(AppError::BadRequest("mobile不能为空".to_string()));
    }

    let mut customers: Vec<CustomerEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "customer", value! {})
            .await
            .context("failed to search customer by mobile")
            .map_err(AppError::Internal)?;
    customers.retain(|item| {
        item.deleted_at.is_none()
            && item
                .mobile
                .as_deref()
                .unwrap_or_default()
                .contains(query.mobile.trim())
    });

    if auth_user.has_role("dealer")
        && !auth_user.has_role("hq_reviewer")
        && !auth_user.has_role("super_admin")
    {
        let store_ids = load_user_store_ids(&state, auth_user.user_id).await?;
        if store_ids.is_empty() {
            customers.clear();
        } else {
            let scoped_tasks = active_tasks(
                db::select_by_map_compat::<TaskEntity>(
                    state.rb.as_ref(),
                    "task",
                    value! { "store_id": store_ids.clone() },
                )
                .await
                .context("failed to query dealer scoped tasks for customer mobile search")
                .map_err(AppError::Internal)?,
            );
            let visible_customer_ids = scoped_tasks
                .into_iter()
                .filter_map(|item| item.customer_id)
                .collect::<std::collections::BTreeSet<_>>();
            customers.retain(|customer| {
                customer
                    .primary_store_id
                    .map(|id| store_ids.contains(&id))
                    .unwrap_or(false)
                    || customer
                        .id
                        .map(|id| visible_customer_ids.contains(&id))
                        .unwrap_or(false)
            });
        }
    }

    let mut result = Vec::new();
    for item in customers {
        let customer_id = item.id.unwrap_or_default();
        let addresses: Vec<CustomerAddressEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "customer_address",
            value! { "customer_id": customer_id },
        )
        .await
        .context("failed to query customer addresses for search")
        .map_err(AppError::Internal)?;
        let tasks = active_tasks(
            db::select_by_map_compat(
                state.rb.as_ref(),
                "task",
                value! { "customer_id": customer_id },
            )
            .await
            .context("failed to query customer tasks for search")
            .map_err(AppError::Internal)?,
        );
        let store_ids = tasks
            .iter()
            .filter_map(|task| task.store_id)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let store_names = if store_ids.is_empty() {
            Vec::new()
        } else {
            db::select_by_map_compat::<crate::models::store::StoreEntity>(
                state.rb.as_ref(),
                "store",
                value! { "id": store_ids.clone() },
            )
            .await
            .context("failed to query customer store names for search")
            .map_err(AppError::Internal)?
            .into_iter()
            .filter_map(|item| item.store_name)
            .collect::<Vec<_>>()
        };
        let primary_store_id = item.primary_store_id.or_else(|| store_ids.first().copied());
        let primary_store_name = load_store_name(&state, primary_store_id).await?;
        let default_address_entity = addresses
            .iter()
            .find(|addr| addr.is_default == Some(1))
            .cloned()
            .or_else(|| addresses.first().cloned());
        result.push(CustomerItem {
            id: customer_id,
            name: item.name.unwrap_or_default(),
            mobile: item.mobile.unwrap_or_default(),
            source: item.source,
            primary_store_id,
            primary_store_name,
            store_ids: store_ids.clone(),
            address_count: addresses.len(),
            default_address: default_address_entity
                .as_ref()
                .and_then(|addr| addr.address.clone()),
            province: default_address_entity
                .as_ref()
                .and_then(|addr| addr.province.clone()),
            city: default_address_entity
                .as_ref()
                .and_then(|addr| addr.city.clone()),
            district: default_address_entity
                .as_ref()
                .and_then(|addr| addr.district.clone()),
            address: default_address_entity
                .as_ref()
                .and_then(|addr| addr.address.clone()),
            order_count: tasks.len(),
            service_count: tasks.len(),
            latest_service_date: tasks
                .iter()
                .filter_map(|task| task.appointment_time)
                .max()
                .map(|dt| dt.format("%Y-%m-%d").to_string()),
            latest_service_status: tasks
                .iter()
                .max_by_key(|task| task.appointment_time)
                .and_then(|task| task.task_status.clone()),
            store_names,
            service_history: build_customer_service_history_items(&state, tasks.clone()).await?,
            created_at_sort: item
                .created_at
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_default(),
        });
    }
    Ok(Json(ApiResponse::success(result)))
}

async fn get_customer_detail_inner(
    state: &AppState,
    customer_id: i64,
) -> Result<Json<ApiResponse<CustomerDetailResponse>>, AppError> {
    let customer = find_customer(state, customer_id).await?;
    let addresses: Vec<CustomerAddressEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "customer_address",
        value! { "customer_id": customer_id },
    )
    .await
    .context("failed to query customer addresses")
    .map_err(AppError::Internal)?;
    let tasks = active_tasks(
        db::select_by_map_compat(
            state.rb.as_ref(),
            "task",
            value! { "customer_id": customer_id },
        )
        .await
        .context("failed to query customer tasks")
        .map_err(AppError::Internal)?,
    );
    let latest_service_date = tasks
        .iter()
        .filter_map(|item| item.appointment_time)
        .max()
        .map(|dt| dt.format("%Y-%m-%d").to_string());
    let latest_service_status = tasks
        .iter()
        .max_by_key(|item| item.appointment_time)
        .and_then(|item| item.task_status.clone());
    let service_history = build_customer_service_history_items(state, tasks.clone()).await?;
    let store_ids = tasks
        .iter()
        .filter_map(|item| item.store_id)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let store_names = if store_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<crate::models::store::StoreEntity>(
            state.rb.as_ref(),
            "store",
            value! { "id": store_ids.clone() },
        )
        .await
        .context("failed to query customer detail store names")
        .map_err(AppError::Internal)?
        .into_iter()
        .filter_map(|item| item.store_name)
        .collect::<Vec<_>>()
    };
    let primary_store_id = customer
        .primary_store_id
        .or_else(|| store_ids.first().copied());
    let primary_store_name = load_store_name(state, primary_store_id).await?;
    let default_address_entity = addresses
        .iter()
        .find(|item| item.is_default == Some(1))
        .cloned()
        .or_else(|| addresses.first().cloned());
    let default_address = default_address_entity
        .as_ref()
        .and_then(|item| item.address.clone());
    let source_ids = resolve_customer_source_ids(state, customer.source.as_deref()).await?;

    Ok(Json(ApiResponse::success(CustomerDetailResponse {
        id: customer.id.unwrap_or_default(),
        name: customer.name.unwrap_or_default(),
        mobile: customer.mobile.unwrap_or_default(),
        source: customer.source,
        source_id: source_ids,
        customer_source_id: source_ids,
        remark: customer.remark,
        store_id: primary_store_id,
        primary_store_id,
        primary_store_name,
        store_ids,
        province: default_address_entity
            .as_ref()
            .and_then(|item| item.province.clone()),
        city: default_address_entity
            .as_ref()
            .and_then(|item| item.city.clone()),
        district: default_address_entity
            .as_ref()
            .and_then(|item| item.district.clone()),
        address: default_address.clone(),
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
        service_history,
    })))
}

async fn build_customer_service_history_items(
    state: &AppState,
    tasks: Vec<TaskEntity>,
) -> Result<Vec<CustomerServiceHistoryItem>, AppError> {
    let mut items = Vec::new();
    for item in tasks {
        let task_id = item.id.unwrap_or_default();
        let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "task_service_item",
            value! { "task_id": task_id },
        )
        .await
        .context("failed to query customer service history task service items")
        .map_err(AppError::Internal)?;
        let technician_name = if let Some(technician_id) = item.current_technician_id {
            load_user_name(state, technician_id)
                .await
                .unwrap_or_default()
        } else {
            String::new()
        };
        let store_name = load_store_name(state, item.store_id)
            .await?
            .unwrap_or_default();
        items.push(CustomerServiceHistoryItem {
            task_id,
            task_no: item.task_no.unwrap_or_default(),
            service_type: customer_service_type_text(item.service_type.as_deref()).to_string(),
            task_status: item.task_status.unwrap_or_default(),
            service_items: service_items
                .into_iter()
                .map(|service_item| CustomerServiceItemPayload {
                    service_type: customer_service_type_text(service_item.service_type.as_deref())
                        .to_string(),
                    service_status: service_item.service_status.unwrap_or_default(),
                })
                .collect(),
            technician_name,
            service_date: item
                .appointment_time
                .map(|dt| dt.format("%Y-%m-%d").to_string()),
            appointment_time: item
                .appointment_time
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string()),
            store_name,
        });
    }

    items.sort_by(|left, right| right.appointment_time.cmp(&left.appointment_time));
    Ok(items)
}

fn active_tasks(tasks: Vec<TaskEntity>) -> Vec<TaskEntity> {
    tasks
        .into_iter()
        .filter(|task| task.deleted_at.is_none())
        .collect()
}

async fn ensure_customer_access(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    customer: &CustomerEntity,
) -> Result<(), AppError> {
    if auth_user.has_role("hq_reviewer") || auth_user.has_role("super_admin") {
        return Ok(());
    }
    if !auth_user.has_role("dealer") {
        return Err(AppError::Forbidden("您没有权限执行该操作".to_string()));
    }

    let store_ids = load_user_store_ids(state, auth_user.user_id).await?;
    if customer
        .primary_store_id
        .map(|store_id| store_ids.contains(&store_id))
        .unwrap_or(false)
    {
        return Ok(());
    }

    let customer_id = customer.id.unwrap_or_default();
    let tasks = active_tasks(
        db::select_by_map_compat(
            state.rb.as_ref(),
            "task",
            value! { "customer_id": customer_id },
        )
        .await
        .context("failed to query customer tasks for delete access")
        .map_err(AppError::Internal)?,
    );
    if tasks
        .iter()
        .filter_map(|task| task.store_id)
        .any(|store_id| store_ids.contains(&store_id))
    {
        return Ok(());
    }

    Err(AppError::Forbidden(
        "您只能删除所属门店范围内的客户".to_string(),
    ))
}

async fn soft_delete_customer(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    customer: &CustomerEntity,
    action: &str,
    remark: &str,
) -> Result<(), AppError> {
    let customer_id = customer
        .id
        .ok_or_else(|| AppError::Internal(anyhow!("customer.id missing")))?;
    CustomerEntity::update_by_map(
        state.rb.as_ref(),
        &CustomerEntity {
            deleted_at: Some(Utc::now().naive_utc()),
            ..Default::default()
        },
        value! { "id": customer_id, "column": ["deleted_at"] },
    )
    .await
    .context("failed to soft delete customer")
    .map_err(AppError::Internal)?;

    let after_customer = db::select_one_by_map_compat::<CustomerEntity>(
        state.rb.as_ref(),
        "customer",
        value! { "id": customer_id },
    )
    .await
    .context("failed to query customer after delete")
    .map_err(AppError::Internal)?;
    audit::write_audit_log(
        state,
        "customer",
        customer_id,
        action,
        auth_user.user_id,
        auth_user.primary_role(),
        Some(customer),
        after_customer.as_ref(),
        Some(remark.to_string()),
    )
    .await
}

async fn find_customer(state: &AppState, customer_id: i64) -> Result<CustomerEntity, AppError> {
    let customer: CustomerEntity =
        db::select_one_by_map_compat(state.rb.as_ref(), "customer", value! { "id": customer_id })
            .await
            .context("failed to query customer by id")
            .map_err(AppError::Internal)?
            .ok_or_else(|| AppError::NotFound("客户不存在或已删除".to_string()))?;
    if customer.deleted_at.is_some() {
        return Err(AppError::NotFound("客户不存在或已删除".to_string()));
    }
    Ok(customer)
}

async fn load_user_name(state: &AppState, user_id: i64) -> Result<String, AppError> {
    let user = db::select_one_by_map_compat::<UserEntity>(
        state.rb.as_ref(),
        "user",
        value! { "id": user_id },
    )
    .await
    .context("failed to query user name for customer history")
    .map_err(AppError::Internal)?;
    let Some(user) = user else {
        return Ok(String::new());
    };
    identity::role_display_name(state, user_id, "technician", user.name.as_deref()).await
}

async fn resolve_customer_source_value(
    state: &AppState,
    source: Option<String>,
    source_id: Option<i64>,
) -> Result<Option<String>, AppError> {
    if let Some(source) = source
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
    {
        return Ok(Some(source));
    }
    let Some(source_id) = source_id else {
        return Ok(None);
    };

    let customer_source = db::select_one_by_map_compat::<CustomerSourceEntity>(
        state.rb.as_ref(),
        "customer_source",
        value! { "id": source_id },
    )
    .await
    .context("failed to query customer source by id")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::BadRequest("客户来源不存在".to_string()))?;

    Ok(customer_source.code.or(customer_source.name))
}

async fn resolve_customer_source_ids(
    state: &AppState,
    source: Option<&str>,
) -> Result<Option<i64>, AppError> {
    let Some(source) = source.map(str::trim).filter(|item| !item.is_empty()) else {
        return Ok(None);
    };
    let items: Vec<CustomerSourceEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "customer_source", value! {})
            .await
            .context("failed to query customer source ids")
            .map_err(AppError::Internal)?;
    Ok(items.into_iter().find_map(|item| {
        let matches = item.code.as_deref() == Some(source) || item.name.as_deref() == Some(source);
        if matches { item.id } else { None }
    }))
}

fn customer_service_type_text(service_type: Option<&str>) -> &'static str {
    match service_type.unwrap_or_default() {
        "measure" => "量尺服务",
        "install" => "安装服务",
        "ironing" => "熨烫服务",
        "cleaning" => "除尘除螨服务",
        _ => "其他服务",
    }
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
    Ok(stores
        .into_iter()
        .filter_map(|item| item.store_id)
        .collect())
}

async fn resolve_customer_store_id(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    requested_store_id: Option<i64>,
) -> Result<Option<i64>, AppError> {
    let store_id = if auth_user.has_role("dealer")
        && !auth_user.has_role("hq_reviewer")
        && !auth_user.has_role("super_admin")
    {
        Some(dealer_owned_store_id(state, auth_user).await?)
    } else if requested_store_id.is_some() {
        requested_store_id
    } else {
        None
    };

    if let Some(store_id) = store_id {
        let exists = db::select_one_by_map_compat::<crate::models::store::StoreEntity>(
            state.rb.as_ref(),
            "store",
            value! { "id": store_id },
        )
        .await
        .context("failed to query customer primary store")
        .map_err(AppError::Internal)?;
        if exists.is_none() {
            return Err(AppError::BadRequest("门店不存在".to_string()));
        }
    }

    Ok(store_id)
}

async fn dealer_owned_store_id(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
) -> Result<i64, AppError> {
    let store_ids = load_user_store_ids(state, auth_user.user_id).await?;

    match store_ids.as_slice() {
        [store_id] => Ok(*store_id),
        [] => Err(AppError::Forbidden("当前经销商账号未绑定门店".to_string())),
        _ => Err(AppError::Conflict(
            "当前经销商账号绑定了多个门店，请先在后台修正为单门店".to_string(),
        )),
    }
}

async fn load_store_name(
    state: &AppState,
    store_id: Option<i64>,
) -> Result<Option<String>, AppError> {
    let Some(store_id) = store_id else {
        return Ok(None);
    };
    let store = db::select_one_by_map_compat::<crate::models::store::StoreEntity>(
        state.rb.as_ref(),
        "store",
        value! { "id": store_id },
    )
    .await
    .context("failed to query store name for customer")
    .map_err(AppError::Internal)?;
    Ok(store.and_then(|item| item.store_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn customer_batch_delete_request_accepts_legacy_string_ids() {
        let payload: BatchDeleteCustomersRequest =
            serde_json::from_value(serde_json::json!({ "ids": ["12", 13] })).unwrap();
        assert_eq!(payload.customer_ids, vec![12, 13]);
    }
}
