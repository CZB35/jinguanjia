use anyhow::Context;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::Utc;
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    auth,
    db,
    error::AppError,
    models::{
        customer::CustomerEntity,
        evidence::EvidenceEntity,
        hq_score_item::HqScoreItemEntity,
        notice::NoticeEntity,
        store::StoreEntity,
        task::TaskEntity,
        task_service_item::TaskServiceItemEntity,
        user::UserEntity,
    },
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct ReviewTaskQuery {
    pub task_status: Option<String>,
    pub service_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReviewTaskItem {
    pub id: i64,
    pub task_id: i64,
    pub task_no: String,
    pub service_type: String,
    pub task_status: String,
    pub service_status: String,
    pub review_status: String,
    pub score_status: String,
    pub store_name: String,
    pub dealer_name: String,
    pub technician_name: String,
    pub technician_mobile: String,
    pub customer_name: String,
    pub customer_mobile: String,
    pub appointment_time: Option<String>,
    pub technician_score: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct ReviewTaskListResponse {
    pub items: Vec<ReviewTaskItem>,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct ReviewTaskDetailResponse {
    pub id: i64,
    pub task_no: String,
    pub service_type: String,
    pub task_status: String,
    pub review_status: String,
    pub score_status: String,
    pub store_name: String,
    pub dealer_name: String,
    pub technician_name: String,
    pub technician_mobile: String,
    pub customer_name: String,
    pub customer_mobile: String,
    pub customer_address: String,
    pub appointment_time: Option<String>,
    pub service_time_slot: Option<String>,
    pub task_desc: Option<String>,
    pub task_remark: Option<String>,
    pub evidence_count: usize,
    pub service_items: Vec<ReviewTaskServiceItem>,
}

#[derive(Debug, Serialize)]
pub struct ReviewTaskServiceItem {
    pub id: i64,
    pub service_type: String,
    pub service_status: String,
    pub technician_id: Option<i64>,
    pub technician_name: Option<String>,
    pub technician_mobile: Option<String>,
    pub technician_job_no: Option<String>,
    pub scheduled_time: Option<String>,
    pub progress_percent: i32,
    pub score: Option<i32>,
    pub deduction_reason: Option<String>,
}

pub async fn list_review_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReviewTaskQuery>,
) -> Result<Json<ApiResponse<ReviewTaskListResponse>>, AppError> {
    ensure_hq_access(&state, &headers)?;

    let tasks: Vec<TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! {})
        .await
        .context("failed to query review tasks")
        .map_err(AppError::Internal)?;
    let mut items = Vec::new();
    for task in tasks {
        if let Some(task_status) = &query.task_status
            && task.task_status.as_deref() != Some(task_status.as_str())
        {
            continue;
        }
        let task_id = task.id.unwrap_or_default();
        let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(state.rb.as_ref(), "task_service_item", value! { "task_id": task_id })
            .await
            .context("failed to query task service items for review list")
            .map_err(AppError::Internal)?;
        for service_item in service_items {
            if let Some(service_type) = &query.service_type
                && service_item.service_type.as_deref() != Some(service_type.as_str())
            {
                continue;
            }
            items.push(build_review_task_item(&state, &task, &service_item).await?);
        }
    }
    let total = items.len();

    Ok(Json(ApiResponse::success(ReviewTaskListResponse { items, total })))
}

pub async fn get_review_task_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<i64>,
) -> Result<Json<ApiResponse<ReviewTaskDetailResponse>>, AppError> {
    ensure_hq_access(&state, &headers)?;
    let task = find_task(&state, task_id).await?;
    let evidences: Vec<EvidenceEntity> = db::select_by_map_compat(state.rb.as_ref(), "evidence", value! { "task_id": task_id })
        .await
        .context("failed to query task evidences for review detail")
        .map_err(AppError::Internal)?;
    let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(state.rb.as_ref(), "task_service_item", value! { "task_id": task_id })
        .await
        .context("failed to query review detail service items")
        .map_err(AppError::Internal)?;
    let customer = load_customer(&state, task.customer_id.unwrap_or_default()).await?;
    let dealer = load_user(&state, task.dealer_user_id.unwrap_or_default()).await?;
    let technician = if let Some(technician_id) = task.current_technician_id {
        Some(load_user(&state, technician_id).await?)
    } else {
        None
    };
    let store = load_store(&state, task.store_id.unwrap_or_default()).await?;
    let address = load_customer_address_text(&state, task.customer_address_id.unwrap_or_default()).await?;

    Ok(Json(ApiResponse::success(ReviewTaskDetailResponse {
        id: task.id.unwrap_or_default(),
        task_no: task.task_no.unwrap_or_default(),
        service_type: task.service_type.unwrap_or_default(),
        task_status: task.task_status.unwrap_or_default(),
        review_status: task.review_status.unwrap_or_default(),
        score_status: task.score_status.unwrap_or_default(),
        store_name: store.store_name.unwrap_or_default(),
        dealer_name: dealer.name.unwrap_or_default(),
        technician_name: technician.as_ref().and_then(|item| item.name.clone()).unwrap_or_default(),
        technician_mobile: technician.as_ref().and_then(|item| item.mobile.clone()).unwrap_or_default(),
        customer_name: customer.name.unwrap_or_default(),
        customer_mobile: customer.mobile.unwrap_or_default(),
        customer_address: address,
        appointment_time: task.appointment_time.map(format_datetime),
        service_time_slot: task.service_time_slot,
        task_desc: task.task_desc,
        task_remark: task.task_remark,
        evidence_count: evidences.len(),
        service_items: build_review_task_service_items(&state, service_items, task_id).await?,
    })))
}

