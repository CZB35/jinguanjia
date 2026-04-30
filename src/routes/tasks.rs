use anyhow::{Context, anyhow};
use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use chrono::{Local, NaiveDateTime, Utc};
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    audit, auth, db,
    error::AppError,
    models::{
        audit_log::AuditLogEntity, customer::CustomerEntity,
        customer_address::CustomerAddressEntity, dealer_review::DealerReviewEntity,
        evidence::EvidenceEntity, store::StoreEntity, task::TaskEntity,
        task_assignment::TaskAssignmentEntity, task_execution::TaskExecutionEntity,
        task_service_item::TaskServiceItemEntity, technician_skill::TechnicianSkillEntity,
        user::UserEntity, user_store::UserStoreEntity,
    },
    notify,
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    pub customer_name: String,
    pub customer_mobile: String,
    pub customer_source: Option<String>,
    pub store_id: Option<i64>,
    pub address_id: Option<i64>,
    pub address: Option<AddressRequest>,
    pub service_type: Option<String>,
    pub service_types: Option<Vec<String>>,
    pub appointment_time: String,
    pub service_time_slot: Option<String>,
    pub priority: Option<String>,
    pub task_desc: Option<String>,
    pub task_remark: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddressRequest {
    pub province: Option<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub detail: String,
}

