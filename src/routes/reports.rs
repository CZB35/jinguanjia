use anyhow::Context;
use axum::{
    Json,
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use rbs::value;
use serde::Serialize;

use crate::{
    auth, db,
    error::AppError,
    models::{hq_score::HqScoreEntity, task::TaskEntity},
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Serialize)]
pub struct OverviewReport {
    pub total_tasks: usize,
    pub completed_tasks: usize,
    pub pending_review_tasks: usize,
    pub pending_score_tasks: usize,
}

#[derive(Debug, Serialize)]
pub struct TechnicianPerformanceItem {
    pub technician_id: i64,
    pub total_tasks: usize,
    pub completed_tasks: usize,
}

#[derive(Debug, Serialize)]
pub struct PerformanceReport {
    pub items: Vec<TechnicianPerformanceItem>,
}

#[derive(Debug, Serialize)]
pub struct QualityDistributionReport {
    pub excellent: usize,
    pub good: usize,
    pub pass: usize,
    pub improve: usize,
}

pub async fn overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<OverviewReport>>, AppError> {
    ensure_hq_access(&state, &headers)?;
    let tasks = all_tasks(&state).await?;
    let total_tasks = tasks.len();
    let completed_tasks = tasks
        .iter()
        .filter(|item| item.task_status.as_deref() == Some("completed"))
        .count();
    let pending_review_tasks = tasks
        .iter()
        .filter(|item| item.task_status.as_deref() == Some("pending_dealer_review"))
        .count();
    let pending_score_tasks = tasks
        .iter()
        .filter(|item| item.task_status.as_deref() == Some("pending_hq_score"))
        .count();

    Ok(Json(ApiResponse::success(OverviewReport {
        total_tasks,
        completed_tasks,
        pending_review_tasks,
        pending_score_tasks,
    })))
}

pub async fn performance(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<PerformanceReport>>, AppError> {
    ensure_hq_access(&state, &headers)?;
    let tasks = all_tasks(&state).await?;
    let mut map = std::collections::BTreeMap::<i64, TechnicianPerformanceItem>::new();

    for task in tasks {
        if let Some(technician_id) = task.current_technician_id {
            let entry = map
                .entry(technician_id)
                .or_insert(TechnicianPerformanceItem {
                    technician_id,
                    total_tasks: 0,
                    completed_tasks: 0,
                });
            entry.total_tasks += 1;
            if task.task_status.as_deref() == Some("completed") {
                entry.completed_tasks += 1;
            }
        }
    }

    Ok(Json(ApiResponse::success(PerformanceReport {
        items: map.into_values().collect(),
    })))
}

pub async fn quality_distribution(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<QualityDistributionReport>>, AppError> {
    ensure_hq_access(&state, &headers)?;
    let scores: Vec<HqScoreEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "hq_score", value! {})
            .await
            .context("failed to query hq scores")
            .map_err(AppError::Internal)?;

    let mut report = QualityDistributionReport {
        excellent: 0,
        good: 0,
        pass: 0,
        improve: 0,
    };
    for score in scores {
        match score.service_level.as_deref() {
            Some("excellent") => report.excellent += 1,
            Some("good") => report.good += 1,
            Some("pass") => report.pass += 1,
            Some("improve") => report.improve += 1,
            _ => {}
        }
    }

    Ok(Json(ApiResponse::success(report)))
}

pub async fn export(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    ensure_hq_access(&state, &headers)?;
    let tasks = all_tasks(&state).await?;

    let mut csv = String::from("\u{feff}任务ID,任务编号,服务类型,任务状态,师傅ID\n");
    for task in tasks {
        csv.push_str(&format!(
            "{},{},{},{},{}\n",
            task.id.unwrap_or_default(),
            task.task_no.unwrap_or_default(),
            report_service_type_text(task.service_type.as_deref()),
            report_task_status_text(task.task_status.as_deref()),
            task.current_technician_id.unwrap_or_default()
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
        HeaderValue::from_static("attachment; filename=\"总部报表导出.csv\""),
    );
    Ok(response)
}

fn report_service_type_text(service_type: Option<&str>) -> &'static str {
    match service_type.unwrap_or_default() {
        "measure" => "量尺服务",
        "install" => "安装服务",
        "ironing" => "熨烫服务",
        "cleaning" => "除尘除螨服务",
        _ => "其他服务",
    }
}

fn report_task_status_text(status: Option<&str>) -> &'static str {
    match status.unwrap_or_default() {
        "pending_assign" => "待分派",
        "pending_execute" => "待执行",
        "executing" => "进行中",
        "pending_dealer_review" => "待评价",
        "pending_hq_score" => "待评分",
        "completed" => "已完成",
        "closed" => "已关闭",
        _ => "未知状态",
    }
}

fn ensure_hq_access(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let auth_user = auth::authenticate(state, headers)?;
    auth_user.require_any_role(&["hq_reviewer", "super_admin"])
}

async fn all_tasks(state: &AppState) -> Result<Vec<TaskEntity>, AppError> {
    let tasks: Vec<TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! {})
        .await
        .context("failed to query tasks for report")
        .map_err(AppError::Internal)?;
    Ok(tasks
        .into_iter()
        .filter(|task| task.deleted_at.is_none())
        .collect())
}
