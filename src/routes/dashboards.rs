use anyhow::Context;
use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use chrono::{Datelike, Local, NaiveDateTime, Utc};
use rbs::value;
use serde::Serialize;

use crate::{
    auth, db,
    error::AppError,
    models::{
        customer::CustomerEntity, customer_address::CustomerAddressEntity, hq_score::HqScoreEntity,
        hq_score_item::HqScoreItemEntity, task::TaskEntity,
        task_service_item::TaskServiceItemEntity, user::UserEntity, user_store::UserStoreEntity,
    },
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Serialize)]
pub struct DashboardResponse {
    pub pending_task_count: usize,
    pub pending_review_task_count: usize,
    pub month_completed_count: usize,
    pub recent_tasks: Vec<DashboardTaskItem>,
    pub total_tasks: usize,
    pub pending_tasks: usize,
    pub executing_tasks: usize,
    pub completed_tasks: usize,
    pub average_score: f64,
    pub service_stats: Vec<ServiceDashboardStat>,
    pub service_trends: Vec<ServiceTrendPoint>,
    pub service_group_counts: Vec<ServiceGroupCount>,
}

#[derive(Debug, Serialize)]
pub struct DashboardTaskItem {
    pub id: i64,
    pub service_item_id: Option<i64>,
    pub task_no: String,
    pub customer_name: String,
    pub customer_mobile: String,
    pub address: String,
    pub service_type: String,
    pub service_items_text: String,
    pub task_status: String,
    pub service_status: Option<String>,
    pub task_status_text: String,
    pub display_status: String,
    pub display_status_text: String,
    pub technician_name: Option<String>,
    pub appointment_time: Option<String>,
    pub progress_percent: i32,
    pub is_overdue: bool,
}

#[derive(Debug, Serialize)]
pub struct ServiceDashboardStat {
    pub service_type: String,
    pub total_count: usize,
    pub completed_count: usize,
    pub completion_rate: f64,
    pub average_score: f64,
}

#[derive(Debug, Serialize)]
pub struct ServiceTrendPoint {
    pub period: String,
    pub measure: usize,
    pub install: usize,
    pub ironing: usize,
    pub cleaning: usize,
}

#[derive(Debug, Serialize)]
pub struct ServiceGroupCount {
    pub service_type: String,
    pub count: usize,
}

#[derive(Debug, serde::Deserialize)]
pub struct DashboardQuery {
    pub year: Option<i32>,
    pub month: Option<u32>,
}

#[derive(Clone)]
struct TechnicianDashboardTask {
    task: TaskEntity,
    service_item: Option<TaskServiceItemEntity>,
}

pub async fn dealer_dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<DashboardQuery>,
) -> Result<Json<ApiResponse<DashboardResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "super_admin"])?;

    let store_relations: Vec<UserStoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_store",
        value! { "user_id": auth_user.user_id },
    )
    .await
    .context("failed to query dealer store relations")
    .map_err(AppError::Internal)?;
    let store_ids: Vec<i64> = store_relations
        .into_iter()
        .filter_map(|item| item.store_id)
        .collect();
    let items = load_dealer_dashboard_tasks(&state, store_ids).await?;
    let filtered_items = filter_technician_tasks_by_year_month(items, query.year, query.month);
    let mut dashboard = build_technician_dashboard(&filtered_items, query.year, query.month);
    dashboard.recent_tasks = enrich_technician_recent_tasks(&state, &filtered_items).await?;
    dashboard.service_group_counts = build_technician_service_group_counts(&filtered_items);
    Ok(Json(ApiResponse::success(dashboard)))
}

pub async fn technician_dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<DashboardQuery>,
) -> Result<Json<ApiResponse<DashboardResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["technician"])?;

    let items = load_technician_dashboard_tasks(&state, auth_user.user_id).await?;
    let filtered_items = filter_technician_tasks_by_year_month(items, query.year, query.month);
    let mut dashboard = build_technician_dashboard(&filtered_items, query.year, query.month);
    dashboard.recent_tasks = enrich_technician_recent_tasks(&state, &filtered_items).await?;
    dashboard.service_group_counts = build_technician_service_group_counts(&filtered_items);
    Ok(Json(ApiResponse::success(dashboard)))
}

