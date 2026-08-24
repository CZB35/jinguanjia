use std::{
    collections::{BTreeSet, HashMap},
    io::Cursor,
};

use crate::{
    auth, db,
    error::AppError,
    identity,
    models::{
        customer::CustomerEntity, customer_address::CustomerAddressEntity,
        customer_review::CustomerReviewEntity, customer_review_image::CustomerReviewImageEntity,
        dealer_review::DealerReviewEntity, evidence::EvidenceEntity,
        hq_score_item::HqScoreItemEntity, notice::NoticeEntity, store::StoreEntity,
        task::TaskEntity, task_execution::TaskExecutionEntity,
        task_service_item::TaskServiceItemEntity, user::UserEntity,
    },
    response::ApiResponse,
    state::AppState,
    storage,
};
use anyhow::Context;
use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use chrono::Utc;
use image::{GenericImageView, ImageFormat, codecs::jpeg::JpegEncoder, imageops::FilterType};
use rbs::value;
use rust_xlsxwriter::{Color, Format, FormatAlign, FormatBorder, Image as XlsxImage, Workbook};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Default)]
pub struct ReviewTaskQuery {
    #[serde(alias = "status", alias = "taskStatus")]
    pub task_status: Option<String>,
    #[serde(alias = "reviewStatus")]
    pub review_status: Option<String>,
    #[serde(alias = "scoreStatus", alias = "inspection_status")]
    pub score_status: Option<String>,
    #[serde(alias = "serviceType")]
    pub service_type: Option<String>,
    #[serde(alias = "taskType")]
    pub task_type: Option<String>,
    #[serde(alias = "storeId")]
    pub store_id: Option<i64>,
    #[serde(alias = "technicianId")]
    pub technician_id: Option<i64>,
    #[serde(alias = "customer_mobile", alias = "customerMobile", alias = "phone")]
    pub mobile: Option<String>,
    #[serde(alias = "customerName")]
    pub customer_name: Option<String>,
    #[serde(alias = "technicianName")]
    pub technician_name: Option<String>,
    #[serde(alias = "startDate", alias = "start_date")]
    pub appointment_start: Option<String>,
    #[serde(alias = "endDate", alias = "end_date")]
    pub appointment_end: Option<String>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct ExportSelectedReviewTasksRequest {
    #[serde(
        default,
        alias = "taskIds",
        alias = "ids",
        alias = "record_ids",
        alias = "recordIds"
    )]
    pub task_ids: Vec<i64>,
    #[serde(default, alias = "serviceItemIds", alias = "service_item_ids")]
    pub service_item_ids: Vec<i64>,
}

impl ExportSelectedReviewTasksRequest {
    fn selected_task_ids(&self) -> BTreeSet<i64> {
        self.task_ids.iter().copied().filter(|id| *id > 0).collect()
    }

    fn selected_service_item_ids(&self) -> BTreeSet<i64> {
        self.service_item_ids
            .iter()
            .copied()
            .filter(|id| *id > 0)
            .collect()
    }
}

#[derive(Debug, Serialize)]
pub struct ReviewTaskItem {
    pub id: i64,
    pub task_id: i64,
    pub task_no: String,
    pub task_group_no: String,
    pub task_type: String,
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
    pub customer_address: String,
    pub technician_location_address: String,
    pub appointment_time: Option<String>,
    pub dealer_score: Option<i32>,
    pub technician_score: Option<i32>,
    pub service_item_id: Option<i64>,
    #[serde(skip_serializing)]
    pub service_sort_no: i32,
    #[serde(skip_serializing)]
    pub created_at_sort: String,
}

#[derive(Default)]
struct ReviewTaskListData {
    customers: HashMap<i64, CustomerEntity>,
    addresses: HashMap<i64, CustomerAddressEntity>,
    stores: HashMap<i64, StoreEntity>,
    users: HashMap<i64, UserEntity>,
    dealer_display_names: HashMap<i64, String>,
    technician_display_names: HashMap<i64, String>,
    scores: HashMap<(i64, String), HqScoreItemEntity>,
    dealer_reviews: HashMap<i64, Vec<DealerReviewEntity>>,
    evidences: HashMap<i64, Vec<EvidenceEntity>>,
    executions: HashMap<i64, Vec<TaskExecutionEntity>>,
}

#[derive(Debug, Serialize)]
pub struct ReviewTaskListResponse {
    pub items: Vec<ReviewTaskItem>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Default)]
struct ReviewExportExtraData {
    dealer_reviews_by_service_item: HashMap<(i64, i64), String>,
    dealer_reviews_by_service_type: HashMap<(i64, String), String>,
    dealer_reviews_by_task: HashMap<i64, String>,
    customer_reviews_by_task: HashMap<i64, String>,
    image_buffers_by_task: HashMap<i64, Vec<Vec<u8>>>,
}

impl ReviewExportExtraData {
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
pub struct ReviewTaskDetailResponse {
    pub id: i64,
    pub task_no: String,
    pub task_type: String,
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
    pub technician_location_address: String,
    pub appointment_time: Option<String>,
    pub service_time_slot: Option<String>,
    pub task_desc: Option<String>,
    pub task_remark: Option<String>,
    pub evidence_count: usize,
    pub dealer_review: Option<ReviewInfoPayload>,
    pub customer_review: Option<CustomerReviewPayload>,
    pub service_items: Vec<ReviewTaskServiceItem>,
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
    pub score_text: String,
    pub deduction_reason: Option<String>,
}

pub async fn list_review_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReviewTaskQuery>,
) -> Result<Json<ApiResponse<ReviewTaskListResponse>>, AppError> {
    ensure_hq_access(&state, &headers)?;
    let mut items = load_review_tasks(&state, &query).await?;
    retain_review_pending_and_completed_items(&mut items);
    let total = items.len();
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).max(1);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let items = items
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();

    Ok(Json(ApiResponse::success(ReviewTaskListResponse {
        items,
        total,
        page,
        page_size,
    })))
}

pub async fn list_review_pending_and_completed_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReviewTaskQuery>,
) -> Result<Json<ApiResponse<ReviewTaskListResponse>>, AppError> {
    ensure_hq_access(&state, &headers)?;
    let mut items = load_review_tasks(&state, &query).await?;
    retain_review_pending_and_completed_items(&mut items);
    let total = items.len();
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).max(1);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let items = items
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();

    Ok(Json(ApiResponse::success(ReviewTaskListResponse {
        items,
        total,
        page,
        page_size,
    })))
}