#[derive(Debug, Deserialize)]
pub struct TaskListQuery {
    pub task_no: Option<String>,
    pub service_type: Option<String>,
    pub task_status: Option<String>,
    pub review_status: Option<String>,
    pub score_status: Option<String>,
    pub store_id: Option<i64>,
    pub dealer_user_id: Option<i64>,
    pub technician_id: Option<i64>,
    pub appointment_start: Option<String>,
    pub appointment_end: Option<String>,
    pub mobile: Option<String>,
    pub customer_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TaskDetailQuery {
    pub service_item_id: Option<i64>,
    pub service_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AssignTaskRequest {
    pub technician_id: Option<i64>,
    pub service_type: Option<String>,
    pub service_item_id: Option<i64>,
    pub assignments: Option<Vec<ServiceAssignmentRequest>>,
    pub assign_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ServiceAssignmentRequest {
    pub service_type: Option<String>,
    pub service_item_id: Option<i64>,
    pub technician_id: i64,
}

#[derive(Debug)]
struct ResolvedAssignment {
    service_item_id: Option<i64>,
    service_type: String,
    technician_id: i64,
    from_technician_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ReassignTaskRequest {
    pub technician_id: i64,
    pub assign_reason: String,
}

#[derive(Debug, Deserialize)]
pub struct StartTaskRequest {
    pub confirm_standard_read: bool,
    pub service_item_id: Option<i64>,
    pub service_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SubmitTaskRequest {
    pub execution_id: i64,
    pub service_item_id: Option<i64>,
    pub service_type: Option<String>,
    pub submit_remark: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CloseTaskRequest {
    pub close_reason: String,
}

#[derive(Debug, Serialize)]
pub struct TaskSummary {
    pub id: i64,
    pub service_item_id: Option<i64>,
    pub task_no: String,
    pub service_type: String,
    pub service_status: Option<String>,
    pub service_items: Vec<TaskServiceItemPayload>,
    pub service_items_text: String,
    pub task_status: String,
    pub task_status_text: String,
    pub display_status: String,
    pub display_status_text: String,
    pub review_status: String,
    pub review_status_text: String,
    pub score_status: String,
    pub score_status_text: String,
    pub appointment_time: Option<String>,
    pub customer_name: String,
    pub customer_mobile: String,
    pub address: String,
    pub store_id: i64,
    pub store_name: String,
    pub dealer_user_id: i64,
    pub dealer_name: String,
    pub current_technician_id: Option<i64>,
    pub technician_name: Option<String>,
    pub technician_mobile: Option<String>,
    pub progress_percent: i32,
    pub is_overdue: bool,
    pub overdue_text: String,
    pub task_tags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TaskListResponse {
    pub items: Vec<TaskSummary>,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct CreateTaskResponse {
    pub id: i64,
    pub task_no: String,
    pub task_status: String,
    pub store_id: i64,
    pub dealer_user_id: i64,
    pub service_items: Vec<TaskServiceItemPayload>,
    pub items: Vec<CreateTaskItemResponse>,
}

#[derive(Debug, Serialize, Clone)]
pub struct CreateTaskItemResponse {
    pub id: i64,
    pub task_no: String,
    pub task_status: String,
    pub service_type: String,
    pub store_id: i64,
    pub dealer_user_id: i64,
    pub service_items: Vec<TaskServiceItemPayload>,
}

#[derive(Debug, Serialize)]
pub struct TaskDetailResponse {
    pub id: i64,
    pub task_no: String,
    pub service_type: String,
    pub service_items: Vec<TaskServiceItemPayload>,
    pub service_items_text: String,
    pub task_tags: Vec<String>,
    pub task_status: String,
    pub task_status_text: String,
    pub display_status: String,
    pub display_status_text: String,
    pub review_status: String,
    pub review_status_text: String,
    pub score_status: String,
    pub score_status_text: String,
    pub is_overdue: bool,
    pub overdue_text: String,
    pub priority: String,
    pub priority_text: String,
    pub appointment_time: Option<String>,
    pub service_time_slot: Option<String>,
    pub task_desc: Option<String>,
    pub task_remark: Option<String>,
    pub customer: RelatedCustomer,
    pub store: RelatedStore,
    pub dealer: Option<RelatedDealer>,
    pub technician: Option<RelatedTechnician>,
    pub progress_percent: i32,
    pub completed_step_count: i32,
    pub total_step_count: i32,
    pub progress_tracks: Vec<ProgressTrackItem>,
    pub can_assign: bool,
    pub can_reassign: bool,
    pub can_submit_review: bool,
    pub can_contact_customer: bool,
    pub review_info: Option<ReviewInfoPayload>,
    pub assignments: Vec<AssignmentPayload>,
    pub executions: Vec<ExecutionPayload>,
    pub evidence_count: usize,
    pub evidence_items: Vec<EvidenceItemPayload>,
}

#[derive(Debug, Serialize)]
pub struct ProgressTrackItem {
    pub key: String,
    pub title: String,
    pub finished: bool,
    pub time: Option<String>,
    pub expected_time: Option<String>,
    pub service_type: Option<String>,
    pub technician_id: Option<i64>,
    pub technician_name: Option<String>,
    pub images: Vec<ProgressTrackImage>,
}

#[derive(Debug, Serialize)]
pub struct ProgressTrackImage {
    pub id: i64,
    pub file_name: String,
    pub file_url: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct TaskServiceItemPayload {
    pub id: i64,
    pub service_type: String,
    pub service_status: String,
    pub technician_id: Option<i64>,
    pub scheduled_time: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub progress_percent: i32,
    pub sort_no: i32,
    pub is_overdue: bool,
}

#[derive(Debug, Serialize)]
pub struct RelatedCustomer {
    pub id: i64,
    pub name: String,
    pub mobile: String,
    pub contact_phone: String,
    pub address: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RelatedStore {
    pub id: i64,
    pub name: String,
    pub business_hours: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RelatedTechnician {
    pub id: i64,
    pub name: String,
    pub mobile: String,
    pub avatar_url: Option<String>,
    pub job_no: Option<String>,
    pub average_score: f64,
}

#[derive(Debug, Serialize)]
pub struct RelatedDealer {
    pub id: i64,
    pub name: String,
    pub mobile: String,
}

#[derive(Debug, Serialize)]
pub struct EvidenceItemPayload {
    pub id: i64,
    pub evidence_type: String,
    pub file_name: String,
    pub file_url: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReviewInfoPayload {
    pub star: i32,
    pub tags: Vec<String>,
    pub content: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AssignmentPayload {
    pub id: i64,
    pub to_technician_id: i64,
    pub assign_type: String,
    pub assign_reason: Option<String>,
    pub assigned_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExecutionPayload {
    pub id: i64,
    pub technician_id: i64,
    pub execution_status: String,
    pub started_at: Option<String>,
    pub submitted_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TaskActionResponse {
    pub task_id: i64,
    pub task_status: String,
    pub service_item_id: Option<i64>,
    pub service_type: Option<String>,
    pub execution_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct TaskLogItem {
    pub id: i64,
    pub action: String,
    pub operator_id: i64,
    pub operator_role: String,
    pub remark: Option<String>,
    pub created_at: Option<String>,
    pub before_value: Option<String>,
    pub after_value: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TaskLogListResponse {
    pub items: Vec<TaskLogItem>,
    pub total: usize,
}

pub async fn create_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateTaskRequest>,
) -> Result<Json<ApiResponse<CreateTaskResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    validate_create_task_request(&payload)?;

    let dealer_user_id = auth_user.user_id;
    let store_id = resolve_store_id(&state, &auth_user, payload.store_id).await?;
    ensure_store_exists(&state, store_id).await?;

    let customer = find_or_create_customer(&state, &payload, dealer_user_id, store_id).await?;
    let customer_id = customer
        .id
        .ok_or_else(|| AppError::Internal(anyhow!("customer.id missing")))?;
    let customer_address_id = resolve_customer_address_id(
        &state,
        customer_id,
        payload.address_id,
        payload.address.as_ref(),
    )
    .await?;

    let appointment_time = parse_datetime(&payload.appointment_time)?;
    let service_types = resolved_service_types(&payload)?;
    let mut created_items = Vec::with_capacity(service_types.len());
    for service_type in service_types {
        let task_no = generate_task_no();
        let task = TaskEntity {
            id: None,
            task_no: Some(task_no.clone()),
            customer_id: Some(customer_id),
            customer_address_id: Some(customer_address_id),
            store_id: Some(store_id),
            dealer_user_id: Some(dealer_user_id),
            current_technician_id: None,
            service_type: Some(service_type.clone()),
            task_type: None,
            priority: Some(
                payload
                    .priority
                    .clone()
                    .unwrap_or_else(|| "normal".to_string()),
            ),
            appointment_time: Some(appointment_time),
            service_time_slot: payload.service_time_slot.clone(),
            task_desc: payload.task_desc.clone(),
            task_remark: payload.task_remark.clone(),
            task_status: Some("pending_assign".to_string()),
            review_status: Some("pending".to_string()),
            score_status: Some("pending".to_string()),
            is_overdue: Some(0),
            latest_assignment_id: None,
            latest_execution_id: None,
            source: Some(
                payload
                    .customer_source
                    .clone()
                    .unwrap_or_else(|| "mini_program".to_string()),
            ),
            created_by: Some(dealer_user_id),
            created_at: None,
            updated_at: None,
            deleted_at: None,
        };

        TaskEntity::insert(state.rb.as_ref(), &task)
            .await
            .context("failed to create task")
            .map_err(AppError::Internal)?;

        let created = find_task_by_task_no(&state, &task_no).await?;
        create_task_service_items(
            &state,
            created.id.unwrap_or_default(),
            std::slice::from_ref(&service_type),
            appointment_time,
        )
        .await?;
        let created_service_items =
            load_task_service_item_entities(&state, created.id.unwrap_or_default()).await?;

        audit::write_audit_log(
            &state,
            "task",
            created.id.unwrap_or_default(),
            "create_task",
            auth_user.user_id,
            primary_role(&auth_user),
            None::<&TaskEntity>,
            Some(&created),
            Some("创建任务".to_string()),
        )
        .await?;

        created_items.push(CreateTaskItemResponse {
            id: created.id.unwrap_or_default(),
            task_no,
            task_status: created
                .task_status
                .clone()
                .unwrap_or_else(|| "pending_assign".to_string()),
            service_type,
            store_id,
            dealer_user_id,
            service_items: map_task_service_item_payloads(created_service_items),
        });
    }

    let first = created_items
        .first()
        .cloned()
        .ok_or_else(|| AppError::Internal(anyhow!("no task created")))?;

    Ok(Json(ApiResponse::success(CreateTaskResponse {
        id: first.id,
        task_no: first.task_no,
        task_status: first.task_status,
        store_id: first.store_id,
        dealer_user_id: first.dealer_user_id,
        service_items: first.service_items,
        items: created_items,
    })))
}

pub async fn list_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TaskListQuery>,
) -> Result<Json<ApiResponse<TaskListResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let items = load_task_summaries(&state, &auth_user, &query).await?;

    let total = items.len();
    Ok(Json(ApiResponse::success(TaskListResponse {
        items,
        total,
    })))
}

pub async fn export_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TaskListQuery>,
) -> Result<Response, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let items = load_task_summaries(&state, &auth_user, &query).await?;

    let mut csv = String::from(
        "task_id,task_no,service_type,service_items,task_status,display_status,review_status,score_status,appointment_time,customer_name,customer_mobile,store_name,dealer_name,technician_name,technician_mobile,is_overdue,task_tags\n",
    );
    for item in items {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            item.id,
            csv_cell(&item.task_no),
            csv_cell(&item.service_type),
            csv_cell(&item.service_items_text),
            csv_cell(&item.task_status_text),
            csv_cell(&item.display_status_text),
            csv_cell(&item.review_status_text),
            csv_cell(&item.score_status_text),
            csv_cell(&item.appointment_time.unwrap_or_default()),
            csv_cell(&item.customer_name),
            csv_cell(&item.customer_mobile),
            csv_cell(&item.store_name),
            csv_cell(&item.dealer_name),
            csv_cell(&item.technician_name.unwrap_or_default()),
            csv_cell(&item.technician_mobile.unwrap_or_default()),
            if item.is_overdue { "是" } else { "否" },
            csv_cell(&item.task_tags.join("|")),
        ));
    }

    let mut response = Response::new(Body::from(csv));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"tasks-export.csv\""),
    );
    Ok(response)
}

async fn load_task_summaries(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    query: &TaskListQuery,
) -> Result<Vec<TaskSummary>, AppError> {
    if is_technician_only(auth_user) {
        return load_technician_task_summaries(state, auth_user, query).await;
    }
    if auth_user.has_role("dealer") {
        return load_dealer_task_summaries(state, auth_user, query).await;
    }

    let mut tasks = db::select_by_map_compat::<TaskEntity>(state.rb.as_ref(), "task", value! {})
        .await
        .context("failed to query tasks")
        .map_err(AppError::Internal)?;

    tasks.retain(|task| filter_task_item(task, query));

    let mut items = Vec::with_capacity(tasks.len());
    for task in tasks {
        items.push(build_task_summary(state, auth_user, task, None).await?);
    }

    items.retain(|item| filter_task_summary(item, query));
    Ok(items)
}

async fn load_dealer_task_summaries(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    query: &TaskListQuery,
) -> Result<Vec<TaskSummary>, AppError> {
    let store_id = dealer_owned_store_id(&state, auth_user.user_id).await?;
    let tasks: Vec<TaskEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task",
        value! { "store_id": store_id },
    )
    .await
    .context("failed to query dealer tasks")
    .map_err(AppError::Internal)?;

    let mut items = Vec::new();
    for task in tasks {
        if !filter_task_item_for_scoped_view(&task, query) {
            continue;
        }
        let Some(task_id) = task.id else {
            continue;
        };
        let service_items = load_task_service_item_entities(state, task_id).await?;
        if service_items.is_empty() {
            items.push(build_task_summary(state, auth_user, task, None).await?);
            continue;
        }
        for service_item in service_items {
            items.push(build_task_summary(state, auth_user, task.clone(), Some(service_item)).await?);
        }
    }

    items.retain(|item| filter_task_summary(item, query));
    Ok(items)
}

async fn load_technician_task_summaries(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    query: &TaskListQuery,
) -> Result<Vec<TaskSummary>, AppError> {
    let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_service_item",
        value! { "technician_id": auth_user.user_id },
    )
    .await
    .context("failed to query technician service item tasks")
    .map_err(AppError::Internal)?;

    let mut items = Vec::new();
    let mut task_ids_with_service_items = std::collections::BTreeSet::new();
    for service_item in service_items {
        let Some(task_id) = service_item.task_id else {
            continue;
        };
        let task = find_task_by_id(state, task_id).await?;
        if !filter_task_item_for_scoped_view(&task, query) {
            continue;
        }
        task_ids_with_service_items.insert(task_id);
        items.push(build_task_summary(state, auth_user, task, Some(service_item)).await?);
    }

    let legacy_tasks: Vec<TaskEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task",
        value! { "current_technician_id": auth_user.user_id },
    )
    .await
    .context("failed to query legacy technician tasks")
    .map_err(AppError::Internal)?;
    for task in legacy_tasks {
        let Some(task_id) = task.id else {
            continue;
        };
        if task_ids_with_service_items.contains(&task_id)
            || !filter_task_item_for_scoped_view(&task, query)
        {
            continue;
        }
        items.push(build_task_summary(state, auth_user, task, None).await?);
    }

    items.retain(|item| filter_task_summary(item, query));
    Ok(items)
}

pub async fn get_task_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<i64>,
    Query(query): Query<TaskDetailQuery>,
) -> Result<Json<ApiResponse<TaskDetailResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let task = find_task_by_id(&state, task_id).await?;
    ensure_task_access(&state, &auth_user, &task).await?;

    let customer = find_customer_by_id(&state, task.customer_id).await?;
    let address = find_address_by_id(&state, task.customer_address_id).await?;
    let store = find_store_by_id(&state, task.store_id).await?;
    let dealer = if let Some(dealer_user_id) = task.dealer_user_id {
        Some(find_user_by_id(&state, Some(dealer_user_id)).await?)
    } else {
        None
    };
    let assignments: Vec<TaskAssignmentEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_assignment",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query task assignments")
    .map_err(AppError::Internal)?;
    let executions: Vec<TaskExecutionEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_execution",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query task executions")
    .map_err(AppError::Internal)?;
    let evidences: Vec<EvidenceEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "evidence", value! { "task_id": task_id })
            .await
            .context("failed to query task evidence")
            .map_err(AppError::Internal)?;
    let service_items = load_task_service_item_entities(&state, task_id).await?;
    let visible_service_items =
        scoped_service_items_for_detail(&auth_user, &service_items, &query)?;
    let dealer_review = find_dealer_review_for_scope(&state, task_id, &visible_service_items).await?;
    let scoped_service_status = if visible_service_items.len() == 1 {
        visible_service_items
            .first()
            .and_then(|item| item.service_status.as_deref())
    } else {
        None
    };
    let task_status = scoped_service_status
        .map(ToString::to_string)
        .or_else(|| task.task_status.clone())
        .unwrap_or_default();
    let review_status = scoped_review_status(task.review_status.as_deref(), scoped_service_status);
    let score_status = scoped_score_status(task.score_status.as_deref(), scoped_service_status);
    let priority = task
        .priority
        .clone()
        .unwrap_or_else(|| "normal".to_string());
    let task_desc = task.task_desc.clone();
    let task_remark = task.task_remark.clone();
    let service_time_slot = task.service_time_slot.clone();
    let progress_percent =
        task_progress_percent(&visible_service_items, Some(task_status.as_str()));
    let progress_tracks = build_progress_tracks(
        &state,
        &task,
        &visible_service_items,
        &assignments,
        &executions,
        &evidences,
    )
    .await?;
    let completed_step_count = progress_tracks.iter().filter(|item| item.finished).count() as i32;
    let total_step_count = progress_tracks.len() as i32;
    let customer_mobile = customer.mobile.clone().unwrap_or_default();
    let service_items_payload = map_task_service_item_payloads(visible_service_items.clone());
    let detail_service_type = visible_service_items
        .first()
        .and_then(|item| item.service_type.clone())
        .or_else(|| task.service_type.clone())
        .unwrap_or_default();
    let technician_only = is_technician_only(&auth_user);
    let task_tags = build_task_tags(&task, &service_items_payload, technician_only);
    let task_status_text = task_status_text_for_viewer(&task_status, technician_only).to_string();
    let display_status = display_status_for_viewer(&task_status, technician_only).to_string();
    let display_status_text =
        display_status_text_for_viewer(&task_status, technician_only).to_string();
    let review_status_text =
        review_status_text_for_viewer(&review_status, &task_status, technician_only).to_string();
    let score_status_text = score_status_text(&score_status).to_string();
    let priority_text_value = priority_text(&priority).to_string();
    let can_contact_customer = !customer_mobile.is_empty();
    let is_overdue = task_is_overdue(&task);
    let overdue_text = overdue_text(is_overdue).to_string();
    let detail_technician_id = visible_service_items
        .first()
        .and_then(|item| item.technician_id)
        .or(task.current_technician_id);
    let technician = if let Some(technician_id) = detail_technician_id {
        Some(find_user_by_id(&state, Some(technician_id)).await?)
    } else {
        None
    };
    let technician_average_score = if let Some(technician_id) = detail_technician_id {
        calculate_technician_average_score(&state, technician_id).await?
    } else {
        0.0
    };
    let evidence_items = evidences
        .iter()
        .map(|item| EvidenceItemPayload {
            id: item.id.unwrap_or_default(),
            evidence_type: item.evidence_type.clone().unwrap_or_default(),
            file_name: item.file_name.clone().unwrap_or_default(),
            file_url: crate::storage::public_url(&item.file_url.clone().unwrap_or_default()),
            created_at: item.created_at.map(format_datetime),
        })
        .collect::<Vec<_>>();

    Ok(Json(ApiResponse::success(TaskDetailResponse {
        id: task.id.unwrap_or_default(),
        task_no: task.task_no.unwrap_or_default(),
        service_type: detail_service_type,
        service_items_text: service_items_text(&service_items_payload),
        task_tags,
        service_items: service_items_payload,
        task_status: task_status.clone(),
        task_status_text,
        display_status,
        display_status_text,
        review_status,
        review_status_text,
        score_status,
        score_status_text,
        is_overdue,
        overdue_text,
        priority,
        priority_text: priority_text_value,
        appointment_time: task.appointment_time.map(format_datetime),
        service_time_slot,
        task_desc,
        task_remark,
        customer: RelatedCustomer {
            id: customer.id.unwrap_or_default(),
            name: customer.name.unwrap_or_default(),
            mobile: customer_mobile.clone(),
            contact_phone: customer_mobile.clone(),
            address: address.address,
        },
        store: RelatedStore {
            id: store.id.unwrap_or_default(),
            name: store.store_name.unwrap_or_default(),
            business_hours: store.business_hours,
        },
        dealer: dealer.map(|item| RelatedDealer {
            id: item.id.unwrap_or_default(),
            name: item.name.unwrap_or_default(),
            mobile: item.mobile.unwrap_or_default(),
        }),
        technician: technician.map(|item| RelatedTechnician {
            id: item.id.unwrap_or_default(),
            name: item.name.unwrap_or_default(),
            mobile: item.mobile.unwrap_or_default(),
            avatar_url: item.avatar_url,
            job_no: item.job_no,
            average_score: technician_average_score,
        }),
        progress_percent,
        completed_step_count,
        total_step_count,
        progress_tracks,
        can_assign: matches!(task.task_status.as_deref(), Some("pending_assign")),
        can_reassign: matches!(
            task.task_status.as_deref(),
            Some("pending_execute") | Some("executing")
        ),
        can_submit_review: task_status == "pending_dealer_review",
        can_contact_customer,
        review_info: dealer_review.map(|review| ReviewInfoPayload {
            star: review.star.unwrap_or_default(),
            tags: review
                .tag_json
                .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
                .unwrap_or_default(),
            content: review.content,
            created_at: review.created_at.map(format_datetime),
        }),
        assignments: assignments
            .into_iter()
            .map(|item| AssignmentPayload {
                id: item.id.unwrap_or_default(),
                to_technician_id: item.to_technician_id.unwrap_or_default(),
                assign_type: item.assign_type.unwrap_or_default(),
                assign_reason: item.assign_reason,
                assigned_at: item.assigned_at.map(format_datetime),
            })
            .collect(),
        executions: executions
            .into_iter()
            .map(|item| ExecutionPayload {
                id: item.id.unwrap_or_default(),
                technician_id: item.technician_id.unwrap_or_default(),
                execution_status: item.execution_status.unwrap_or_default(),
                started_at: item.started_at.map(format_datetime),
                submitted_at: item.submitted_at.map(format_datetime),
            })
            .collect(),
        evidence_count: evidences.len(),
        evidence_items,
    })))
}