pub async fn hq_dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<DashboardQuery>,
) -> Result<Json<ApiResponse<DashboardResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["hq_reviewer", "super_admin"])?;

    let tasks: Vec<TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! {})
        .await
        .context("failed to query hq dashboard tasks")
        .map_err(AppError::Internal)?;
    let scores: Vec<HqScoreEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "hq_score", value! {})
            .await
            .context("failed to query hq dashboard scores")
            .map_err(AppError::Internal)?;
    let score_items: Vec<HqScoreItemEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "hq_score_item", value! {})
            .await
            .context("failed to query hq dashboard score items")
            .map_err(AppError::Internal)?;
    let service_items: Vec<TaskServiceItemEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "task_service_item", value! {})
            .await
            .context("failed to query hq dashboard service items")
            .map_err(AppError::Internal)?;

    let filtered_tasks = filter_tasks_by_year_month(tasks, query.year, query.month);
    let filtered_service_items =
        filter_service_items_by_year_month(service_items, query.year, query.month);
    let mut dashboard = build_dashboard(
        filtered_tasks.clone(),
        scores,
        filtered_service_items.clone(),
        query.year,
        query.month,
    );
    dashboard.recent_tasks = enrich_recent_tasks(&state, &filtered_tasks).await?;
    dashboard.service_stats = build_service_stats(filtered_service_items.clone(), score_items);
    dashboard.service_trends = build_service_trends(filtered_service_items);
    dashboard.service_group_counts = build_service_group_counts(&filtered_tasks);
    Ok(Json(ApiResponse::success(dashboard)))
}

async fn load_dealer_dashboard_tasks(
    state: &AppState,
    store_ids: Vec<i64>,
) -> Result<Vec<TechnicianDashboardTask>, AppError> {
    if store_ids.is_empty() {
        return Ok(Vec::new());
    }
    let tasks: Vec<TaskEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task",
        value! { "store_id": store_ids },
    )
    .await
    .context("failed to query dealer dashboard tasks")
    .map_err(AppError::Internal)?;

    let mut items = Vec::new();
    for task in tasks {
        let Some(task_id) = task.id else {
            continue;
        };
        let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "task_service_item",
            value! { "task_id": task_id },
        )
        .await
        .context("failed to query dealer dashboard service items")
        .map_err(AppError::Internal)?;
        if service_items.is_empty() {
            items.push(TechnicianDashboardTask {
                task,
                service_item: None,
            });
            continue;
        }
        for service_item in service_items {
            items.push(TechnicianDashboardTask {
                task: task.clone(),
                service_item: Some(service_item),
            });
        }
    }

    Ok(items)
}

async fn load_technician_dashboard_tasks(
    state: &AppState,
    technician_id: i64,
) -> Result<Vec<TechnicianDashboardTask>, AppError> {
    let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_service_item",
        value! { "technician_id": technician_id },
    )
    .await
    .context("failed to query technician dashboard service items")
    .map_err(AppError::Internal)?;

    let service_item_task_ids = service_items
        .iter()
        .filter_map(|item| item.task_id)
        .collect::<std::collections::BTreeSet<_>>();
    let tasks_by_id = load_dashboard_tasks_by_ids(
        state,
        service_item_task_ids.iter().copied().collect::<Vec<_>>(),
    )
    .await?;

    let mut items = Vec::new();
    for service_item in service_items {
        let Some(task_id) = service_item.task_id else {
            continue;
        };
        let Some(task) = tasks_by_id.get(&task_id) else {
            continue;
        };
        items.push(TechnicianDashboardTask {
            task: task.clone(),
            service_item: Some(service_item),
        });
    }

    let legacy_tasks: Vec<TaskEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task",
        value! { "current_technician_id": technician_id },
    )
    .await
    .context("failed to query technician dashboard legacy tasks")
    .map_err(AppError::Internal)?;
    let legacy_task_ids = legacy_tasks
        .iter()
        .filter_map(|item| item.id)
        .collect::<Vec<_>>();
    let task_ids_with_any_service_items =
        load_task_ids_with_service_items(state, &legacy_task_ids).await?;

    for task in legacy_tasks {
        let Some(task_id) = task.id else {
            continue;
        };
        if service_item_task_ids.contains(&task_id)
            || task_ids_with_any_service_items.contains(&task_id)
        {
            continue;
        }
        items.push(TechnicianDashboardTask {
            task,
            service_item: None,
        });
    }

    Ok(items)
}