pub async fn list_review_reminders(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<ReviewTaskListResponse>>, AppError> {
    ensure_hq_access(&state, &headers)?;
    let tasks: Vec<TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! { "task_status": "pending_dealer_review" })
        .await
        .context("failed to query review reminder tasks")
        .map_err(AppError::Internal)?;

    let mut items = Vec::new();
    for task in tasks {
        let task_id = task.id.unwrap_or_default();
        let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(state.rb.as_ref(), "task_service_item", value! { "task_id": task_id })
            .await
            .context("failed to query reminder task service items")
            .map_err(AppError::Internal)?;
        for service_item in service_items {
            items.push(build_review_task_item(&state, &task, &service_item).await?);
        }
    }
    let total = items.len();

    Ok(Json(ApiResponse::success(ReviewTaskListResponse { items, total })))
}

pub async fn send_review_remind(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["hq_reviewer", "super_admin"])?;
    let task = find_task(&state, task_id).await?;

    NoticeEntity::insert(
        state.rb.as_ref(),
        &NoticeEntity {
            id: None,
            receiver_id: task.dealer_user_id,
            notice_type: Some("site".to_string()),
            biz_type: Some("task_review_reminder".to_string()),
            biz_id: Some(task_id),
            title: Some("任务评价提醒".to_string()),
            content: Some("您有一条待评价任务，请尽快处理".to_string()),
            is_read: Some(0),
            read_at: None,
            send_status: Some("sent".to_string()),
            sent_at: Some(Utc::now().naive_utc()),
            created_at: None,
        },
    )
    .await
    .context("failed to create review reminder notice")
    .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(serde_json::json!({ "ok": true }))))
}

fn ensure_hq_access(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let auth_user = auth::authenticate(state, headers)?;
    auth_user.require_any_role(&["hq_reviewer", "super_admin"])
}

async fn find_task(state: &AppState, task_id: i64) -> Result<TaskEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "task", value! { "id": task_id })
        .await
        .context("failed to query review task")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("任务不存在或已删除".to_string()))
}

