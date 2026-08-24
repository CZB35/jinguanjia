use std::{
    collections::{BTreeSet, HashMap},
    io::Cursor,
};

use crate::{
    audit, auth, db,
    error::AppError,
    identity,
    models::{
        audit_log::AuditLogEntity, customer::CustomerEntity,
        customer_address::CustomerAddressEntity, customer_review::CustomerReviewEntity,
        customer_review_image::CustomerReviewImageEntity, dealer_review::DealerReviewEntity,
        evidence::EvidenceEntity, hq_score::HqScoreEntity, hq_score_item::HqScoreItemEntity,
        store::StoreEntity, task::TaskEntity, task_assignment::TaskAssignmentEntity,
        task_execution::TaskExecutionEntity, task_service_item::TaskServiceItemEntity,
        technician_skill::TechnicianSkillEntity, user::UserEntity, user_role::UserRoleEntity,
        user_store::UserStoreEntity,
    },
    notify,
    response::ApiResponse,
    routes::imports::{deserialize_vec_i64_from_mixed, normalize_delete_ids},
    sms,
    state::AppState,
    storage,
};
use anyhow::{Context, anyhow};
use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use chrono::{Local, NaiveDateTime};
use image::{GenericImageView, ImageFormat, codecs::jpeg::JpegEncoder, imageops::FilterType};
use rbs::value;
use rust_xlsxwriter::{Color, Format, FormatAlign, FormatBorder, Image as XlsxImage, Workbook};
use serde::{Deserialize, Deserializer, Serialize};

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

#[derive(Debug)]
pub struct AddressRequest {
    pub province: Option<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub detail: String,
}

impl<'de> Deserialize<'de> for AddressRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum AddressInput {
            Object {
                province: Option<String>,
                city: Option<String>,
                district: Option<String>,
                #[serde(alias = "address")]
                detail: String,
            },
            Text(String),
        }

        match AddressInput::deserialize(deserializer)? {
            AddressInput::Object {
                province,
                city,
                district,
                detail,
            } => Ok(Self {
                province,
                city,
                district,
                detail,
            }),
            AddressInput::Text(detail) => Ok(Self {
                province: None,
                city: None,
                district: None,
                detail,
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct TaskListQuery {
    #[serde(alias = "taskNo")]
    pub task_no: Option<String>,
    #[serde(alias = "serviceType")]
    pub service_type: Option<String>,
    #[serde(alias = "taskType")]
    pub task_type: Option<String>,
    #[serde(alias = "status", alias = "taskStatus")]
    pub task_status: Option<String>,
    #[serde(alias = "reviewStatus")]
    pub review_status: Option<String>,
    #[serde(
        alias = "scoreStatus",
        alias = "check_status",
        alias = "checkStatus",
        alias = "verification_status",
        alias = "verificationStatus",
        alias = "hq_review_status",
        alias = "hqReviewStatus"
    )]
    pub score_status: Option<String>,
    #[serde(alias = "storeId")]
    pub store_id: Option<i64>,
    #[serde(alias = "dealerUserId")]
    pub dealer_user_id: Option<i64>,
    #[serde(alias = "technicianId")]
    pub technician_id: Option<i64>,
    #[serde(alias = "appointmentStart", alias = "startDate", alias = "start_date")]
    pub appointment_start: Option<String>,
    #[serde(alias = "appointmentEnd", alias = "endDate", alias = "end_date")]
    pub appointment_end: Option<String>,
    #[serde(alias = "customer_mobile", alias = "customerMobile", alias = "phone")]
    pub mobile: Option<String>,
    #[serde(alias = "customerName")]
    pub customer_name: Option<String>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct TaskDetailQuery {
    #[serde(alias = "serviceItemId")]
    pub service_item_id: Option<i64>,
    #[serde(alias = "serviceType")]
    pub service_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BatchDeleteTasksRequest {
    #[serde(
        default,
        alias = "ids",
        alias = "taskIds",
        deserialize_with = "deserialize_vec_i64_from_mixed"
    )]
    pub task_ids: Vec<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssignTaskRequest {
    #[serde(alias = "technicianId")]
    pub technician_id: Option<i64>,
    #[serde(alias = "serviceType")]
    pub service_type: Option<String>,
    #[serde(alias = "serviceItemId")]
    pub service_item_id: Option<i64>,
    pub assignments: Option<Vec<ServiceAssignmentRequest>>,
    #[serde(alias = "assignReason")]
    pub assign_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServiceAssignmentRequest {
    #[serde(alias = "serviceType")]
    pub service_type: Option<String>,
    #[serde(alias = "serviceItemId")]
    pub service_item_id: Option<i64>,
    #[serde(alias = "technicianId")]
    pub technician_id: i64,
}

#[derive(Debug)]
struct ResolvedAssignment {
    task_id: i64,
    service_item_id: Option<i64>,
    service_type: String,
    technician_id: i64,
    from_technician_id: Option<i64>,
    is_legacy_scope: bool,
}

#[derive(Debug, Clone)]
struct AssignableTaskScope {
    task: TaskEntity,
    service_items: Vec<TaskServiceItemEntity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WholeReassignTypePlan {
    type_update_requested: bool,
    target_service_type: Option<String>,
    service_item_id: Option<i64>,
    original_service_item_type: Option<String>,
    task_service_type_changed: bool,
    service_item_type_changed: bool,
}

impl WholeReassignTypePlan {
    fn service_type_changed(&self) -> bool {
        self.task_service_type_changed || self.service_item_type_changed
    }
}

#[derive(Debug, Deserialize)]
pub struct ReassignTaskRequest {
    #[serde(alias = "technicianId")]
    pub technician_id: Option<i64>,
    #[serde(alias = "serviceType")]
    pub service_type: Option<String>,
    #[serde(alias = "serviceItemId")]
    pub service_item_id: Option<i64>,
    pub assignments: Option<Vec<ServiceAssignmentRequest>>,
    #[serde(alias = "assignReason")]
    pub assign_reason: Option<String>,
    #[serde(alias = "customerName")]
    pub customer_name: Option<String>,
    pub name: Option<String>,
    #[serde(alias = "customerMobile")]
    pub customer_mobile: Option<String>,
    pub mobile: Option<String>,
    pub phone: Option<String>,
    #[serde(alias = "customerSource")]
    pub customer_source: Option<String>,
    #[serde(alias = "addressId")]
    pub address_id: Option<i64>,
    pub address: Option<AddressRequest>,
    #[serde(alias = "customerAddress")]
    pub customer_address: Option<AddressRequest>,
    #[serde(alias = "serviceAddress")]
    pub service_address: Option<AddressRequest>,
    #[serde(alias = "appointmentTime")]
    pub appointment_time: Option<String>,
    #[serde(alias = "serviceTimeSlot")]
    pub service_time_slot: Option<String>,
    pub priority: Option<String>,
    #[serde(alias = "taskDesc")]
    pub task_desc: Option<String>,
    #[serde(alias = "taskRemark")]
    pub task_remark: Option<String>,
    pub customer: Option<ReassignCustomerRequest>,
}

#[derive(Debug, Deserialize)]
pub struct ReassignCustomerRequest {
    pub name: Option<String>,
    #[serde(alias = "customerName")]
    pub customer_name: Option<String>,
    pub mobile: Option<String>,
    #[serde(alias = "customerMobile")]
    pub customer_mobile: Option<String>,
    #[serde(alias = "contactPhone")]
    pub contact_phone: Option<String>,
    pub phone: Option<String>,
    pub source: Option<String>,
    #[serde(alias = "customerSource")]
    pub customer_source: Option<String>,
    pub address: Option<AddressRequest>,
    #[serde(alias = "customerAddress")]
    pub customer_address: Option<AddressRequest>,
    #[serde(alias = "serviceAddress")]
    pub service_address: Option<AddressRequest>,
}

impl ReassignTaskRequest {
    fn assignment_request(&self) -> AssignTaskRequest {
        AssignTaskRequest {
            technician_id: self.technician_id,
            service_type: self.service_type.clone(),
            service_item_id: self.service_item_id,
            assignments: self.assignments.clone(),
            assign_reason: self.assign_reason.clone(),
        }
    }

    fn resolved_assign_reason(&self) -> String {
        self.assign_reason
            .as_deref()
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .unwrap_or("重新分派")
            .to_string()
    }

    fn has_task_detail_edits(&self) -> bool {
        self.customer_name.is_some()
            || self.name.is_some()
            || self.customer_mobile.is_some()
            || self.mobile.is_some()
            || self.phone.is_some()
            || self.customer_source.is_some()
            || self.address_id.is_some()
            || self.address.is_some()
            || self.customer_address.is_some()
            || self.service_address.is_some()
            || self.appointment_time.is_some()
            || self.service_time_slot.is_some()
            || self.priority.is_some()
            || self.task_desc.is_some()
            || self.task_remark.is_some()
            || self.customer.is_some()
    }

    fn uses_scoped_reassign(&self) -> bool {
        self.assignments.is_some() || self.service_item_id.is_some()
    }

    fn resolved_customer_name(&self) -> Option<&str> {
        self.customer_name
            .as_deref()
            .or(self.name.as_deref())
            .or_else(|| {
                self.customer.as_ref().and_then(|customer| {
                    customer
                        .name
                        .as_deref()
                        .or(customer.customer_name.as_deref())
                })
            })
    }

    fn resolved_customer_mobile(&self) -> Option<&str> {
        self.customer_mobile
            .as_deref()
            .or(self.mobile.as_deref())
            .or(self.phone.as_deref())
            .or_else(|| {
                self.customer.as_ref().and_then(|customer| {
                    customer
                        .mobile
                        .as_deref()
                        .or(customer.customer_mobile.as_deref())
                        .or(customer.contact_phone.as_deref())
                        .or(customer.phone.as_deref())
                })
            })
    }

    fn resolved_customer_source(&self) -> Option<&str> {
        self.customer_source.as_deref().or_else(|| {
            self.customer.as_ref().and_then(|customer| {
                customer
                    .source
                    .as_deref()
                    .or(customer.customer_source.as_deref())
            })
        })
    }

    fn resolved_address(&self) -> Option<&AddressRequest> {
        self.address
            .as_ref()
            .or(self.customer_address.as_ref())
            .or(self.service_address.as_ref())
            .or_else(|| {
                self.customer.as_ref().and_then(|customer| {
                    customer
                        .address
                        .as_ref()
                        .or(customer.customer_address.as_ref())
                        .or(customer.service_address.as_ref())
                })
            })
    }
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
    #[serde(
        alias = "locationAddress",
        alias = "location_text",
        alias = "locationText",
        alias = "current_address",
        alias = "currentAddress",
        alias = "formatted_address",
        alias = "formattedAddress",
        alias = "address"
    )]
    pub location_address: Option<String>,
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
    pub technician_location_address: String,
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
    #[serde(skip_serializing)]
    pub task_group_no: String,
    #[serde(skip_serializing)]
    pub service_sort_no: i32,
    #[serde(skip_serializing)]
    pub created_at_sort: String,
}

#[derive(Debug, Serialize)]
pub struct TaskListResponse {
    pub items: Vec<TaskSummary>,
    pub total: usize,
    pub summary: TaskListStatusSummary,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct TaskListStatusSummary {
    pub pending_execute: usize,
    pub executing: usize,
    pub pending_review: usize,
    pub completed: usize,
}

struct TaskExportRow {
    summary: TaskSummary,
    image_buffers: Vec<Vec<u8>>,
}

struct TaskExportSimpleRow {
    summary: TaskSummary,
    technician_score: Option<i32>,
    dealer_review: String,
    customer_review: String,
}

#[derive(Default)]
struct TaskExportExtraData {
    scores_by_task_service: HashMap<(i64, String), i32>,
    fallback_scores_by_task: HashMap<i64, i32>,
    dealer_reviews_by_service_item: HashMap<(i64, i64), String>,
    dealer_reviews_by_service_type: HashMap<(i64, String), String>,
    dealer_reviews_by_task: HashMap<i64, String>,
    customer_reviews_by_task: HashMap<i64, String>,
    image_buffers_by_task: HashMap<i64, Vec<Vec<u8>>>,
}

impl TaskExportExtraData {
    fn technician_score(&self, task_id: i64, service_type: &str) -> Option<i32> {
        self.scores_by_task_service
            .get(&(task_id, service_type.to_string()))
            .copied()
            .or_else(|| self.fallback_scores_by_task.get(&task_id).copied())
    }

    fn dealer_review(
        &self,
        task_id: i64,
        service_item_id: Option<i64>,
        service_type: &str,
    ) -> String {
        service_item_id
            .and_then(|id| {
                self.dealer_reviews_by_service_item
                    .get(&(task_id, id))
                    .cloned()
            })
            .or_else(|| {
                self.dealer_reviews_by_service_type
                    .get(&(task_id, service_type.to_string()))
                    .cloned()
            })
            .or_else(|| self.dealer_reviews_by_task.get(&task_id).cloned())
            .unwrap_or_default()
    }

    fn customer_review(&self, task_id: i64) -> String {
        self.customer_reviews_by_task
            .get(&task_id)
            .cloned()
            .unwrap_or_default()
    }
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
    pub customer_source: Option<String>,
    pub address_id: Option<i64>,
    pub customer_address: String,
    pub technician_location_address: String,
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
    pub dealer_review: Option<ReviewInfoPayload>,
    pub customer_review: Option<CustomerReviewPayload>,
    pub hq_score: Option<TaskHqScorePayload>,
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
    pub source: Option<String>,
    pub address_id: Option<i64>,
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
    pub average_score_text: String,
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
    pub execution_id: Option<i64>,
    pub service_type: Option<String>,
    pub evidence_type: String,
    pub file_name: String,
    pub file_url: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ReviewInfoPayload {
    pub star: i32,
    pub score: i32,
    pub score_text: String,
    pub tags: Vec<String>,
    pub content: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CustomerReviewPayload {
    pub star: i32,
    pub score: i32,
    pub score_text: String,
    pub tags: Vec<String>,
    pub content: String,
    pub images: Vec<CustomerReviewImageItemPayload>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct CustomerReviewImageItemPayload {
    pub id: i64,
    pub file_url: String,
}

#[derive(Debug, Serialize)]
pub struct TaskHqScorePayload {
    pub total_score: i32,
    pub total_score_text: String,
    pub score_remark: String,
    pub items: Vec<TaskHqScoreItemPayload>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct TaskHqScoreItemPayload {
    pub service_type: String,
    pub score: i32,
    pub score_text: String,
    pub deduction_reason: String,
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
    pub service_item_id: Option<i64>,
    pub service_type: Option<String>,
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

#[derive(Debug, Deserialize)]
pub struct ExportSelectedTasksRequest {
    pub task_ids: Vec<i64>,
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
    let task_group_no = generate_task_group_no();
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
            task_group_no: Some(task_group_no.clone()),
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
    let summary =
        build_task_list_status_summary(items.iter().map(|item| item.display_status.as_str()));
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).max(1);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let items = items
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();
    Ok(Json(ApiResponse::success(TaskListResponse {
        items,
        total,
        summary,
        page,
        page_size,
    })))
}

pub async fn delete_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let task = find_task_by_id(&state, task_id).await?;
    ensure_task_access(&state, &auth_user, &task).await?;
    soft_delete_task(&state, &auth_user, &task, "delete_task", "后台删除任务").await?;

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn batch_delete_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<BatchDeleteTasksRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    let task_ids = normalize_delete_ids(payload.task_ids, "task_ids")?;

    let mut tasks = Vec::with_capacity(task_ids.len());
    for task_id in task_ids {
        let task = find_task_by_id(&state, task_id).await?;
        ensure_task_access(&state, &auth_user, &task).await?;
        tasks.push(task);
    }

    let deleted_count = tasks.len();
    for task in &tasks {
        soft_delete_task(
            &state,
            &auth_user,
            task,
            "batch_delete_task",
            "后台批量删除任务",
        )
        .await?;
    }

    Ok(Json(ApiResponse::success(serde_json::json!({
        "ok": true,
        "deleted_count": deleted_count
    }))))
}

pub async fn export_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TaskListQuery>,
) -> Result<Response, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let items = load_task_export_summaries(&state, &auth_user, &query).await?;
    render_task_export_simple_workbook(&state, items).await
}

pub async fn export_selected_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ExportSelectedTasksRequest>,
) -> Result<Response, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    if payload.task_ids.is_empty() {
        return Err(AppError::BadRequest("task_ids不能为空".to_string()));
    }

    let items = load_selected_task_export_summaries(&state, &auth_user, &payload.task_ids).await?;
    if items.is_empty() {
        return Err(AppError::BadRequest("所选任务暂无可导出数据".to_string()));
    }
    render_task_export_simple_workbook(&state, items).await
}

async fn render_task_export_workbook(
    state: &AppState,
    items: Vec<TaskSummary>,
) -> Result<Response, AppError> {
    let task_ids = items
        .iter()
        .map(|item| item.id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let image_buffers_by_task_id =
        load_task_export_image_buffers_by_task_ids(&state, &task_ids).await?;
    let mut export_rows = Vec::with_capacity(items.len());
    let mut max_image_count = 0usize;
    for item in items {
        let image_buffers = image_buffers_by_task_id
            .get(&item.id)
            .cloned()
            .unwrap_or_default();
        max_image_count = max_image_count.max(image_buffers.len());
        export_rows.push(TaskExportRow {
            summary: item,
            image_buffers,
        });
    }

    let mut workbook = Workbook::new();
    {
        let header_format = Format::new()
            .set_bold()
            .set_font_color(Color::White)
            .set_background_color(Color::RGB(0x1F4E78))
            .set_border(FormatBorder::Thin)
            .set_align(FormatAlign::Center)
            .set_align(FormatAlign::VerticalCenter)
            .set_text_wrap();
        let cell_format = Format::new()
            .set_border(FormatBorder::Thin)
            .set_align(FormatAlign::VerticalCenter)
            .set_text_wrap();
        let image_cell_format = Format::new()
            .set_border(FormatBorder::Thin)
            .set_align(FormatAlign::Center)
            .set_align(FormatAlign::VerticalCenter);
        let worksheet = workbook
            .add_worksheet()
            .set_name("任务导出")
            .context("failed to create task export worksheet")
            .map_err(AppError::Internal)?;

        let base_headers = vec![
            "任务ID",
            "任务编号",
            "服务类型",
            "服务项目",
            "任务状态",
            "展示状态",
            "评价状态",
            "评分状态",
            "预约时间",
            "客户姓名",
            "客户手机号",
            "客户地址",
            "师傅定位地址",
            "门店名称",
            "经销商",
            "技师姓名",
            "技师手机号",
            "是否逾期",
            "任务标签",
        ];
        let base_column_widths = [
            90_u32, 140, 100, 160, 100, 100, 100, 100, 150, 110, 130, 240, 240, 140, 110, 110, 130,
            90, 220,
        ];
        let mut headers = base_headers
            .iter()
            .map(|item| item.to_string())
            .collect::<Vec<_>>();
        for image_index in 0..max_image_count {
            headers.push(format!("现场图片{}", image_index + 1));
        }

        worksheet
            .set_row_height_pixels(0, 28)
            .context("failed to set export header row height")
            .map_err(AppError::Internal)?;
        for (index, width) in base_column_widths.iter().enumerate() {
            worksheet
                .set_column_width_pixels(index as u16, *width)
                .context("failed to set export base column width")
                .map_err(AppError::Internal)?;
        }
        for image_index in 0..max_image_count {
            worksheet
                .set_column_width_pixels((base_headers.len() + image_index) as u16, 180)
                .context("failed to set export image column width")
                .map_err(AppError::Internal)?;
        }

        for (col, title) in headers.iter().enumerate() {
            worksheet
                .write_string_with_format(0, col as u16, title, &header_format)
                .context("failed to write export header cell")
                .map_err(AppError::Internal)?;
        }

        for (row_index, row) in export_rows.iter().enumerate() {
            let excel_row = (row_index + 1) as u32;
            let summary = &row.summary;
            let row_height = if row.image_buffers.is_empty() {
                26
            } else {
                140
            };
            worksheet
                .set_row_height_pixels(excel_row, row_height)
                .context("failed to set export data row height")
                .map_err(AppError::Internal)?;

            let text_columns = vec![
                summary.task_no.clone(),
                task_export_service_type_text(&summary.service_type),
                summary.service_items_text.clone(),
                summary.task_status_text.clone(),
                summary.display_status_text.clone(),
                summary.review_status_text.clone(),
                summary.score_status_text.clone(),
                summary.appointment_time.clone().unwrap_or_default(),
                summary.customer_name.clone(),
                summary.customer_mobile.clone(),
                summary.address.clone(),
                summary.technician_location_address.clone(),
                summary.store_name.clone(),
                summary.dealer_name.clone(),
                summary.technician_name.clone().unwrap_or_default(),
                summary.technician_mobile.clone().unwrap_or_default(),
                if summary.is_overdue {
                    "是".to_string()
                } else {
                    "否".to_string()
                },
                summary.task_tags.join(" | "),
            ];

            worksheet
                .write_with_format(excel_row, 0, summary.id, &cell_format)
                .context("failed to write export task id")
                .map_err(AppError::Internal)?;
            for (offset, value) in text_columns.iter().enumerate() {
                worksheet
                    .write_string_with_format(excel_row, (offset + 1) as u16, value, &cell_format)
                    .context("failed to write export text cell")
                    .map_err(AppError::Internal)?;
            }

            for image_index in 0..max_image_count {
                let col = (base_headers.len() + image_index) as u16;
                worksheet
                    .write_string_with_format(excel_row, col, "", &image_cell_format)
                    .context("failed to write export image placeholder cell")
                    .map_err(AppError::Internal)?;
                if let Some(image_buffer) = row.image_buffers.get(image_index) {
                    let image = XlsxImage::new_from_buffer(image_buffer)
                        .context("failed to build export image from buffer")
                        .map_err(AppError::Internal)?;
                    worksheet
                        .insert_image_fit_to_cell_centered(excel_row, col, &image)
                        .context("failed to insert export image into worksheet")
                        .map_err(AppError::Internal)?;
                }
            }
        }
    }

    let workbook_bytes = workbook
        .save_to_buffer()
        .context("failed to render task export workbook")
        .map_err(AppError::Internal)?;

    let mut response = Response::new(Body::from(workbook_bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"tasks-export.xlsx\""),
    );
    Ok(response)
}

async fn render_task_export_simple_workbook(
    state: &AppState,
    items: Vec<TaskSummary>,
) -> Result<Response, AppError> {
    let task_ids = items
        .iter()
        .map(|item| item.id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let extra_data = load_task_export_extra_data(state, &task_ids).await?;
    let mut export_rows = Vec::with_capacity(items.len());
    let mut max_image_count = 0usize;
    for item in items {
        let technician_score = extra_data.technician_score(item.id, &item.service_type);
        let dealer_review =
            extra_data.dealer_review(item.id, item.service_item_id, &item.service_type);
        let customer_review = extra_data.customer_review(item.id);
        max_image_count = max_image_count.max(
            extra_data
                .image_buffers_by_task
                .get(&item.id)
                .map(Vec::len)
                .unwrap_or_default(),
        );
        export_rows.push(TaskExportSimpleRow {
            summary: item,
            technician_score,
            dealer_review,
            customer_review,
        });
    }
    let image_column_count = max_image_count.max(1);

    let mut workbook = Workbook::new();
    {
        let header_format = Format::new()
            .set_bold()
            .set_font_color(Color::White)
            .set_background_color(Color::RGB(0x1F4E78))
            .set_border(FormatBorder::Thin)
            .set_align(FormatAlign::Center)
            .set_align(FormatAlign::VerticalCenter)
            .set_text_wrap();
        let cell_format = Format::new()
            .set_border(FormatBorder::Thin)
            .set_align(FormatAlign::VerticalCenter)
            .set_text_wrap();
        let image_cell_format = Format::new()
            .set_border(FormatBorder::Thin)
            .set_align(FormatAlign::Center)
            .set_align(FormatAlign::VerticalCenter);
        let worksheet = workbook
            .add_worksheet()
            .set_name("任务导出")
            .context("failed to create task export worksheet")
            .map_err(AppError::Internal)?;
        let headers = export_simple_text_headers();
        let column_widths = [
            130_u32, 120, 110, 120, 130, 100, 140, 140, 220, 220, 260, 260,
        ];

        worksheet
            .set_row_height_pixels(0, 28)
            .context("failed to set export header row height")
            .map_err(AppError::Internal)?;
        for (index, width) in column_widths.iter().enumerate() {
            worksheet
                .set_column_width_pixels(index as u16, *width)
                .context("failed to set export column width")
                .map_err(AppError::Internal)?;
        }
        for image_index in 0..image_column_count {
            worksheet
                .set_column_width_pixels((headers.len() + image_index) as u16, 180)
                .context("failed to set export image column width")
                .map_err(AppError::Internal)?;
        }
        for (col, title) in headers.iter().enumerate() {
            worksheet
                .write_string_with_format(0, col as u16, *title, &header_format)
                .context("failed to write export header cell")
                .map_err(AppError::Internal)?;
        }
        for image_index in 0..image_column_count {
            let title = if image_column_count == 1 {
                "附件图片".to_string()
            } else {
                format!("附件图片{}", image_index + 1)
            };
            worksheet
                .write_string_with_format(
                    0,
                    (headers.len() + image_index) as u16,
                    &title,
                    &header_format,
                )
                .context("failed to write export image header cell")
                .map_err(AppError::Internal)?;
        }
        for (row_index, row) in export_rows.iter().enumerate() {
            let excel_row = (row_index + 1) as u32;
            let values = task_export_simple_text_values(row);
            let image_buffers = extra_data
                .image_buffers_by_task
                .get(&row.summary.id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let row_height = if image_buffers.is_empty() { 48 } else { 140 };
            worksheet
                .set_row_height_pixels(excel_row, row_height)
                .context("failed to set export data row height")
                .map_err(AppError::Internal)?;
            for (col, value) in values.iter().enumerate() {
                worksheet
                    .write_string_with_format(excel_row, col as u16, value, &cell_format)
                    .context("failed to write export cell")
                    .map_err(AppError::Internal)?;
            }
            for image_index in 0..image_column_count {
                let col = (headers.len() + image_index) as u16;
                worksheet
                    .write_string_with_format(excel_row, col, "", &image_cell_format)
                    .context("failed to write export image placeholder cell")
                    .map_err(AppError::Internal)?;
                if let Some(image_buffer) = image_buffers.get(image_index) {
                    let image = XlsxImage::new_from_buffer(image_buffer)
                        .context("failed to build export image from buffer")
                        .map_err(AppError::Internal)?;
                    worksheet
                        .insert_image_fit_to_cell_centered(excel_row, col, &image)
                        .context("failed to insert export image into worksheet")
                        .map_err(AppError::Internal)?;
                }
            }
        }
    }

    let workbook_bytes = workbook
        .save_to_buffer()
        .context("failed to render task export workbook")
        .map_err(AppError::Internal)?;
    let mut response = Response::new(Body::from(workbook_bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"tasks-export.xlsx\""),
    );
    Ok(response)
}

fn export_simple_text_headers() -> [&'static str; 12] {
    [
        "客户手机号",
        "客户姓名",
        "任务类型",
        "师傅姓名",
        "师傅手机号",
        "师傅评分",
        "门店",
        "经销商名称",
        "经销商评价",
        "客户评价",
        "客户地址",
        "师傅定位地址",
    ]
}

fn task_export_simple_text_values(row: &TaskExportSimpleRow) -> [String; 12] {
    let summary = &row.summary;
    [
        summary.customer_mobile.clone(),
        summary.customer_name.clone(),
        task_export_service_type_text(&summary.service_type),
        summary.technician_name.clone().unwrap_or_default(),
        summary.technician_mobile.clone().unwrap_or_default(),
        row.technician_score
            .map(|score| score.to_string())
            .unwrap_or_default(),
        summary.store_name.clone(),
        summary.dealer_name.clone(),
        row.dealer_review.clone(),
        row.customer_review.clone(),
        summary.address.clone(),
        summary.technician_location_address.clone(),
    ]
}

async fn load_task_export_extra_data(
    state: &AppState,
    task_ids: &[i64],
) -> Result<TaskExportExtraData, AppError> {
    if task_ids.is_empty() {
        return Ok(TaskExportExtraData::default());
    }

    let mut extra = TaskExportExtraData::default();

    let score_items: Vec<HqScoreItemEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "hq_score_item",
        value! { "task_id": task_ids.to_vec() },
    )
    .await
    .context("failed to query export score items")
    .map_err(AppError::Internal)?;
    for item in score_items {
        let Some(task_id) = item.task_id else {
            continue;
        };
        let Some(score) = item.score else {
            continue;
        };
        extra
            .fallback_scores_by_task
            .entry(task_id)
            .or_insert(score);
        if let Some(service_type) = item
            .service_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            extra
                .scores_by_task_service
                .insert((task_id, service_type.to_string()), score);
        }
    }

    let mut dealer_reviews: Vec<DealerReviewEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "dealer_review",
        value! { "task_id": task_ids.to_vec() },
    )
    .await
    .context("failed to query export dealer reviews")
    .map_err(AppError::Internal)?;
    dealer_reviews.sort_by_key(|item| item.id.unwrap_or_default());
    for review in dealer_reviews {
        let Some(task_id) = review.task_id else {
            continue;
        };
        let text = export_review_text(review.star, review.content.as_deref());
        if text.is_empty() {
            continue;
        }
        extra.dealer_reviews_by_task.insert(task_id, text.clone());
        if let Some(service_item_id) = review.service_item_id {
            extra
                .dealer_reviews_by_service_item
                .insert((task_id, service_item_id), text.clone());
        }
        if let Some(service_type) = review
            .service_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            extra
                .dealer_reviews_by_service_type
                .insert((task_id, service_type.to_string()), text);
        }
    }

    let mut customer_reviews: Vec<CustomerReviewEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "customer_review",
        value! { "task_id": task_ids.to_vec() },
    )
    .await
    .context("failed to query export customer reviews")
    .map_err(AppError::Internal)?;
    customer_reviews.sort_by_key(|item| item.id.unwrap_or_default());
    for review in customer_reviews {
        let Some(task_id) = review.task_id else {
            continue;
        };
        let text = export_review_text(review.star, review.content.as_deref());
        if !text.is_empty() {
            extra.customer_reviews_by_task.insert(task_id, text);
        }
    }

    extra.image_buffers_by_task =
        load_task_export_image_buffers_by_task_ids(state, task_ids).await?;

    Ok(extra)
}