async fn load_dashboard_tasks_by_ids(
    state: &AppState,
    task_ids: Vec<i64>,
) -> Result<std::collections::BTreeMap<i64, TaskEntity>, AppError> {
    if task_ids.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }
    let tasks: Vec<TaskEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task",
        value! { "id": task_ids },
    )
    .await
    .context("failed to query technician dashboard item tasks")
    .map_err(AppError::Internal)?;
    Ok(tasks
        .into_iter()
        .filter_map(|item| item.id.map(|id| (id, item)))
        .collect())
}

async fn load_task_ids_with_service_items(
    state: &AppState,
    task_ids: &[i64],
) -> Result<std::collections::BTreeSet<i64>, AppError> {
    if task_ids.is_empty() {
        return Ok(std::collections::BTreeSet::new());
    }
    let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_service_item",
        value! { "task_id": task_ids.to_vec() },
    )
    .await
    .context("failed to query technician dashboard legacy service items")
    .map_err(AppError::Internal)?;
    Ok(service_items
        .into_iter()
        .filter_map(|item| item.task_id)
        .collect())
}

fn build_dashboard(
    tasks: Vec<TaskEntity>,
    scores: Vec<HqScoreEntity>,
    _service_items: Vec<TaskServiceItemEntity>,
    year: Option<i32>,
    month: Option<u32>,
) -> DashboardResponse {
    let now = Utc::now();
    let target_year = year.unwrap_or(now.year());
    let target_month = month.unwrap_or(now.month());
    let total_tasks = tasks.len();
    let pending_tasks = tasks
        .iter()
        .filter(|item| {
            matches!(
                item.task_status.as_deref(),
                Some("pending_assign")
                    | Some("pending_execute")
                    | Some("pending_dealer_review")
                    | Some("pending_hq_score")
            )
        })
        .count();
    let executing_tasks = tasks
        .iter()
        .filter(|item| item.task_status.as_deref() == Some("executing"))
        .count();
    let completed_tasks = tasks
        .iter()
        .filter(|item| item.task_status.as_deref() == Some("completed"))
        .count();
    let pending_task_count = tasks
        .iter()
        .filter(|item| {
            matches!(
                item.task_status.as_deref(),
                Some("pending_execute") | Some("executing") | Some("pending_dealer_review")
            )
        })
        .count();
    let pending_review_task_count = tasks
        .iter()
        .filter(|item| item.task_status.as_deref() == Some("pending_dealer_review"))
        .count();
    let month_completed_count = tasks
        .iter()
        .filter(|item| {
            item.task_status.as_deref() == Some("completed")
                && item
                    .updated_at
                    .map(|dt| dt.month() == target_month && dt.year() == target_year)
                    .unwrap_or(false)
        })
        .count();
    let average_score = if scores.is_empty() {
        0.0
    } else {
        scores
            .iter()
            .map(|item| item.total_score.unwrap_or_default() as f64)
            .sum::<f64>()
            / scores.len() as f64
    };

    DashboardResponse {
        pending_task_count,
        pending_review_task_count,
        month_completed_count,
        recent_tasks: Vec::new(),
        total_tasks,
        pending_tasks,
        executing_tasks,
        completed_tasks,
        average_score,
        service_stats: Vec::new(),
        service_trends: Vec::new(),
        service_group_counts: Vec::new(),
    }
}

fn build_technician_dashboard(
    items: &[TechnicianDashboardTask],
    year: Option<i32>,
    month: Option<u32>,
) -> DashboardResponse {
    let now = Utc::now();
    let target_year = year.unwrap_or(now.year());
    let target_month = month.unwrap_or(now.month());
    let total_tasks = items.len();
    let pending_tasks = items
        .iter()
        .filter(|item| {
            matches!(
                technician_task_status(item),
                Some("pending_assign")
                    | Some("pending_execute")
                    | Some("pending_dealer_review")
                    | Some("pending_hq_score")
            )
        })
        .count();
    let executing_tasks = items
        .iter()
        .filter(|item| technician_task_status(item) == Some("executing"))
        .count();
    let completed_tasks = items
        .iter()
        .filter(|item| technician_task_status(item) == Some("completed"))
        .count();
    let pending_task_count = items
        .iter()
        .filter(|item| {
            matches!(
                technician_task_status(item),
                Some("pending_execute") | Some("executing") | Some("pending_dealer_review")
            )
        })
        .count();
    let pending_review_task_count = items
        .iter()
        .filter(|item| technician_task_status(item) == Some("pending_dealer_review"))
        .count();
    let month_completed_count = items
        .iter()
        .filter(|item| {
            technician_task_status(item) == Some("completed")
                && technician_task_completed_at(item)
                    .map(|dt| dt.month() == target_month && dt.year() == target_year)
                    .unwrap_or(false)
        })
        .count();

    DashboardResponse {
        pending_task_count,
        pending_review_task_count,
        month_completed_count,
        recent_tasks: Vec::new(),
        total_tasks,
        pending_tasks,
        executing_tasks,
        completed_tasks,
        average_score: 0.0,
        service_stats: Vec::new(),
        service_trends: Vec::new(),
        service_group_counts: Vec::new(),
    }
}