async fn build_review_task_item(
    state: &AppState,
    task: &TaskEntity,
    service_item: &TaskServiceItemEntity,
) -> Result<ReviewTaskItem, AppError> {
    let dealer = load_user(state, task.dealer_user_id.unwrap_or_default()).await?;
    let customer = load_customer(state, task.customer_id.unwrap_or_default()).await?;
    let store = load_store(state, task.store_id.unwrap_or_default()).await?;
    let technician = if let Some(technician_id) = service_item.technician_id.or(task.current_technician_id) {
        Some(load_user(state, technician_id).await?)
    } else {
        None
    };
    let score_item: Option<HqScoreItemEntity> = db::select_one_by_map_compat(
        state.rb.as_ref(),
        "hq_score_item",
        value! { "task_id": task.id.unwrap_or_default(), "service_type": service_item.service_type.as_deref().unwrap_or_default() },
    )
    .await
    .context("failed to query score item for review task")
    .map_err(AppError::Internal)?;

    Ok(ReviewTaskItem {
        id: service_item.id.unwrap_or_default(),
        task_id: task.id.unwrap_or_default(),
        task_no: task.task_no.clone().unwrap_or_default(),
        service_type: service_item.service_type.clone().unwrap_or_default(),
        task_status: task.task_status.clone().unwrap_or_default(),
        service_status: service_item.service_status.clone().unwrap_or_default(),
        review_status: task.review_status.clone().unwrap_or_default(),
        score_status: task.score_status.clone().unwrap_or_default(),
        store_name: store.store_name.unwrap_or_default(),
        dealer_name: dealer.name.unwrap_or_default(),
        technician_name: technician.as_ref().and_then(|item| item.name.clone()).unwrap_or_default(),
        technician_mobile: technician.as_ref().and_then(|item| item.mobile.clone()).unwrap_or_default(),
        customer_name: customer.name.unwrap_or_default(),
        customer_mobile: customer.mobile.unwrap_or_default(),
        appointment_time: service_item.scheduled_time.or(task.appointment_time).map(format_datetime),
        technician_score: score_item.and_then(|item| item.score),
    })
}

async fn build_review_task_service_items(
    state: &AppState,
    items: Vec<TaskServiceItemEntity>,
    task_id: i64,
) -> Result<Vec<ReviewTaskServiceItem>, AppError> {
    let mut result = Vec::new();
    for item in items {
        let technician = if let Some(technician_id) = item.technician_id {
            Some(load_user(state, technician_id).await?)
        } else {
            None
        };
        let score_item: Option<HqScoreItemEntity> = db::select_one_by_map_compat(
            state.rb.as_ref(),
            "hq_score_item",
            value! { "task_id": task_id, "service_type": item.service_type.as_deref().unwrap_or_default() },
        )
        .await
        .context("failed to query score item for review detail")
        .map_err(AppError::Internal)?;

        result.push(ReviewTaskServiceItem {
            id: item.id.unwrap_or_default(),
            service_type: item.service_type.unwrap_or_default(),
            service_status: item.service_status.unwrap_or_default(),
            technician_id: item.technician_id,
            technician_name: technician.as_ref().and_then(|x| x.name.clone()),
            technician_mobile: technician.as_ref().and_then(|x| x.mobile.clone()),
            technician_job_no: technician.as_ref().and_then(|x| x.job_no.clone()),
            scheduled_time: item.scheduled_time.map(format_datetime),
            progress_percent: item.progress_percent.unwrap_or_default(),
            score: score_item.as_ref().and_then(|x| x.score),
            deduction_reason: score_item.and_then(|x| x.deduction_reason),
        });
    }
    Ok(result)
}

async fn load_user(state: &AppState, user_id: i64) -> Result<UserEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "user", value! { "id": user_id })
        .await
        .context("failed to query user in hq review")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("用户不存在".to_string()))
}

async fn load_customer(state: &AppState, customer_id: i64) -> Result<CustomerEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "customer", value! { "id": customer_id })
        .await
        .context("failed to query customer in hq review")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("客户不存在".to_string()))
}

async fn load_store(state: &AppState, store_id: i64) -> Result<StoreEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "store", value! { "id": store_id })
        .await
        .context("failed to query store in hq review")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("门店不存在".to_string()))
}

async fn load_customer_address_text(state: &AppState, address_id: i64) -> Result<String, AppError> {
    let addresses: Vec<crate::models::customer_address::CustomerAddressEntity> = db::select_by_map_compat(state.rb.as_ref(), "customer_address", value! { "id": address_id })
        .await
        .context("failed to query customer address in hq review")
        .map_err(AppError::Internal)?;
    Ok(addresses.into_iter().next().and_then(|item| item.address).unwrap_or_default())
}

fn format_datetime(input: chrono::NaiveDateTime) -> String {
    input.format("%Y-%m-%d %H:%M:%S").to_string()
}