fn retain_review_pending_and_completed_items(items: &mut Vec<ReviewTaskItem>) {
    items.retain(|item| {
        item.task_status == "pending_dealer_review"
            || item.task_status == "pending_hq_score"
            || item.task_status == "completed"
            || item.service_status == "pending_dealer_review"
            || item.service_status == "pending_hq_score"
            || item.service_status == "completed"
    });
}

pub async fn list_review_pending_execute_and_executing_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReviewTaskQuery>,
) -> Result<Json<ApiResponse<ReviewTaskListResponse>>, AppError> {
    ensure_hq_access(&state, &headers)?;
    let mut items = load_review_tasks(&state, &query).await?;
    items.retain(|item| {
        item.task_status == "pending_execute"
            || item.task_status == "executing"
            || item.service_status == "pending_execute"
            || item.service_status == "executing"
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

    Ok(Json(ApiResponse::success(ReviewTaskListResponse {
        items,
        total,
        page,
        page_size,
    })))
}

pub async fn export_review_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReviewTaskQuery>,
) -> Result<Response, AppError> {
    ensure_hq_access(&state, &headers)?;
    let items = load_review_tasks(&state, &query).await?;

    render_review_export_workbook(&state, items).await
}

pub async fn export_selected_review_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ExportSelectedReviewTasksRequest>,
) -> Result<Response, AppError> {
    ensure_hq_access(&state, &headers)?;
    let selected_task_ids = payload.selected_task_ids();
    let selected_service_item_ids = payload.selected_service_item_ids();
    if selected_task_ids.is_empty() && selected_service_item_ids.is_empty() {
        return Err(AppError::BadRequest(
            "task_ids或service_item_ids不能为空".to_string(),
        ));
    }

    let mut items = load_review_tasks(&state, &ReviewTaskQuery::default()).await?;
    items.retain(|item| {
        selected_task_ids.contains(&item.task_id)
            || item
                .service_item_id
                .map(|id| selected_service_item_ids.contains(&id))
                .unwrap_or(false)
    });
    if items.is_empty() {
        return Err(AppError::BadRequest("所选记录暂无可导出数据".to_string()));
    }

    render_review_export_workbook(&state, items).await
}