fn filter_technician_tasks_by_year_month(
    items: Vec<TechnicianDashboardTask>,
    year: Option<i32>,
    month: Option<u32>,
) -> Vec<TechnicianDashboardTask> {
    if year.is_none() && month.is_none() {
        return items;
    }
    let now = Utc::now();
    let target_year = year.unwrap_or(now.year());
    let target_month = month.unwrap_or(now.month());
    items
        .into_iter()
        .filter(|item| {
            technician_task_scheduled_at(item)
                .map(|dt| dt.year() == target_year && dt.month() == target_month)
                .unwrap_or(false)
        })
        .collect()
}

fn filter_tasks_by_year_month(
    tasks: Vec<TaskEntity>,
    year: Option<i32>,
    month: Option<u32>,
) -> Vec<TaskEntity> {
    if year.is_none() && month.is_none() {
        return tasks;
    }
    let now = Utc::now();
    let target_year = year.unwrap_or(now.year());
    let target_month = month.unwrap_or(now.month());
    tasks
        .into_iter()
        .filter(|item| {
            item.appointment_time
                .map(|dt| dt.year() == target_year && dt.month() == target_month)
                .unwrap_or(false)
        })
        .collect()
}

fn filter_service_items_by_year_month(
    items: Vec<TaskServiceItemEntity>,
    year: Option<i32>,
    month: Option<u32>,
) -> Vec<TaskServiceItemEntity> {
    if year.is_none() && month.is_none() {
        return items;
    }
    let now = Utc::now();
    let target_year = year.unwrap_or(now.year());
    let target_month = month.unwrap_or(now.month());
    items
        .into_iter()
        .filter(|item| {
            item.scheduled_time
                .map(|dt| dt.year() == target_year && dt.month() == target_month)
                .unwrap_or(false)
        })
        .collect()
}

fn build_service_stats(
    service_items: Vec<TaskServiceItemEntity>,
    score_items: Vec<HqScoreItemEntity>,
) -> Vec<ServiceDashboardStat> {
    let service_types = ["measure", "install", "ironing", "cleaning"];
    service_types
        .iter()
        .map(|service_type| {
            let items = service_items
                .iter()
                .filter(|item| item.service_type.as_deref() == Some(*service_type))
                .collect::<Vec<_>>();
            let total_count = items.len();
            let completed_count = items
                .iter()
                .filter(|item| {
                    matches!(
                        item.service_status.as_deref(),
                        Some("completed") | Some("closed")
                    )
                })
                .count();
            let completion_rate = if total_count == 0 {
                0.0
            } else {
                (completed_count as f64 / total_count as f64) * 100.0
            };
            let average_score = if items.is_empty() {
                0.0
            } else {
                let service_scores = score_items
                    .iter()
                    .filter(|score_item| score_item.service_type.as_deref() == Some(*service_type))
                    .collect::<Vec<_>>();
                if service_scores.is_empty() {
                    0.0
                } else {
                    service_scores
                        .iter()
                        .map(|item| item.score.unwrap_or_default() as f64)
                        .sum::<f64>()
                        / service_scores.len() as f64
                }
            };

            ServiceDashboardStat {
                service_type: (*service_type).to_string(),
                total_count,
                completed_count,
                completion_rate,
                average_score,
            }
        })
        .collect()
}

fn build_service_trends(service_items: Vec<TaskServiceItemEntity>) -> Vec<ServiceTrendPoint> {
    let now = Utc::now();
    let mut points = Vec::new();
    for offset in (0..7).rev() {
        let day = now - chrono::Duration::days(offset);
        let period = day.format("%m-%d").to_string();
        let mut point = ServiceTrendPoint {
            period,
            measure: 0,
            install: 0,
            ironing: 0,
            cleaning: 0,
        };
        for item in &service_items {
            let same_day = item
                .created_at
                .map(|dt| dt.date() == day.naive_utc().date())
                .unwrap_or(false);
            if !same_day {
                continue;
            }
            match item.service_type.as_deref() {
                Some("measure") => point.measure += 1,
                Some("install") => point.install += 1,
                Some("ironing") => point.ironing += 1,
                Some("cleaning") => point.cleaning += 1,
                _ => {}
            }
        }
        points.push(point);
    }
    points
}