fn export_review_text(star: Option<i32>, content: Option<&str>) -> String {
    let content = content.unwrap_or_default().trim();
    match (star.filter(|value| *value > 0), content.is_empty()) {
        (Some(star), false) => format!("{star}分 {content}"),
        (Some(star), true) => format!("{star}分"),
        (None, false) => content.to_string(),
        (None, true) => String::new(),
    }
}

async fn load_task_summaries(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    query: &TaskListQuery,
) -> Result<Vec<TaskSummary>, AppError> {
    if is_technician_only(auth_user) {
        return load_prefetched_task_summaries(state, auth_user, query, true).await;
    }
    if auth_user.has_role("dealer") {
        return load_prefetched_task_summaries(state, auth_user, query, true).await;
    }

    load_prefetched_task_summaries(state, auth_user, query, false).await
}

async fn load_task_export_summaries(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    query: &TaskListQuery,
) -> Result<Vec<TaskSummary>, AppError> {
    load_prefetched_task_summaries(state, auth_user, query, true).await
}

async fn load_prefetched_task_summaries(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    query: &TaskListQuery,
    split_service_items: bool,
) -> Result<Vec<TaskSummary>, AppError> {
    let technician_scoped = is_technician_only(auth_user);
    let dealer_scoped = auth_user.has_role("dealer");
    let mut technician_assigned_task_ids = BTreeSet::new();
    let mut tasks = if technician_scoped {
        let assigned_service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "task_service_item",
            value! { "technician_id": auth_user.user_id },
        )
        .await
        .context("failed to query technician service item tasks")
        .map_err(AppError::Internal)?;
        technician_assigned_task_ids.extend(
            assigned_service_items
                .into_iter()
                .filter_map(|item| item.task_id),
        );
        let legacy_tasks: Vec<TaskEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "task",
            value! { "current_technician_id": auth_user.user_id },
        )
        .await
        .context("failed to query legacy technician tasks")
        .map_err(AppError::Internal)?;
        let task_ids = technician_assigned_task_ids
            .iter()
            .copied()
            .chain(legacy_tasks.iter().filter_map(|task| task.id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if task_ids.is_empty() {
            Vec::new()
        } else {
            db::select_by_map_compat::<TaskEntity>(
                state.rb.as_ref(),
                "task",
                value! { "id": task_ids },
            )
            .await
            .context("failed to batch query technician tasks")
            .map_err(AppError::Internal)?
        }
    } else if dealer_scoped {
        let store_id = dealer_owned_store_id(state, auth_user.user_id).await?;
        db::select_by_map_compat::<TaskEntity>(
            state.rb.as_ref(),
            "task",
            value! { "store_id": store_id },
        )
        .await
        .context("failed to query dealer tasks")
        .map_err(AppError::Internal)?
    } else {
        db::select_by_map_compat::<TaskEntity>(state.rb.as_ref(), "task", value! {})
            .await
            .context("failed to query tasks")
            .map_err(AppError::Internal)?
    };
    let task_type_filters = normalized_task_type_filters(query.task_type.as_deref())?;
    let group_service_counts = build_task_group_service_counts(&tasks);
    tasks.retain(|task| {
        (if dealer_scoped || technician_scoped {
            filter_task_item_for_scoped_view(task, query)
        } else {
            filter_task_item(task, query)
        }) && matches_task_type_filters(task, &task_type_filters, &group_service_counts)
    });
    if tasks.is_empty() {
        return Ok(Vec::new());
    }

    let task_ids = tasks.iter().filter_map(|item| item.id).collect::<Vec<_>>();
    let customer_ids = tasks
        .iter()
        .filter_map(|item| item.customer_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let address_ids = tasks
        .iter()
        .filter_map(|item| item.customer_address_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let store_ids = tasks
        .iter()
        .filter_map(|item| item.store_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let service_items = db::select_by_map_compat::<TaskServiceItemEntity>(
        state.rb.as_ref(),
        "task_service_item",
        value! { "task_id": task_ids.clone() },
    )
    .await
    .context("failed to query task service items for export")
    .map_err(AppError::Internal)?;

    let mut service_items_by_task_id = HashMap::<i64, Vec<TaskServiceItemEntity>>::new();
    let mut related_user_ids = BTreeSet::new();
    for item in &tasks {
        if let Some(user_id) = item.dealer_user_id {
            related_user_ids.insert(user_id);
        }
        if let Some(user_id) = item.current_technician_id {
            related_user_ids.insert(user_id);
        }
    }
    for service_item in service_items {
        if let Some(user_id) = service_item.technician_id {
            related_user_ids.insert(user_id);
        }
        if let Some(task_id) = service_item.task_id {
            service_items_by_task_id
                .entry(task_id)
                .or_default()
                .push(service_item);
        }
    }
    for items in service_items_by_task_id.values_mut() {
        items.sort_by_key(|item| item.sort_no.unwrap_or_default());
    }

    let customers = if customer_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<CustomerEntity>(
            state.rb.as_ref(),
            "customer",
            value! { "id": customer_ids },
        )
        .await
        .context("failed to query customers for export")
        .map_err(AppError::Internal)?
    };
    let addresses = if address_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<CustomerAddressEntity>(
            state.rb.as_ref(),
            "customer_address",
            value! { "id": address_ids },
        )
        .await
        .context("failed to query addresses for export")
        .map_err(AppError::Internal)?
    };
    let stores = if store_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<StoreEntity>(
            state.rb.as_ref(),
            "store",
            value! { "id": store_ids },
        )
        .await
        .context("failed to query stores for export")
        .map_err(AppError::Internal)?
    };
    let users = if related_user_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<UserEntity>(
            state.rb.as_ref(),
            "user",
            value! { "id": related_user_ids.into_iter().collect::<Vec<_>>() },
        )
        .await
        .context("failed to query users for export")
        .map_err(AppError::Internal)?
    };
    let evidences = db::select_by_map_compat::<EvidenceEntity>(
        state.rb.as_ref(),
        "evidence",
        value! { "task_id": task_ids.clone() },
    )
    .await
    .context("failed to query evidences for export")
    .map_err(AppError::Internal)?;
    let executions = db::select_by_map_compat::<TaskExecutionEntity>(
        state.rb.as_ref(),
        "task_execution",
        value! { "task_id": task_ids.clone() },
    )
    .await
    .context("failed to query executions for export")
    .map_err(AppError::Internal)?;
    let dealer_reviews = db::select_by_map_compat::<DealerReviewEntity>(
        state.rb.as_ref(),
        "dealer_review",
        value! { "task_id": task_ids.clone() },
    )
    .await
    .context("failed to query dealer reviews for task summaries")
    .map_err(AppError::Internal)?;

    let customer_by_id = customers
        .into_iter()
        .filter_map(|item| item.id.map(|id| (id, item)))
        .collect::<HashMap<_, _>>();
    let address_by_id = addresses
        .into_iter()
        .filter_map(|item| item.id.map(|id| (id, item)))
        .collect::<HashMap<_, _>>();
    let store_by_id = stores
        .into_iter()
        .filter_map(|item| item.id.map(|id| (id, item)))
        .collect::<HashMap<_, _>>();
    let user_by_id = users
        .into_iter()
        .filter_map(|item| item.id.map(|id| (id, item)))
        .collect::<HashMap<_, _>>();
    let related_user_ids = user_by_id.keys().copied().collect::<Vec<_>>();
    let dealer_display_names =
        identity::role_display_names(state, &related_user_ids, "dealer").await?;
    let technician_display_names =
        identity::role_display_names(state, &related_user_ids, "technician").await?;
    let mut evidences_by_task_id = HashMap::<i64, Vec<EvidenceEntity>>::new();
    for evidence in evidences {
        if let Some(task_id) = evidence.task_id {
            evidences_by_task_id
                .entry(task_id)
                .or_default()
                .push(evidence);
        }
    }
    let mut executions_by_task_id = HashMap::<i64, Vec<TaskExecutionEntity>>::new();
    for execution in executions {
        if let Some(task_id) = execution.task_id {
            executions_by_task_id
                .entry(task_id)
                .or_default()
                .push(execution);
        }
    }
    let mut dealer_reviews_by_task_id = HashMap::<i64, Vec<DealerReviewEntity>>::new();
    for review in dealer_reviews {
        if let Some(task_id) = review.task_id {
            dealer_reviews_by_task_id
                .entry(task_id)
                .or_default()
                .push(review);
        }
    }

    let mut items = Vec::new();
    for task in tasks {
        let task_id = task.id.unwrap_or_default();
        let task_service_items = service_items_by_task_id
            .get(&task_id)
            .cloned()
            .unwrap_or_default();
        if technician_scoped {
            if technician_assigned_task_ids.contains(&task_id) {
                for service_item in task_service_items
                    .iter()
                    .filter(|item| item.technician_id == Some(auth_user.user_id))
                    .cloned()
                {
                    items.push(build_task_summary_from_prefetched(
                        auth_user,
                        task.clone(),
                        Some(service_item),
                        &task_service_items,
                        &customer_by_id,
                        &address_by_id,
                        &store_by_id,
                        &user_by_id,
                        &dealer_display_names,
                        &technician_display_names,
                        &evidences_by_task_id,
                        &executions_by_task_id,
                        &dealer_reviews_by_task_id,
                    )?);
                }
            } else if task.current_technician_id == Some(auth_user.user_id) {
                items.push(build_task_summary_from_prefetched(
                    auth_user,
                    task,
                    None,
                    &task_service_items,
                    &customer_by_id,
                    &address_by_id,
                    &store_by_id,
                    &user_by_id,
                    &dealer_display_names,
                    &technician_display_names,
                    &evidences_by_task_id,
                    &executions_by_task_id,
                    &dealer_reviews_by_task_id,
                )?);
            }
            continue;
        }
        if !split_service_items || task_service_items.is_empty() {
            items.push(build_task_summary_from_prefetched(
                auth_user,
                task,
                None,
                &task_service_items,
                &customer_by_id,
                &address_by_id,
                &store_by_id,
                &user_by_id,
                &dealer_display_names,
                &technician_display_names,
                &evidences_by_task_id,
                &executions_by_task_id,
                &dealer_reviews_by_task_id,
            )?);
            continue;
        }
        for service_item in task_service_items.iter().cloned() {
            items.push(build_task_summary_from_prefetched(
                auth_user,
                task.clone(),
                Some(service_item),
                &task_service_items,
                &customer_by_id,
                &address_by_id,
                &store_by_id,
                &user_by_id,
                &dealer_display_names,
                &technician_display_names,
                &evidences_by_task_id,
                &executions_by_task_id,
                &dealer_reviews_by_task_id,
            )?);
        }
    }

    items.retain(|item| filter_task_summary(item, query));
    sort_task_summaries_desc(&mut items);
    Ok(items)
}