async fn render_review_export_workbook(
    state: &AppState,
    items: Vec<ReviewTaskItem>,
) -> Result<Response, AppError> {
    let task_ids = items
        .iter()
        .map(|item| item.task_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let extra_data = load_review_export_extra_data(state, &task_ids).await?;
    let mut export_rows = Vec::with_capacity(items.len());
    let mut max_image_count = 0usize;
    for item in items {
        let dealer_review =
            extra_data.dealer_review(item.task_id, item.service_item_id, &item.service_type);
        let customer_review = extra_data.customer_review(item.task_id);
        max_image_count = max_image_count.max(
            extra_data
                .image_buffers_by_task
                .get(&item.task_id)
                .map(Vec::len)
                .unwrap_or_default(),
        );
        export_rows.push((item, dealer_review, customer_review));
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
            .set_name("环节记录导出")
            .context("failed to create review export worksheet")
            .map_err(AppError::Internal)?;
        let headers = review_export_text_headers();
        let column_widths = [
            130_u32, 120, 110, 120, 130, 100, 140, 140, 220, 220, 260, 260,
        ];

        worksheet
            .set_row_height_pixels(0, 28)
            .context("failed to set review export header row height")
            .map_err(AppError::Internal)?;
        for (index, width) in column_widths.iter().enumerate() {
            worksheet
                .set_column_width_pixels(index as u16, *width)
                .context("failed to set review export column width")
                .map_err(AppError::Internal)?;
        }
        for image_index in 0..image_column_count {
            worksheet
                .set_column_width_pixels((headers.len() + image_index) as u16, 180)
                .context("failed to set review export image column width")
                .map_err(AppError::Internal)?;
        }
        for (col, title) in headers.iter().enumerate() {
            worksheet
                .write_string_with_format(0, col as u16, *title, &header_format)
                .context("failed to write review export header cell")
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
                .context("failed to write review export image header cell")
                .map_err(AppError::Internal)?;
        }

        for (row_index, (item, dealer_review, customer_review)) in export_rows.iter().enumerate() {
            let excel_row = (row_index + 1) as u32;
            let image_buffers = extra_data
                .image_buffers_by_task
                .get(&item.task_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let row_height = if image_buffers.is_empty() { 48 } else { 140 };
            worksheet
                .set_row_height_pixels(excel_row, row_height)
                .context("failed to set review export data row height")
                .map_err(AppError::Internal)?;
            let values = review_export_text_values(item, dealer_review, customer_review);
            for (col, value) in values.iter().enumerate() {
                worksheet
                    .write_string_with_format(excel_row, col as u16, value, &cell_format)
                    .context("failed to write review export cell")
                    .map_err(AppError::Internal)?;
            }
            for image_index in 0..image_column_count {
                let col = (headers.len() + image_index) as u16;
                worksheet
                    .write_string_with_format(excel_row, col, "", &image_cell_format)
                    .context("failed to write review export image placeholder cell")
                    .map_err(AppError::Internal)?;
                if let Some(image_buffer) = image_buffers.get(image_index) {
                    let image = XlsxImage::new_from_buffer(image_buffer)
                        .context("failed to build review export image from buffer")
                        .map_err(AppError::Internal)?;
                    worksheet
                        .insert_image_fit_to_cell_centered(excel_row, col, &image)
                        .context("failed to insert review export image into worksheet")
                        .map_err(AppError::Internal)?;
                }
            }
        }
    }

    let workbook_bytes = workbook
        .save_to_buffer()
        .context("failed to render review export workbook")
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
        HeaderValue::from_static("attachment; filename=\"review-tasks-export.xlsx\""),
    );
    Ok(response)
}

async fn load_review_task_list_data(
    state: &AppState,
    tasks: &[TaskEntity],
    service_items_by_task_id: &HashMap<i64, Vec<TaskServiceItemEntity>>,
) -> Result<ReviewTaskListData, AppError> {
    let task_ids = tasks.iter().filter_map(|task| task.id).collect::<Vec<_>>();
    if task_ids.is_empty() {
        return Ok(ReviewTaskListData::default());
    }
    let customer_ids = tasks
        .iter()
        .filter_map(|task| task.customer_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let address_ids = tasks
        .iter()
        .filter_map(|task| task.customer_address_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let store_ids = tasks
        .iter()
        .filter_map(|task| task.store_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut user_ids = tasks
        .iter()
        .flat_map(|task| [task.dealer_user_id, task.current_technician_id])
        .flatten()
        .collect::<BTreeSet<_>>();
    for service_item in service_items_by_task_id.values().flatten() {
        if let Some(technician_id) = service_item.technician_id {
            user_ids.insert(technician_id);
        }
    }
    let user_ids = user_ids.into_iter().collect::<Vec<_>>();

    let (customers, addresses, stores, users, scores, reviews, evidences, executions) = tokio::try_join!(
        load_review_customers(state, &customer_ids),
        load_review_addresses(state, &address_ids),
        load_review_stores(state, &store_ids),
        load_review_users(state, &user_ids),
        load_review_scores(state, &task_ids),
        load_review_dealer_reviews(state, &task_ids),
        load_review_evidences(state, &task_ids),
        load_review_executions(state, &task_ids),
    )?;

    let user_map = users
        .into_iter()
        .filter_map(|item| item.id.map(|id| (id, item)))
        .collect::<HashMap<_, _>>();
    let user_ids = user_map.keys().copied().collect::<Vec<_>>();
    let (dealer_display_names, technician_display_names) = tokio::try_join!(
        identity::role_display_names(state, &user_ids, "dealer"),
        identity::role_display_names(state, &user_ids, "technician"),
    )?;
    let mut data = ReviewTaskListData {
        customers: customers
            .into_iter()
            .filter_map(|item| item.id.map(|id| (id, item)))
            .collect(),
        addresses: addresses
            .into_iter()
            .filter_map(|item| item.id.map(|id| (id, item)))
            .collect(),
        stores: stores
            .into_iter()
            .filter_map(|item| item.id.map(|id| (id, item)))
            .collect(),
        users: user_map,
        dealer_display_names,
        technician_display_names,
        ..Default::default()
    };
    for score in scores {
        if let (Some(task_id), Some(service_type)) = (score.task_id, score.service_type.clone()) {
            data.scores.entry((task_id, service_type)).or_insert(score);
        }
    }
    for review in reviews {
        if let Some(task_id) = review.task_id {
            data.dealer_reviews.entry(task_id).or_default().push(review);
        }
    }
    for evidence in evidences {
        if let Some(task_id) = evidence.task_id {
            data.evidences.entry(task_id).or_default().push(evidence);
        }
    }
    for execution in executions {
        if let Some(task_id) = execution.task_id {
            data.executions.entry(task_id).or_default().push(execution);
        }
    }
    Ok(data)
}

macro_rules! define_review_list_loader {
    ($name:ident, $entity:ty, $table:literal, $column:literal, $context:literal) => {
        async fn $name(state: &AppState, ids: &[i64]) -> Result<Vec<$entity>, AppError> {
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            db::select_by_map_compat(state.rb.as_ref(), $table, value! { $column: ids.to_vec() })
                .await
                .context($context)
                .map_err(AppError::Internal)
        }
    };
}

define_review_list_loader!(
    load_review_customers,
    CustomerEntity,
    "customer",
    "id",
    "failed to batch query review customers"
);
define_review_list_loader!(
    load_review_addresses,
    CustomerAddressEntity,
    "customer_address",
    "id",
    "failed to batch query review addresses"
);
define_review_list_loader!(
    load_review_stores,
    StoreEntity,
    "store",
    "id",
    "failed to batch query review stores"
);
define_review_list_loader!(
    load_review_users,
    UserEntity,
    "user",
    "id",
    "failed to batch query review users"
);
define_review_list_loader!(
    load_review_scores,
    HqScoreItemEntity,
    "hq_score_item",
    "task_id",
    "failed to batch query review scores"
);
define_review_list_loader!(
    load_review_dealer_reviews,
    DealerReviewEntity,
    "dealer_review",
    "task_id",
    "failed to batch query dealer reviews"
);
define_review_list_loader!(
    load_review_evidences,
    EvidenceEntity,
    "evidence",
    "task_id",
    "failed to batch query review evidences"
);
define_review_list_loader!(
    load_review_executions,
    TaskExecutionEntity,
    "task_execution",
    "task_id",
    "failed to batch query review executions"
);

async fn load_review_tasks(
    state: &AppState,
    query: &ReviewTaskQuery,
) -> Result<Vec<ReviewTaskItem>, AppError> {
    let task_type_filters = normalized_task_type_filters(query.task_type.as_deref())?;
    let mut tasks: Vec<TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! {})
        .await
        .context("failed to query review tasks")
        .map_err(AppError::Internal)?;
    tasks.retain(|task| task.deleted_at.is_none());
    let task_ids = tasks.iter().filter_map(|task| task.id).collect::<Vec<_>>();
    let all_service_items = if task_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<TaskServiceItemEntity>(
            state.rb.as_ref(),
            "task_service_item",
            value! { "task_id": task_ids.clone() },
        )
        .await
        .context("failed to query task service items for review filter")
        .map_err(AppError::Internal)?
    };
    let mut service_items_by_task_id =
        std::collections::HashMap::<i64, Vec<TaskServiceItemEntity>>::new();
    for service_item in all_service_items {
        if let Some(task_id) = service_item.task_id {
            service_items_by_task_id
                .entry(task_id)
                .or_default()
                .push(service_item);
        }
    }
    let group_service_counts = build_task_group_service_counts(&tasks, &service_items_by_task_id);
    let list_data = load_review_task_list_data(state, &tasks, &service_items_by_task_id).await?;
    let filter_special_task_type = !task_type_filters.is_empty();
    let mut items = Vec::new();
    for task in tasks {
        if let Some(task_status) = &query.task_status
            && !matches_review_task_status_query(task.task_status.as_deref(), task_status)
        {
            continue;
        }
        if let Some(store_id) = query.store_id
            && task.store_id != Some(store_id)
        {
            continue;
        }
        if let Some(appointment_start) = &query.appointment_start {
            let Ok(start_at) =
                chrono::NaiveDateTime::parse_from_str(appointment_start, "%Y-%m-%d %H:%M:%S")
            else {
                continue;
            };
            if task
                .appointment_time
                .map(|dt| dt < start_at)
                .unwrap_or(true)
            {
                continue;
            }
        }
        if let Some(appointment_end) = &query.appointment_end {
            let Ok(end_at) =
                chrono::NaiveDateTime::parse_from_str(appointment_end, "%Y-%m-%d %H:%M:%S")
            else {
                continue;
            };
            if task.appointment_time.map(|dt| dt > end_at).unwrap_or(true) {
                continue;
            }
        }
        if !matches_task_type_filters(&task, &task_type_filters, &group_service_counts) {
            continue;
        }
        if filter_special_task_type && is_pending_execute_status(task.task_status.as_deref()) {
            continue;
        }
        let task_id = task.id.unwrap_or_default();
        let mut service_items = service_items_by_task_id
            .remove(&task_id)
            .unwrap_or_default();
        if service_items.is_empty() {
            service_items.push(TaskServiceItemEntity {
                id: None,
                task_id: Some(task_id),
                service_type: task.service_type.clone(),
                service_status: task.task_status.clone(),
                technician_id: task.current_technician_id,
                scheduled_time: task.appointment_time,
                started_at: None,
                completed_at: None,
                progress_percent: None,
                sort_no: Some(service_type_sort_key(
                    task.service_type.as_deref().unwrap_or_default(),
                )),
                created_at: None,
                updated_at: None,
            });
        }
        for service_item in service_items {
            if filter_special_task_type
                && is_pending_execute_status(service_item.service_status.as_deref())
            {
                continue;
            }
            if let Some(service_type) = &query.service_type
                && !super::multi_query_value_matches(Some(service_type), |expected| {
                    service_item.service_type.as_deref() == Some(expected)
                })
            {
                continue;
            }
            if let Some(technician_id) = query.technician_id
                && service_item.technician_id.or(task.current_technician_id) != Some(technician_id)
            {
                continue;
            }
            let customer = list_data
                .customers
                .get(&task.customer_id.unwrap_or_default())
                .ok_or_else(|| AppError::NotFound("客户不存在".to_string()))?;
            if let Some(mobile) = &query.mobile
                && !customer
                    .mobile
                    .as_deref()
                    .unwrap_or_default()
                    .contains(mobile)
            {
                continue;
            }
            if let Some(customer_name) = &query.customer_name
                && !customer
                    .name
                    .as_deref()
                    .unwrap_or_default()
                    .contains(customer_name)
            {
                continue;
            }
            let review_item =
                build_review_task_item_from_prefetched(&task, &service_item, &list_data)?;
            if !matches_review_task_query(&review_item, &task, &service_item, query)? {
                continue;
            }
            items.push(review_item);
        }
    }
    sort_review_task_items(&mut items);
    Ok(items)
}

pub async fn get_review_task_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<i64>,
) -> Result<Json<ApiResponse<ReviewTaskDetailResponse>>, AppError> {
    ensure_hq_access(&state, &headers)?;
    let task = find_task(&state, task_id).await?;
    let evidences: Vec<EvidenceEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "evidence", value! { "task_id": task_id })
            .await
            .context("failed to query task evidences for review detail")
            .map_err(AppError::Internal)?;
    let executions: Vec<TaskExecutionEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_execution",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query task executions for review detail")
    .map_err(AppError::Internal)?;
    let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_service_item",
        value! { "task_id": task_id },
    )
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
    let address =
        load_customer_address_text(&state, task.customer_address_id.unwrap_or_default()).await?;
    let technician_location_address =
        crate::routes::technician_location::latest_technician_location_address(
            &evidences,
            &executions,
            &service_items,
            &address,
        );
    let service_time_slot =
        format_review_service_time_slot(task.appointment_time, task.service_time_slot.as_deref());
    let dealer_review = find_dealer_review_for_scope(&state, task_id, &service_items).await?;
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
    let dealer_display_name = identity::role_display_name(
        &state,
        dealer.id.unwrap_or_default(),
        "dealer",
        dealer.name.as_deref(),
    )
    .await?;
    let technician_display_name = if let Some(technician) = technician.as_ref() {
        identity::role_display_name(
            &state,
            technician.id.unwrap_or_default(),
            "technician",
            technician.name.as_deref(),
        )
        .await?
    } else {
        String::new()
    };
    let customer_review = find_customer_review_for_task(&state, task_id).await?;
    let service_items = build_review_task_service_items(&state, service_items, task_id).await?;
    let score_status = review_detail_score_status(&service_items);

    Ok(Json(ApiResponse::success(ReviewTaskDetailResponse {
        id: task.id.unwrap_or_default(),
        task_no: task.task_no.unwrap_or_default(),
        task_type: task.task_type.unwrap_or_default(),
        service_type: task.service_type.unwrap_or_default(),
        task_status: task
            .task_status
            .as_deref()
            .map(normalize_review_task_status)
            .unwrap_or_default()
            .to_string(),
        review_status: task.review_status.unwrap_or_default(),
        score_status,
        store_name: store.store_name.unwrap_or_default(),
        dealer_name: dealer_display_name,
        technician_name: technician_display_name,
        technician_mobile: technician
            .as_ref()
            .and_then(|item| item.mobile.clone())
            .unwrap_or_default(),
        customer_name: customer.name.unwrap_or_default(),
        customer_mobile: customer.mobile.unwrap_or_default(),
        customer_address: address,
        technician_location_address,
        appointment_time: task.appointment_time.map(format_datetime),
        service_time_slot,
        task_desc: task.task_desc,
        task_remark: task.task_remark,
        evidence_count: evidences.len(),
        dealer_review: dealer_review_payload,
        customer_review,
        service_items,
    })))
}