async fn enrich_recent_tasks(
    state: &AppState,
    tasks: &[TaskEntity],
) -> Result<Vec<DashboardTaskItem>, AppError> {
    let mut sorted = tasks.to_vec();
    sorted.sort_by_key(|item| item.appointment_time);
    sorted.reverse();

    let mut items = Vec::new();
    for task in sorted.into_iter().take(5) {
        let customer = db::select_one_by_map_compat::<CustomerEntity>(
            state.rb.as_ref(),
            "customer",
            value! { "id": task.customer_id.unwrap_or_default() },
        )
        .await
        .context("failed to query dashboard customer")
        .map_err(AppError::Internal)?
        .unwrap_or_default();
        let address = db::select_one_by_map_compat::<CustomerAddressEntity>(
            state.rb.as_ref(),
            "customer_address",
            value! { "id": task.customer_address_id.unwrap_or_default() },
        )
        .await
        .context("failed to query dashboard address")
        .map_err(AppError::Internal)?
        .unwrap_or_default();
        let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "task_service_item",
            value! { "task_id": task.id.unwrap_or_default() },
        )
        .await
        .context("failed to query dashboard service items")
        .map_err(AppError::Internal)?;
        let technician_name = if let Some(technician_id) = task.current_technician_id {
            db::select_one_by_map_compat::<UserEntity>(
                state.rb.as_ref(),
                "user",
                value! { "id": technician_id },
            )
            .await
            .context("failed to query dashboard technician")
            .map_err(AppError::Internal)?
            .and_then(|item| item.name)
        } else {
            None
        };
        let progress_percent = progress_percent_for_status(task.task_status.as_deref());
        let task_status = task.task_status.clone().unwrap_or_default();
        let service_type = task.service_type.clone().unwrap_or_default();
        let is_overdue = task_is_overdue(&task);
        items.push(DashboardTaskItem {
            id: task.id.unwrap_or_default(),
            service_item_id: None,
            task_no: task.task_no.unwrap_or_default(),
            customer_name: customer.name.unwrap_or_default(),
            customer_mobile: customer.mobile.unwrap_or_default(),
            address: address.address.unwrap_or_default(),
            service_type,
            service_items_text: service_items_text(&service_items),
            display_status: display_status(&task_status).to_string(),
            display_status_text: display_status_text(&task_status).to_string(),
            task_status_text: task_status_text(&task_status).to_string(),
            technician_name,
            task_status,
            service_status: None,
            appointment_time: task
                .appointment_time
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string()),
            progress_percent,
            is_overdue,
        });
    }

    Ok(items)
}

async fn enrich_technician_recent_tasks(
    state: &AppState,
    tasks: &[TechnicianDashboardTask],
) -> Result<Vec<DashboardTaskItem>, AppError> {
    let mut sorted = tasks.to_vec();
    sorted.sort_by_key(technician_task_scheduled_at);
    sorted.reverse();

    let mut items = Vec::new();
    for item in sorted.into_iter().take(5) {
        let task = &item.task;
        let customer = db::select_one_by_map_compat::<CustomerEntity>(
            state.rb.as_ref(),
            "customer",
            value! { "id": task.customer_id.unwrap_or_default() },
        )
        .await
        .context("failed to query technician dashboard customer")
        .map_err(AppError::Internal)?
        .unwrap_or_default();
        let address = db::select_one_by_map_compat::<CustomerAddressEntity>(
            state.rb.as_ref(),
            "customer_address",
            value! { "id": task.customer_address_id.unwrap_or_default() },
        )
        .await
        .context("failed to query technician dashboard address")
        .map_err(AppError::Internal)?
        .unwrap_or_default();
        let technician_id = item
            .service_item
            .as_ref()
            .and_then(|service_item| service_item.technician_id)
            .or(task.current_technician_id);
        let technician_name = if let Some(technician_id) = technician_id {
            db::select_one_by_map_compat::<UserEntity>(
                state.rb.as_ref(),
                "user",
                value! { "id": technician_id },
            )
            .await
            .context("failed to query technician dashboard technician")
            .map_err(AppError::Internal)?
            .and_then(|item| item.name)
        } else {
            None
        };
        let task_status = technician_task_status(&item).unwrap_or_default().to_string();
        let service_type = technician_task_service_type(&item)
            .unwrap_or_default()
            .to_string();
        let service_items_text = service_type_label(&service_type).to_string();

        items.push(DashboardTaskItem {
            id: task.id.unwrap_or_default(),
            service_item_id: item.service_item.as_ref().and_then(|service_item| service_item.id),
            task_no: task.task_no.clone().unwrap_or_default(),
            customer_name: customer.name.unwrap_or_default(),
            customer_mobile: customer.mobile.unwrap_or_default(),
            address: address.address.unwrap_or_default(),
            service_type,
            service_items_text,
            display_status: display_status(&task_status).to_string(),
            display_status_text: display_status_text(&task_status).to_string(),
            task_status_text: task_status_text(&task_status).to_string(),
            technician_name,
            task_status,
            service_status: item
                .service_item
                .as_ref()
                .and_then(|service_item| service_item.service_status.clone()),
            appointment_time: technician_task_scheduled_at(&item)
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string()),
            progress_percent: technician_task_progress_percent(&item),
            is_overdue: technician_task_is_overdue(&item),
        });
    }

    Ok(items)
}