pub async fn assign_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<i64>,
    Json(payload): Json<AssignTaskRequest>,
) -> Result<Json<ApiResponse<TaskActionResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let task = find_task_by_id(&state, task_id).await?;
    let before_task = task.clone();
    ensure_task_access(&state, &auth_user, &task).await?;
    if task.task_status.as_deref() != Some("pending_assign") {
        return Err(AppError::Conflict("当前任务状态不允许分派技师".to_string()));
    }

    if is_legacy_whole_task_assignment(&payload) {
        let technician_id = payload
            .technician_id
            .ok_or_else(|| AppError::BadRequest("请选择技师".to_string()))?;
        ensure_technician_assignable(&state, &task, technician_id).await?;

        let assignment = TaskAssignmentEntity {
            id: None,
            task_id: Some(task_id),
            from_technician_id: task.current_technician_id,
            to_technician_id: Some(technician_id),
            assign_type: Some("assign".to_string()),
            assign_reason: payload.assign_reason.clone(),
            assigned_by: Some(auth_user.user_id),
                    assigned_at: Some(local_now()),
        };
        TaskAssignmentEntity::insert(state.rb.as_ref(), &assignment)
            .await
            .context("failed to create task assignment")
            .map_err(AppError::Internal)?;

        let latest_assignment = latest_task_assignment(&state, task_id).await?;
        TaskEntity::update_by_map(
            state.rb.as_ref(),
            &TaskEntity {
                current_technician_id: Some(technician_id),
                task_status: Some("pending_execute".to_string()),
                latest_assignment_id: latest_assignment.id,
                ..Default::default()
            },
            value! { "id": task_id },
        )
        .await
        .context("failed to update task assignment state")
        .map_err(AppError::Internal)?;
        sync_task_service_items_on_assign(&state, task_id, technician_id, "pending_execute", 0)
            .await?;

        let after_task = find_task_by_id(&state, task_id).await?;
        let assigned_service_items = load_task_service_item_entities(&state, task_id).await?;
        if assigned_service_items.is_empty() {
            send_task_assignment_notice(
                &state,
                technician_id,
                task_id,
                task.service_type.as_deref(),
                "task_assign",
            )
            .await;
        } else {
            for service_item in assigned_service_items
                .iter()
                .filter(|item| item.technician_id == Some(technician_id))
            {
                send_task_assignment_notice(
                    &state,
                    technician_id,
                    task_id,
                    service_item.service_type.as_deref(),
                    "task_assign",
                )
                .await;
            }
        }
        audit::write_audit_log(
            &state,
            "task",
            task_id,
            "assign_task",
            auth_user.user_id,
            primary_role(&auth_user),
            Some(&before_task),
            Some(&after_task),
            payload.assign_reason.clone(),
        )
        .await?;

        return Ok(Json(ApiResponse::success(TaskActionResponse {
            task_id,
            task_status: "pending_execute".to_string(),
            service_item_id: None,
            service_type: None,
            execution_id: None,
        })));
    }

    let service_items = load_task_service_item_entities(&state, task_id).await?;
    let assignments = resolve_assignment_targets(&payload, &task, &service_items)?;
    ensure_unique_assignment_targets(&assignments)?;

    for assignment in &assignments {
        ensure_technician_assignable_for_service(
            &state,
            &task,
            assignment.technician_id,
            &assignment.service_type,
        )
        .await?;
        TaskAssignmentEntity::insert(
            state.rb.as_ref(),
            &TaskAssignmentEntity {
                id: None,
                task_id: Some(task_id),
                from_technician_id: assignment.from_technician_id,
                to_technician_id: Some(assignment.technician_id),
                assign_type: Some("assign".to_string()),
                assign_reason: payload.assign_reason.clone(),
                assigned_by: Some(auth_user.user_id),
                assigned_at: Some(local_now()),
            },
        )
        .await
        .context("failed to create task service assignment")
        .map_err(AppError::Internal)?;

        if let Some(service_item_id) = assignment.service_item_id {
            TaskServiceItemEntity::update_by_map(
                state.rb.as_ref(),
                &TaskServiceItemEntity {
                    technician_id: Some(assignment.technician_id),
                    service_status: Some("pending_execute".to_string()),
                    progress_percent: Some(0),
                    ..Default::default()
                },
                value! { "id": service_item_id },
            )
            .await
            .context("failed to update task service item assignment")
            .map_err(AppError::Internal)?;
        }
    }

    let latest_assignment = latest_task_assignment(&state, task_id).await?;
    let updated_service_items = load_task_service_item_entities(&state, task_id).await?;
    let all_service_items_assigned = updated_service_items.is_empty()
        || updated_service_items
            .iter()
            .all(|item| item.technician_id.is_some());
    let task_status = if all_service_items_assigned {
        "pending_execute".to_string()
    } else {
        task.task_status
            .clone()
            .unwrap_or_else(|| "pending_assign".to_string())
    };
    let primary_technician_id = assignments
        .first()
        .map(|item| item.technician_id)
        .ok_or_else(|| AppError::BadRequest("请选择技师".to_string()))?;
    TaskEntity::update_by_map(
        state.rb.as_ref(),
        &TaskEntity {
            current_technician_id: Some(primary_technician_id),
            task_status: Some(task_status.clone()),
            latest_assignment_id: latest_assignment.id,
            ..Default::default()
        },
        value! { "id": task_id },
    )
    .await
    .context("failed to update task assignment state")
    .map_err(AppError::Internal)?;

    let after_task = find_task_by_id(&state, task_id).await?;
    for assignment in &assignments {
        send_task_assignment_notice(
            &state,
            assignment.technician_id,
            task_id,
            Some(assignment.service_type.as_str()),
            "task_assign",
        )
        .await;
    }
    audit::write_audit_log(
        &state,
        "task",
        task_id,
        "assign_task",
        auth_user.user_id,
        primary_role(&auth_user),
        Some(&before_task),
        Some(&after_task),
        payload.assign_reason.clone(),
    )
    .await?;

    Ok(Json(ApiResponse::success(TaskActionResponse {
        task_id,
        task_status,
        service_item_id: None,
        service_type: None,
        execution_id: None,
    })))
}

pub async fn start_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<i64>,
    Json(payload): Json<StartTaskRequest>,
) -> Result<Json<ApiResponse<TaskActionResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["technician"])?;

    if !payload.confirm_standard_read {
        return Err(AppError::BadRequest("请先确认已阅读标准示例".to_string()));
    }

    let task = find_task_by_id(&state, task_id).await?;
    let before_task = task.clone();
    if task.task_status.as_deref() == Some("executing") {
        let executing_service_item = resolve_technician_service_item_for_action(
            &state,
            &task,
            auth_user.user_id,
            payload.service_item_id,
            payload.service_type.as_deref(),
            &["executing"],
            "开始执行",
        )
        .await?;
        let service_item_id = executing_service_item.as_ref().and_then(|item| item.id);
        let service_type = executing_service_item
            .as_ref()
            .and_then(|item| item.service_type.clone())
            .or_else(|| task.service_type.clone());
        let execution_id = find_active_execution_id(
            &state,
            task_id,
            auth_user.user_id,
            service_item_id,
            service_type.as_deref(),
        )
        .await?;

        return Ok(Json(ApiResponse::success(TaskActionResponse {
            task_id,
            task_status: "executing".to_string(),
            service_item_id,
            service_type,
            execution_id: Some(execution_id),
        })));
    }
    let service_item = resolve_technician_service_item_for_action(
        &state,
        &task,
        auth_user.user_id,
        payload.service_item_id,
        payload.service_type.as_deref(),
        &["pending_execute"],
        "开始执行",
    )
    .await?;
    let service_item_id = service_item.as_ref().and_then(|item| item.id);
    let service_type = service_item
        .as_ref()
        .and_then(|item| item.service_type.clone())
        .or_else(|| task.service_type.clone());

    let execution = TaskExecutionEntity {
        id: None,
        task_id: Some(task_id),
        service_item_id,
        service_type: service_type.clone(),
        technician_id: Some(auth_user.user_id),
        started_at: Some(local_now()),
        submitted_at: None,
        submit_remark: None,
        execution_status: Some("executing".to_string()),
        created_at: None,
        updated_at: None,
    };
    TaskExecutionEntity::insert(state.rb.as_ref(), &execution)
        .await
        .context("failed to create execution record")
        .map_err(AppError::Internal)?;

    let latest_execution = latest_task_execution(&state, task_id).await?;
    TaskEntity::update_by_map(
        state.rb.as_ref(),
        &TaskEntity {
            task_status: Some("executing".to_string()),
            latest_execution_id: latest_execution.id,
            ..Default::default()
        },
        value! { "id": task_id },
    )
    .await
    .context("failed to update task execution state")
    .map_err(AppError::Internal)?;
    if let Some(service_item_id) = service_item_id {
        TaskServiceItemEntity::update_by_map(
            state.rb.as_ref(),
            &TaskServiceItemEntity {
                service_status: Some("executing".to_string()),
                started_at: Some(local_now()),
                progress_percent: Some(60),
                ..Default::default()
            },
            value! { "id": service_item_id },
        )
        .await
        .context("failed to update task service item on start")
        .map_err(AppError::Internal)?;
    } else {
        sync_task_service_items_on_start(&state, task_id, auth_user.user_id).await?;
    }

    let after_task = find_task_by_id(&state, task_id).await?;
    audit::write_audit_log(
        &state,
        "task",
        task_id,
        "start_task",
        auth_user.user_id,
        primary_role(&auth_user),
        Some(&before_task),
        Some(&after_task),
        Some("技师开始执行任务".to_string()),
    )
    .await?;

    Ok(Json(ApiResponse::success(TaskActionResponse {
        task_id,
        task_status: "executing".to_string(),
        service_item_id,
        service_type,
        execution_id: latest_execution.id,
    })))
}