pub async fn list_review_reminders(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<ReviewTaskListResponse>>, AppError> {
    ensure_hq_access(&state, &headers)?;
    let mut tasks: Vec<TaskEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task",
        value! { "task_status": "pending_dealer_review" },
    )
    .await
    .context("failed to query review reminder tasks")
    .map_err(AppError::Internal)?;
    tasks.retain(|task| task.deleted_at.is_none());

    let mut items = Vec::new();
    for task in tasks {
        let task_id = task.id.unwrap_or_default();
        let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "task_service_item",
            value! { "task_id": task_id },
        )
        .await
        .context("failed to query reminder task service items")
        .map_err(AppError::Internal)?;
        for service_item in service_items {
            items.push(build_review_task_item(&state, &task, &service_item).await?);
        }
    }
    sort_review_task_items(&mut items);
    let total = items.len();
    let page = 1;
    let page_size = total.max(1);

    Ok(Json(ApiResponse::success(ReviewTaskListResponse {
        items,
        total,
        page,
        page_size,
    })))
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

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

fn ensure_hq_access(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let auth_user = auth::authenticate(state, headers)?;
    auth_user.require_any_role(&["hq_reviewer", "super_admin"])
}

async fn find_task(state: &AppState, task_id: i64) -> Result<TaskEntity, AppError> {
    let task: TaskEntity =
        db::select_one_by_map_compat(state.rb.as_ref(), "task", value! { "id": task_id })
            .await
            .context("failed to query review task")
            .map_err(AppError::Internal)?
            .ok_or_else(|| AppError::NotFound("任务不存在或已删除".to_string()))?;
    if task.deleted_at.is_some() {
        return Err(AppError::NotFound("任务不存在或已删除".to_string()));
    }
    Ok(task)
}