fn build_service_group_counts(tasks: &[TaskEntity]) -> Vec<ServiceGroupCount> {
    let mut map = std::collections::BTreeMap::<String, usize>::new();
    for task in tasks {
        if let Some(service_type) = &task.service_type {
            *map.entry(service_type.clone()).or_default() += 1;
        }
    }
    map.into_iter()
        .map(|(service_type, count)| ServiceGroupCount {
            service_type,
            count,
        })
        .collect()
}

fn build_technician_service_group_counts(
    items: &[TechnicianDashboardTask],
) -> Vec<ServiceGroupCount> {
    let mut map = std::collections::BTreeMap::<String, usize>::new();
    for item in items {
        if let Some(service_type) = technician_task_service_type(item) {
            *map.entry(service_type.to_string()).or_default() += 1;
        }
    }
    map.into_iter()
        .map(|(service_type, count)| ServiceGroupCount {
            service_type,
            count,
        })
        .collect()
}

fn technician_task_status(item: &TechnicianDashboardTask) -> Option<&str> {
    item.service_item
        .as_ref()
        .and_then(|service_item| service_item.service_status.as_deref())
        .or(item.task.task_status.as_deref())
}

fn technician_task_service_type(item: &TechnicianDashboardTask) -> Option<&str> {
    item.service_item
        .as_ref()
        .and_then(|service_item| service_item.service_type.as_deref())
        .or(item.task.service_type.as_deref())
}

fn technician_task_scheduled_at(item: &TechnicianDashboardTask) -> Option<NaiveDateTime> {
    item.service_item
        .as_ref()
        .and_then(|service_item| service_item.scheduled_time)
        .or(item.task.appointment_time)
}

fn technician_task_completed_at(item: &TechnicianDashboardTask) -> Option<NaiveDateTime> {
    item.service_item
        .as_ref()
        .and_then(|service_item| service_item.completed_at)
        .or(item.task.updated_at)
}

fn technician_task_progress_percent(item: &TechnicianDashboardTask) -> i32 {
    item.service_item
        .as_ref()
        .and_then(|service_item| service_item.progress_percent)
        .unwrap_or_else(|| progress_percent_for_status(technician_task_status(item)))
}

fn technician_task_is_overdue(item: &TechnicianDashboardTask) -> bool {
    if matches!(technician_task_status(item), Some("completed") | Some("closed")) {
        return false;
    }
    item.task.is_overdue == Some(1)
        || technician_task_scheduled_at(item)
            .map(|dt| dt < Local::now().naive_local())
            .unwrap_or(false)
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

fn service_items_text(items: &[TaskServiceItemEntity]) -> String {
    items
        .iter()
        .filter_map(|item| item.service_type.as_deref())
        .map(service_type_label)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("、")
}

fn service_type_label(service_type: &str) -> &'static str {
    match service_type {
        "measure" => "量尺",
        "install" => "安装",
        "ironing" => "熨烫",
        "cleaning" => "除尘除螨",
        _ => "服务",
    }
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

fn task_is_overdue(task: &TaskEntity) -> bool {
    task.is_overdue == Some(1)
        || (task
            .appointment_time
            .map(|dt| dt < Local::now().naive_local())
            .unwrap_or(false)
            && !matches!(
                task.task_status.as_deref(),
                Some("completed") | Some("closed")
            ))
}