pub async fn submit_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<i64>,
    Json(payload): Json<SubmitTaskRequest>,
) -> Result<Json<ApiResponse<TaskActionResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["technician"])?;

    let task = find_task_by_id(&state, task_id).await?;
    let before_task = task.clone();
    if task.task_status.as_deref() != Some("executing") {
        return Err(AppError::Conflict("当前任务状态不允许提交检核".to_string()));
    }

    let execution = find_task_execution_by_id(&state, payload.execution_id).await?;
    if execution.task_id != Some(task_id) || execution.technician_id != Some(auth_user.user_id) {
        return Err(AppError::Forbidden("执行记录与当前任务不匹配".to_string()));
    }
    if execution.execution_status.as_deref() == Some("submitted") {
        return Err(AppError::Conflict("当前服务环节已提交检核".to_string()));
    }

    let service_item =
        if let Some(service_item_id) = execution.service_item_id.or(payload.service_item_id) {
            if let Some(payload_service_item_id) = payload.service_item_id
                && execution.service_item_id.is_some()
                && execution.service_item_id != Some(payload_service_item_id)
            {
                return Err(AppError::BadRequest("提交环节与执行记录不匹配".to_string()));
            }
            let items = load_task_service_item_entities(&state, task_id).await?;
            let item = items
                .into_iter()
                .find(|item| item.id == Some(service_item_id))
                .ok_or_else(|| AppError::BadRequest("服务环节不属于当前任务".to_string()))?;
            if item.technician_id != Some(auth_user.user_id) {
                return Err(AppError::Forbidden(
                    "您不是该服务环节当前执行技师".to_string(),
                ));
            }
            if item.service_status.as_deref() != Some("executing") {
                return Err(AppError::Conflict(
                    "当前服务环节状态不允许提交检核".to_string(),
                ));
            }
            Some(item)
        } else {
            resolve_technician_service_item_for_action(
                &state,
                &task,
                auth_user.user_id,
                None,
                payload
                    .service_type
                    .as_deref()
                    .or(execution.service_type.as_deref()),
                &["executing"],
                "提交检核",
            )
            .await?
        };
    let service_item_id = service_item.as_ref().and_then(|item| item.id);
    let service_type = service_item
        .as_ref()
        .and_then(|item| item.service_type.clone())
        .or_else(|| execution.service_type.clone())
        .or_else(|| task.service_type.clone());

    let evidences: Vec<EvidenceEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "evidence",
        value! { "task_id": task_id, "execution_id": payload.execution_id },
    )
    .await
    .context("failed to query evidence before submit")
    .map_err(AppError::Internal)?;
    if evidences.is_empty() {
        return Err(AppError::Conflict("请先上传完整现场实据".to_string()));
    }

    TaskExecutionEntity::update_by_map(
        state.rb.as_ref(),
        &TaskExecutionEntity {
            submitted_at: Some(local_now()),
            submit_remark: payload.submit_remark.clone(),
            execution_status: Some("submitted".to_string()),
            ..Default::default()
        },
        value! { "id": payload.execution_id },
    )
    .await
    .context("failed to update execution submit state")
    .map_err(AppError::Internal)?;

    if let Some(service_item_id) = service_item_id {
        TaskServiceItemEntity::update_by_map(
            state.rb.as_ref(),
            &TaskServiceItemEntity {
                service_status: Some("pending_dealer_review".to_string()),
                completed_at: Some(local_now()),
                progress_percent: Some(90),
                ..Default::default()
            },
            value! { "id": service_item_id },
        )
        .await
        .context("failed to update task service item on submit")
        .map_err(AppError::Internal)?;
    } else {
        sync_task_service_items_on_submit(&state, task_id).await?;
    }

    let updated_service_items = load_task_service_item_entities(&state, task_id).await?;
    let task_status = if updated_service_items.is_empty()
        || updated_service_items.iter().all(|item| {
            matches!(
                item.service_status.as_deref(),
                Some("pending_dealer_review")
                    | Some("pending_hq_score")
                    | Some("completed")
                    | Some("closed")
            )
        }) {
        "pending_dealer_review".to_string()
    } else {
        "executing".to_string()
    };

    TaskEntity::update_by_map(
        state.rb.as_ref(),
        &TaskEntity {
            task_status: Some(task_status.clone()),
            ..Default::default()
        },
        value! { "id": task_id },
    )
    .await
    .context("failed to update task submit state")
    .map_err(AppError::Internal)?;

    let after_task = find_task_by_id(&state, task_id).await?;
    if task_status == "pending_dealer_review"
        && let Some(dealer_user_id) = task.dealer_user_id
    {
        let _ = notify::create_notice(
            &state,
            dealer_user_id,
            "task_submit",
            task_id,
            "任务待评价",
            "技师已提交检核，请尽快评价",
        )
        .await;
    }
    audit::write_audit_log(
        &state,
        "task",
        task_id,
        "submit_task",
        auth_user.user_id,
        primary_role(&auth_user),
        Some(&before_task),
        Some(&after_task),
        payload.submit_remark.clone(),
    )
    .await?;

    Ok(Json(ApiResponse::success(TaskActionResponse {
        task_id,
        task_status,
        service_item_id,
        service_type,
        execution_id: Some(payload.execution_id),
    })))
}

pub async fn reassign_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<i64>,
    Json(payload): Json<ReassignTaskRequest>,
) -> Result<Json<ApiResponse<TaskActionResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    if payload.assign_reason.trim().is_empty() {
        return Err(AppError::BadRequest("请填写重派原因".to_string()));
    }

    let task = find_task_by_id(&state, task_id).await?;
    let before_task = task.clone();
    if !matches!(
        task.task_status.as_deref(),
        Some("pending_execute") | Some("executing")
    ) {
        return Err(AppError::Conflict("当前任务状态不允许重派技师".to_string()));
    }
    if task.current_technician_id == Some(payload.technician_id) {
        return Err(AppError::Conflict("新技师不能与当前技师相同".to_string()));
    }
    if let Some(execution_id) = task.latest_execution_id {
        let latest_execution = find_task_execution_by_id(&state, execution_id).await?;
        if latest_execution.execution_status.as_deref() == Some("submitted") {
            return Err(AppError::Conflict("任务已提交检核，不能再重派".to_string()));
        }
    }

    ensure_technician_assignable(&state, &task, payload.technician_id).await?;

    TaskAssignmentEntity::insert(
        state.rb.as_ref(),
        &TaskAssignmentEntity {
            id: None,
            task_id: Some(task_id),
            from_technician_id: task.current_technician_id,
            to_technician_id: Some(payload.technician_id),
            assign_type: Some("reassign".to_string()),
            assign_reason: Some(payload.assign_reason.clone()),
            assigned_by: Some(auth_user.user_id),
            assigned_at: Some(local_now()),
        },
    )
    .await
    .context("failed to create reassign record")
    .map_err(AppError::Internal)?;

    let latest_assignment = latest_task_assignment(&state, task_id).await?;
    TaskEntity::update_by_map(
        state.rb.as_ref(),
        &TaskEntity {
            current_technician_id: Some(payload.technician_id),
            task_status: Some("pending_execute".to_string()),
            latest_assignment_id: latest_assignment.id,
            ..Default::default()
        },
        value! { "id": task_id },
    )
    .await
    .context("failed to update task after reassign")
    .map_err(AppError::Internal)?;
    sync_task_service_items_on_assign(&state, task_id, payload.technician_id, "pending_execute", 0)
        .await?;

    let after_task = find_task_by_id(&state, task_id).await?;
    let _ = notify::create_notice(
        &state,
        payload.technician_id,
        "task_reassign",
        task_id,
        "任务已重派给您",
        "您收到一条重派任务，请及时处理",
    )
    .await;
    audit::write_audit_log(
        &state,
        "task",
        task_id,
        "reassign_task",
        auth_user.user_id,
        primary_role(&auth_user),
        Some(&before_task),
        Some(&after_task),
        Some(payload.assign_reason),
    )
    .await?;

    Ok(Json(ApiResponse::success(TaskActionResponse {
        task_id,
        task_status: "pending_execute".to_string(),
        service_item_id: None,
        service_type: None,
        execution_id: None,
    })))
}

pub async fn close_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<i64>,
    Json(payload): Json<CloseTaskRequest>,
) -> Result<Json<ApiResponse<TaskActionResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    if payload.close_reason.trim().is_empty() {
        return Err(AppError::BadRequest("请填写关闭原因".to_string()));
    }

    let task = find_task_by_id(&state, task_id).await?;
    let before_task = task.clone();
    if matches!(
        task.task_status.as_deref(),
        Some("completed") | Some("closed")
    ) {
        return Err(AppError::Conflict("当前任务状态不允许关闭".to_string()));
    }

    TaskEntity::update_by_map(
        state.rb.as_ref(),
        &TaskEntity {
            task_status: Some("closed".to_string()),
            ..Default::default()
        },
        value! { "id": task_id },
    )
    .await
    .context("failed to close task")
    .map_err(AppError::Internal)?;
    sync_task_service_items_status(&state, task_id, "closed", 100).await?;

    let after_task = find_task_by_id(&state, task_id).await?;
    audit::write_audit_log(
        &state,
        "task",
        task_id,
        "close_task",
        auth_user.user_id,
        primary_role(&auth_user),
        Some(&before_task),
        Some(&after_task),
        Some(payload.close_reason),
    )
    .await?;

    Ok(Json(ApiResponse::success(TaskActionResponse {
        task_id,
        task_status: "closed".to_string(),
        service_item_id: None,
        service_type: None,
        execution_id: None,
    })))
}

pub async fn task_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<i64>,
) -> Result<Json<ApiResponse<TaskLogListResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let task = find_task_by_id(&state, task_id).await?;
    ensure_task_access(&state, &auth_user, &task).await?;

    let logs: Vec<AuditLogEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "audit_log",
        value! { "biz_type": "task", "biz_id": task_id },
    )
    .await
    .context("failed to query task logs")
    .map_err(AppError::Internal)?;
    let items = logs
        .into_iter()
        .map(|item| TaskLogItem {
            id: item.id.unwrap_or_default(),
            action: item.action.unwrap_or_default(),
            operator_id: item.operator_id.unwrap_or_default(),
            operator_role: item.operator_role.unwrap_or_default(),
            remark: item.remark,
            created_at: item.created_at.map(format_datetime),
            before_value: item.before_value,
            after_value: item.after_value,
        })
        .collect::<Vec<_>>();
    let total = items.len();

    Ok(Json(ApiResponse::success(TaskLogListResponse {
        items,
        total,
    })))
}

fn validate_create_task_request(payload: &CreateTaskRequest) -> Result<(), AppError> {
    if payload.customer_name.trim().is_empty() {
        return Err(AppError::BadRequest("请输入客户姓名".to_string()));
    }
    if payload.customer_mobile.trim().is_empty() {
        return Err(AppError::BadRequest("请输入客户手机号".to_string()));
    }
    let has_service_type = payload
        .service_type
        .as_ref()
        .map(|item| !item.trim().is_empty())
        .unwrap_or(false);
    let has_service_types = payload
        .service_types
        .as_ref()
        .map(|items| items.iter().any(|item| !item.trim().is_empty()))
        .unwrap_or(false);
    if !has_service_type && !has_service_types {
        return Err(AppError::BadRequest("请选择服务环节".to_string()));
    }
    if payload.appointment_time.trim().is_empty() {
        return Err(AppError::BadRequest("请选择预约时间".to_string()));
    }
    if payload.address_id.is_none() {
        let address = payload
            .address
            .as_ref()
            .ok_or_else(|| AppError::BadRequest("请输入服务地址".to_string()))?;
        if address.detail.trim().is_empty() {
            return Err(AppError::BadRequest("请输入服务地址".to_string()));
        }
    }
    Ok(())
}

fn filter_task_item(task: &TaskEntity, query: &TaskListQuery) -> bool {
    if let Some(task_no) = &query.task_no
        && !task
            .task_no
            .as_deref()
            .unwrap_or_default()
            .contains(task_no)
    {
        return false;
    }
    if let Some(task_status) = &query.task_status
        && task.task_status.as_deref() != Some(task_status.as_str())
    {
        return false;
    }
    if let Some(review_status) = &query.review_status
        && task.review_status.as_deref() != Some(review_status.as_str())
    {
        return false;
    }
    if let Some(score_status) = &query.score_status
        && task.score_status.as_deref() != Some(score_status.as_str())
    {
        return false;
    }
    if let Some(store_id) = query.store_id
        && task.store_id != Some(store_id)
    {
        return false;
    }
    if let Some(dealer_user_id) = query.dealer_user_id
        && task.dealer_user_id != Some(dealer_user_id)
    {
        return false;
    }
    if let Some(technician_id) = query.technician_id
        && task.current_technician_id != Some(technician_id)
    {
        return false;
    }
    if let Some(appointment_start) = &query.appointment_start {
        let Ok(start_at) = NaiveDateTime::parse_from_str(appointment_start, "%Y-%m-%d %H:%M:%S")
        else {
            return false;
        };
        if task
            .appointment_time
            .map(|dt| dt < start_at)
            .unwrap_or(true)
        {
            return false;
        }
    }
    if let Some(appointment_end) = &query.appointment_end {
        let Ok(end_at) = NaiveDateTime::parse_from_str(appointment_end, "%Y-%m-%d %H:%M:%S") else {
            return false;
        };
        if task.appointment_time.map(|dt| dt > end_at).unwrap_or(true) {
            return false;
        }
    }
    true
}