fn build_review_task_item_from_prefetched(
    task: &TaskEntity,
    service_item: &TaskServiceItemEntity,
    data: &ReviewTaskListData,
) -> Result<ReviewTaskItem, AppError> {
    let task_id = task.id.unwrap_or_default();
    let customer = data
        .customers
        .get(&task.customer_id.unwrap_or_default())
        .ok_or_else(|| AppError::NotFound("客户不存在".to_string()))?;
    let address = data
        .addresses
        .get(&task.customer_address_id.unwrap_or_default())
        .ok_or_else(|| AppError::NotFound("客户地址不存在".to_string()))?;
    let store = data
        .stores
        .get(&task.store_id.unwrap_or_default())
        .ok_or_else(|| AppError::NotFound("门店不存在".to_string()))?;
    let dealer = data
        .users
        .get(&task.dealer_user_id.unwrap_or_default())
        .ok_or_else(|| AppError::NotFound("用户不存在".to_string()))?;
    let technician = service_item
        .technician_id
        .or(task.current_technician_id)
        .and_then(|technician_id| data.users.get(&technician_id));
    let service_type = service_item.service_type.clone().unwrap_or_default();
    let score_item = data.scores.get(&(task_id, service_type.clone()));
    let score_status = if score_item.is_some() {
        "scored"
    } else {
        "pending"
    };
    let reviews = data
        .dealer_reviews
        .get(&task_id)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let dealer_score =
        find_prefetched_dealer_review(reviews, service_item).and_then(|review| review.star);
    let customer_address = format_customer_address_entity(address);
    let technician_location_address =
        crate::routes::technician_location::latest_technician_location_address(
            data.evidences
                .get(&task_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            data.executions
                .get(&task_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            std::slice::from_ref(service_item),
            &customer_address,
        );
    Ok(ReviewTaskItem {
        id: task_id,
        task_id,
        task_no: task.task_no.clone().unwrap_or_default(),
        task_group_no: task.task_group_no.clone().unwrap_or_default(),
        task_type: task.task_type.clone().unwrap_or_default(),
        service_type,
        task_status: task
            .task_status
            .as_deref()
            .map(normalize_review_task_status)
            .unwrap_or_default()
            .to_string(),
        service_status: service_item
            .service_status
            .as_deref()
            .map(normalize_review_task_status)
            .unwrap_or_default()
            .to_string(),
        review_status: task.review_status.clone().unwrap_or_default(),
        score_status: score_status.to_string(),
        store_name: store.store_name.clone().unwrap_or_default(),
        dealer_name: dealer
            .id
            .and_then(|user_id| data.dealer_display_names.get(&user_id).cloned())
            .unwrap_or_else(|| dealer.name.clone().unwrap_or_default()),
        technician_name: technician
            .and_then(|item| item.id)
            .and_then(|user_id| data.technician_display_names.get(&user_id).cloned())
            .or_else(|| technician.and_then(|item| item.name.clone()))
            .unwrap_or_default(),
        technician_mobile: technician
            .and_then(|item| item.mobile.clone())
            .unwrap_or_default(),
        customer_name: customer.name.clone().unwrap_or_default(),
        customer_mobile: customer.mobile.clone().unwrap_or_default(),
        customer_address,
        technician_location_address,
        appointment_time: service_item
            .scheduled_time
            .or(task.appointment_time)
            .map(format_datetime),
        dealer_score,
        technician_score: score_item.and_then(|item| item.score),
        service_item_id: service_item.id,
        service_sort_no: service_item.sort_no.unwrap_or_default(),
        created_at_sort: task.created_at.map(format_datetime).unwrap_or_default(),
    })
}

fn find_prefetched_dealer_review<'a>(
    reviews: &'a [DealerReviewEntity],
    service_item: &TaskServiceItemEntity,
) -> Option<&'a DealerReviewEntity> {
    if let Some(service_item_id) = service_item.id
        && let Some(review) = reviews
            .iter()
            .find(|review| review.service_item_id == Some(service_item_id))
    {
        return Some(review);
    }
    if let Some(service_type) = service_item.service_type.as_deref() {
        return reviews
            .iter()
            .find(|review| review.service_type.as_deref() == Some(service_type));
    }
    if reviews.len() == 1 {
        reviews.first()
    } else {
        None
    }
}

fn format_customer_address_entity(address: &CustomerAddressEntity) -> String {
    address.address.clone().unwrap_or_default()
}