async fn load_selected_task_export_summaries(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    task_ids: &[i64],
) -> Result<Vec<TaskSummary>, AppError> {
    if task_ids.is_empty() {
        return Ok(Vec::new());
    }
    if is_technician_only(auth_user) || auth_user.has_role("dealer") {
        let query = TaskListQuery {
            task_no: None,
            service_type: None,
            task_type: None,
            task_status: None,
            review_status: None,
            score_status: None,
            store_id: None,
            dealer_user_id: None,
            technician_id: None,
            appointment_start: None,
            appointment_end: None,
            mobile: None,
            customer_name: None,
            page: None,
            page_size: None,
        };
        let items = load_task_summaries(state, auth_user, &query).await?;
        let selected = task_ids.iter().copied().collect::<BTreeSet<_>>();
        return Ok(items
            .into_iter()
            .filter(|item| selected.contains(&item.id))
            .collect());
    }

    let selected_task_ids = task_ids.iter().copied().collect::<BTreeSet<_>>();
    let mut tasks = db::select_by_map_compat::<TaskEntity>(
        state.rb.as_ref(),
        "task",
        value! { "id": task_ids.to_vec() },
    )
    .await
    .context("failed to query selected tasks for export")
    .map_err(AppError::Internal)?;
    tasks.retain(|task| {
        task.deleted_at.is_none()
            && task
                .id
                .map(|id| selected_task_ids.contains(&id))
                .unwrap_or(false)
    });
    if tasks.is_empty() {
        return Ok(Vec::new());
    }

    let query = TaskListQuery {
        task_no: None,
        service_type: None,
        task_type: None,
        task_status: None,
        review_status: None,
        score_status: None,
        store_id: None,
        dealer_user_id: None,
        technician_id: None,
        appointment_start: None,
        appointment_end: None,
        mobile: None,
        customer_name: None,
        page: None,
        page_size: None,
    };
    let mut items = load_task_export_summaries(state, auth_user, &query).await?;
    items.retain(|item| selected_task_ids.contains(&item.id));
    Ok(items)
}

fn sort_task_summaries_desc(items: &mut [TaskSummary]) {
    items.sort_by(|left, right| {
        right
            .created_at_sort
            .cmp(&left.created_at_sort)
            .then_with(|| right.id.cmp(&left.id))
    });
}

fn task_log_action_text(action: &str, remark: Option<&str>) -> String {
    match action {
        "create_task" => "创建任务".to_string(),
        "delete_task" => "删除任务".to_string(),
        "batch_delete_task" => "批量删除任务".to_string(),
        "assign_task" => "分派任务".to_string(),
        "reassign_task" => "任务重派".to_string(),
        "start_task" => "技师开始执行任务".to_string(),
        "submit_task" => "技师提交检核".to_string(),
        "close_task" => "关闭任务".to_string(),
        "hq_score" => "总部评分完成".to_string(),
        "dealer_review" => {
            let label = review_level_text_from_remark(remark).unwrap_or("已评价");
            format!("客户评价：{label}")
        }
        other => other.to_string(),
    }
}

fn review_level_text_from_remark(remark: Option<&str>) -> Option<&'static str> {
    let text = remark?.trim();
    if text.is_empty() {
        return None;
    }
    if text.contains("非常") || text.contains("很好") || text.contains("满意") {
        return Some("非常好");
    }
    if text.contains("一般") {
        return Some("一般");
    }
    if text.contains("差") {
        return Some("较差");
    }
    Some("已评价")
}