fn filter_task_item_for_scoped_view(task: &TaskEntity, query: &TaskListQuery) -> bool {
    if let Some(task_no) = &query.task_no
        && !task
            .task_no
            .as_deref()
            .unwrap_or_default()
            .contains(task_no)
    {
        return false;
    }
    if let Some(store_id) = query.store_id
        && task.store_id != Some(store_id)
    {
        return false;
    }
    if let Some(dealer_user_id) = query.dealer_user_id
        && task.dealer_user_id != Some(dealer_user_id)
    {
        return false;
    }
    if let Some(appointment_start) = &query.appointment_start {
        let Ok(start_at) = NaiveDateTime::parse_from_str(appointment_start, "%Y-%m-%d %H:%M:%S")
        else {
            return false;
        };
        if task
            .appointment_time
            .map(|dt| dt < start_at)
            .unwrap_or(true)
        {
            return false;
        }
    }
    if let Some(appointment_end) = &query.appointment_end {
        let Ok(end_at) = NaiveDateTime::parse_from_str(appointment_end, "%Y-%m-%d %H:%M:%S") else {
            return false;
        };
        if task.appointment_time.map(|dt| dt > end_at).unwrap_or(true) {
            return false;
        }
    }
    true
}

fn filter_task_summary(item: &TaskSummary, query: &TaskListQuery) -> bool {
    if let Some(task_status) = &query.task_status
        && item.task_status != *task_status
    {
        return false;
    }
    if let Some(review_status) = &query.review_status
        && item.review_status != *review_status
    {
        return false;
    }
    if let Some(score_status) = &query.score_status
        && item.score_status != *score_status
    {
        return false;
    }
    if let Some(technician_id) = query.technician_id
        && item.current_technician_id != Some(technician_id)
    {
        return false;
    }
    if let Some(service_type) = &query.service_type
        && !item
            .service_items
            .iter()
            .any(|task_item| task_item.service_type == *service_type)
    {
        return false;
    }
    if let Some(mobile) = &query.mobile
        && !item.customer_mobile.contains(mobile)
    {
        return false;
    }
    if let Some(customer_name) = &query.customer_name
        && !item.customer_name.contains(customer_name)
    {
        return false;
    }
    true
}

async fn build_task_summary(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    task: TaskEntity,
    service_item_scope: Option<TaskServiceItemEntity>,
) -> Result<TaskSummary, AppError> {
    let customer = find_customer_by_id(state, task.customer_id).await?;
    let address = find_address_by_id(state, task.customer_address_id).await?;
    let store = find_store_by_id(state, task.store_id).await?;
    let service_items = load_task_service_item_entities(state, task.id.unwrap_or_default()).await?;
    let customer_name = customer.name.unwrap_or_default();
    let customer_mobile = customer.mobile.unwrap_or_default();
    let task_no = task.task_no.clone().unwrap_or_default();
    let task_status = service_item_scope
        .as_ref()
        .and_then(|item| item.service_status.clone())
        .or_else(|| task.task_status.clone())
        .unwrap_or_default();
    let service_status = service_item_scope
        .as_ref()
        .and_then(|item| item.service_status.as_deref());
    let review_status = scoped_review_status(task.review_status.as_deref(), service_status);
    let score_status = scoped_score_status(task.score_status.as_deref(), service_status);
    let technician_only = is_technician_only(auth_user);
    let task_status_text_value =
        task_status_text_for_viewer(&task_status, technician_only).to_string();
    let display_status_value = display_status_for_viewer(&task_status, technician_only).to_string();
    let display_status_text_value =
        display_status_text_for_viewer(&task_status, technician_only).to_string();
    let review_status_text_value =
        review_status_text_for_viewer(&review_status, &task_status, technician_only).to_string();
    let score_status_text_value = score_status_text(&score_status).to_string();
    let visible_service_items = service_item_scope
        .as_ref()
        .map(|item| vec![item.clone()])
        .unwrap_or_else(|| service_items.clone());
    let progress_percent =
        task_progress_percent(&visible_service_items, Some(task_status.as_str()));
    let is_overdue = task_is_overdue(&task);
    let overdue_text_value = overdue_text(is_overdue).to_string();
    let dealer = if let Some(dealer_user_id) = task.dealer_user_id {
        find_user_by_id(state, Some(dealer_user_id)).await.ok()
    } else {
        None
    };
    let current_technician_id = service_item_scope
        .as_ref()
        .and_then(|item| item.technician_id)
        .or(task.current_technician_id);
    let technician = if let Some(technician_id) = current_technician_id {
        find_user_by_id(state, Some(technician_id)).await.ok()
    } else {
        None
    };
    let primary_service_type = visible_service_items
        .first()
        .and_then(|item| item.service_type.clone())
        .or(task.service_type.clone())
        .unwrap_or_default();
    let service_items_payload = map_task_service_item_payloads(visible_service_items);
    let task_tags = build_task_tags(&task, &service_items_payload, technician_only);

    Ok(TaskSummary {
        id: task.id.unwrap_or_default(),
        service_item_id: service_item_scope.as_ref().and_then(|item| item.id),
        task_no,
        service_type: primary_service_type,
        service_status: service_item_scope.and_then(|item| item.service_status),
        service_items_text: service_items_text(&service_items_payload),
        service_items: service_items_payload,
        task_status,
        task_status_text: task_status_text_value,
        display_status: display_status_value,
        display_status_text: display_status_text_value,
        review_status,
        review_status_text: review_status_text_value,
        score_status,
        score_status_text: score_status_text_value,
        appointment_time: task.appointment_time.map(format_datetime),
        customer_name,
        customer_mobile,
        address: address.address.unwrap_or_default(),
        store_id: task.store_id.unwrap_or_default(),
        store_name: store.store_name.unwrap_or_default(),
        dealer_user_id: task.dealer_user_id.unwrap_or_default(),
        dealer_name: dealer
            .as_ref()
            .and_then(|item| item.name.clone())
            .unwrap_or_default(),
        current_technician_id,
        technician_name: technician.as_ref().and_then(|item| item.name.clone()),
        technician_mobile: technician.as_ref().and_then(|item| item.mobile.clone()),
        progress_percent,
        is_overdue,
        overdue_text: overdue_text_value,
        task_tags,
    })
}

async fn resolve_store_id(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    requested_store_id: Option<i64>,
) -> Result<i64, AppError> {
    if auth_user.has_role("dealer") {
        return dealer_owned_store_id(state, auth_user.user_id).await;
    }

    requested_store_id.ok_or_else(|| AppError::BadRequest("store_id不能为空".to_string()))
}

async fn ensure_store_exists(state: &AppState, store_id: i64) -> Result<(), AppError> {
    let _ = find_store_by_id(state, Some(store_id)).await?;
    Ok(())
}

async fn find_or_create_customer(
    state: &AppState,
    payload: &CreateTaskRequest,
    created_by: i64,
    store_id: i64,
) -> Result<CustomerEntity, AppError> {
    let mut customers: Vec<CustomerEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "customer",
        value! { "mobile": payload.customer_mobile.as_str() },
    )
    .await
    .context("failed to query customer by mobile")
    .map_err(AppError::Internal)?;

    if let Some(customer) = customers.pop() {
        if customer.primary_store_id.is_none() {
            CustomerEntity::update_by_map(
                state.rb.as_ref(),
                &CustomerEntity {
                    primary_store_id: Some(store_id),
                    ..Default::default()
                },
                value! { "id": customer.id.unwrap_or_default(), "column": ["primary_store_id"] },
            )
            .await
            .context("failed to backfill customer primary store")
            .map_err(AppError::Internal)?;

            let refreshed = db::select_one_by_map_compat::<CustomerEntity>(
                state.rb.as_ref(),
                "customer",
                value! { "id": customer.id.unwrap_or_default() },
            )
            .await
            .context("failed to reload customer after primary store backfill")
            .map_err(AppError::Internal)?
            .ok_or_else(|| {
                AppError::Internal(anyhow!(
                    "customer reload failed after primary store backfill"
                ))
            })?;
            return Ok(refreshed);
        }
        return Ok(customer);
    }

    let customer = CustomerEntity {
        id: None,
        name: Some(payload.customer_name.clone()),
        mobile: Some(payload.customer_mobile.clone()),
        source: payload
            .customer_source
            .clone()
            .or_else(|| Some("mini_program".to_string())),
        remark: None,
        primary_store_id: Some(store_id),
        created_by: Some(created_by),
        created_at: None,
        updated_at: None,
    };
    CustomerEntity::insert(state.rb.as_ref(), &customer)
        .await
        .context("failed to create customer")
        .map_err(AppError::Internal)?;

    let mut customers: Vec<CustomerEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "customer",
        value! { "mobile": payload.customer_mobile.as_str() },
    )
    .await
    .context("failed to query created customer")
    .map_err(AppError::Internal)?;

    customers
        .pop()
        .ok_or_else(|| AppError::Internal(anyhow!("customer create succeeded but fetch failed")))
}

async fn resolve_customer_address_id(
    state: &AppState,
    customer_id: i64,
    address_id: Option<i64>,
    address: Option<&AddressRequest>,
) -> Result<i64, AppError> {
    if let Some(address_id) = address_id {
        let address = find_address_by_id(state, Some(address_id)).await?;
        if address.customer_id != Some(customer_id) {
            return Err(AppError::Forbidden("地址不属于当前客户".to_string()));
        }
        return Ok(address_id);
    }

    let address = address.ok_or_else(|| AppError::BadRequest("请输入服务地址".to_string()))?;
    let entity = CustomerAddressEntity {
        id: None,
        customer_id: Some(customer_id),
        province: address.province.clone(),
        city: address.city.clone(),
        district: address.district.clone(),
        address: Some(address.detail.clone()),
        is_default: Some(0),
        created_at: None,
    };

    CustomerAddressEntity::insert(state.rb.as_ref(), &entity)
        .await
        .context("failed to create customer address")
        .map_err(AppError::Internal)?;

    let addresses: Vec<CustomerAddressEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "customer_address",
        value! { "customer_id": customer_id, "address": address.detail.as_str() },
    )
    .await
    .context("failed to query created customer address")
    .map_err(AppError::Internal)?;

    addresses
        .into_iter()
        .last()
        .and_then(|item| item.id)
        .ok_or_else(|| AppError::Internal(anyhow!("address create succeeded but fetch failed")))
}

async fn ensure_technician_assignable(
    state: &AppState,
    task: &TaskEntity,
    technician_id: i64,
) -> Result<(), AppError> {
    let task_id = task
        .id
        .ok_or_else(|| AppError::Internal(anyhow!("task.id missing")))?;
    let task_service_items = load_task_service_item_entities(state, task_id).await?;
    let required_service_types = if task_service_items.is_empty() {
        vec![
            task.service_type
                .clone()
                .ok_or_else(|| AppError::Internal(anyhow!("task.service_type missing")))?,
        ]
    } else {
        task_service_items
            .into_iter()
            .filter_map(|item| item.service_type)
            .collect::<Vec<_>>()
    };

    for service_type in required_service_types {
        ensure_technician_assignable_for_service(state, task, technician_id, &service_type).await?;
    }

    Ok(())
}

async fn ensure_technician_assignable_for_service(
    state: &AppState,
    task: &TaskEntity,
    technician_id: i64,
    service_type: &str,
) -> Result<(), AppError> {
    let technician = find_user_by_id(state, Some(technician_id)).await?;
    if technician.status.as_deref() == Some("disabled") {
        return Err(AppError::Conflict("所选技师已禁用，无法分派".to_string()));
    }

    let store_id = task
        .store_id
        .ok_or_else(|| AppError::Internal(anyhow!("task.store_id missing")))?;
    let user_store: Vec<UserStoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_store",
        value! { "user_id": technician_id, "store_id": store_id },
    )
    .await
    .context("failed to query technician store relation")
    .map_err(AppError::Internal)?;
    if user_store.is_empty() {
        return Err(AppError::Conflict("不能分派非当前门店技师".to_string()));
    }

    let skills: Vec<TechnicianSkillEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "technician_skill",
        value! { "user_id": technician_id, "service_type": service_type },
    )
    .await
    .context("failed to query technician skill")
    .map_err(AppError::Internal)?;
    if skills.is_empty() {
        return Err(AppError::Conflict(format!(
            "所选技师不具备{}服务能力",
            service_type_label(Some(service_type))
        )));
    }

    Ok(())
}