async fn build_review_task_item(
    state: &AppState,
    task: &TaskEntity,
    service_item: &TaskServiceItemEntity,
) -> Result<ReviewTaskItem, AppError> {
    let dealer = load_user(state, task.dealer_user_id.unwrap_or_default()).await?;
    let customer = load_customer(state, task.customer_id.unwrap_or_default()).await?;
    let customer_address =
        load_customer_address_text(state, task.customer_address_id.unwrap_or_default()).await?;
    let store = load_store(state, task.store_id.unwrap_or_default()).await?;
    let technician =
        if let Some(technician_id) = service_item.technician_id.or(task.current_technician_id) {
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
    let technician_score = score_item.as_ref().and_then(|item| item.score);
    let score_status = if score_item.is_some() {
        "scored"
    } else {
        "pending"
    };
    let dealer_score = find_dealer_review_for_scope(
        state,
        task.id.unwrap_or_default(),
        std::slice::from_ref(service_item),
    )
    .await?
    .and_then(|review| review.star);
    let evidences: Vec<EvidenceEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "evidence",
        value! { "task_id": task.id.unwrap_or_default() },
    )
    .await
    .context("failed to query evidences for review task item")
    .map_err(AppError::Internal)?;
    let executions: Vec<TaskExecutionEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_execution",
        value! { "task_id": task.id.unwrap_or_default() },
    )
    .await
    .context("failed to query executions for review task item")
    .map_err(AppError::Internal)?;
    let technician_location_address =
        crate::routes::technician_location::latest_technician_location_address(
            &evidences,
            &executions,
            std::slice::from_ref(service_item),
            &customer_address,
        );
    let dealer_display_name = identity::role_display_name(
        state,
        dealer.id.unwrap_or_default(),
        "dealer",
        dealer.name.as_deref(),
    )
    .await?;
    let technician_display_name = if let Some(technician) = technician.as_ref() {
        identity::role_display_name(
            state,
            technician.id.unwrap_or_default(),
            "technician",
            technician.name.as_deref(),
        )
        .await?
    } else {
        String::new()
    };

    Ok(ReviewTaskItem {
        id: task.id.unwrap_or_default(),
        task_id: task.id.unwrap_or_default(),
        task_no: task.task_no.clone().unwrap_or_default(),
        task_group_no: task.task_group_no.clone().unwrap_or_default(),
        task_type: task.task_type.clone().unwrap_or_default(),
        service_type: service_item.service_type.clone().unwrap_or_default(),
        task_status: task
            .task_status
            .as_deref()
            .map(normalize_review_task_status)
            .unwrap_or_default()
            .to_string(),
        service_status: service_item
            .service_status
            .as_deref()
            .map(normalize_review_task_status)
            .unwrap_or_default()
            .to_string(),
        review_status: task.review_status.clone().unwrap_or_default(),
        score_status: score_status.to_string(),
        store_name: store.store_name.unwrap_or_default(),
        dealer_name: dealer_display_name,
        technician_name: technician_display_name,
        technician_mobile: technician
            .as_ref()
            .and_then(|item| item.mobile.clone())
            .unwrap_or_default(),
        customer_name: customer.name.unwrap_or_default(),
        customer_mobile: customer.mobile.unwrap_or_default(),
        customer_address,
        technician_location_address,
        appointment_time: service_item
            .scheduled_time
            .or(task.appointment_time)
            .map(format_datetime),
        dealer_score,
        technician_score,
        service_item_id: service_item.id,
        service_sort_no: service_item.sort_no.unwrap_or_default(),
        created_at_sort: task.created_at.map(format_datetime).unwrap_or_default(),
    })
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
            .context("failed to query dealer review by service item in review detail")
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
            .context("failed to query dealer review by service type in review detail")
            .map_err(AppError::Internal);
        }
    }

    let reviews: Vec<DealerReviewEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "dealer_review",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query dealer reviews for review detail")
    .map_err(AppError::Internal)?;
    Ok(if reviews.len() == 1 {
        reviews.into_iter().next()
    } else {
        None
    })
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
    .context("failed to query customer review for review detail")
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
        .context("failed to query customer review images for review detail")
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
        let technician_display_name = if let Some(technician) = technician.as_ref() {
            identity::role_display_name(
                state,
                technician.id.unwrap_or_default(),
                "technician",
                technician.name.as_deref(),
            )
            .await?
        } else {
            String::new()
        };
        let score_item: Option<HqScoreItemEntity> = db::select_one_by_map_compat(
            state.rb.as_ref(),
            "hq_score_item",
            value! { "task_id": task_id, "service_type": item.service_type.as_deref().unwrap_or_default() },
        )
        .await
        .context("failed to query score item for review detail")
        .map_err(AppError::Internal)?;

        let score = score_item.as_ref().and_then(|x| x.score);
        result.push(ReviewTaskServiceItem {
            id: item.id.unwrap_or_default(),
            service_type: item.service_type.unwrap_or_default(),
            service_status: item.service_status.unwrap_or_default(),
            technician_id: item.technician_id,
            technician_name: (!technician_display_name.is_empty())
                .then_some(technician_display_name),
            technician_mobile: technician.as_ref().and_then(|x| x.mobile.clone()),
            technician_job_no: technician.as_ref().and_then(|x| x.job_no.clone()),
            scheduled_time: item.scheduled_time.map(format_datetime),
            progress_percent: item.progress_percent.unwrap_or_default(),
            score,
            score_text: optional_numeric_score_text(score),
            deduction_reason: score_item.and_then(|x| x.deduction_reason),
        });
    }
    Ok(result)
}

fn review_detail_score_status(items: &[ReviewTaskServiceItem]) -> String {
    if !items.is_empty() && items.iter().all(|item| item.score.is_some()) {
        "scored".to_string()
    } else {
        "pending".to_string()
    }
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
    let addresses: Vec<crate::models::customer_address::CustomerAddressEntity> =
        db::select_by_map_compat(
            state.rb.as_ref(),
            "customer_address",
            value! { "id": address_id },
        )
        .await
        .context("failed to query customer address in hq review")
        .map_err(AppError::Internal)?;
    Ok(addresses
        .into_iter()
        .next()
        .map(|item| format_review_customer_address(&item))
        .unwrap_or_default())
}