async fn resolve_task_submit_project_name(
    state: &AppState,
    task: &TaskEntity,
) -> Result<String, AppError> {
    if let Some(store_id) = task.store_id {
        if let Some(store) = db::select_one_by_map_compat::<StoreEntity>(
            state.rb.as_ref(),
            "store",
            value! { "id": store_id },
        )
        .await
        .context("failed to query store for task submit project name")
        .map_err(AppError::Internal)?
            && let Some(name) = store.store_name.filter(|item| !item.trim().is_empty())
        {
            return Ok(name);
        }
    }

    let customer = find_customer_by_id(state, task.customer_id).await?;
    if let Some(name) = customer.name.filter(|item| !item.trim().is_empty()) {
        return Ok(name);
    }

    Ok(task
        .task_no
        .clone()
        .unwrap_or_else(|| "项目服务".to_string()))
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
    let dealer_review =
        find_dealer_review_for_scope(&state, task_id, &visible_service_items).await?;
    let dealer_review_exists = dealer_review.is_some();
    let customer_review = find_customer_review_for_task(&state, task_id).await?;
    let dealer_review_payload = dealer_review.map(|review| {
        let star = review.star.unwrap_or_default();
        ReviewInfoPayload {
            star,
            score: star,
            score_text: review_score_text(star),
            tags: review
                .tag_json
                .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
                .unwrap_or_default(),
            content: review.content,
            created_at: review.created_at.map(format_datetime),
        }
    });
    let hq_score = find_hq_score_for_task(&state, task_id).await?;
    let scoped_service_status = if visible_service_items.len() == 1 {
        visible_service_items
            .first()
            .and_then(|item| item.service_status.as_deref())
    } else {
        None
    };
    let scoped_service_status =
        if dealer_review_exists && scoped_service_status == Some("pending_dealer_review") {
            Some("completed")
        } else {
            scoped_service_status
        };
    let task_status = normalize_task_status_for_client(
        scoped_service_status
            .or(task.task_status.as_deref())
            .unwrap_or_default(),
    )
    .to_string();
    let review_status = scoped_review_status(task.review_status.as_deref(), scoped_service_status);
    let score_status = scoped_score_status(task.score_status.as_deref(), scoped_service_status);
    let priority = task
        .priority
        .clone()
        .unwrap_or_else(|| "normal".to_string());
    let task_desc = task.task_desc.clone();
    let task_remark = task.task_remark.clone();
    let service_time_slot =
        format_task_service_time_slot(task.appointment_time, task.service_time_slot.as_deref());
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
    let execution_service_type_by_id = executions
        .iter()
        .filter_map(|item| item.id.map(|id| (id, item.service_type.clone())))
        .collect::<HashMap<_, _>>();
    let evidence_items = evidences
        .iter()
        .map(|item| EvidenceItemPayload {
            id: item.id.unwrap_or_default(),
            execution_id: item.execution_id,
            service_type: item.execution_id.and_then(|execution_id| {
                execution_service_type_by_id
                    .get(&execution_id)
                    .cloned()
                    .flatten()
            }),
            evidence_type: item.evidence_type.clone().unwrap_or_default(),
            file_name: item.file_name.clone().unwrap_or_default(),
            file_url: crate::storage::public_url(&item.file_url.clone().unwrap_or_default()),
            created_at: item.created_at.map(format_datetime),
        })
        .collect::<Vec<_>>();
    let customer_address = format_customer_address(&address);
    let technician_location_address =
        crate::routes::technician_location::latest_technician_location_address(
            &evidences,
            &executions,
            &visible_service_items,
            &customer_address,
        );

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
        customer_source: customer.source.clone(),
        address_id: task.customer_address_id,
        customer_address: customer_address.clone(),
        technician_location_address,
        customer: RelatedCustomer {
            id: customer.id.unwrap_or_default(),
            name: customer.name.unwrap_or_default(),
            mobile: customer_mobile.clone(),
            contact_phone: customer_mobile.clone(),
            source: customer.source,
            address_id: task.customer_address_id,
            address: Some(customer_address),
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
            average_score_text: format_score(technician_average_score),
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
        review_info: dealer_review_payload.clone(),
        dealer_review: dealer_review_payload,
        customer_review,
        hq_score,
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
                service_item_id: item.service_item_id,
                service_type: item.service_type,
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

    let grouped_tasks = load_grouped_assignable_tasks(&state, &task).await?;
    let assignments = resolve_assignment_targets(&payload, &grouped_tasks)?;
    ensure_unique_assignment_targets(&assignments)?;

    for assignment in &assignments {
        let assignment_task = grouped_tasks
            .iter()
            .find(|item| item.task.id == Some(assignment.task_id))
            .ok_or_else(|| AppError::BadRequest("服务环节不属于当前任务".to_string()))?;
        ensure_technician_assignable_for_service(
            &state,
            &assignment_task.task,
            assignment.technician_id,
            &assignment.service_type,
        )
        .await?;
        TaskAssignmentEntity::insert(
            state.rb.as_ref(),
            &TaskAssignmentEntity {
                id: None,
                task_id: Some(assignment.task_id),
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

    for grouped_task in &grouped_tasks {
        let grouped_task_id = grouped_task.task.id.unwrap_or_default();
        let latest_assignment = latest_task_assignment(&state, grouped_task_id).await?;
        let updated_service_items =
            load_task_service_item_entities(&state, grouped_task_id).await?;
        let all_service_items_assigned = updated_service_items.is_empty()
            || updated_service_items
                .iter()
                .all(|item| item.technician_id.is_some());
        let grouped_task_status = if all_service_items_assigned {
            "pending_execute".to_string()
        } else {
            grouped_task
                .task
                .task_status
                .clone()
                .unwrap_or_else(|| "pending_assign".to_string())
        };
        let primary_technician_id = updated_service_items
            .first()
            .and_then(|item| item.technician_id)
            .or_else(|| {
                assignments
                    .iter()
                    .find(|item| item.task_id == grouped_task_id)
                    .map(|item| item.technician_id)
            });
        TaskEntity::update_by_map(
            state.rb.as_ref(),
            &TaskEntity {
                current_technician_id: primary_technician_id,
                task_status: Some(grouped_task_status),
                latest_assignment_id: latest_assignment.id,
                ..Default::default()
            },
            value! { "id": grouped_task_id },
        )
        .await
        .context("failed to update grouped task assignment state")
        .map_err(AppError::Internal)?;
    }

    let after_task = find_task_by_id(&state, task_id).await?;
    for assignment in &assignments {
        send_task_assignment_notice(
            &state,
            assignment.technician_id,
            assignment.task_id,
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
        task_status: "pending_execute".to_string(),
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
    let service_items_to_start = if let Some(service_item) = service_item.as_ref() {
        vec![service_item.clone()]
    } else {
        load_task_service_item_entities(&state, task_id).await?
    };
    let started_at = local_now();

    let execution = TaskExecutionEntity {
        id: None,
        task_id: Some(task_id),
        service_item_id,
        service_type: service_type.clone(),
        technician_id: Some(auth_user.user_id),
        started_at: Some(started_at),
        submitted_at: None,
        submit_remark: None,
        execution_status: Some("executing".to_string()),
        created_at: None,
        updated_at: None,
    };
    let tx = state
        .rb
        .acquire_begin()
        .await
        .context("failed to begin task start transaction")
        .map_err(AppError::Internal)?
        .defer_async(|tx| async move {
            if !tx.done() {
                let _ = tx.rollback().await;
            }
        });
    let write_result: Result<i64, AppError> = async {
        let execution_insert_result = TaskExecutionEntity::insert(&tx, &execution)
            .await
            .context("failed to create execution record")
            .map_err(AppError::Internal)?;
        let execution_id = execution_insert_result
            .last_insert_id
            .as_i64()
            .ok_or_else(|| AppError::Internal(anyhow!("task execution id missing")))?;
        let task_update_result = TaskEntity::update_by_map(
            &tx,
            &TaskEntity {
                task_status: Some("executing".to_string()),
                latest_execution_id: Some(execution_id),
                ..Default::default()
            },
            value! {
                "id": task_id,
                "task_status": task.task_status.clone(),
                "current_technician_id": task.current_technician_id,
                "latest_assignment_id": task.latest_assignment_id,
                "latest_execution_id": task.latest_execution_id,
                "updated_at": task.updated_at
            },
        )
        .await
        .context("failed to update task execution state")
        .map_err(AppError::Internal)?;
        if task_update_result.rows_affected == 0 {
            return Err(AppError::Conflict(
                "任务已被重新分派或更新，请刷新后重试".to_string(),
            ));
        }

        for service_item in &service_items_to_start {
            let Some(service_item_id) = service_item.id else {
                continue;
            };
            let service_item_update_result = TaskServiceItemEntity::update_by_map(
                &tx,
                &TaskServiceItemEntity {
                    technician_id: Some(auth_user.user_id),
                    service_status: Some("executing".to_string()),
                    started_at: Some(started_at),
                    progress_percent: Some(60),
                    ..Default::default()
                },
                value! {
                    "id": service_item_id,
                    "technician_id": service_item.technician_id,
                    "service_status": service_item.service_status.clone(),
                    "updated_at": service_item.updated_at,
                    "column": [
                        "technician_id",
                        "service_status",
                        "started_at",
                        "progress_percent"
                    ]
                },
            )
            .await
            .context("failed to update task service item on start")
            .map_err(AppError::Internal)?;
            if service_item_update_result.rows_affected == 0 {
                return Err(AppError::Conflict(
                    "任务服务环节已被重新分派或更新，请刷新后重试".to_string(),
                ));
            }
        }

        Ok(execution_id)
    }
    .await;
    let execution_id = match write_result {
        Ok(execution_id) => execution_id,
        Err(error) => {
            if let Err(rollback_error) = tx.rollback().await {
                tracing::error!(%rollback_error, "failed to roll back task start transaction");
            }
            return Err(error);
        }
    };
    tx.commit()
        .await
        .context("failed to commit task start transaction")
        .map_err(AppError::Internal)?;

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
        execution_id: Some(execution_id),
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
    match execution.execution_status.as_deref() {
        Some("executing") => {}
        Some("submitted") => {
            return Err(AppError::Conflict("当前服务环节已提交检核".to_string()));
        }
        _ => {
            return Err(AppError::Conflict(
                "当前执行记录已失效，请重新开始任务".to_string(),
            ));
        }
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

    if let Some(location_address) = payload
        .location_address
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        super::files::refresh_execution_evidence_watermarks(
            &state,
            &task,
            payload.execution_id,
            location_address,
        )
        .await?;
    }

    let execution_update_result = TaskExecutionEntity::update_by_map(
        state.rb.as_ref(),
        &TaskExecutionEntity {
            submitted_at: Some(local_now()),
            submit_remark: payload.submit_remark.clone(),
            execution_status: Some("submitted".to_string()),
            ..Default::default()
        },
        value! { "id": payload.execution_id, "execution_status": "executing" },
    )
    .await
    .context("failed to update execution submit state")
    .map_err(AppError::Internal)?;
    if execution_update_result.rows_affected == 0 {
        return Err(AppError::Conflict(
            "当前执行记录状态已变化，请刷新后重试".to_string(),
        ));
    }

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
    let customer = find_customer_by_id(&state, task.customer_id).await?;
    if task_status == "pending_dealer_review" {
        match customer.mobile.as_deref().map(str::trim) {
            Some(customer_mobile) if is_valid_sms_mobile(customer_mobile) => {
                let review_link_token = ensure_customer_review_link_for_task(&state, &task).await?;
                match sms::send_customer_review_sms(
                    &state,
                    customer_mobile,
                    sms::CustomerReviewSmsPayload {
                        review_link_code: &review_link_token,
                    },
                )
                .await
                {
                    Ok(Some(result)) => {
                        mark_customer_review_link_sent(&state, task_id, &review_link_token).await?;
                        tracing::info!(
                            task_id,
                            customer_mobile,
                            request_id = ?result.request_id,
                            biz_id = ?result.biz_id,
                            "customer review sms sent"
                        );
                    }
                    Ok(None) => {
                        tracing::info!(
                            task_id,
                            customer_mobile,
                            "customer review sms skipped by sms configuration"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(
                            task_id,
                            customer_mobile,
                            error = %error,
                            "customer review sms failed"
                        );
                    }
                }
            }
            Some(customer_mobile) if !customer_mobile.is_empty() => {
                tracing::warn!(
                    task_id,
                    customer_mobile,
                    "customer review sms skipped because customer mobile is invalid"
                );
            }
            _ => {
                tracing::warn!(
                    task_id,
                    "customer review sms skipped because customer mobile is empty"
                );
            }
        }
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

    let task = find_task_by_id(&state, task_id).await?;
    ensure_task_access(&state, &auth_user, &task).await?;
    let before_task = task.clone();
    let assignment_request = payload.assignment_request();
    let assign_reason = payload.resolved_assign_reason();
    if payload.uses_scoped_reassign() {
        if payload.has_task_detail_edits() {
            return Err(AppError::BadRequest(
                "按服务环节重派时不能同时修改客户、地址、时间或任务资料，请分开操作".to_string(),
            ));
        }
        validate_scoped_reassign_identifiers(&assignment_request)?;
        let response = reassign_task_scopes(
            &state,
            &auth_user,
            &task,
            &assignment_request,
            &assign_reason,
        )
        .await?;
        return Ok(Json(ApiResponse::success(response)));
    }

    let technician_id = payload
        .technician_id
        .ok_or_else(|| AppError::BadRequest("请选择技师".to_string()))?;
    if !matches!(
        task.task_status.as_deref(),
        Some("pending_execute") | Some("executing")
    ) {
        return Err(AppError::Conflict("当前任务状态不允许重派技师".to_string()));
    }
    let service_items = load_task_service_item_entities(&state, task_id).await?;
    let before_service_items = service_items.clone();
    let type_plan =
        build_whole_reassign_type_plan(&task, &service_items, payload.service_type.as_deref())?;
    if type_plan.service_type_changed() {
        let target_service_type = type_plan.target_service_type.as_deref().ok_or_else(|| {
            AppError::Internal(anyhow!("target service type missing during reassign"))
        })?;
        ensure_reassign_service_type_available_in_group(&state, &task, target_service_type).await?;
    }
    if service_items
        .iter()
        .any(|item| service_status_blocks_reassign(item.service_status.as_deref()))
    {
        return Err(AppError::Conflict(
            "任务已有服务环节提交检核，不能再重派".to_string(),
        ));
    }
    let executions: Vec<TaskExecutionEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_execution",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query task executions during reassign")
    .map_err(AppError::Internal)?;
    if executions
        .iter()
        .any(|item| item.execution_status.as_deref() == Some("submitted"))
    {
        return Err(AppError::Conflict("任务已提交检核，不能再重派".to_string()));
    }
    let customer = find_customer_by_id(&state, task.customer_id).await?;
    if customer.deleted_at.is_some() {
        return Err(AppError::Conflict(
            "当前任务客户已删除，无法重派".to_string(),
        ));
    }
    let customer_id = customer
        .id
        .ok_or_else(|| AppError::Internal(anyhow!("customer.id missing")))?;
    let customer_name_update =
        normalize_required_reassign_edit(payload.resolved_customer_name(), "请输入客户姓名")?;
    let customer_mobile_update =
        normalize_required_reassign_edit(payload.resolved_customer_mobile(), "请输入客户手机号")?;
    let customer_source_update = payload
        .resolved_customer_source()
        .map(str::trim)
        .map(ToString::to_string);

    if let Some(customer_mobile) = customer_mobile_update.as_deref() {
        let customers: Vec<CustomerEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "customer",
            value! { "mobile": customer_mobile },
        )
        .await
        .context("failed to query customer mobile during task reassign")
        .map_err(AppError::Internal)?;
        if customers
            .into_iter()
            .any(|item| item.id != Some(customer_id))
        {
            return Err(AppError::Conflict("该手机号客户已存在".to_string()));
        }
    }

    let appointment_time_update = payload
        .appointment_time
        .as_deref()
        .map(str::trim)
        .map(parse_datetime)
        .transpose()?;
    let next_appointment_time = appointment_time_update
        .or(task.appointment_time)
        .ok_or_else(|| AppError::Internal(anyhow!("task.appointment_time missing")))?;

    let mut selected_address_id = task
        .customer_address_id
        .ok_or_else(|| AppError::Internal(anyhow!("task.customer_address_id missing")))?;
    let mut new_address = None;
    if let Some(address) = payload.resolved_address() {
        if address.detail.trim().is_empty() {
            return Err(AppError::BadRequest("请输入服务地址".to_string()));
        }
        let current_address = find_address_by_id(&state, Some(selected_address_id)).await?;
        if !address_request_matches_entity(address, &current_address) {
            new_address = Some(CustomerAddressEntity {
                id: None,
                customer_id: Some(customer_id),
                province: address.province.clone(),
                city: address.city.clone(),
                district: address.district.clone(),
                address: Some(address.detail.trim().to_string()),
                is_default: Some(0),
                created_at: None,
            });
        }
    } else if let Some(address_id) = payload.address_id {
        let address = find_address_by_id(&state, Some(address_id)).await?;
        if address.customer_id != Some(customer_id) {
            return Err(AppError::Forbidden("地址不属于当前客户".to_string()));
        }
        selected_address_id = address_id;
    }

    if type_plan.type_update_requested {
        let target_service_type = type_plan.target_service_type.as_deref().ok_or_else(|| {
            AppError::Internal(anyhow!("target service type missing during reassign"))
        })?;
        ensure_technician_assignable_for_service(&state, &task, technician_id, target_service_type)
            .await?;
    } else {
        ensure_technician_assignable(&state, &task, technician_id).await?;
    }

    let next_customer_name = customer_name_update
        .clone()
        .or_else(|| customer.name.clone());
    let next_customer_mobile = customer_mobile_update
        .clone()
        .or_else(|| customer.mobile.clone());
    let next_customer_source = customer_source_update
        .clone()
        .or_else(|| customer.source.clone());
    let next_service_time_slot = payload
        .service_time_slot
        .clone()
        .or_else(|| task.service_time_slot.clone());
    let next_priority = payload.priority.clone().or_else(|| task.priority.clone());
    let next_task_desc = payload.task_desc.clone().or_else(|| task.task_desc.clone());
    let next_task_remark = payload
        .task_remark
        .clone()
        .or_else(|| task.task_remark.clone());
    let next_task_service_type = if type_plan.type_update_requested {
        type_plan.target_service_type.clone()
    } else {
        task.service_type.clone()
    };
    let response_service_type = type_plan
        .type_update_requested
        .then(|| type_plan.target_service_type.clone())
        .flatten();
    let detached_service_item_id = type_plan
        .service_item_type_changed
        .then_some(type_plan.service_item_id)
        .flatten();
    let detached_execution_service_type = type_plan
        .original_service_item_type
        .clone()
        .or_else(|| task.service_type.clone());
    let has_customer_update = next_customer_name != customer.name
        || next_customer_mobile != customer.mobile
        || next_customer_source != customer.source;

    let tx = state
        .rb
        .acquire_begin()
        .await
        .context("failed to begin task reassign transaction")
        .map_err(AppError::Internal)?
        .defer_async(|tx| async move {
            if !tx.done() {
                let _ = tx.rollback().await;
            }
        });

    let write_result: Result<(), AppError> = async {
        if type_plan.service_type_changed() {
            let task_group_no = task
                .task_group_no
                .as_deref()
                .map(str::trim)
                .filter(|task_group_no| !task_group_no.is_empty());
            if let (Some(task_group_no), Some(target_service_type)) =
                (task_group_no, type_plan.target_service_type.as_deref())
            {
                let grouped_tasks: Vec<TaskEntity> = db::select_by_map_for_update_compat(
                    &tx,
                    "task",
                    value! { "task_group_no": task_group_no },
                )
                .await
                .context("failed to lock grouped tasks before service type change")
                .map_err(AppError::Internal)?;
                if task_group_has_service_type_conflict(
                    task.id,
                    target_service_type,
                    &grouped_tasks,
                ) {
                    return Err(AppError::Conflict(
                        "同一批任务中已存在该服务类型，不能重复修改".to_string(),
                    ));
                }
            }
        }

        if has_customer_update {
            let customer_update_result = CustomerEntity::update_by_map(
                &tx,
                &CustomerEntity {
                    name: next_customer_name,
                    mobile: next_customer_mobile,
                    source: next_customer_source,
                    ..Default::default()
                },
                value! {
                    "id": customer_id,
                    "name": customer.name.clone(),
                    "mobile": customer.mobile.clone(),
                    "updated_at": customer.updated_at,
                    "column": ["name", "mobile", "source"]
                },
            )
            .await
            .map_err(|error| {
                let message = error.to_string();
                if message.contains("Duplicate entry") || message.contains("1062") {
                    AppError::Conflict("该手机号客户已存在".to_string())
                } else {
                    AppError::Internal(anyhow!(
                        "failed to update customer during task reassign: {error}"
                    ))
                }
            })?;
            if customer_update_result.rows_affected == 0 {
                return Err(AppError::Conflict(
                    "客户资料已被其他操作更新，请刷新后重试".to_string(),
                ));
            }
        }

        if let Some(address) = new_address.as_ref() {
            let insert_result = CustomerAddressEntity::insert(&tx, address)
                .await
                .context("failed to create customer address during task reassign")
                .map_err(AppError::Internal)?;
            selected_address_id = insert_result
                .last_insert_id
                .as_i64()
                .ok_or_else(|| AppError::Internal(anyhow!("customer address id missing")))?;
        }

        for execution in &executions {
            let Some(execution_id) = execution.id else {
                continue;
            };
            let should_cancel = execution.execution_status.as_deref() == Some("executing");
            let should_detach = detached_service_item_id
                .is_some_and(|service_item_id| execution.service_item_id == Some(service_item_id));
            if !should_cancel && !should_detach {
                continue;
            }

            let update_result = if should_detach {
                TaskExecutionEntity::update_by_map(
                    &tx,
                    &TaskExecutionEntity {
                        service_item_id: None,
                        service_type: execution
                            .service_type
                            .clone()
                            .or_else(|| detached_execution_service_type.clone()),
                        execution_status: if should_cancel {
                            Some("cancelled".to_string())
                        } else {
                            execution.execution_status.clone()
                        },
                        ..Default::default()
                    },
                    value! {
                        "id": execution_id,
                        "service_item_id": detached_service_item_id,
                        "execution_status": execution.execution_status.clone(),
                        "updated_at": execution.updated_at,
                        "column": ["service_item_id", "service_type", "execution_status"]
                    },
                )
                .await
                .context("failed to detach historical execution during service type change")
                .map_err(AppError::Internal)?
            } else {
                TaskExecutionEntity::update_by_map(
                    &tx,
                    &TaskExecutionEntity {
                        execution_status: Some("cancelled".to_string()),
                        ..Default::default()
                    },
                    value! {
                        "id": execution_id,
                        "execution_status": "executing",
                        "column": ["execution_status"]
                    },
                )
                .await
                .context("failed to cancel active task execution during reassign")
                .map_err(AppError::Internal)?
            };
            if update_result.rows_affected == 0 {
                return Err(AppError::Conflict(
                    "任务执行状态已变化，请刷新后重试".to_string(),
                ));
            }
        }

        let assignment_result = TaskAssignmentEntity::insert(
            &tx,
            &TaskAssignmentEntity {
                id: None,
                task_id: Some(task_id),
                from_technician_id: task.current_technician_id,
                to_technician_id: Some(technician_id),
                assign_type: Some("reassign".to_string()),
                assign_reason: Some(assign_reason.clone()),
                assigned_by: Some(auth_user.user_id),
                assigned_at: Some(local_now()),
            },
        )
        .await
        .context("failed to create reassign record")
        .map_err(AppError::Internal)?;
        let assignment_id = assignment_result
            .last_insert_id
            .as_i64()
            .ok_or_else(|| AppError::Internal(anyhow!("task assignment id missing")))?;

        let task_update_result = TaskEntity::update_by_map(
            &tx,
            &TaskEntity {
                customer_address_id: Some(selected_address_id),
                current_technician_id: Some(technician_id),
                service_type: next_task_service_type.clone(),
                priority: next_priority,
                appointment_time: Some(next_appointment_time),
                service_time_slot: next_service_time_slot,
                task_desc: next_task_desc,
                task_remark: next_task_remark,
                task_status: Some("pending_execute".to_string()),
                latest_assignment_id: Some(assignment_id),
                latest_execution_id: None,
                ..Default::default()
            },
            value! {
                "id": task_id,
                "task_status": task.task_status.clone(),
                "current_technician_id": task.current_technician_id,
                "service_type": task.service_type.clone(),
                "latest_assignment_id": task.latest_assignment_id,
                "latest_execution_id": task.latest_execution_id,
                "updated_at": task.updated_at,
                "column": [
                    "customer_address_id",
                    "current_technician_id",
                    "service_type",
                    "priority",
                    "appointment_time",
                    "service_time_slot",
                    "task_desc",
                    "task_remark",
                    "task_status",
                    "latest_assignment_id",
                    "latest_execution_id"
                ]
            },
        )
        .await
        .context("failed to update task after reassign")
        .map_err(AppError::Internal)?;
        if task_update_result.rows_affected == 0 {
            return Err(AppError::Conflict(
                "任务已被其他操作更新，请刷新后重试".to_string(),
            ));
        }

        for service_item in &service_items {
            let Some(service_item_id) = service_item.id else {
                continue;
            };
            let service_item_update_result = TaskServiceItemEntity::update_by_map(
                &tx,
                &TaskServiceItemEntity {
                    service_type: if type_plan.type_update_requested {
                        type_plan.target_service_type.clone()
                    } else {
                        service_item.service_type.clone()
                    },
                    technician_id: Some(technician_id),
                    service_status: Some("pending_execute".to_string()),
                    scheduled_time: resolve_reassign_scheduled_time(
                        appointment_time_update,
                        service_item.scheduled_time,
                    ),
                    started_at: None,
                    completed_at: None,
                    progress_percent: Some(0),
                    ..Default::default()
                },
                value! {
                    "id": service_item_id,
                    "service_type": service_item.service_type.clone(),
                    "updated_at": service_item.updated_at,
                    "column": [
                        "service_type",
                        "technician_id",
                        "service_status",
                        "scheduled_time",
                        "started_at",
                        "completed_at",
                        "progress_percent"
                    ]
                },
            )
            .await
            .context("failed to update task service item during reassign")
            .map_err(AppError::Internal)?;
            if type_plan.service_item_type_changed
                && service_items.len() == 1
                && service_item_update_result.rows_affected == 0
            {
                return Err(AppError::Conflict(
                    "服务环节已被其他操作更新，请刷新后重试".to_string(),
                ));
            }
        }

        Ok(())
    }
    .await;

    if let Err(error) = write_result {
        if let Err(rollback_error) = tx.rollback().await {
            tracing::error!(%rollback_error, "failed to roll back task reassign transaction");
        }
        return Err(error);
    }
    tx.commit()
        .await
        .context("failed to commit task reassign transaction")
        .map_err(AppError::Internal)?;

    let notice_service_type = (service_items.len() <= 1)
        .then_some(type_plan.target_service_type.as_deref())
        .flatten();
    send_task_assignment_notice(
        &state,
        technician_id,
        task_id,
        notice_service_type,
        "task_reassign",
    )
    .await;
    match find_task_by_id(&state, task_id).await {
        Ok(after_task) => {
            let after_customer = match find_customer_by_id(&state, after_task.customer_id).await {
                Ok(customer) => Some(customer),
                Err(error) => {
                    tracing::error!(?error, task_id, "failed to reload customer after reassign");
                    None
                }
            };
            let after_service_items = match load_task_service_item_entities(&state, task_id).await {
                Ok(service_items) => Some(service_items),
                Err(error) => {
                    tracing::error!(
                        ?error,
                        task_id,
                        "failed to reload service items after reassign"
                    );
                    None
                }
            };
            let before_snapshot = serde_json::json!({
                "task": &before_task,
                "customer": &customer,
                "service_items": &before_service_items,
            });
            let after_snapshot = serde_json::json!({
                "task": &after_task,
                "customer": after_customer,
                "service_items": after_service_items,
            });
            if let Err(error) = audit::write_audit_log(
                &state,
                "task",
                task_id,
                "reassign_task",
                auth_user.user_id,
                primary_role(&auth_user),
                Some(&before_snapshot),
                Some(&after_snapshot),
                Some(assign_reason.clone()),
            )
            .await
            {
                tracing::error!(?error, task_id, "failed to write reassign audit log");
            }
        }
        Err(error) => {
            tracing::error!(?error, task_id, "failed to reload task after reassign");
        }
    }

    Ok(Json(ApiResponse::success(TaskActionResponse {
        task_id,
        task_status: "pending_execute".to_string(),
        service_item_id: None,
        service_type: response_service_type,
        execution_id: None,
    })))
}

async fn reassign_task_scopes(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    root_task: &TaskEntity,
    payload: &AssignTaskRequest,
    assign_reason: &str,
) -> Result<TaskActionResponse, AppError> {
    let root_task_id = root_task
        .id
        .ok_or_else(|| AppError::Internal(anyhow!("task.id missing")))?;
    let task_scopes = load_grouped_assignable_tasks(state, root_task).await?;
    let mut assignments = resolve_assignment_targets(payload, &task_scopes)?;
    ensure_unique_assignment_targets(&assignments)?;
    sort_reassign_assignments(&mut assignments);

    let affected_task_ids = assignments
        .iter()
        .map(|assignment| assignment.task_id)
        .collect::<BTreeSet<_>>();
    for assignment in &assignments {
        let scope = assignment_task_scope(&task_scopes, assignment)?;
        ensure_task_access(state, auth_user, &scope.task).await?;
        validate_scoped_reassign_target(scope, assignment)?;
        ensure_technician_assignable_for_service(
            state,
            &scope.task,
            assignment.technician_id,
            &assignment.service_type,
        )
        .await?;
    }

    let mut executions = Vec::<TaskExecutionEntity>::new();
    for affected_task_id in &affected_task_ids {
        let mut task_executions: Vec<TaskExecutionEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "task_execution",
            value! { "task_id": *affected_task_id },
        )
        .await
        .context("failed to query task executions during scoped reassign")
        .map_err(AppError::Internal)?;
        executions.append(&mut task_executions);
    }
    if executions.iter().any(|execution| {
        execution.execution_status.as_deref() == Some("submitted")
            && assignments
                .iter()
                .any(|assignment| execution_matches_assignment(execution, assignment))
    }) {
        return Err(AppError::Conflict(
            "选中的服务环节已提交检核，不能重派".to_string(),
        ));
    }
    let active_execution_ids = executions
        .iter()
        .filter(|execution| execution.execution_status.as_deref() == Some("executing"))
        .filter(|execution| {
            assignments
                .iter()
                .any(|assignment| execution_matches_assignment(execution, assignment))
        })
        .filter_map(|execution| execution.id)
        .collect::<BTreeSet<_>>();

    let tx = state
        .rb
        .acquire_begin()
        .await
        .context("failed to begin scoped task reassign transaction")
        .map_err(AppError::Internal)?
        .defer_async(|tx| async move {
            if !tx.done() {
                let _ = tx.rollback().await;
            }
        });

    let write_result: Result<HashMap<i64, String>, AppError> = async {
        for execution_id in &active_execution_ids {
            let update_result = TaskExecutionEntity::update_by_map(
                &tx,
                &TaskExecutionEntity {
                    execution_status: Some("cancelled".to_string()),
                    ..Default::default()
                },
                value! {
                    "id": *execution_id,
                    "execution_status": "executing",
                    "column": ["execution_status"]
                },
            )
            .await
            .context("failed to cancel scoped task execution during reassign")
            .map_err(AppError::Internal)?;
            if update_result.rows_affected == 0 {
                return Err(AppError::Conflict(
                    "选中环节的执行状态已变化，请刷新后重试".to_string(),
                ));
            }
        }

        let mut latest_assignment_ids = HashMap::<i64, i64>::new();
        for assignment in &assignments {
            let insert_result = TaskAssignmentEntity::insert(
                &tx,
                &TaskAssignmentEntity {
                    id: None,
                    task_id: Some(assignment.task_id),
                    from_technician_id: assignment.from_technician_id,
                    to_technician_id: Some(assignment.technician_id),
                    assign_type: Some("reassign".to_string()),
                    assign_reason: Some(assign_reason.to_string()),
                    assigned_by: Some(auth_user.user_id),
                    assigned_at: Some(local_now()),
                },
            )
            .await
            .context("failed to create scoped reassign record")
            .map_err(AppError::Internal)?;
            let assignment_id = insert_result
                .last_insert_id
                .as_i64()
                .ok_or_else(|| AppError::Internal(anyhow!("task assignment id missing")))?;
            latest_assignment_ids.insert(assignment.task_id, assignment_id);
        }

        for assignment in &assignments {
            let scope = assignment_task_scope(&task_scopes, assignment)?;
            let Some(service_item) = assignment_service_item(scope, assignment) else {
                continue;
            };
            let service_item_id = service_item.id.ok_or_else(|| {
                AppError::Internal(anyhow!("task_service_item.id missing during reassign"))
            })?;
            let service_item_would_change =
                service_item_would_change_on_reassign(service_item, assignment.technician_id);
            let service_item_update_result = TaskServiceItemEntity::update_by_map(
                &tx,
                &TaskServiceItemEntity {
                    technician_id: Some(assignment.technician_id),
                    service_status: Some("pending_execute".to_string()),
                    started_at: None,
                    completed_at: None,
                    progress_percent: Some(0),
                    ..Default::default()
                },
                value! {
                    "id": service_item_id,
                    "technician_id": service_item.technician_id,
                    "service_status": service_item.service_status.clone(),
                    "updated_at": service_item.updated_at,
                    "column": [
                        "technician_id",
                        "service_status",
                        "started_at",
                        "completed_at",
                        "progress_percent"
                    ]
                },
            )
            .await
            .context("failed to reset scoped task service item during reassign")
            .map_err(AppError::Internal)?;
            if service_item_update_result.rows_affected == 0 && service_item_would_change {
                return Err(AppError::Conflict(
                    "选中服务环节已被其他操作更新，请刷新后重试".to_string(),
                ));
            }
        }

        let mut updated_task_statuses = HashMap::<i64, String>::new();
        for affected_task_id in &affected_task_ids {
            let scope = task_scopes
                .iter()
                .find(|scope| scope.task.id == Some(*affected_task_id))
                .ok_or_else(|| AppError::Internal(anyhow!("reassign task scope missing")))?;
            let scoped_assignments = assignments
                .iter()
                .filter(|assignment| assignment.task_id == *affected_task_id)
                .collect::<Vec<_>>();
            let mut next_service_items = scope.service_items.clone();
            for assignment in &scoped_assignments {
                if let Some(service_item) = next_service_items.iter_mut().find(|item| {
                    assignment
                        .service_item_id
                        .map(|id| item.id == Some(id))
                        .unwrap_or_else(|| {
                            item.service_type.as_deref() == Some(assignment.service_type.as_str())
                        })
                }) {
                    service_item.technician_id = Some(assignment.technician_id);
                    service_item.service_status = Some("pending_execute".to_string());
                    service_item.started_at = None;
                    service_item.completed_at = None;
                    service_item.progress_percent = Some(0);
                }
            }

            let remaining_active_execution = executions.iter().any(|execution| {
                execution.task_id == Some(*affected_task_id)
                    && execution.execution_status.as_deref() == Some("executing")
                    && execution
                        .id
                        .map(|id| !active_execution_ids.contains(&id))
                        .unwrap_or(false)
            });
            let next_task_status =
                scoped_reassign_task_status(&next_service_items, remaining_active_execution)
                    .to_string();
            let next_technician_id = scoped_reassign_primary_technician(
                &next_service_items,
                scoped_assignments.last().map(|item| item.technician_id),
                scope.task.current_technician_id,
            );
            let next_execution_id = executions
                .iter()
                .filter(|execution| execution.task_id == Some(*affected_task_id))
                .filter(|execution| execution.execution_status.as_deref() == Some("executing"))
                .filter(|execution| {
                    execution
                        .id
                        .map(|id| !active_execution_ids.contains(&id))
                        .unwrap_or(false)
                })
                .filter_map(|execution| execution.id)
                .max();
            let latest_assignment_id = latest_assignment_ids
                .get(affected_task_id)
                .copied()
                .ok_or_else(|| AppError::Internal(anyhow!("latest assignment id missing")))?;
            let task_update_result = TaskEntity::update_by_map(
                &tx,
                &TaskEntity {
                    current_technician_id: next_technician_id,
                    task_status: Some(next_task_status.clone()),
                    latest_assignment_id: Some(latest_assignment_id),
                    latest_execution_id: next_execution_id,
                    ..Default::default()
                },
                value! {
                    "id": *affected_task_id,
                    "task_status": scope.task.task_status.clone(),
                    "current_technician_id": scope.task.current_technician_id,
                    "latest_assignment_id": scope.task.latest_assignment_id,
                    "latest_execution_id": scope.task.latest_execution_id,
                    "updated_at": scope.task.updated_at,
                    "column": [
                        "current_technician_id",
                        "task_status",
                        "latest_assignment_id",
                        "latest_execution_id"
                    ]
                },
            )
            .await
            .context("failed to update scoped task after reassign")
            .map_err(AppError::Internal)?;
            if task_update_result.rows_affected == 0 {
                return Err(AppError::Conflict(
                    "任务已被其他操作更新，请刷新后重试".to_string(),
                ));
            }
            updated_task_statuses.insert(*affected_task_id, next_task_status);
        }

        Ok(updated_task_statuses)
    }
    .await;

    let updated_task_statuses = match write_result {
        Ok(statuses) => statuses,
        Err(error) => {
            if let Err(rollback_error) = tx.rollback().await {
                tracing::error!(%rollback_error, "failed to roll back scoped task reassign transaction");
            }
            return Err(error);
        }
    };
    tx.commit()
        .await
        .context("failed to commit scoped task reassign transaction")
        .map_err(AppError::Internal)?;

    for assignment in &assignments {
        send_task_assignment_notice(
            state,
            assignment.technician_id,
            assignment.task_id,
            Some(&assignment.service_type),
            "task_reassign",
        )
        .await;
    }
    for affected_task_id in &affected_task_ids {
        let Some(before_task) = task_scopes
            .iter()
            .find(|scope| scope.task.id == Some(*affected_task_id))
            .map(|scope| &scope.task)
        else {
            continue;
        };
        match find_task_by_id(state, *affected_task_id).await {
            Ok(after_task) => {
                if let Err(error) = audit::write_audit_log(
                    state,
                    "task",
                    *affected_task_id,
                    "reassign_task",
                    auth_user.user_id,
                    primary_role(auth_user),
                    Some(before_task),
                    Some(&after_task),
                    Some(assign_reason.to_string()),
                )
                .await
                {
                    tracing::error!(
                        ?error,
                        task_id = *affected_task_id,
                        "failed to write scoped reassign audit log"
                    );
                }
            }
            Err(error) => {
                tracing::error!(
                    ?error,
                    task_id = *affected_task_id,
                    "failed to reload scoped task after reassign"
                );
            }
        }
    }

    let response_status = updated_task_statuses
        .get(&root_task_id)
        .cloned()
        .or_else(|| root_task.task_status.clone())
        .unwrap_or_else(|| "pending_execute".to_string());
    let (response_service_item_id, response_service_type) = if assignments.len() == 1 {
        let assignment = &assignments[0];
        (
            assignment.service_item_id,
            Some(assignment.service_type.clone()),
        )
    } else {
        (None, None)
    };
    Ok(TaskActionResponse {
        task_id: root_task_id,
        task_status: response_status,
        service_item_id: response_service_item_id,
        service_type: response_service_type,
        execution_id: None,
    })
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
        .filter(|item| !is_task_review_log_action(item.action.as_deref().unwrap_or_default()))
        .map(|item| TaskLogItem {
            id: item.id.unwrap_or_default(),
            action: task_log_action_text(
                item.action.as_deref().unwrap_or_default(),
                item.remark.as_deref(),
            ),
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

fn is_task_review_log_action(action: &str) -> bool {
    matches!(
        action,
        "dealer_review"
            | "customer_review"
            | "submit_customer_review"
            | "customer_review_submit"
            | "review_customer"
            | "review_dealer"
    )
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
    if task.deleted_at.is_some() {
        return false;
    }
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
        && !matches_task_status_query(task.task_status.as_deref(), task_status)
    {
        return false;
    }
    if let Some(review_status) = &query.review_status
        && !super::multi_query_value_matches(Some(review_status), |expected| {
            task.review_status.as_deref() == Some(expected)
        })
    {
        return false;
    }
    if !matches_score_status_query(task.score_status.as_deref(), query.score_status.as_deref()) {
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

fn normalized_task_type_filter(value: Option<&str>) -> Result<Option<&'static str>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };

    match value.trim().to_ascii_lowercase().as_str() {
        "三专" | "3" | "three_special" | "three-special" => Ok(Some("three_special")),
        "四专" | "4" | "four_special" | "four-special" => Ok(Some("four_special")),
        _ => Err(AppError::BadRequest(
            "task_type只能为三专/四专或three_special/four_special".to_string(),
        )),
    }
}

fn build_task_group_service_counts(tasks: &[TaskEntity]) -> HashMap<String, usize> {
    let mut counts = HashMap::<String, usize>::new();
    for task in tasks {
        if task.deleted_at.is_some() || is_pending_execute_status(task.task_status.as_deref()) {
            continue;
        }
        let Some(group_no) = task
            .task_group_no
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
        else {
            continue;
        };
        *counts.entry(group_no.to_string()).or_insert(0) += 1;
    }
    counts
}

fn normalized_task_type_filters(value: Option<&str>) -> Result<Vec<&'static str>, AppError> {
    let mut filters = Vec::new();
    for value in super::parse_multi_query_values(value) {
        if super::is_all_query_value(&value) {
            return Ok(Vec::new());
        }
        if let Some(normalized) = normalized_task_type_filter(Some(&value))?
            && !filters.contains(&normalized)
        {
            filters.push(normalized);
        }
    }
    Ok(filters)
}

fn matches_task_type_filters(
    task: &TaskEntity,
    task_type_filters: &[&str],
    group_service_counts: &HashMap<String, usize>,
) -> bool {
    task_type_filters.is_empty()
        || task_type_filters.iter().any(|task_type| {
            matches_optional_task_type_filter(task, Some(task_type), group_service_counts)
        })
}

fn matches_optional_task_type_filter(
    task: &TaskEntity,
    task_type_filter: Option<&str>,
    group_service_counts: &HashMap<String, usize>,
) -> bool {
    let Some(task_type_filter) = task_type_filter else {
        return true;
    };
    matches_task_type_filter(task, task_type_filter, group_service_counts)
}

fn matches_task_type_filter(
    task: &TaskEntity,
    task_type: &str,
    group_service_counts: &HashMap<String, usize>,
) -> bool {
    let grouped_count = task
        .task_group_no
        .as_deref()
        .and_then(|group_no| group_service_counts.get(group_no))
        .copied();
    match task_type {
        "three_special" => grouped_count == Some(3),
        "four_special" => grouped_count == Some(4),
        _ => task.task_type.as_deref() == Some(task_type),
    }
}

fn filter_task_item_for_scoped_view(task: &TaskEntity, query: &TaskListQuery) -> bool {
    if task.deleted_at.is_some() {
        return false;
    }
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
        && !matches_task_status_query(Some(item.task_status.as_str()), task_status)
    {
        return false;
    }
    if let Some(review_status) = &query.review_status
        && !super::multi_query_value_matches(Some(review_status), |expected| {
            item.review_status == expected
        })
    {
        return false;
    }
    if !matches_score_status_query(Some(&item.score_status), query.score_status.as_deref()) {
        return false;
    }
    if let Some(technician_id) = query.technician_id
        && item.current_technician_id != Some(technician_id)
    {
        return false;
    }
    if let Some(service_type) = &query.service_type
        && !super::multi_query_value_matches(Some(service_type), |expected| {
            item.service_type == expected
                || (item.service_item_id.is_none()
                    && item
                        .service_items
                        .iter()
                        .any(|task_item| task_item.service_type == expected))
        })
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

fn canonical_score_status_filter(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() || matches!(value, "全部" | "不限") || value.eq_ignore_ascii_case("all")
    {
        return None;
    }

    let normalized = match value.to_ascii_lowercase().as_str() {
        "pending" => "pending",
        "scored" => "scored",
        _ => match value {
            "待核验" => "pending",
            "已核验" => "scored",
            _ => value,
        },
    };
    Some(normalized.to_string())
}

fn matches_score_status_query(actual: Option<&str>, expected: Option<&str>) -> bool {
    super::multi_query_value_matches(expected, |value| {
        canonical_score_status_filter(Some(value))
            .map(|expected| actual == Some(expected.as_str()))
            .unwrap_or(true)
    })
}

fn prefetched_dealer_review_exists(
    reviews: &[DealerReviewEntity],
    service_items: &[TaskServiceItemEntity],
) -> bool {
    if service_items.len() == 1 {
        let service_item = &service_items[0];
        if let Some(service_item_id) = service_item.id
            && reviews
                .iter()
                .any(|review| review.service_item_id == Some(service_item_id))
        {
            return true;
        }
        if let Some(service_type) = service_item.service_type.as_deref() {
            return reviews
                .iter()
                .any(|review| review.service_type.as_deref() == Some(service_type));
        }
    }
    reviews.len() == 1
}

fn build_task_summary_from_prefetched(
    auth_user: &auth::CurrentAuthUser,
    task: TaskEntity,
    service_item_scope: Option<TaskServiceItemEntity>,
    service_items: &[TaskServiceItemEntity],
    customer_by_id: &HashMap<i64, CustomerEntity>,
    address_by_id: &HashMap<i64, CustomerAddressEntity>,
    store_by_id: &HashMap<i64, StoreEntity>,
    user_by_id: &HashMap<i64, UserEntity>,
    dealer_display_names: &HashMap<i64, String>,
    technician_display_names: &HashMap<i64, String>,
    evidences_by_task_id: &HashMap<i64, Vec<EvidenceEntity>>,
    executions_by_task_id: &HashMap<i64, Vec<TaskExecutionEntity>>,
    dealer_reviews_by_task_id: &HashMap<i64, Vec<DealerReviewEntity>>,
) -> Result<TaskSummary, AppError> {
    let task_id = task.id.unwrap_or_default();
    let customer = customer_by_id
        .get(
            &task
                .customer_id
                .ok_or_else(|| AppError::Internal(anyhow!("customer_id missing")))?,
        )
        .cloned()
        .ok_or_else(|| AppError::NotFound("客户不存在".to_string()))?;
    let address = address_by_id
        .get(
            &task
                .customer_address_id
                .ok_or_else(|| AppError::Internal(anyhow!("address_id missing")))?,
        )
        .cloned()
        .ok_or_else(|| AppError::NotFound("客户地址不存在".to_string()))?;
    let store = store_by_id
        .get(
            &task
                .store_id
                .ok_or_else(|| AppError::Internal(anyhow!("store_id missing")))?,
        )
        .cloned()
        .ok_or_else(|| AppError::NotFound("门店不存在".to_string()))?;
    let customer_name = customer.name.unwrap_or_default();
    let customer_mobile = customer.mobile.unwrap_or_default();
    let task_no = task.task_no.clone().unwrap_or_default();
    let mut raw_task_status = service_item_scope
        .as_ref()
        .and_then(|item| item.service_status.clone())
        .or_else(|| task.task_status.clone())
        .unwrap_or_default();
    let mut raw_service_status = service_item_scope
        .as_ref()
        .and_then(|item| item.service_status.clone());
    if raw_task_status == "pending_dealer_review" {
        let review_scope = service_item_scope
            .as_ref()
            .map(std::slice::from_ref)
            .unwrap_or(service_items);
        let review_exists = dealer_reviews_by_task_id
            .get(&task_id)
            .map(|reviews| prefetched_dealer_review_exists(reviews, review_scope))
            .unwrap_or(false);
        if review_exists {
            raw_task_status = "completed".to_string();
            raw_service_status = Some("completed".to_string());
        }
    }
    let task_status = normalize_task_status_for_client(&raw_task_status).to_string();
    let service_status = raw_service_status
        .as_deref()
        .map(normalize_task_status_for_client);
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
        .unwrap_or_else(|| service_items.to_vec());
    let progress_percent =
        task_progress_percent(&visible_service_items, Some(task_status.as_str()));
    let is_overdue = task_is_overdue(&task);
    let overdue_text_value = overdue_text(is_overdue).to_string();
    let dealer = task
        .dealer_user_id
        .and_then(|user_id| user_by_id.get(&user_id).cloned());
    let current_technician_id = service_item_scope
        .as_ref()
        .and_then(|item| item.technician_id)
        .or(task.current_technician_id);
    let technician = current_technician_id.and_then(|user_id| user_by_id.get(&user_id).cloned());
    let primary_service_type = visible_service_items
        .first()
        .and_then(|item| item.service_type.clone())
        .or(task.service_type.clone())
        .unwrap_or_default();
    let service_items_payload = map_task_service_item_payloads(visible_service_items.clone());
    let task_tags = build_task_tags(&task, &service_items_payload, technician_only);
    let service_status_value = service_status.map(ToString::to_string);
    let service_sort_no = service_item_scope
        .as_ref()
        .and_then(|item| item.sort_no)
        .or_else(|| service_items.first().and_then(|item| item.sort_no))
        .unwrap_or_default();
    let customer_address = format_customer_address(&address);
    let technician_location_address =
        crate::routes::technician_location::latest_technician_location_address(
            evidences_by_task_id
                .get(&task_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            executions_by_task_id
                .get(&task_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            &visible_service_items,
            &customer_address,
        );

    Ok(TaskSummary {
        id: task_id,
        service_item_id: service_item_scope.as_ref().and_then(|item| item.id),
        task_no,
        service_type: primary_service_type,
        service_status: service_status_value,
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
        address: customer_address,
        technician_location_address,
        store_id: task.store_id.unwrap_or_default(),
        store_name: store.store_name.unwrap_or_default(),
        dealer_user_id: task.dealer_user_id.unwrap_or_default(),
        dealer_name: dealer
            .as_ref()
            .and_then(|item| item.id)
            .and_then(|user_id| dealer_display_names.get(&user_id).cloned())
            .or_else(|| dealer.as_ref().and_then(|item| item.name.clone()))
            .unwrap_or_default(),
        current_technician_id,
        technician_name: technician
            .as_ref()
            .and_then(|item| item.id)
            .and_then(|user_id| technician_display_names.get(&user_id).cloned())
            .or_else(|| technician.as_ref().and_then(|item| item.name.clone())),
        technician_mobile: technician.as_ref().and_then(|item| item.mobile.clone()),
        progress_percent,
        is_overdue,
        overdue_text: overdue_text_value,
        task_tags,
        task_group_no: task.task_group_no.clone().unwrap_or_default(),
        service_sort_no,
        created_at_sort: task.created_at.map(format_datetime).unwrap_or_default(),
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
        if customer.deleted_at.is_some() {
            return Err(AppError::Conflict(
                "该手机号客户已删除，请联系管理员处理".to_string(),
            ));
        }
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
        deleted_at: None,
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
    if technician.deleted_at.is_some() || technician.status.as_deref() == Some("disabled") {
        return Err(AppError::Conflict(
            "所选技师已删除或禁用，无法分派".to_string(),
        ));
    }
    if technician_role_status(state, technician_id).await? != "enabled" {
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
    task_scopes: &[AssignableTaskScope],
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
                task_scopes,
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
        task_scopes,
        payload.service_item_id,
        payload.service_type.as_deref(),
        technician_id,
    )?);
    Ok(assignments)
}

fn resolve_assignment_target(
    task_scopes: &[AssignableTaskScope],
    service_item_id: Option<i64>,
    service_type: Option<&str>,
    technician_id: i64,
) -> Result<ResolvedAssignment, AppError> {
    if let Some(service_item_id) = service_item_id {
        let scope = task_scopes
            .iter()
            .find(|scope| {
                scope
                    .service_items
                    .iter()
                    .any(|item| item.id == Some(service_item_id))
            })
            .ok_or_else(|| AppError::BadRequest("服务环节不属于当前任务".to_string()))?;
        let item = scope
            .service_items
            .iter()
            .find(|item| item.id == Some(service_item_id))
            .ok_or_else(|| AppError::BadRequest("服务环节不属于当前任务".to_string()))?;
        let resolved_service_type = item
            .service_type
            .clone()
            .ok_or_else(|| AppError::Internal(anyhow!("task_service_item.service_type missing")))?;
        if let Some(requested_service_type) = service_type
            .map(str::trim)
            .filter(|service_type| !service_type.is_empty())
        {
            if requested_service_type != resolved_service_type {
                return Err(AppError::BadRequest(
                    "service_type与service_item_id不匹配，请刷新后重试".to_string(),
                ));
            }
        }
        return Ok(ResolvedAssignment {
            task_id: scope.task.id.unwrap_or_default(),
            service_item_id: Some(service_item_id),
            service_type: resolved_service_type,
            technician_id,
            from_technician_id: item.technician_id,
            is_legacy_scope: false,
        });
    }

    let service_type = service_type
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .ok_or_else(|| AppError::BadRequest("请选择服务环节".to_string()))?;

    let scope = task_scopes
        .iter()
        .find(|scope| {
            if scope.service_items.is_empty() {
                scope.task.service_type.as_deref() == Some(service_type)
            } else {
                scope
                    .service_items
                    .iter()
                    .any(|item| item.service_type.as_deref() == Some(service_type))
            }
        })
        .ok_or_else(|| AppError::BadRequest("服务环节不属于当前任务".to_string()))?;
    let item = scope
        .service_items
        .iter()
        .find(|item| item.service_type.as_deref() == Some(service_type));
    Ok(ResolvedAssignment {
        task_id: scope.task.id.unwrap_or_default(),
        service_item_id: item.and_then(|item| item.id),
        service_type: service_type.to_string(),
        technician_id,
        from_technician_id: item
            .and_then(|item| item.technician_id)
            .or(scope.task.current_technician_id),
        is_legacy_scope: scope.service_items.is_empty(),
    })
}

fn ensure_unique_assignment_targets(assignments: &[ResolvedAssignment]) -> Result<(), AppError> {
    let mut seen = Vec::new();
    for assignment in assignments {
        let key = assignment.task_id.to_string()
            + ":"
            + &assignment
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

fn sort_reassign_assignments(assignments: &mut [ResolvedAssignment]) {
    assignments.sort_by(|left, right| {
        left.task_id.cmp(&right.task_id).then_with(|| {
            match (left.service_item_id, right.service_item_id) {
                (Some(left_id), Some(right_id)) => left_id.cmp(&right_id),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => left.service_type.cmp(&right.service_type),
            }
        })
    });
}

fn assignment_task_scope<'a>(
    task_scopes: &'a [AssignableTaskScope],
    assignment: &ResolvedAssignment,
) -> Result<&'a AssignableTaskScope, AppError> {
    task_scopes
        .iter()
        .find(|scope| scope.task.id == Some(assignment.task_id))
        .ok_or_else(|| AppError::BadRequest("服务环节不属于当前任务".to_string()))
}

fn assignment_service_item<'a>(
    scope: &'a AssignableTaskScope,
    assignment: &ResolvedAssignment,
) -> Option<&'a TaskServiceItemEntity> {
    if let Some(service_item_id) = assignment.service_item_id {
        return scope
            .service_items
            .iter()
            .find(|item| item.id == Some(service_item_id));
    }
    scope
        .service_items
        .iter()
        .find(|item| item.service_type.as_deref() == Some(assignment.service_type.as_str()))
}

fn validate_scoped_reassign_target(
    scope: &AssignableTaskScope,
    assignment: &ResolvedAssignment,
) -> Result<(), AppError> {
    if !matches!(
        scope.task.task_status.as_deref(),
        Some("pending_execute") | Some("executing")
    ) {
        return Err(AppError::Conflict(
            "选中环节所属任务的当前状态不允许重派".to_string(),
        ));
    }
    if let Some(service_item) = assignment_service_item(scope, assignment) {
        if !matches!(
            service_item.service_status.as_deref(),
            Some("pending_execute") | Some("executing")
        ) {
            return Err(AppError::Conflict(
                "选中服务环节的当前状态不允许重派".to_string(),
            ));
        }
    } else if !scope.service_items.is_empty() {
        return Err(AppError::BadRequest("服务环节不属于当前任务".to_string()));
    }
    Ok(())
}

fn validate_scoped_reassign_identifiers(payload: &AssignTaskRequest) -> Result<(), AppError> {
    if let Some(assignments) = payload.assignments.as_ref() {
        if payload.technician_id.is_some()
            || payload.service_item_id.is_some()
            || payload.service_type.is_some()
        {
            return Err(AppError::BadRequest(
                "批量重派时请省略顶层technician_id、service_item_id和service_type".to_string(),
            ));
        }
        if assignments
            .iter()
            .any(|assignment| assignment.service_item_id.is_none())
        {
            return Err(AppError::BadRequest(
                "按服务环节重派时 assignments[].service_item_id 不能为空".to_string(),
            ));
        }
    } else if payload.service_item_id.is_none() {
        return Err(AppError::BadRequest(
            "按服务环节重派时 service_item_id 不能为空".to_string(),
        ));
    }
    Ok(())
}

fn build_whole_reassign_type_plan(
    task: &TaskEntity,
    service_items: &[TaskServiceItemEntity],
    requested_service_type: Option<&str>,
) -> Result<WholeReassignTypePlan, AppError> {
    let requested_service_type = normalize_whole_reassign_service_type(requested_service_type)?;
    if requested_service_type.is_some() && service_items.len() != 1 {
        return Err(AppError::Conflict(
            "修改任务类型要求当前任务且仅有一个服务环节".to_string(),
        ));
    }

    let service_item = (service_items.len() == 1).then(|| &service_items[0]);
    let original_service_item_type = service_item.and_then(|item| item.service_type.clone());
    let current_service_type = original_service_item_type
        .clone()
        .or_else(|| task.service_type.clone());
    let type_update_requested = requested_service_type.is_some();
    let target_service_type = requested_service_type.or(current_service_type);

    Ok(WholeReassignTypePlan {
        type_update_requested,
        task_service_type_changed: type_update_requested
            && task.service_type != target_service_type,
        service_item_type_changed: type_update_requested
            && service_item.is_some()
            && original_service_item_type != target_service_type,
        target_service_type,
        service_item_id: service_item.and_then(|item| item.id),
        original_service_item_type,
    })
}

fn normalize_whole_reassign_service_type(
    requested_service_type: Option<&str>,
) -> Result<Option<String>, AppError> {
    let Some(requested_service_type) = requested_service_type else {
        return Ok(None);
    };
    let requested_service_type = requested_service_type.trim();
    if requested_service_type.is_empty() {
        return Err(AppError::BadRequest(
            "service_type不能为空；不修改任务类型时请省略该字段".to_string(),
        ));
    }
    if !matches!(
        requested_service_type,
        "measure" | "install" | "ironing" | "cleaning"
    ) {
        return Err(AppError::BadRequest(
            "service_type仅支持measure/install/ironing/cleaning".to_string(),
        ));
    }
    Ok(Some(requested_service_type.to_string()))
}

fn execution_matches_assignment(
    execution: &TaskExecutionEntity,
    assignment: &ResolvedAssignment,
) -> bool {
    if execution.task_id != Some(assignment.task_id) {
        return false;
    }
    if let Some(service_item_id) = assignment.service_item_id {
        return execution.service_item_id == Some(service_item_id)
            || (execution.service_item_id.is_none()
                && execution.service_type.as_deref() == Some(assignment.service_type.as_str()));
    }
    execution.service_type.as_deref() == Some(assignment.service_type.as_str())
        || (assignment.is_legacy_scope
            && execution.service_item_id.is_none()
            && execution.service_type.is_none())
}

fn scoped_reassign_task_status(
    service_items: &[TaskServiceItemEntity],
    has_remaining_active_execution: bool,
) -> &'static str {
    if has_remaining_active_execution
        || service_items
            .iter()
            .any(|item| item.service_status.as_deref() == Some("executing"))
    {
        "executing"
    } else {
        "pending_execute"
    }
}

fn service_item_would_change_on_reassign(
    service_item: &TaskServiceItemEntity,
    technician_id: i64,
) -> bool {
    service_item.technician_id != Some(technician_id)
        || service_item.service_status.as_deref() != Some("pending_execute")
        || service_item.started_at.is_some()
        || service_item.completed_at.is_some()
        || service_item.progress_percent != Some(0)
}

fn scoped_reassign_primary_technician(
    service_items: &[TaskServiceItemEntity],
    last_assigned_technician_id: Option<i64>,
    current_technician_id: Option<i64>,
) -> Option<i64> {
    service_items
        .first()
        .and_then(|item| item.technician_id)
        .or(last_assigned_technician_id)
        .or(current_technician_id)
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

async fn load_grouped_assignable_tasks(
    state: &AppState,
    task: &TaskEntity,
) -> Result<Vec<AssignableTaskScope>, AppError> {
    let tasks = if let Some(task_group_no) = task.task_group_no.as_deref() {
        let grouped: Vec<TaskEntity> = db::select_by_map_compat::<TaskEntity>(
            state.rb.as_ref(),
            "task",
            value! { "task_group_no": task_group_no },
        )
        .await
        .context("failed to query grouped tasks")
        .map_err(AppError::Internal)?
        .into_iter()
        .filter(|item| item.deleted_at.is_none())
        .collect();
        if grouped.is_empty() {
            vec![task.clone()]
        } else {
            grouped
        }
    } else {
        vec![task.clone()]
    };

    let mut scopes = Vec::with_capacity(tasks.len());
    for item in tasks {
        let task_id = item
            .id
            .ok_or_else(|| AppError::Internal(anyhow!("task.id missing")))?;
        let service_items = load_task_service_item_entities(state, task_id).await?;
        scopes.push(AssignableTaskScope {
            task: item,
            service_items,
        });
    }
    Ok(scopes)
}

async fn ensure_reassign_service_type_available_in_group(
    state: &AppState,
    task: &TaskEntity,
    service_type: &str,
) -> Result<(), AppError> {
    let Some(task_group_no) = task
        .task_group_no
        .as_deref()
        .map(str::trim)
        .filter(|task_group_no| !task_group_no.is_empty())
    else {
        return Ok(());
    };
    let grouped_tasks: Vec<TaskEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task",
        value! { "task_group_no": task_group_no },
    )
    .await
    .context("failed to query grouped tasks before service type change")
    .map_err(AppError::Internal)?;
    if task_group_has_service_type_conflict(task.id, service_type, &grouped_tasks) {
        return Err(AppError::Conflict(
            "同一批任务中已存在该服务类型，不能重复修改".to_string(),
        ));
    }
    Ok(())
}

fn task_group_has_service_type_conflict(
    current_task_id: Option<i64>,
    service_type: &str,
    grouped_tasks: &[TaskEntity],
) -> bool {
    grouped_tasks.iter().any(|grouped_task| {
        grouped_task.deleted_at.is_none()
            && grouped_task.id != current_task_id
            && grouped_task.service_type.as_deref() == Some(service_type)
    })
}

async fn technician_role_status(state: &AppState, user_id: i64) -> Result<String, AppError> {
    let role_id = find_role_id_by_code(state, "technician").await?;
    let relation = db::select_one_by_map_compat::<UserRoleEntity>(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": user_id, "role_id": role_id },
    )
    .await
    .context("failed to query technician role status")
    .map_err(AppError::Internal)?;
    Ok(
        match relation
            .and_then(|item| item.status)
            .unwrap_or_else(|| "enabled".to_string())
            .as_str()
        {
            "disabled" | "inactive" => "disabled".to_string(),
            _ => "enabled".to_string(),
        },
    )
}

async fn find_role_id_by_code(state: &AppState, role_code: &str) -> Result<i64, AppError> {
    db::select_one_by_map_compat::<crate::models::role::RoleEntity>(
        state.rb.as_ref(),
        "role",
        value! { "role_code": role_code },
    )
    .await
    .context("failed to query role by code")
    .map_err(AppError::Internal)?
    .and_then(|item| item.id)
    .ok_or_else(|| AppError::Internal(anyhow!("role not found: {role_code}")))
}

async fn soft_delete_task(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    task: &TaskEntity,
    action: &str,
    remark: &str,
) -> Result<(), AppError> {
    let task_id = task
        .id
        .ok_or_else(|| AppError::Internal(anyhow!("task.id missing")))?;
    TaskEntity::update_by_map(
        state.rb.as_ref(),
        &TaskEntity {
            deleted_at: Some(local_now()),
            ..Default::default()
        },
        value! { "id": task_id, "column": ["deleted_at"] },
    )
    .await
    .context("failed to soft delete task")
    .map_err(AppError::Internal)?;

    let after_task = find_task_record(state, task_id).await?;
    audit::write_audit_log(
        state,
        "task",
        task_id,
        action,
        auth_user.user_id,
        auth_user.primary_role(),
        Some(task),
        after_task.as_ref(),
        Some(remark.to_string()),
    )
    .await
}

async fn find_task_by_id(state: &AppState, task_id: i64) -> Result<TaskEntity, AppError> {
    let task = find_task_record(state, task_id)
        .await?
        .ok_or_else(|| AppError::NotFound("任务不存在或已删除".to_string()))?;
    if task.deleted_at.is_some() {
        return Err(AppError::NotFound("任务不存在或已删除".to_string()));
    }
    Ok(task)
}

async fn find_task_record(state: &AppState, task_id: i64) -> Result<Option<TaskEntity>, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "task", value! { "id": task_id })
        .await
        .context("failed to query task by id")
        .map_err(AppError::Internal)
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

async fn ensure_customer_review_link_for_task(
    state: &AppState,
    task: &TaskEntity,
) -> Result<String, AppError> {
    let task_id = task
        .id
        .ok_or_else(|| AppError::Internal(anyhow!("task.id missing for customer review link")))?;
    let existing = db::select_one_by_map_compat::<
        crate::models::customer_review_link::CustomerReviewLinkEntity,
    >(
        state.rb.as_ref(),
        "customer_review_link",
        value! { "task_id": task_id, "status": "pending" },
    )
    .await
    .context("failed to query pending customer review link")
    .map_err(AppError::Internal)?;
    if let Some(link) = existing
        && let Some(token) = link.token.filter(|item| !item.trim().is_empty())
    {
        return Ok(token);
    }

    let token = generate_customer_review_token();
    let expired_at = chrono::Utc::now() + chrono::Duration::days(3);
    crate::models::customer_review_link::CustomerReviewLinkEntity::insert(
        state.rb.as_ref(),
        &crate::models::customer_review_link::CustomerReviewLinkEntity {
            id: None,
            task_id: task.id,
            customer_id: task.customer_id,
            token: Some(token.clone()),
            expired_at: Some(expired_at.naive_utc()),
            status: Some("pending".to_string()),
            sent_at: None,
            created_at: None,
        },
    )
    .await
    .context("failed to create customer review link during task submit")
    .map_err(AppError::Internal)?;
    Ok(token)
}

async fn mark_customer_review_link_sent(
    state: &AppState,
    task_id: i64,
    token: &str,
) -> Result<(), AppError> {
    crate::models::customer_review_link::CustomerReviewLinkEntity::update_by_map(
        state.rb.as_ref(),
        &crate::models::customer_review_link::CustomerReviewLinkEntity {
            sent_at: Some(chrono::Utc::now().naive_utc()),
            ..Default::default()
        },
        value! { "task_id": task_id, "token": token, "status": "pending", "column": ["sent_at"] },
    )
    .await
    .context("failed to mark customer review link sent")
    .map(|_| ())
    .map_err(AppError::Internal)
}

fn is_valid_sms_mobile(mobile: &str) -> bool {
    let mobile = mobile.trim();
    let bytes = mobile.as_bytes();
    mobile.len() == 11
        && bytes[0] == b'1'
        && (b'3'..=b'9').contains(&bytes[1])
        && bytes.iter().all(|item| item.is_ascii_digit())
}

fn generate_customer_review_token() -> String {
    let seed = uuid::Uuid::new_v4().as_u128() % 100_000_000;
    format!("{:08}", seed)
}

async fn find_customer_review_for_task(
    state: &AppState,
    task_id: i64,
) -> Result<Option<CustomerReviewPayload>, AppError> {
    let review = db::select_one_by_map_compat::<CustomerReviewEntity>(
        state.rb.as_ref(),
        "customer_review",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query customer review for task detail")
    .map_err(AppError::Internal)?;
    let Some(review) = review else {
        return Ok(None);
    };

    let mut images = Vec::new();
    if let Some(review_id) = review.id {
        images = db::select_by_map_compat::<CustomerReviewImageEntity>(
            state.rb.as_ref(),
            "customer_review_image",
            value! { "review_id": review_id },
        )
        .await
        .context("failed to query customer review images for task detail")
        .map_err(AppError::Internal)?
        .into_iter()
        .map(|item| CustomerReviewImageItemPayload {
            id: item.id.unwrap_or_default(),
            file_url: storage::public_url(&item.file_url.unwrap_or_default()),
        })
        .collect();
    }

    let star = review.star.unwrap_or_default();
    Ok(Some(CustomerReviewPayload {
        star,
        score: star,
        score_text: review_score_text(star),
        tags: review
            .tag_json
            .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
            .unwrap_or_default(),
        content: review.content.unwrap_or_default(),
        images,
        created_at: review.created_at.map(format_datetime).unwrap_or_default(),
    }))
}

async fn find_hq_score_for_task(
    state: &AppState,
    task_id: i64,
) -> Result<Option<TaskHqScorePayload>, AppError> {
    let score = db::select_one_by_map_compat::<HqScoreEntity>(
        state.rb.as_ref(),
        "hq_score",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query hq score for task detail")
    .map_err(AppError::Internal)?;
    let Some(score) = score else {
        return Ok(None);
    };

    let items = db::select_by_map_compat::<HqScoreItemEntity>(
        state.rb.as_ref(),
        "hq_score_item",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query hq score items for task detail")
    .map_err(AppError::Internal)?
    .into_iter()
    .map(|item| TaskHqScoreItemPayload {
        service_type: item.service_type.unwrap_or_default(),
        score: item.score.unwrap_or_default(),
        score_text: numeric_score_text(item.score.unwrap_or_default()),
        deduction_reason: item.deduction_reason.unwrap_or_default(),
    })
    .collect();

    let total_score = score.total_score.unwrap_or_default();
    Ok(Some(TaskHqScorePayload {
        total_score,
        total_score_text: numeric_score_text(total_score),
        score_remark: score.score_remark.unwrap_or_default(),
        items,
        created_at: score.created_at.map(format_datetime).unwrap_or_default(),
    }))
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

fn normalize_required_reassign_edit(
    value: Option<&str>,
    empty_message: &str,
) -> Result<Option<String>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest(empty_message.to_string()));
    }
    Ok(Some(value.to_string()))
}

fn resolve_reassign_scheduled_time(
    appointment_time_update: Option<NaiveDateTime>,
    current_scheduled_time: Option<NaiveDateTime>,
) -> Option<NaiveDateTime> {
    appointment_time_update.or(current_scheduled_time)
}

fn service_status_blocks_reassign(status: Option<&str>) -> bool {
    matches!(
        normalize_optional_task_status_for_client(status),
        Some("pending_dealer_review") | Some("completed") | Some("closed")
    )
}

fn address_request_matches_entity(
    request: &AddressRequest,
    entity: &CustomerAddressEntity,
) -> bool {
    let detail = request.detail.trim();
    if request.province.is_none() && request.city.is_none() && request.district.is_none() {
        return detail == entity.address.as_deref().unwrap_or_default().trim()
            || detail == format_customer_address(entity);
    }

    optional_text_matches(request.province.as_deref(), entity.province.as_deref())
        && optional_text_matches(request.city.as_deref(), entity.city.as_deref())
        && optional_text_matches(request.district.as_deref(), entity.district.as_deref())
        && detail == entity.address.as_deref().unwrap_or_default().trim()
}

fn optional_text_matches(left: Option<&str>, right: Option<&str>) -> bool {
    left.unwrap_or_default().trim() == right.unwrap_or_default().trim()
}

fn format_datetime(input: NaiveDateTime) -> String {
    input.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn format_customer_address(address: &CustomerAddressEntity) -> String {
    let mut parts = Vec::new();
    for part in [
        address.province.as_deref(),
        address.city.as_deref(),
        address.district.as_deref(),
        address.address.as_deref(),
    ] {
        if let Some(part) = part.map(str::trim).filter(|item| !item.is_empty()) {
            parts.push(part.to_string());
        }
    }
    parts.join("")
}

fn format_task_service_time_slot(
    appointment_time: Option<NaiveDateTime>,
    service_time_slot: Option<&str>,
) -> Option<String> {
    if let Some(service_time_slot) = service_time_slot
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        return Some(service_time_slot.to_string());
    }
    if let Some(appointment_time) = appointment_time {
        return Some(appointment_time.format("%H:%M").to_string());
    }
    None
}

fn generate_task_no() -> String {
    format!("RW{}", Local::now().format("%Y%m%d%H%M%S%6f"))
}

fn generate_task_group_no() -> String {
    format!("TG{}", Local::now().format("%Y%m%d%H%M%S%6f"))
}

fn progress_percent_for_status(status: Option<&str>) -> i32 {
    match normalize_optional_task_status_for_client(status) {
        Some("pending_assign") => 0,
        Some("pending_execute") => 0,
        Some("executing") => 60,
        Some("pending_dealer_review") => 90,
        Some("completed") => 100,
        Some("closed") => 100,
        _ => 0,
    }
}

fn scoped_progress_execution_ids(
    service_item: Option<&TaskServiceItemEntity>,
    executions: &[TaskExecutionEntity],
) -> Option<BTreeSet<i64>> {
    service_item.map(|service_item| {
        executions
            .iter()
            .filter(|execution| execution.execution_status.as_deref() != Some("cancelled"))
            .filter(|execution| {
                if let Some(service_item_id) = service_item.id {
                    execution.service_item_id == Some(service_item_id)
                } else {
                    execution.service_item_id.is_none()
                        && execution.service_type == service_item.service_type
                }
            })
            .filter_map(|execution| execution.id)
            .collect()
    })
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
    let assigned_technician_id =
        scoped_technician_id
            .or(task.current_technician_id)
            .or_else(|| {
                if assigned_technician_ids.len() == 1 {
                    assigned_technician_ids.first().copied()
                } else {
                    latest_assignment.and_then(|item| item.to_technician_id)
                }
            });
    let assigned_technician_name = if let Some(technician_id) = assigned_technician_id {
        let technician = find_user_by_id(state, Some(technician_id)).await.ok();
        if let Some(technician) = technician {
            let name = identity::role_display_name(
                state,
                technician_id,
                "technician",
                technician.name.as_deref(),
            )
            .await?;
            (!name.is_empty()).then_some(name)
        } else {
            None
        }
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
    let scoped_execution_ids = scoped_progress_execution_ids(scoped_service_item, executions);
    let image_evidences = evidences
        .iter()
        .filter(|item| {
            scoped_execution_ids
                .as_ref()
                .map(|ids| {
                    item.execution_id
                        .map(|id| ids.contains(&id))
                        .unwrap_or(false)
                })
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
                service_status: item
                    .service_status
                    .as_deref()
                    .map(normalize_task_status_for_client)
                    .unwrap_or_default()
                    .to_string(),
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

fn task_export_service_type_text(service_type: &str) -> String {
    match service_type {
        "measure" => "量尺服务".to_string(),
        "install" => "安装服务".to_string(),
        "ironing" => "熨烫服务".to_string(),
        "cleaning" => "除尘除螨服务".to_string(),
        "" => String::new(),
        other => other.to_string(),
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
    match normalize_task_status_for_client(status) {
        "pending_assign" => "待分配",
        "pending_execute" => "待执行",
        "executing" => "进行中",
        "pending_dealer_review" => "待评价",
        "completed" => "已完成",
        "closed" => "已关闭",
        _ => "未知状态",
    }
}

fn display_status(status: &str) -> &'static str {
    match normalize_task_status_for_client(status) {
        "pending_assign" | "pending_execute" => "pending_execute",
        "executing" => "executing",
        "pending_dealer_review" => "pending_review",
        "completed" | "closed" => "completed",
        _ => "pending_execute",
    }
}

fn build_task_list_status_summary<'a>(
    statuses: impl IntoIterator<Item = &'a str>,
) -> TaskListStatusSummary {
    let mut summary = TaskListStatusSummary::default();
    for status in statuses {
        match status {
            "executing" => summary.executing += 1,
            "pending_review" => summary.pending_review += 1,
            "completed" => summary.completed += 1,
            _ => summary.pending_execute += 1,
        }
    }
    summary
}

fn display_status_text(status: &str) -> &'static str {
    match normalize_task_status_for_client(status) {
        "pending_assign" | "pending_execute" => "待执行",
        "executing" => "进行中",
        "pending_dealer_review" => "待评价",
        "completed" | "closed" => "已完成",
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
        "pending" => "待核验",
        "scored" => "已核验",
        _ => "未知状态",
    }
}

fn local_now() -> NaiveDateTime {
    Local::now().naive_local()
}

fn scoped_review_status(task_review_status: Option<&str>, service_status: Option<&str>) -> String {
    match normalize_optional_task_status_for_client(service_status) {
        Some("pending_dealer_review") => "pending".to_string(),
        Some("completed") | Some("closed") => "reviewed".to_string(),
        _ => task_review_status.unwrap_or_default().to_string(),
    }
}

fn scoped_score_status(task_score_status: Option<&str>, service_status: Option<&str>) -> String {
    match normalize_optional_task_status_for_client(service_status) {
        Some("closed") => "scored".to_string(),
        _ => task_score_status.unwrap_or_default().to_string(),
    }
}

fn normalize_task_status_for_client(status: &str) -> &str {
    if status == "pending_hq_score" {
        "completed"
    } else {
        status
    }
}

fn normalize_optional_task_status_for_client(status: Option<&str>) -> Option<&str> {
    status.map(normalize_task_status_for_client)
}

fn matches_task_status_filter(actual: Option<&str>, expected: &str) -> bool {
    normalize_optional_task_status_for_client(actual)
        == Some(normalize_task_status_for_client(expected))
}

fn matches_task_status_query(actual: Option<&str>, expected: &str) -> bool {
    super::multi_query_value_matches(Some(expected), |value| {
        matches_task_status_filter(actual, value)
    })
}

fn is_pending_execute_status(status: Option<&str>) -> bool {
    matches!(status, Some("pending_assign") | Some("pending_execute"))
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
            normalize_optional_task_status_for_client(task.task_status.as_deref()),
            Some("completed") | Some("closed")
        )
}

fn service_item_is_overdue(item: &TaskServiceItemEntity) -> bool {
    item.scheduled_time
        .map(|dt| dt < local_now())
        .unwrap_or(false)
        && !matches!(
            normalize_optional_task_status_for_client(item.service_status.as_deref()),
            Some("completed") | Some("closed")
        )
}

fn overdue_text(is_overdue: bool) -> &'static str {
    if is_overdue { "已逾期" } else { "正常" }
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
        tags.push("已核验".to_string());
    }
    tags
}

fn task_status_text_for_viewer(status: &str, technician_only: bool) -> &'static str {
    let status = normalize_task_status_for_client(status);
    if technician_only && status == "pending_hq_score" {
        return "已完成";
    }

    task_status_text(status)
}

fn display_status_for_viewer(status: &str, technician_only: bool) -> &'static str {
    let status = normalize_task_status_for_client(status);
    if technician_only && status == "pending_hq_score" {
        return "completed";
    }

    display_status(status)
}

fn display_status_text_for_viewer(status: &str, technician_only: bool) -> &'static str {
    let status = normalize_task_status_for_client(status);
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
    let task_status = normalize_task_status_for_client(task_status);
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

async fn load_task_export_image_buffers_by_task_ids(
    state: &AppState,
    task_ids: &[i64],
) -> Result<HashMap<i64, Vec<Vec<u8>>>, AppError> {
    if task_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut evidences: Vec<EvidenceEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "evidence",
        value! { "task_id": task_ids.to_vec() },
    )
    .await
    .context("failed to query task export evidences")
    .map_err(AppError::Internal)?;
    evidences.retain(|item| item.evidence_type.as_deref() == Some("image"));
    evidences.sort_by(|left, right| {
        left.task_id
            .unwrap_or_default()
            .cmp(&right.task_id.unwrap_or_default())
            .then(
                left.id
                    .unwrap_or_default()
                    .cmp(&right.id.unwrap_or_default()),
            )
    });

    let mut image_buffers_by_task_id = HashMap::<i64, Vec<Vec<u8>>>::new();
    for evidence in evidences {
        let Some(task_id) = evidence.task_id else {
            continue;
        };
        let relative_path = evidence.file_url.unwrap_or_default();
        if relative_path.trim().is_empty() {
            continue;
        }
        let bytes = match storage::read_bytes(state, &relative_path).await {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        if let Some(buffer) = normalize_export_image_buffer(&bytes) {
            image_buffers_by_task_id
                .entry(task_id)
                .or_default()
                .push(buffer);
        }
    }

    Ok(image_buffers_by_task_id)
}

fn normalize_export_image_buffer(bytes: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(bytes).ok()?;
    let (width, height) = image.dimensions();
    let max_edge = 480_u32;
    let resized = if width > max_edge || height > max_edge {
        image.resize(max_edge, max_edge, FilterType::Triangle)
    } else {
        image
    };

    let mut out = Cursor::new(Vec::new());
    let mut encoder = JpegEncoder::new_with_quality(&mut out, 68);
    encoder.encode_image(&resized).ok()?;
    let compressed = out.into_inner();
    if XlsxImage::new_from_buffer(&compressed).is_ok() {
        return Some(compressed);
    }

    let mut fallback = Cursor::new(Vec::new());
    resized.write_to(&mut fallback, ImageFormat::Png).ok()?;
    Some(fallback.into_inner())
}

fn primary_role(auth_user: &auth::CurrentAuthUser) -> &str {
    auth_user.primary_role()
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
    let task_ids: Vec<i64> = tasks
        .into_iter()
        .filter(|item| item.deleted_at.is_none())
        .filter_map(|item| item.id)
        .collect();
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

    Ok(round_score(
        scores
            .iter()
            .map(|item| item.total_score.unwrap_or_default() as f64)
            .sum::<f64>()
            / scores.len() as f64,
    ))
}

fn round_score(score: f64) -> f64 {
    (score * 100.0).round() / 100.0
}

fn format_score(score: f64) -> String {
    format!("{:.2}", round_score(score))
}

fn review_score_text(star: i32) -> String {
    if star > 0 {
        format!("{star}分")
    } else {
        "--".to_string()
    }
}

fn numeric_score_text(score: i32) -> String {
    format!("{score}分")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefetched_task_review_detection_matches_scoped_lookup() {
        let reviews = vec![DealerReviewEntity {
            service_item_id: Some(101),
            service_type: Some("install".to_string()),
            ..Default::default()
        }];
        let matching = TaskServiceItemEntity {
            id: Some(101),
            service_type: Some("install".to_string()),
            ..Default::default()
        };
        let missing = TaskServiceItemEntity {
            id: Some(102),
            service_type: Some("measure".to_string()),
            ..Default::default()
        };

        assert!(prefetched_dealer_review_exists(&reviews, &[matching]));
        assert!(!prefetched_dealer_review_exists(&reviews, &[missing]));
        assert!(prefetched_dealer_review_exists(&reviews, &[]));
    }

    #[test]
    fn batch_delete_request_accepts_legacy_string_ids() {
        let payload: BatchDeleteTasksRequest =
            serde_json::from_value(serde_json::json!({ "ids": ["10", 11] })).unwrap();
        assert_eq!(payload.task_ids, vec![10, 11]);
    }

    #[test]
    fn task_list_filters_soft_deleted_tasks() {
        let query: TaskListQuery = serde_json::from_value(serde_json::json!({})).unwrap();
        let task = TaskEntity {
            deleted_at: Some(local_now()),
            ..Default::default()
        };
        assert!(!filter_task_item(&task, &query));
        assert!(!filter_task_item_for_scoped_view(&task, &query));
    }

    #[test]
    fn task_group_counts_ignore_soft_deleted_tasks() {
        let active = TaskEntity {
            task_group_no: Some("group-1".to_string()),
            task_status: Some("completed".to_string()),
            ..Default::default()
        };
        let deleted = TaskEntity {
            task_group_no: Some("group-1".to_string()),
            task_status: Some("completed".to_string()),
            deleted_at: Some(local_now()),
            ..Default::default()
        };
        let counts = build_task_group_service_counts(&[active, deleted]);
        assert_eq!(counts.get("group-1"), Some(&1));
    }

    #[test]
    fn task_logs_hide_review_actions() {
        assert!(is_task_review_log_action("dealer_review"));
        assert!(is_task_review_log_action("customer_review"));
        assert!(is_task_review_log_action("submit_customer_review"));
        assert!(!is_task_review_log_action("create_task"));
        assert!(!is_task_review_log_action("submit_task"));
    }

    #[test]
    fn score_status_filter_accepts_verification_text() {
        assert_eq!(
            canonical_score_status_filter(Some("待核验")).as_deref(),
            Some("pending")
        );
        assert_eq!(
            canonical_score_status_filter(Some("已核验")).as_deref(),
            Some("scored")
        );
        assert_eq!(canonical_score_status_filter(Some("全部")), None);
    }

    #[test]
    fn task_list_status_and_type_filters_accept_multiple_values() {
        assert!(matches_task_status_query(
            Some("executing"),
            "pending_execute,executing"
        ));
        assert!(!matches_task_status_query(
            Some("closed"),
            "pending_execute,executing"
        ));
        assert!(matches_score_status_query(
            Some("scored"),
            Some(r#"["pending","scored"]"#)
        ));
        assert_eq!(
            normalized_task_type_filters(Some("three_special,four_special")).unwrap(),
            vec!["three_special", "four_special"]
        );
    }

    #[test]
    fn display_status_text_keeps_dealer_review_and_hq_score_separate() {
        assert_eq!(display_status_text("pending_dealer_review"), "待评价");
        assert_eq!(display_status_text("pending_hq_score"), "已完成");
        assert_eq!(display_status_text("completed"), "已完成");
        assert_eq!(
            scoped_score_status(Some("pending"), Some("completed")),
            "pending"
        );
    }

    #[test]
    fn task_list_statistics_cover_all_display_status_groups() {
        let statuses = [
            "pending_assign",
            "pending_execute",
            "executing",
            "pending_dealer_review",
            "completed",
            "closed",
            "pending_hq_score",
        ];
        let summary = build_task_list_status_summary(statuses.into_iter().map(display_status));

        assert_eq!(
            summary,
            TaskListStatusSummary {
                pending_execute: 2,
                executing: 1,
                pending_review: 1,
                completed: 3,
            }
        );
    }

    #[test]
    fn reassign_request_keeps_legacy_payload_compatible() {
        let payload: ReassignTaskRequest = serde_json::from_value(serde_json::json!({
            "technician_id": 29,
            "assign_reason": "重新派发"
        }))
        .unwrap();

        assert_eq!(payload.technician_id, Some(29));
        assert_eq!(payload.assign_reason.as_deref(), Some("重新派发"));
        assert_eq!(payload.resolved_assign_reason(), "重新派发");
        assert!(is_legacy_whole_task_assignment(
            &payload.assignment_request()
        ));
        assert!(payload.appointment_time.is_none());
        assert!(payload.resolved_customer_name().is_none());
        assert!(payload.resolved_address().is_none());
    }

    #[test]
    fn reassign_request_defaults_reason_and_accepts_assign_shapes() {
        let single: ReassignTaskRequest = serde_json::from_value(serde_json::json!({
            "technicianId": 29,
            "serviceItemId": 101
        }))
        .unwrap();
        assert_eq!(single.resolved_assign_reason(), "重新分派");
        assert!(!is_legacy_whole_task_assignment(
            &single.assignment_request()
        ));

        let batch: ReassignTaskRequest = serde_json::from_value(serde_json::json!({
            "assignments": [
                { "service_item_id": 101, "technician_id": 29 },
                { "service_item_id": 102, "service_type": "ironing", "technician_id": 30 }
            ]
        }))
        .unwrap();
        assert_eq!(batch.technician_id, None);
        assert_eq!(batch.assignments.as_ref().map(Vec::len), Some(2));
        assert!(!is_legacy_whole_task_assignment(
            &batch.assignment_request()
        ));
    }

    #[test]
    fn assign_and_reassign_accept_the_same_assignment_fields() {
        let value = serde_json::json!({
            "assignments": [
                {
                    "serviceItemId": 101,
                    "serviceType": "install",
                    "technicianId": 29
                }
            ],
            "assignReason": "任务分派"
        });
        let assigned: AssignTaskRequest = serde_json::from_value(value.clone()).unwrap();
        let reassigned: ReassignTaskRequest = serde_json::from_value(value).unwrap();
        let reassigned = reassigned.assignment_request();

        assert_eq!(assigned.assign_reason, reassigned.assign_reason);
        assert_eq!(assigned.assignments.as_ref().map(Vec::len), Some(1));
        assert_eq!(reassigned.assignments.as_ref().map(Vec::len), Some(1));
        let assigned_item = &assigned.assignments.unwrap()[0];
        let reassigned_item = &reassigned.assignments.unwrap()[0];
        assert_eq!(assigned_item.service_item_id, Some(101));
        assert_eq!(reassigned_item.service_item_id, Some(101));
        assert_eq!(assigned_item.service_type.as_deref(), Some("install"));
        assert_eq!(reassigned_item.service_type.as_deref(), Some("install"));
        assert_eq!(assigned_item.technician_id, 29);
        assert_eq!(reassigned_item.technician_id, 29);
    }

    #[test]
    fn task_detail_query_accepts_assignment_field_aliases() {
        let query: TaskDetailQuery = serde_json::from_value(serde_json::json!({
            "serviceItemId": 101,
            "serviceType": "install"
        }))
        .unwrap();

        assert_eq!(query.service_item_id, Some(101));
        assert_eq!(query.service_type.as_deref(), Some("install"));
    }

    #[test]
    fn reassign_mode_uses_service_type_for_whole_task_and_ids_for_scopes() {
        let whole: ReassignTaskRequest = serde_json::from_value(serde_json::json!({
            "technician_id": 29,
            "service_type": "install"
        }))
        .unwrap();
        assert!(!whole.has_task_detail_edits());
        assert!(!whole.uses_scoped_reassign());

        let scoped: ReassignTaskRequest = serde_json::from_value(serde_json::json!({
            "technician_id": 29,
            "service_item_id": 101,
            "service_type": "install"
        }))
        .unwrap();
        assert!(scoped.uses_scoped_reassign());

        let edited: ReassignTaskRequest = serde_json::from_value(serde_json::json!({
            "technician_id": 29,
            "service_item_id": 101,
            "task_desc": "不应在按环节重派时同时提交"
        }))
        .unwrap();
        assert!(edited.uses_scoped_reassign());
        assert!(edited.has_task_detail_edits());
    }

    #[test]
    fn whole_reassign_type_plan_updates_task_and_single_service_item() {
        let task = TaskEntity {
            service_type: Some("measure".to_string()),
            ..Default::default()
        };
        let service_items = vec![TaskServiceItemEntity {
            id: Some(101),
            service_type: Some("measure".to_string()),
            ..Default::default()
        }];

        let plan =
            build_whole_reassign_type_plan(&task, &service_items, Some(" install ")).unwrap();

        assert!(plan.type_update_requested);
        assert!(plan.service_type_changed());
        assert!(plan.task_service_type_changed);
        assert!(plan.service_item_type_changed);
        assert_eq!(plan.target_service_type.as_deref(), Some("install"));
        assert_eq!(plan.service_item_id, Some(101));
        assert_eq!(plan.original_service_item_type.as_deref(), Some("measure"));

        let same_type_plan =
            build_whole_reassign_type_plan(&task, &service_items, Some("measure")).unwrap();
        assert!(same_type_plan.type_update_requested);
        assert!(!same_type_plan.service_type_changed());
        assert_eq!(
            same_type_plan.target_service_type.as_deref(),
            Some("measure")
        );
    }

    #[test]
    fn whole_reassign_type_plan_rejects_task_without_service_item() {
        let task = TaskEntity {
            service_type: Some("measure".to_string()),
            ..Default::default()
        };

        assert!(matches!(
            build_whole_reassign_type_plan(&task, &[], Some("install")),
            Err(AppError::Conflict(_))
        ));
    }

    #[test]
    fn whole_reassign_type_plan_rejects_invalid_and_multi_item_changes() {
        let task = TaskEntity {
            service_type: Some("measure".to_string()),
            ..Default::default()
        };
        let service_items = vec![
            TaskServiceItemEntity {
                id: Some(101),
                service_type: Some("measure".to_string()),
                ..Default::default()
            },
            TaskServiceItemEntity {
                id: Some(102),
                service_type: Some("install".to_string()),
                ..Default::default()
            },
        ];

        assert!(matches!(
            build_whole_reassign_type_plan(&task, &[], Some("repair")),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            build_whole_reassign_type_plan(&task, &[], Some("  ")),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            build_whole_reassign_type_plan(&task, &service_items, Some("cleaning")),
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            build_whole_reassign_type_plan(&task, &service_items, Some("measure")),
            Err(AppError::Conflict(_))
        ));
    }

    #[test]
    fn scoped_reassign_requires_service_item_ids() {
        let missing_id: ReassignTaskRequest = serde_json::from_value(serde_json::json!({
            "assignments": [
                { "service_type": "install", "technician_id": 29 }
            ]
        }))
        .unwrap();
        assert!(matches!(
            validate_scoped_reassign_identifiers(&missing_id.assignment_request()),
            Err(AppError::BadRequest(_))
        ));

        let with_id: ReassignTaskRequest = serde_json::from_value(serde_json::json!({
            "assignments": [
                { "service_item_id": 101, "service_type": "install", "technician_id": 29 }
            ]
        }))
        .unwrap();
        validate_scoped_reassign_identifiers(&with_id.assignment_request()).unwrap();

        let ignored_top_level: ReassignTaskRequest = serde_json::from_value(serde_json::json!({
            "service_type": "install",
            "assignments": [
                { "service_item_id": 101, "technician_id": 29 }
            ]
        }))
        .unwrap();
        assert!(matches!(
            validate_scoped_reassign_identifiers(&ignored_top_level.assignment_request()),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn task_group_type_change_rejects_duplicate_active_sibling() {
        let current = TaskEntity {
            id: Some(1),
            service_type: Some("measure".to_string()),
            ..Default::default()
        };
        let sibling = TaskEntity {
            id: Some(2),
            service_type: Some("install".to_string()),
            ..Default::default()
        };
        let deleted_sibling = TaskEntity {
            id: Some(3),
            service_type: Some("cleaning".to_string()),
            deleted_at: Some(local_now()),
            ..Default::default()
        };

        assert!(task_group_has_service_type_conflict(
            current.id,
            "install",
            &[current.clone(), sibling, deleted_sibling.clone()]
        ));
        assert!(!task_group_has_service_type_conflict(
            current.id,
            "cleaning",
            &[current, deleted_sibling]
        ));
    }

    #[test]
    fn scoped_reassign_reuses_batch_assignment_resolution() {
        let scopes = vec![
            AssignableTaskScope {
                task: TaskEntity {
                    id: Some(1),
                    current_technician_id: Some(10),
                    task_status: Some("pending_execute".to_string()),
                    ..Default::default()
                },
                service_items: vec![TaskServiceItemEntity {
                    id: Some(101),
                    task_id: Some(1),
                    service_type: Some("install".to_string()),
                    service_status: Some("pending_execute".to_string()),
                    technician_id: Some(10),
                    ..Default::default()
                }],
            },
            AssignableTaskScope {
                task: TaskEntity {
                    id: Some(2),
                    current_technician_id: Some(11),
                    task_status: Some("executing".to_string()),
                    ..Default::default()
                },
                service_items: vec![TaskServiceItemEntity {
                    id: Some(102),
                    task_id: Some(2),
                    service_type: Some("ironing".to_string()),
                    service_status: Some("executing".to_string()),
                    technician_id: Some(11),
                    ..Default::default()
                }],
            },
        ];
        let payload: ReassignTaskRequest = serde_json::from_value(serde_json::json!({
            "assignments": [
                { "service_item_id": 101, "technician_id": 29 },
                { "service_item_id": 102, "service_type": "ironing", "technician_id": 30 }
            ]
        }))
        .unwrap();
        let resolved = resolve_assignment_targets(&payload.assignment_request(), &scopes).unwrap();

        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].task_id, 1);
        assert_eq!(resolved[0].service_item_id, Some(101));
        assert_eq!(resolved[0].from_technician_id, Some(10));
        assert_eq!(resolved[1].task_id, 2);
        assert_eq!(resolved[1].service_item_id, Some(102));
        assert_eq!(resolved[1].from_technician_id, Some(11));
        ensure_unique_assignment_targets(&resolved).unwrap();

        let mismatched: ReassignTaskRequest = serde_json::from_value(serde_json::json!({
            "service_item_id": 101,
            "service_type": "ironing",
            "technician_id": 29
        }))
        .unwrap();
        assert!(matches!(
            resolve_assignment_targets(&mismatched.assignment_request(), &scopes),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn scoped_reassign_matches_only_target_execution_and_preserves_task_status() {
        let assignment = ResolvedAssignment {
            task_id: 1,
            service_item_id: Some(101),
            service_type: "install".to_string(),
            technician_id: 29,
            from_technician_id: Some(10),
            is_legacy_scope: false,
        };
        let selected = TaskExecutionEntity {
            id: Some(201),
            task_id: Some(1),
            service_item_id: Some(101),
            service_type: Some("install".to_string()),
            execution_status: Some("executing".to_string()),
            ..Default::default()
        };
        let other = TaskExecutionEntity {
            id: Some(202),
            task_id: Some(1),
            service_item_id: Some(102),
            service_type: Some("ironing".to_string()),
            execution_status: Some("executing".to_string()),
            ..Default::default()
        };
        assert!(execution_matches_assignment(&selected, &assignment));
        assert!(!execution_matches_assignment(&other, &assignment));

        let service_items = vec![TaskServiceItemEntity {
            service_status: Some("pending_execute".to_string()),
            ..Default::default()
        }];
        assert_eq!(
            scoped_reassign_task_status(&service_items, true),
            "executing"
        );
        assert_eq!(
            scoped_reassign_task_status(&service_items, false),
            "pending_execute"
        );

        let unchanged = TaskServiceItemEntity {
            technician_id: Some(29),
            service_status: Some("pending_execute".to_string()),
            started_at: None,
            completed_at: None,
            progress_percent: Some(0),
            ..Default::default()
        };
        assert!(!service_item_would_change_on_reassign(&unchanged, 29));
        assert!(service_item_would_change_on_reassign(&unchanged, 30));
    }

    #[test]
    fn current_progress_does_not_reuse_detached_historical_execution() {
        let service_item = TaskServiceItemEntity {
            id: Some(101),
            service_type: Some("install".to_string()),
            ..Default::default()
        };
        let detached_history = TaskExecutionEntity {
            id: Some(201),
            service_item_id: None,
            service_type: Some("install".to_string()),
            execution_status: Some("cancelled".to_string()),
            ..Default::default()
        };
        let current_execution = TaskExecutionEntity {
            id: Some(202),
            service_item_id: Some(101),
            service_type: Some("install".to_string()),
            execution_status: Some("executing".to_string()),
            ..Default::default()
        };
        let linked_cancelled_history = TaskExecutionEntity {
            id: Some(203),
            service_item_id: Some(101),
            service_type: Some("install".to_string()),
            execution_status: Some("cancelled".to_string()),
            ..Default::default()
        };

        let historical_only = scoped_progress_execution_ids(
            Some(&service_item),
            &[detached_history.clone(), linked_cancelled_history],
        )
        .unwrap();
        assert!(historical_only.is_empty());

        let with_current = scoped_progress_execution_ids(
            Some(&service_item),
            &[detached_history, current_execution],
        )
        .unwrap();
        assert_eq!(with_current, BTreeSet::from([202]));
    }

    #[test]
    fn reassign_request_accepts_nested_customer_and_camel_case_edits() {
        let payload: ReassignTaskRequest = serde_json::from_value(serde_json::json!({
            "technicianId": 29,
            "assignReason": "任务资料调整后重新派发",
            "appointmentTime": "2026-07-18 14:30:00",
            "serviceTimeSlot": "下午",
            "taskDesc": "更新后的任务描述",
            "taskRemark": "更新后的备注",
            "customer": {
                "name": "张三",
                "mobile": "13800000000",
                "contact_phone": "13800000000",
                "source": "门店到访",
                "address": {
                    "province": "浙江省",
                    "city": "杭州市",
                    "district": "西湖区",
                    "address": "文一路 123 号"
                }
            }
        }))
        .unwrap();

        assert_eq!(payload.technician_id, Some(29));
        assert_eq!(payload.resolved_customer_name(), Some("张三"));
        assert_eq!(payload.resolved_customer_mobile(), Some("13800000000"));
        assert_eq!(payload.resolved_customer_source(), Some("门店到访"));
        assert_eq!(
            payload.resolved_address().map(|item| item.detail.as_str()),
            Some("文一路 123 号")
        );
        assert_eq!(
            payload.appointment_time.as_deref(),
            Some("2026-07-18 14:30:00")
        );
    }

    #[test]
    fn reassign_request_prefers_new_address_over_detail_address() {
        let payload: ReassignTaskRequest = serde_json::from_value(serde_json::json!({
            "technician_id": 29,
            "assign_reason": "修改地址后重新派发",
            "customer_address": "旧地址",
            "address": "新地址"
        }))
        .unwrap();

        assert_eq!(
            payload.resolved_address().map(|item| item.detail.as_str()),
            Some("新地址")
        );
    }

    #[test]
    fn reassign_address_reuses_unchanged_address_and_detects_changes() {
        let entity = CustomerAddressEntity {
            province: Some("浙江省".to_string()),
            city: Some("杭州市".to_string()),
            district: Some("西湖区".to_string()),
            address: Some("文一路 123 号".to_string()),
            ..Default::default()
        };
        let unchanged: AddressRequest =
            serde_json::from_value(serde_json::json!("浙江省杭州市西湖区文一路 123 号")).unwrap();
        let changed: AddressRequest =
            serde_json::from_value(serde_json::json!("浙江省杭州市西湖区文二路 456 号")).unwrap();

        assert!(address_request_matches_entity(&unchanged, &entity));
        assert!(!address_request_matches_entity(&changed, &entity));
    }

    #[test]
    fn reassign_keeps_service_schedule_unless_appointment_changes() {
        let task_time = parse_datetime("2026-07-18 14:30:00").unwrap();
        let service_time = parse_datetime("2026-07-19 09:00:00").unwrap();

        assert_eq!(
            resolve_reassign_scheduled_time(None, Some(service_time)),
            Some(service_time)
        );
        assert_eq!(
            resolve_reassign_scheduled_time(Some(task_time), Some(service_time)),
            Some(task_time)
        );
    }

    #[test]
    fn task_detail_keeps_explicit_service_time_slot() {
        let appointment_time = parse_datetime("2026-07-18 14:30:00").unwrap();

        assert_eq!(
            format_task_service_time_slot(Some(appointment_time), Some("下午")),
            Some("下午".to_string())
        );
        assert_eq!(
            format_task_service_time_slot(Some(appointment_time), None),
            Some("14:30".to_string())
        );
    }

    #[test]
    fn reassign_rejects_submitted_or_completed_service_statuses() {
        assert!(!service_status_blocks_reassign(Some("pending_execute")));
        assert!(!service_status_blocks_reassign(Some("executing")));
        assert!(service_status_blocks_reassign(Some(
            "pending_dealer_review"
        )));
        assert!(service_status_blocks_reassign(Some("pending_hq_score")));
        assert!(service_status_blocks_reassign(Some("completed")));
        assert!(service_status_blocks_reassign(Some("closed")));
    }

    #[test]
    fn create_task_address_accepts_text_or_object() {
        let text_payload = serde_json::json!({
            "customer_name": "张三",
            "customer_mobile": "13800000000",
            "address": "北京市东城区测试地址",
            "service_types": ["measure"],
            "appointment_time": "2026-06-17 21:57:00"
        });
        let text_request: CreateTaskRequest = serde_json::from_value(text_payload).unwrap();
        assert_eq!(
            text_request
                .address
                .as_ref()
                .map(|item| item.detail.as_str()),
            Some("北京市东城区测试地址")
        );

        let object_payload = serde_json::json!({
            "customer_name": "张三",
            "customer_mobile": "13800000000",
            "address": {
                "province": "北京市",
                "city": "北京市",
                "district": "东城区",
                "detail": "测试地址"
            },
            "service_types": ["measure"],
            "appointment_time": "2026-06-17 21:57:00"
        });
        let object_request: CreateTaskRequest = serde_json::from_value(object_payload).unwrap();
        let address = object_request.address.unwrap();
        assert_eq!(address.province.as_deref(), Some("北京市"));
        assert_eq!(address.detail, "测试地址");
    }

    #[test]
    fn task_export_service_type_uses_chinese_text() {
        assert_eq!(task_export_service_type_text("measure"), "量尺服务");
        assert_eq!(task_export_service_type_text("install"), "安装服务");
        assert_eq!(task_export_service_type_text("ironing"), "熨烫服务");
        assert_eq!(task_export_service_type_text("cleaning"), "除尘除螨服务");
    }

    #[test]
    fn sms_mobile_must_be_valid_mainland_number() {
        assert!(is_valid_sms_mobile("13012345678"));
        assert!(is_valid_sms_mobile("14712345678"));
        assert!(is_valid_sms_mobile("16612345678"));
        assert!(is_valid_sms_mobile("18838962531"));
        assert!(is_valid_sms_mobile("18812340002"));
        assert!(is_valid_sms_mobile("19812345678"));
        assert!(!is_valid_sms_mobile("188"));
        assert!(!is_valid_sms_mobile(""));
        assert!(!is_valid_sms_mobile("12812345678"));
        assert!(!is_valid_sms_mobile("28812340002"));
        assert!(!is_valid_sms_mobile("1881234abcd"));
        assert!(!is_valid_sms_mobile("188123456789"));
        assert!(!is_valid_sms_mobile("+8618838962531"));
    }
}