fn is_legacy_whole_task_assignment(payload: &AssignTaskRequest) -> bool {
    payload.assignments.is_none()
        && payload.service_item_id.is_none()
        && payload
            .service_type
            .as_ref()
            .map(|item| item.trim().is_empty())
            .unwrap_or(true)
}

fn resolve_assignment_targets(
    payload: &AssignTaskRequest,
    task: &TaskEntity,
    service_items: &[TaskServiceItemEntity],
) -> Result<Vec<ResolvedAssignment>, AppError> {
    let mut assignments = Vec::new();
    if let Some(items) = &payload.assignments {
        if items.is_empty() {
            return Err(AppError::BadRequest(
                "请至少选择一个服务环节分配".to_string(),
            ));
        }
        for item in items {
            assignments.push(resolve_assignment_target(
                task,
                service_items,
                item.service_item_id,
                item.service_type.as_deref(),
                item.technician_id,
            )?);
        }
        return Ok(assignments);
    }

    let technician_id = payload
        .technician_id
        .ok_or_else(|| AppError::BadRequest("请选择技师".to_string()))?;
    assignments.push(resolve_assignment_target(
        task,
        service_items,
        payload.service_item_id,
        payload.service_type.as_deref(),
        technician_id,
    )?);
    Ok(assignments)
}

fn resolve_assignment_target(
    task: &TaskEntity,
    service_items: &[TaskServiceItemEntity],
    service_item_id: Option<i64>,
    service_type: Option<&str>,
    technician_id: i64,
) -> Result<ResolvedAssignment, AppError> {
    if let Some(service_item_id) = service_item_id {
        let item = service_items
            .iter()
            .find(|item| item.id == Some(service_item_id))
            .ok_or_else(|| AppError::BadRequest("服务环节不属于当前任务".to_string()))?;
        return Ok(ResolvedAssignment {
            service_item_id: Some(service_item_id),
            service_type: item.service_type.clone().ok_or_else(|| {
                AppError::Internal(anyhow!("task_service_item.service_type missing"))
            })?,
            technician_id,
            from_technician_id: item.technician_id,
        });
    }

    let service_type = service_type
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .ok_or_else(|| AppError::BadRequest("请选择服务环节".to_string()))?;

    if service_items.is_empty() {
        if task.service_type.as_deref() != Some(service_type) {
            return Err(AppError::BadRequest("服务环节不属于当前任务".to_string()));
        }
        return Ok(ResolvedAssignment {
            service_item_id: None,
            service_type: service_type.to_string(),
            technician_id,
            from_technician_id: task.current_technician_id,
        });
    }

    let item = service_items
        .iter()
        .find(|item| item.service_type.as_deref() == Some(service_type))
        .ok_or_else(|| AppError::BadRequest("服务环节不属于当前任务".to_string()))?;
    Ok(ResolvedAssignment {
        service_item_id: item.id,
        service_type: service_type.to_string(),
        technician_id,
        from_technician_id: item.technician_id,
    })
}

fn ensure_unique_assignment_targets(assignments: &[ResolvedAssignment]) -> Result<(), AppError> {
    let mut seen = Vec::new();
    for assignment in assignments {
        let key = assignment
            .service_item_id
            .map(|id| format!("id:{id}"))
            .unwrap_or_else(|| format!("type:{}", assignment.service_type));
        if seen.contains(&key) {
            return Err(AppError::BadRequest(
                "同一个服务环节不能重复分配".to_string(),
            ));
        }
        seen.push(key);
    }
    Ok(())
}

fn scoped_service_items_for_detail(
    auth_user: &auth::CurrentAuthUser,
    service_items: &[TaskServiceItemEntity],
    query: &TaskDetailQuery,
) -> Result<Vec<TaskServiceItemEntity>, AppError> {
    if let Some(service_item_id) = query.service_item_id {
        let item = service_items
            .iter()
            .find(|item| item.id == Some(service_item_id))
            .ok_or_else(|| AppError::BadRequest("服务环节不属于当前任务".to_string()))?;
        ensure_service_item_view_access(auth_user, item)?;
        return Ok(vec![item.clone()]);
    }

    if let Some(service_type) = query
        .service_type
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let item = service_items
            .iter()
            .find(|item| item.service_type.as_deref() == Some(service_type))
            .ok_or_else(|| AppError::BadRequest("服务环节不属于当前任务".to_string()))?;
        ensure_service_item_view_access(auth_user, item)?;
        return Ok(vec![item.clone()]);
    }

    if is_technician_only(auth_user) {
        let scoped = service_items
            .iter()
            .filter(|item| item.technician_id == Some(auth_user.user_id))
            .cloned()
            .collect::<Vec<_>>();
        if !scoped.is_empty() {
            return Ok(scoped);
        }
    }

    Ok(service_items.to_vec())
}

fn ensure_service_item_view_access(
    auth_user: &auth::CurrentAuthUser,
    service_item: &TaskServiceItemEntity,
) -> Result<(), AppError> {
    if is_technician_only(auth_user) && service_item.technician_id != Some(auth_user.user_id) {
        return Err(AppError::Forbidden("您只能查看本人服务环节".to_string()));
    }
    Ok(())
}

async fn resolve_technician_service_item_for_action(
    state: &AppState,
    task: &TaskEntity,
    technician_id: i64,
    service_item_id: Option<i64>,
    service_type: Option<&str>,
    allowed_statuses: &[&str],
    action_name: &str,
) -> Result<Option<TaskServiceItemEntity>, AppError> {
    let task_id = task
        .id
        .ok_or_else(|| AppError::Internal(anyhow!("task.id missing")))?;
    let service_items = load_task_service_item_entities(state, task_id).await?;
    if service_items.is_empty() {
        if task.current_technician_id != Some(technician_id) {
            return Err(AppError::Forbidden("您不是该任务当前执行技师".to_string()));
        }
        if !status_allowed(task.task_status.as_deref(), allowed_statuses) {
            return Err(AppError::Conflict(format!(
                "当前任务状态不允许{action_name}"
            )));
        }
        return Ok(None);
    }

    if let Some(service_item_id) = service_item_id {
        let item = service_items
            .into_iter()
            .find(|item| item.id == Some(service_item_id))
            .ok_or_else(|| AppError::BadRequest("服务环节不属于当前任务".to_string()))?;
        validate_technician_service_item_action(
            &item,
            technician_id,
            allowed_statuses,
            action_name,
        )?;
        return Ok(Some(item));
    }

    if let Some(service_type) = service_type.map(str::trim).filter(|item| !item.is_empty()) {
        let item = service_items
            .into_iter()
            .find(|item| item.service_type.as_deref() == Some(service_type))
            .ok_or_else(|| AppError::BadRequest("服务环节不属于当前任务".to_string()))?;
        validate_technician_service_item_action(
            &item,
            technician_id,
            allowed_statuses,
            action_name,
        )?;
        return Ok(Some(item));
    }

    let assigned_items = service_items
        .into_iter()
        .filter(|item| item.technician_id == Some(technician_id))
        .collect::<Vec<_>>();
    if assigned_items.is_empty() {
        return Err(AppError::Forbidden("您不是该任务当前执行技师".to_string()));
    }
    let actionable_items = assigned_items
        .iter()
        .filter(|item| status_allowed(item.service_status.as_deref(), allowed_statuses))
        .cloned()
        .collect::<Vec<_>>();
    match actionable_items.as_slice() {
        [item] => Ok(Some(item.clone())),
        [] => Err(AppError::Conflict(format!(
            "当前服务环节状态不允许{action_name}"
        ))),
        _ => Err(AppError::BadRequest(
            "当前任务包含多个服务环节，请传 service_item_id 或 service_type".to_string(),
        )),
    }
}

fn validate_technician_service_item_action(
    item: &TaskServiceItemEntity,
    technician_id: i64,
    allowed_statuses: &[&str],
    action_name: &str,
) -> Result<(), AppError> {
    if item.technician_id != Some(technician_id) {
        return Err(AppError::Forbidden(
            "您不是该服务环节当前执行技师".to_string(),
        ));
    }
    if !status_allowed(item.service_status.as_deref(), allowed_statuses) {
        return Err(AppError::Conflict(format!(
            "当前服务环节状态不允许{action_name}"
        )));
    }
    Ok(())
}

fn status_allowed(status: Option<&str>, allowed_statuses: &[&str]) -> bool {
    status
        .map(|status| allowed_statuses.iter().any(|allowed| allowed == &status))
        .unwrap_or(false)
}

async fn ensure_task_access(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    task: &TaskEntity,
) -> Result<(), AppError> {
    if auth_user.has_role("hq_reviewer") || auth_user.has_role("super_admin") {
        return Ok(());
    }

    if auth_user.has_role("technician") {
        if task.current_technician_id == Some(auth_user.user_id) {
            return Ok(());
        }
        let task_id = task
            .id
            .ok_or_else(|| AppError::Internal(anyhow!("task.id missing")))?;
        let service_items = load_task_service_item_entities(state, task_id).await?;
        if service_items
            .iter()
            .any(|item| item.technician_id == Some(auth_user.user_id))
        {
            return Ok(());
        }
        return Err(AppError::Forbidden("您只能查看本人任务".to_string()));
    }

    if auth_user.has_role("dealer") {
        let store_id = dealer_owned_store_id(state, auth_user.user_id).await?;
        if task.store_id == Some(store_id) {
            return Ok(());
        }
        return Err(AppError::Forbidden("您只能查看所属门店任务".to_string()));
    }

    Err(AppError::Forbidden("您没有权限执行该操作".to_string()))
}

async fn dealer_owned_store_id(state: &AppState, user_id: i64) -> Result<i64, AppError> {
    let relations: Vec<UserStoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_store",
        value! { "user_id": user_id },
    )
    .await
    .context("failed to query user store relation")
    .map_err(AppError::Internal)?;

    let store_ids = relations
        .into_iter()
        .filter_map(|item| item.store_id)
        .collect::<Vec<_>>();
    match store_ids.as_slice() {
        [store_id] => Ok(*store_id),
        [] => Err(AppError::Forbidden("当前账号未绑定门店".to_string())),
        _ => Err(AppError::Conflict(
            "当前经销商账号绑定了多个门店，请先在后台修正为单门店".to_string(),
        )),
    }
}

async fn latest_task_assignment(
    state: &AppState,
    task_id: i64,
) -> Result<TaskAssignmentEntity, AppError> {
    let assignments: Vec<TaskAssignmentEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_assignment",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query latest task assignment")
    .map_err(AppError::Internal)?;

    assignments
        .into_iter()
        .max_by_key(|item| item.id.unwrap_or_default())
        .ok_or_else(|| AppError::Internal(anyhow!("task assignment missing after insert")))
}

async fn latest_task_execution(
    state: &AppState,
    task_id: i64,
) -> Result<TaskExecutionEntity, AppError> {
    let executions: Vec<TaskExecutionEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_execution",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query latest task execution")
    .map_err(AppError::Internal)?;

    executions
        .into_iter()
        .max_by_key(|item| item.id.unwrap_or_default())
        .ok_or_else(|| AppError::Internal(anyhow!("task execution missing after insert")))
}

async fn find_task_by_task_no(state: &AppState, task_no: &str) -> Result<TaskEntity, AppError> {
    let tasks: Vec<TaskEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "task", value! { "task_no": task_no })
            .await
            .context("failed to query task by task_no")
            .map_err(AppError::Internal)?;

    tasks
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Internal(anyhow!("task create succeeded but fetch failed")))
}