fn format_review_customer_address(
    address: &crate::models::customer_address::CustomerAddressEntity,
) -> String {
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

fn format_review_service_time_slot(
    appointment_time: Option<chrono::NaiveDateTime>,
    service_time_slot: Option<&str>,
) -> Option<String> {
    if let Some(appointment_time) = appointment_time {
        return Some(appointment_time.format("%H:%M").to_string());
    }

    service_time_slot
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
}

fn format_datetime(input: chrono::NaiveDateTime) -> String {
    input.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn review_export_text_headers() -> [&'static str; 12] {
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

fn review_export_text_values(
    item: &ReviewTaskItem,
    dealer_review: &str,
    customer_review: &str,
) -> [String; 12] {
    [
        item.customer_mobile.clone(),
        item.customer_name.clone(),
        review_service_type_text(&item.service_type),
        item.technician_name.clone(),
        item.technician_mobile.clone(),
        item.technician_score
            .map(|score| score.to_string())
            .unwrap_or_default(),
        item.store_name.clone(),
        item.dealer_name.clone(),
        dealer_review.to_string(),
        customer_review.to_string(),
        item.customer_address.clone(),
        item.technician_location_address.clone(),
    ]
}

async fn load_review_export_extra_data(
    state: &AppState,
    task_ids: &[i64],
) -> Result<ReviewExportExtraData, AppError> {
    if task_ids.is_empty() {
        return Ok(ReviewExportExtraData::default());
    }

    let mut extra = ReviewExportExtraData::default();
    let mut dealer_reviews: Vec<DealerReviewEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "dealer_review",
        value! { "task_id": task_ids.to_vec() },
    )
    .await
    .context("failed to query review export dealer reviews")
    .map_err(AppError::Internal)?;
    dealer_reviews.sort_by_key(|item| item.id.unwrap_or_default());
    for review in dealer_reviews {
        let Some(task_id) = review.task_id else {
            continue;
        };
        let text = review_export_review_text(review.star, review.content.as_deref());
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
    .context("failed to query review export customer reviews")
    .map_err(AppError::Internal)?;
    customer_reviews.sort_by_key(|item| item.id.unwrap_or_default());
    for review in customer_reviews {
        let Some(task_id) = review.task_id else {
            continue;
        };
        let text = review_export_review_text(review.star, review.content.as_deref());
        if !text.is_empty() {
            extra.customer_reviews_by_task.insert(task_id, text);
        }
    }

    extra.image_buffers_by_task =
        load_review_export_image_buffers_by_task_ids(state, task_ids).await?;
    Ok(extra)
}

async fn load_review_export_image_buffers_by_task_ids(
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
    .context("failed to query review export evidences")
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
        if let Some(buffer) = normalize_review_export_image_buffer(&bytes) {
            image_buffers_by_task_id
                .entry(task_id)
                .or_default()
                .push(buffer);
        }
    }

    Ok(image_buffers_by_task_id)
}

fn normalize_review_export_image_buffer(bytes: &[u8]) -> Option<Vec<u8>> {
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

fn review_export_review_text(star: Option<i32>, content: Option<&str>) -> String {
    let content = content.unwrap_or_default().trim();
    match (star.filter(|value| *value > 0), content.is_empty()) {
        (Some(star), false) => format!("{star}分 {content}"),
        (Some(star), true) => format!("{star}分"),
        (None, false) => content.to_string(),
        (None, true) => String::new(),
    }
}

fn review_service_type_text(service_type: &str) -> String {
    match service_type {
        "measure" => "量尺服务".to_string(),
        "install" => "安装服务".to_string(),
        "ironing" => "熨烫服务".to_string(),
        "cleaning" => "除尘除螨服务".to_string(),
        _ => "其他服务".to_string(),
    }
}

fn review_task_status_text(status: &str) -> String {
    match normalize_review_task_status(status) {
        "pending_assign" => "待分派".to_string(),
        "pending_execute" => "待执行".to_string(),
        "executing" => "进行中".to_string(),
        "pending_dealer_review" => "待评价".to_string(),
        "completed" => "已完成".to_string(),
        "closed" => "已关闭".to_string(),
        _ => status.to_string(),
    }
}

fn review_review_status_text(status: &str) -> String {
    match status {
        "pending" => "待评价".to_string(),
        "reviewed" => "已评价".to_string(),
        _ => status.to_string(),
    }
}

fn review_score_status_text(status: &str) -> String {
    match status {
        "pending" => "待核验".to_string(),
        "scored" => "已核验".to_string(),
        _ => status.to_string(),
    }
}

fn canonical_score_status_filter(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() || matches!(value, "全部" | "不限") || value.eq_ignore_ascii_case("all")
    {
        return None;
    }
    Some(match value {
        "待核验" => "pending".to_string(),
        "已核验" => "scored".to_string(),
        _ => value.to_string(),
    })
}

fn matches_review_status_query(item: &ReviewTaskItem, expected: Option<&str>) -> bool {
    super::multi_query_value_matches(expected, |value| {
        let normalized = value.trim();
        if matches!(normalized, "pending" | "completed") {
            let expected_score_status = if normalized == "completed" {
                "scored"
            } else {
                "pending"
            };
            item.score_status == expected_score_status
        } else {
            item.review_status == normalized
        }
    })
}

fn matches_review_score_status_query(item: &ReviewTaskItem, expected: Option<&str>) -> bool {
    super::multi_query_value_matches(expected, |value| {
        canonical_score_status_filter(Some(value))
            .map(|expected| item.score_status == expected)
            .unwrap_or(true)
    })
}

fn review_score_text(star: i32) -> String {
    if star > 0 {
        format!("{star}分")
    } else {
        "--".to_string()
    }
}

fn optional_numeric_score_text(score: Option<i32>) -> String {
    score
        .map(|value| format!("{value}分"))
        .unwrap_or_else(|| "--".to_string())
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

fn build_task_group_service_counts(
    tasks: &[TaskEntity],
    service_items_by_task_id: &std::collections::HashMap<i64, Vec<TaskServiceItemEntity>>,
) -> std::collections::HashMap<String, usize> {
    let mut grouped_services =
        std::collections::HashMap::<String, std::collections::BTreeSet<String>>::new();
    for task in tasks {
        let Some(group_no) = task
            .task_group_no
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
        else {
            continue;
        };
        if is_pending_execute_status(task.task_status.as_deref()) {
            continue;
        }

        let services = grouped_services.entry(group_no.to_string()).or_default();
        let mut has_service_items = false;
        let mut used_service_item = false;
        if let Some(task_id) = task.id
            && let Some(items) = service_items_by_task_id.get(&task_id)
        {
            has_service_items = true;
            for item in items {
                if is_pending_execute_status(item.service_status.as_deref()) {
                    continue;
                }
                if let Some(service_type) = normalized_service_type(
                    item.service_type
                        .as_deref()
                        .or(task.service_type.as_deref()),
                ) {
                    services.insert(service_type);
                    used_service_item = true;
                }
            }
        }

        if !has_service_items
            && !used_service_item
            && let Some(service_type) = normalized_service_type(task.service_type.as_deref())
        {
            services.insert(service_type);
        }
    }
    grouped_services
        .into_iter()
        .map(|(group_no, services)| (group_no, services.len()))
        .collect()
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
    group_service_counts: &std::collections::HashMap<String, usize>,
) -> bool {
    task_type_filters.is_empty()
        || task_type_filters
            .iter()
            .any(|task_type| matches_task_type_filter(task, task_type, group_service_counts))
}

fn matches_task_type_filter(
    task: &TaskEntity,
    task_type: &str,
    group_service_counts: &std::collections::HashMap<String, usize>,
) -> bool {
    let grouped_count = task
        .task_group_no
        .as_deref()
        .map(str::trim)
        .filter(|group_no| !group_no.is_empty())
        .and_then(|group_no| group_service_counts.get(group_no))
        .copied();
    match task_type {
        "three_special" => grouped_count
            .map(|count| count == 3)
            .unwrap_or_else(|| task.task_type.as_deref() == Some("three_special")),
        "four_special" => grouped_count
            .map(|count| count == 4)
            .unwrap_or_else(|| task.task_type.as_deref() == Some("four_special")),
        _ => task.task_type.as_deref() == Some(task_type),
    }
}

fn is_pending_execute_status(status: Option<&str>) -> bool {
    matches!(
        status.map(str::trim),
        Some("pending_assign") | Some("pending_execute")
    )
}

fn normalize_review_task_status(status: &str) -> &str {
    if status == "pending_hq_score" {
        "completed"
    } else {
        status
    }
}

fn normalize_optional_review_task_status(status: Option<&str>) -> Option<&str> {
    status.map(normalize_review_task_status)
}

fn matches_review_task_status_filter(actual: Option<&str>, expected: &str) -> bool {
    normalize_optional_review_task_status(actual) == Some(normalize_review_task_status(expected))
}

fn matches_review_task_status_query(actual: Option<&str>, expected: &str) -> bool {
    super::multi_query_value_matches(Some(expected), |value| {
        matches_review_task_status_filter(actual, value)
    })
}

fn normalized_service_type(service_type: Option<&str>) -> Option<String> {
    service_type
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
}

fn sort_review_task_items(items: &mut [ReviewTaskItem]) {
    items.sort_by(|left, right| {
        right
            .created_at_sort
            .cmp(&left.created_at_sort)
            .then_with(|| right.task_id.cmp(&left.task_id))
            .then_with(|| left.service_sort_no.cmp(&right.service_sort_no))
    });
}

fn service_type_sort_key(service_type: &str) -> i32 {
    match service_type {
        "measure" => 1,
        "install" => 2,
        "ironing" => 3,
        "cleaning" => 4,
        _ => 99,
    }
}

fn matches_review_task_query(
    item: &ReviewTaskItem,
    task: &TaskEntity,
    service_item: &TaskServiceItemEntity,
    query: &ReviewTaskQuery,
) -> Result<bool, AppError> {
    if !matches_review_status_query(item, query.review_status.as_deref()) {
        return Ok(false);
    }
    if !matches_review_score_status_query(item, query.score_status.as_deref()) {
        return Ok(false);
    }
    if let Some(mobile) = &query.mobile
        && !item.customer_mobile.contains(mobile)
    {
        return Ok(false);
    }
    if let Some(customer_name) = &query.customer_name
        && !item.customer_name.contains(customer_name)
    {
        return Ok(false);
    }
    if let Some(technician_name) = &query.technician_name
        && !item.technician_name.contains(technician_name)
    {
        return Ok(false);
    }
    if let Some(appointment_start) = &query.appointment_start {
        let Ok(start_at) =
            chrono::NaiveDateTime::parse_from_str(appointment_start, "%Y-%m-%d %H:%M:%S")
        else {
            return Ok(false);
        };
        let actual_time = service_item.scheduled_time.or(task.appointment_time);
        if actual_time.map(|dt| dt < start_at).unwrap_or(true) {
            return Ok(false);
        }
    }
    if let Some(appointment_end) = &query.appointment_end {
        let Ok(end_at) =
            chrono::NaiveDateTime::parse_from_str(appointment_end, "%Y-%m-%d %H:%M:%S")
        else {
            return Ok(false);
        };
        let actual_time = service_item.scheduled_time.or(task.appointment_time);
        if actual_time.map(|dt| dt > end_at).unwrap_or(true) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefetched_dealer_review_prefers_service_item_then_service_type() {
        let reviews = vec![
            DealerReviewEntity {
                id: Some(1),
                service_item_id: Some(101),
                service_type: Some("install".to_string()),
                ..Default::default()
            },
            DealerReviewEntity {
                id: Some(2),
                service_item_id: Some(102),
                service_type: Some("measure".to_string()),
                ..Default::default()
            },
        ];
        let by_id = TaskServiceItemEntity {
            id: Some(102),
            service_type: Some("install".to_string()),
            ..Default::default()
        };
        let by_type = TaskServiceItemEntity {
            id: Some(999),
            service_type: Some("install".to_string()),
            ..Default::default()
        };

        assert_eq!(
            find_prefetched_dealer_review(&reviews, &by_id).and_then(|item| item.id),
            Some(2)
        );
        assert_eq!(
            find_prefetched_dealer_review(&reviews, &by_type).and_then(|item| item.id),
            Some(1)
        );
    }

    #[test]
    fn special_task_type_counts_only_non_pending_execute_service_types() {
        let tasks = vec![
            review_task(1, "GROUP-1", "measure", "pending_hq_score"),
            review_task(2, "GROUP-1", "install", "pending_hq_score"),
            review_task(3, "GROUP-1", "cleaning", "pending_hq_score"),
            review_task(4, "GROUP-1", "ironing", "pending_execute"),
        ];
        let service_items_by_task_id = std::collections::HashMap::from([
            (
                1,
                vec![
                    review_service_item(1, "measure", "pending_hq_score"),
                    review_service_item(1, "measure", "pending_hq_score"),
                ],
            ),
            (2, vec![review_service_item(2, "install", "completed")]),
            (3, vec![review_service_item(3, "cleaning", "completed")]),
            (
                4,
                vec![review_service_item(4, "ironing", "pending_execute")],
            ),
        ]);

        let counts = build_task_group_service_counts(&tasks, &service_items_by_task_id);

        assert_eq!(counts.get("GROUP-1"), Some(&3));
        assert!(matches_task_type_filter(
            &tasks[0],
            "three_special",
            &counts
        ));
        assert!(!matches_task_type_filter(
            &tasks[0],
            "four_special",
            &counts
        ));
    }

    #[test]
    fn special_task_type_falls_back_to_legacy_task_type_without_group() {
        let task = TaskEntity {
            task_type: Some("four_special".to_string()),
            ..Default::default()
        };
        let counts = std::collections::HashMap::new();

        assert!(matches_task_type_filter(&task, "four_special", &counts));
        assert!(!matches_task_type_filter(&task, "three_special", &counts));
    }

    #[test]
    fn review_list_status_and_type_filters_accept_multiple_values() {
        assert!(matches_review_task_status_query(
            Some("pending_hq_score"),
            "pending_execute,completed"
        ));
        assert!(!matches_review_task_status_query(
            Some("closed"),
            "pending_execute,completed"
        ));
        assert_eq!(
            normalized_task_type_filters(Some(r#"["three_special","four_special"]"#)).unwrap(),
            vec!["three_special", "four_special"]
        );
    }

    fn review_task(
        id: i64,
        task_group_no: &str,
        service_type: &str,
        task_status: &str,
    ) -> TaskEntity {
        TaskEntity {
            id: Some(id),
            task_group_no: Some(task_group_no.to_string()),
            service_type: Some(service_type.to_string()),
            task_status: Some(task_status.to_string()),
            ..Default::default()
        }
    }

    fn review_service_item(
        task_id: i64,
        service_type: &str,
        service_status: &str,
    ) -> TaskServiceItemEntity {
        TaskServiceItemEntity {
            task_id: Some(task_id),
            service_type: Some(service_type.to_string()),
            service_status: Some(service_status.to_string()),
            ..Default::default()
        }
    }
}