async fn find_task_by_id(state: &AppState, task_id: i64) -> Result<TaskEntity, AppError> {
    let tasks: Vec<TaskEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "task", value! { "id": task_id })
            .await
            .context("failed to query task by id")
            .map_err(AppError::Internal)?;

    let task = tasks
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("任务不存在或已删除".to_string()))?;
    if task.deleted_at.is_some() {
        return Err(AppError::NotFound("任务不存在或已删除".to_string()));
    }
    Ok(task)
}

async fn find_dealer_review_for_scope(
    state: &AppState,
    task_id: i64,
    service_items: &[TaskServiceItemEntity],
) -> Result<Option<DealerReviewEntity>, AppError> {
    if service_items.len() == 1 {
        let service_item = &service_items[0];
        if let Some(service_item_id) = service_item.id {
            if let Some(review) = db::select_one_by_map_compat::<DealerReviewEntity>(
                state.rb.as_ref(),
                "dealer_review",
                value! { "task_id": task_id, "service_item_id": service_item_id },
            )
            .await
            .context("failed to query dealer review by service item")
            .map_err(AppError::Internal)?
            {
                return Ok(Some(review));
            }
        }
        if let Some(service_type) = service_item.service_type.as_deref() {
            return db::select_one_by_map_compat(
                state.rb.as_ref(),
                "dealer_review",
                value! { "task_id": task_id, "service_type": service_type },
            )
            .await
            .context("failed to query dealer review by service type")
            .map_err(AppError::Internal);
        }
    }

    let reviews: Vec<DealerReviewEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "dealer_review",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query dealer reviews for task detail")
    .map_err(AppError::Internal)?;
    Ok(if reviews.len() == 1 {
        reviews.into_iter().next()
    } else {
        None
    })
}

async fn find_task_execution_by_id(
    state: &AppState,
    execution_id: i64,
) -> Result<TaskExecutionEntity, AppError> {
    let executions: Vec<TaskExecutionEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_execution",
        value! { "id": execution_id },
    )
    .await
    .context("failed to query task execution by id")
    .map_err(AppError::Internal)?;

    executions
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("执行记录不存在".to_string()))
}

async fn find_active_execution_id(
    state: &AppState,
    task_id: i64,
    technician_id: i64,
    service_item_id: Option<i64>,
    service_type: Option<&str>,
) -> Result<i64, AppError> {
    let executions: Vec<TaskExecutionEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_execution",
        value! { "task_id": task_id, "technician_id": technician_id },
    )
    .await
    .context("failed to query active execution")
    .map_err(AppError::Internal)?;

    let execution = executions
        .into_iter()
        .filter(|item| item.execution_status.as_deref() == Some("executing"))
        .filter(|item| match service_item_id {
            Some(id) => item.service_item_id == Some(id),
            None => true,
        })
        .filter(|item| match service_type {
            Some(service_type) => item.service_type.as_deref() == Some(service_type),
            None => true,
        })
        .max_by_key(|item| item.id.unwrap_or_default())
        .ok_or_else(|| AppError::Conflict("任务尚未生成执行记录".to_string()))?;

    execution
        .id
        .ok_or_else(|| AppError::Internal(anyhow!("execution.id missing")))
}

async fn find_customer_by_id(
    state: &AppState,
    customer_id: Option<i64>,
) -> Result<CustomerEntity, AppError> {
    let customer_id =
        customer_id.ok_or_else(|| AppError::Internal(anyhow!("customer_id missing")))?;
    let customers: Vec<CustomerEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "customer", value! { "id": customer_id })
            .await
            .context("failed to query customer by id")
            .map_err(AppError::Internal)?;

    customers
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("客户不存在".to_string()))
}

async fn find_address_by_id(
    state: &AppState,
    address_id: Option<i64>,
) -> Result<CustomerAddressEntity, AppError> {
    let address_id = address_id.ok_or_else(|| AppError::Internal(anyhow!("address_id missing")))?;
    let addresses: Vec<CustomerAddressEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "customer_address",
        value! { "id": address_id },
    )
    .await
    .context("failed to query address by id")
    .map_err(AppError::Internal)?;

    addresses
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("客户地址不存在".to_string()))
}

async fn find_store_by_id(
    state: &AppState,
    store_id: Option<i64>,
) -> Result<StoreEntity, AppError> {
    let store_id = store_id.ok_or_else(|| AppError::Internal(anyhow!("store_id missing")))?;
    let stores: Vec<StoreEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "store", value! { "id": store_id })
            .await
            .context("failed to query store by id")
            .map_err(AppError::Internal)?;

    stores
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("门店不存在".to_string()))
}

async fn find_user_by_id(state: &AppState, user_id: Option<i64>) -> Result<UserEntity, AppError> {
    let user_id = user_id.ok_or_else(|| AppError::Internal(anyhow!("user_id missing")))?;
    let users: Vec<UserEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "user", value! { "id": user_id })
            .await
            .context("failed to query user by id")
            .map_err(AppError::Internal)?;

    users
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("用户不存在".to_string()))
}

fn parse_datetime(input: &str) -> Result<NaiveDateTime, AppError> {
    NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M:%S")
        .map_err(|_| AppError::BadRequest("预约时间格式错误，应为 YYYY-MM-DD HH:MM:SS".to_string()))
}

fn format_datetime(input: NaiveDateTime) -> String {
    input.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn generate_task_no() -> String {
    format!("RW{}", Utc::now().format("%Y%m%d%H%M%S%6f"))
}

fn progress_percent_for_status(status: Option<&str>) -> i32 {
    match status {
        Some("pending_assign") => 0,
        Some("pending_execute") => 0,
        Some("executing") => 60,
        Some("pending_dealer_review") => 90,
        Some("pending_hq_score") => 95,
        Some("completed") => 100,
        Some("closed") => 100,
        _ => 0,
    }
}

async fn build_progress_tracks(
    state: &AppState,
    task: &TaskEntity,
    service_items: &[TaskServiceItemEntity],
    assignments: &[TaskAssignmentEntity],
    executions: &[TaskExecutionEntity],
    evidences: &[EvidenceEntity],
) -> Result<Vec<ProgressTrackItem>, AppError> {
    let scoped_service_item = if service_items.len() == 1 {
        service_items.first()
    } else {
        None
    };
    let scoped_technician_id = scoped_service_item.and_then(|item| item.technician_id);
    let scoped_assignments = scoped_technician_id
        .map(|technician_id| {
            assignments
                .iter()
                .filter(|item| item.to_technician_id == Some(technician_id))
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| assignments.iter().collect::<Vec<_>>());
    let latest_assignment = scoped_assignments
        .iter()
        .copied()
        .max_by_key(|item| item.id.unwrap_or_default());
    let mut assigned_technician_ids = scoped_assignments
        .iter()
        .filter_map(|item| item.to_technician_id)
        .collect::<Vec<_>>();
    assigned_technician_ids.sort_unstable();
    assigned_technician_ids.dedup();
    let assigned_technician_id = scoped_technician_id
        .or(task.current_technician_id)
        .or_else(|| {
            if assigned_technician_ids.len() == 1 {
                assigned_technician_ids.first().copied()
            } else {
                latest_assignment.and_then(|item| item.to_technician_id)
            }
        });
    let assigned_technician_name = if let Some(technician_id) = assigned_technician_id {
        find_user_by_id(state, Some(technician_id))
            .await
            .ok()
            .and_then(|item| item.name)
    } else if assigned_technician_ids.len() > 1 {
        Some("多个师傅".to_string())
    } else {
        None
    };

    let assigned_at = scoped_assignments
        .iter()
        .filter_map(|item| item.assigned_at)
        .min()
        .map(format_datetime);
    let scoped_execution_ids = scoped_service_item
        .map(|service_item| {
            executions
                .iter()
                .filter(|item| {
                    item.service_item_id == service_item.id
                        || (item.service_item_id.is_none()
                            && item.service_type == service_item.service_type)
                })
                .filter_map(|item| item.id)
                .collect::<std::collections::BTreeSet<_>>()
        })
        .filter(|ids| !ids.is_empty());
    let image_evidences = evidences
        .iter()
        .filter(|item| {
            scoped_execution_ids
                .as_ref()
                .map(|ids| item.execution_id.map(|id| ids.contains(&id)).unwrap_or(false))
                .unwrap_or(true)
        })
        .filter(|item| item.evidence_type.as_deref() == Some("image"))
        .collect::<Vec<_>>();
    let shot_at = image_evidences
        .iter()
        .filter_map(|item| item.shot_at.or(item.created_at))
        .min()
        .map(format_datetime);
    let images = image_evidences
        .into_iter()
        .map(|item| ProgressTrackImage {
            id: item.id.unwrap_or_default(),
            file_name: item.file_name.clone().unwrap_or_default(),
            file_url: crate::storage::public_url(&item.file_url.clone().unwrap_or_default()),
        })
        .collect::<Vec<_>>();

    let service_type = service_items
        .first()
        .and_then(|item| item.service_type.clone())
        .or(task.service_type.clone());
    let scheduled_at = service_items
        .first()
        .and_then(|item| item.scheduled_time)
        .or(task.appointment_time);
    let completed_at = service_items
        .iter()
        .filter_map(|item| item.completed_at)
        .max()
        .or_else(|| {
            executions
                .iter()
                .filter(|item| {
                    scoped_execution_ids
                        .as_ref()
                        .map(|ids| item.id.map(|id| ids.contains(&id)).unwrap_or(false))
                        .unwrap_or(true)
                })
                .filter_map(|item| item.submitted_at)
                .max()
        });
    let completion_finished = if !service_items.is_empty() {
        service_items.iter().all(|item| {
            matches!(
                item.service_status.as_deref(),
                Some("pending_dealer_review")
                    | Some("pending_hq_score")
                    | Some("completed")
                    | Some("closed")
            )
        })
    } else {
        matches!(
            task.task_status.as_deref(),
            Some("pending_dealer_review")
                | Some("pending_hq_score")
                | Some("completed")
                | Some("closed")
        )
    };

    let tracks = vec![
        ProgressTrackItem {
            key: "created".to_string(),
            title: "任务创建".to_string(),
            finished: true,
            time: task.created_at.map(format_datetime),
            expected_time: None,
            service_type: None,
            technician_id: None,
            technician_name: None,
            images: Vec::new(),
        },
        ProgressTrackItem {
            key: "assigned".to_string(),
            title: assigned_technician_name
                .as_ref()
                .map(|name| format!("已分派给{name}"))
                .unwrap_or_else(|| "已分派师傅".to_string()),
            finished: assigned_at.is_some(),
            time: assigned_at,
            expected_time: None,
            service_type: service_type.clone(),
            technician_id: assigned_technician_id,
            technician_name: assigned_technician_name,
            images: Vec::new(),
        },
        ProgressTrackItem {
            key: "photos".to_string(),
            title: "现场照片".to_string(),
            finished: !images.is_empty(),
            time: shot_at,
            expected_time: None,
            service_type: service_type.clone(),
            technician_id: assigned_technician_id,
            technician_name: None,
            images,
        },
        ProgressTrackItem {
            key: "completed".to_string(),
            title: format!("完成{}", service_type_label(service_type.as_deref())),
            finished: completion_finished,
            time: completed_at.map(format_datetime),
            expected_time: scheduled_at.map(format_datetime),
            service_type,
            technician_id: assigned_technician_id,
            technician_name: None,
            images: Vec::new(),
        },
    ];

    Ok(tracks)
}

fn task_progress_percent(service_items: &[TaskServiceItemEntity], status: Option<&str>) -> i32 {
    if !service_items.is_empty() {
        let total = service_items
            .iter()
            .map(|item| item.progress_percent.unwrap_or_default())
            .sum::<i32>();
        return total / service_items.len() as i32;
    }

    progress_percent_for_status(status)
}

fn resolved_service_types(payload: &CreateTaskRequest) -> Result<Vec<String>, AppError> {
    if let Some(service_types) = &payload.service_types {
        let items = service_types
            .iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>();
        if !items.is_empty() {
            return Ok(items);
        }
    }

    if let Some(service_type) = &payload.service_type
        && !service_type.trim().is_empty()
    {
        return Ok(vec![service_type.trim().to_string()]);
    }

    Err(AppError::BadRequest("请选择服务环节".to_string()))
}

async fn create_task_service_items(
    state: &AppState,
    task_id: i64,
    service_types: &[String],
    appointment_time: NaiveDateTime,
) -> Result<(), AppError> {
    for (index, service_type) in service_types.iter().enumerate() {
        TaskServiceItemEntity::insert(
            state.rb.as_ref(),
            &TaskServiceItemEntity {
                id: None,
                task_id: Some(task_id),
                service_type: Some(service_type.clone()),
                service_status: Some("pending_assign".to_string()),
                technician_id: None,
                scheduled_time: Some(appointment_time),
                started_at: None,
                completed_at: None,
                progress_percent: Some(0),
                sort_no: Some(index as i32 + 1),
                created_at: None,
                updated_at: None,
            },
        )
        .await
        .context("failed to create task service item")
        .map_err(AppError::Internal)?;
    }
    Ok(())
}

async fn load_task_service_item_entities(
    state: &AppState,
    task_id: i64,
) -> Result<Vec<TaskServiceItemEntity>, AppError> {
    let mut items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_service_item",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query task service items")
    .map_err(AppError::Internal)?;
    items.sort_by_key(|item| item.sort_no.unwrap_or_default());
    Ok(items)
}

fn map_task_service_item_payloads(
    items: Vec<TaskServiceItemEntity>,
) -> Vec<TaskServiceItemPayload> {
    items
        .into_iter()
        .map(|item| {
            let is_overdue = service_item_is_overdue(&item);
            TaskServiceItemPayload {
                id: item.id.unwrap_or_default(),
                service_type: item.service_type.unwrap_or_default(),
                service_status: item.service_status.unwrap_or_default(),
                technician_id: item.technician_id,
                scheduled_time: item.scheduled_time.map(format_datetime),
                started_at: item.started_at.map(format_datetime),
                completed_at: item.completed_at.map(format_datetime),
                progress_percent: item.progress_percent.unwrap_or_default(),
                sort_no: item.sort_no.unwrap_or(1),
                is_overdue,
            }
        })
        .collect()
}

async fn sync_task_service_items_on_assign(
    state: &AppState,
    task_id: i64,
    technician_id: i64,
    service_status: &str,
    progress_percent: i32,
) -> Result<(), AppError> {
    let items = load_task_service_item_entities(state, task_id).await?;
    for item in items {
        if let Some(id) = item.id {
            TaskServiceItemEntity::update_by_map(
                state.rb.as_ref(),
                &TaskServiceItemEntity {
                    technician_id: Some(technician_id),
                    service_status: Some(service_status.to_string()),
                    progress_percent: Some(progress_percent),
                    ..Default::default()
                },
                value! { "id": id },
            )
            .await
            .context("failed to update task service item on assign")
            .map_err(AppError::Internal)?;
        }
    }
    Ok(())
}

async fn sync_task_service_items_on_start(
    state: &AppState,
    task_id: i64,
    technician_id: i64,
) -> Result<(), AppError> {
    let items = load_task_service_item_entities(state, task_id).await?;
    for item in items {
        if let Some(id) = item.id {
            TaskServiceItemEntity::update_by_map(
                state.rb.as_ref(),
                &TaskServiceItemEntity {
                    technician_id: Some(technician_id),
                    service_status: Some("executing".to_string()),
                    started_at: Some(local_now()),
                    progress_percent: Some(60),
                    ..Default::default()
                },
                value! { "id": id },
            )
            .await
            .context("failed to update task service item on start")
            .map_err(AppError::Internal)?;
        }
    }
    Ok(())
}

async fn sync_task_service_items_on_submit(state: &AppState, task_id: i64) -> Result<(), AppError> {
    let items = load_task_service_item_entities(state, task_id).await?;
    for item in items {
        if let Some(id) = item.id {
            TaskServiceItemEntity::update_by_map(
                state.rb.as_ref(),
                &TaskServiceItemEntity {
                    service_status: Some("pending_dealer_review".to_string()),
                    completed_at: Some(local_now()),
                    progress_percent: Some(90),
                    ..Default::default()
                },
                value! { "id": id },
            )
            .await
            .context("failed to update task service item on submit")
            .map_err(AppError::Internal)?;
        }
    }
    Ok(())
}

async fn sync_task_service_items_status(
    state: &AppState,
    task_id: i64,
    service_status: &str,
    progress_percent: i32,
) -> Result<(), AppError> {
    let items = load_task_service_item_entities(state, task_id).await?;
    for item in items {
        if let Some(id) = item.id {
            TaskServiceItemEntity::update_by_map(
                state.rb.as_ref(),
                &TaskServiceItemEntity {
                    service_status: Some(service_status.to_string()),
                    progress_percent: Some(progress_percent),
                    completed_at: if progress_percent >= 100 {
                        Some(local_now())
                    } else {
                        None
                    },
                    ..Default::default()
                },
                value! { "id": id },
            )
            .await
            .context("failed to update task service item status")
            .map_err(AppError::Internal)?;
        }
    }
    Ok(())
}

fn service_type_label(service_type: Option<&str>) -> &'static str {
    match service_type {
        Some("measure") => "量尺",
        Some("install") => "安装",
        Some("ironing") => "熨烫",
        Some("cleaning") => "除尘除螨",
        _ => "服务",
    }
}

fn service_items_text(items: &[TaskServiceItemPayload]) -> String {
    items
        .iter()
        .map(|item| service_type_label(Some(item.service_type.as_str())).to_string())
        .collect::<Vec<_>>()
        .join("、")
}

fn task_status_text(status: &str) -> &'static str {
    match status {
        "pending_assign" => "待分配",
        "pending_execute" => "待执行",
        "executing" => "进行中",
        "pending_dealer_review" => "待评价",
        "pending_hq_score" => "待评分",
        "completed" => "已完成",
        "closed" => "已关闭",
        _ => "未知状态",
    }
}

fn display_status(status: &str) -> &'static str {
    match status {
        "pending_assign" | "pending_execute" => "pending_execute",
        "executing" => "executing",
        "pending_dealer_review" | "pending_hq_score" => "pending_review",
        "completed" | "closed" => "completed",
        _ => "pending_execute",
    }
}

fn display_status_text(status: &str) -> &'static str {
    match display_status(status) {
        "pending_execute" => "待执行",
        "executing" => "进行中",
        "pending_review" => "待评价",
        "completed" => "已完成",
        _ => "待执行",
    }
}

fn review_status_text(status: &str) -> &'static str {
    match status {
        "pending" => "待评价",
        "reviewed" => "已评价",
        _ => "未知状态",
    }
}

fn score_status_text(status: &str) -> &'static str {
    match status {
        "pending" => "待评分",
        "scored" => "已评分",
        _ => "未知状态",
    }
}

fn local_now() -> NaiveDateTime {
    Local::now().naive_local()
}

fn scoped_review_status(task_review_status: Option<&str>, service_status: Option<&str>) -> String {
    match service_status {
        Some("pending_dealer_review") => "pending".to_string(),
        Some("pending_hq_score") | Some("completed") | Some("closed") => "reviewed".to_string(),
        _ => task_review_status.unwrap_or_default().to_string(),
    }
}

fn scoped_score_status(task_score_status: Option<&str>, service_status: Option<&str>) -> String {
    match service_status {
        Some("completed") | Some("closed") => "scored".to_string(),
        Some("pending_hq_score") => "pending".to_string(),
        _ => task_score_status.unwrap_or_default().to_string(),
    }
}

fn priority_text(priority: &str) -> &'static str {
    match priority {
        "low" => "低",
        "normal" => "普通",
        "high" => "紧急",
        _ => "普通",
    }
}

fn task_is_overdue(task: &TaskEntity) -> bool {
    if task.is_overdue == Some(1) {
        return true;
    }
    let overdue_by_time = task
        .appointment_time
        .map(|dt| dt < local_now())
        .unwrap_or(false);
    overdue_by_time
        && !matches!(
            task.task_status.as_deref(),
            Some("completed") | Some("closed")
        )
}

fn service_item_is_overdue(item: &TaskServiceItemEntity) -> bool {
    item.scheduled_time
        .map(|dt| dt < local_now())
        .unwrap_or(false)
        && !matches!(
            item.service_status.as_deref(),
            Some("completed") | Some("closed")
        )
}

fn overdue_text(is_overdue: bool) -> &'static str {
    if is_overdue { "已逾期" } else { "正常" }
}

fn csv_cell(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn build_task_tags(
    task: &TaskEntity,
    service_items: &[TaskServiceItemPayload],
    technician_only: bool,
) -> Vec<String> {
    let mut tags = service_items
        .iter()
        .map(|item| service_type_label(Some(item.service_type.as_str())).to_string())
        .collect::<Vec<_>>();
    if task_is_overdue(task) {
        tags.push("已逾期".to_string());
    }
    if task.review_status.as_deref() == Some("reviewed") && !technician_only {
        tags.push("已评价".to_string());
    }
    if task.score_status.as_deref() == Some("scored") {
        tags.push("已评分".to_string());
    }
    tags
}

fn task_status_text_for_viewer(status: &str, technician_only: bool) -> &'static str {
    if technician_only && status == "pending_hq_score" {
        return "已完成";
    }

    task_status_text(status)
}

fn display_status_for_viewer(status: &str, technician_only: bool) -> &'static str {
    if technician_only && status == "pending_hq_score" {
        return "completed";
    }

    display_status(status)
}

fn display_status_text_for_viewer(status: &str, technician_only: bool) -> &'static str {
    if technician_only && status == "pending_hq_score" {
        return "已完成";
    }

    display_status_text(status)
}

fn review_status_text_for_viewer(
    review_status: &str,
    task_status: &str,
    technician_only: bool,
) -> &'static str {
    if technician_only && task_status == "pending_hq_score" {
        return "已完成";
    }

    review_status_text(review_status)
}

fn is_technician_only(auth_user: &auth::CurrentAuthUser) -> bool {
    auth_user.has_role("technician")
        && !auth_user.has_role("dealer")
        && !auth_user.has_role("hq_reviewer")
        && !auth_user.has_role("super_admin")
}

fn primary_role(auth_user: &auth::CurrentAuthUser) -> &str {
    auth_user
        .roles
        .first()
        .map(|item| item.as_str())
        .unwrap_or("unknown")
}

async fn send_task_assignment_notice(
    state: &AppState,
    technician_id: i64,
    task_id: i64,
    service_type: Option<&str>,
    notice_type: &str,
) {
    let title = service_type
        .map(|service_type| format!("{}任务待执行", service_type_label(Some(service_type))))
        .unwrap_or_else(|| "新任务待执行".to_string());
    let content = service_type
        .map(|service_type| {
            format!(
                "您有一条新的{}任务，请及时处理",
                service_type_label(Some(service_type))
            )
        })
        .unwrap_or_else(|| "您有一条新的待执行任务，请及时处理".to_string());
    let _ =
        notify::create_notice(state, technician_id, notice_type, task_id, &title, &content).await;
}

async fn calculate_technician_average_score(
    state: &AppState,
    technician_id: i64,
) -> Result<f64, AppError> {
    let tasks: Vec<TaskEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task",
        value! { "current_technician_id": technician_id },
    )
    .await
    .context("failed to query technician tasks for detail score")
    .map_err(AppError::Internal)?;
    let task_ids: Vec<i64> = tasks.into_iter().filter_map(|item| item.id).collect();
    if task_ids.is_empty() {
        return Ok(0.0);
    }

    let scores: Vec<crate::models::hq_score::HqScoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "hq_score",
        value! { "task_id": task_ids },
    )
    .await
    .context("failed to query hq scores for detail score")
    .map_err(AppError::Internal)?;
    if scores.is_empty() {
        return Ok(0.0);
    }

    Ok(scores
        .iter()
        .map(|item| item.total_score.unwrap_or_default() as f64)
        .sum::<f64>()
        / scores.len() as f64)
}
