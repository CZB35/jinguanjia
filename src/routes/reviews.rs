use anyhow::{Context, anyhow};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    audit,
    auth,
    db,
    error::AppError,
    models::{
        dealer_review::DealerReviewEntity,
        hq_score::HqScoreEntity,
        hq_score_item::HqScoreItemEntity,
        task::TaskEntity,
        task_service_item::TaskServiceItemEntity,
        user_store::UserStoreEntity,
    },
    notify,
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct DealerReviewRequest {
    pub star: i32,
    pub tags: Option<Vec<String>>,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HqScoreRequest {
    pub items: Vec<HqScoreItemRequest>,
    pub score_remark: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HqScoreItemRequest {
    pub service_type: String,
    pub score: i32,
    pub deduction_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReviewActionResponse {
    pub task_id: i64,
    pub task_status: String,
}

pub async fn dealer_review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<i64>,
    Json(payload): Json<DealerReviewRequest>,
) -> Result<Json<ApiResponse<ReviewActionResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "super_admin"])?;

    if !(1..=5).contains(&payload.star) {
        return Err(AppError::BadRequest("评分范围只能为 1 到 5 星".to_string()));
    }

    let task = find_task(&state, task_id).await?;
    let before_task = task.clone();
    if task.task_status.as_deref() != Some("pending_dealer_review") {
        return Err(AppError::Conflict("当前任务状态不允许评价".to_string()));
    }

    ensure_dealer_can_review_task(&state, auth_user.user_id, &task).await?;

    let existing: Vec<DealerReviewEntity> = db::select_by_map_compat(state.rb.as_ref(), "dealer_review", value! { "task_id": task_id })
        .await
        .context("failed to query existing dealer review")
        .map_err(AppError::Internal)?;
    if !existing.is_empty() {
        return Err(AppError::Conflict("该任务已完成评价，请勿重复提交".to_string()));
    }

    let tag_json = payload
        .tags
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("failed to serialize dealer review tags")
        .map_err(AppError::Internal)?;

    DealerReviewEntity::insert(
        state.rb.as_ref(),
        &DealerReviewEntity {
            id: None,
            task_id: Some(task_id),
            reviewer_id: Some(auth_user.user_id),
            star: Some(payload.star),
            tag_json,
            content: payload.content.clone(),
            created_at: None,
        },
    )
    .await
    .context("failed to create dealer review")
    .map_err(AppError::Internal)?;

    TaskEntity::update_by_map(
        state.rb.as_ref(),
        &TaskEntity {
            review_status: Some("reviewed".to_string()),
            task_status: Some("pending_hq_score".to_string()),
            ..Default::default()
        },
        value! { "id": task_id },
    )
    .await
    .context("failed to update task after dealer review")
    .map_err(AppError::Internal)?;

    let after_task = find_task(&state, task_id).await?;
    let _ = notify::create_notice(
        &state,
        auth_user.user_id,
        "dealer_review_done",
        task_id,
        "评价已提交",
        "经销商评价已提交，任务进入总部评分环节",
    )
    .await;
    audit::write_audit_log(
        &state,
        "task",
        task_id,
        "dealer_review",
        auth_user.user_id,
        primary_role(&auth_user),
        Some(&before_task),
        Some(&after_task),
        payload.content.clone(),
    )
    .await?;

    Ok(Json(ApiResponse::success(ReviewActionResponse {
        task_id,
        task_status: "pending_hq_score".to_string(),
    })))
}

pub async fn hq_score(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<i64>,
    Json(payload): Json<HqScoreRequest>,
) -> Result<Json<ApiResponse<ReviewActionResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["hq_reviewer", "super_admin"])?;

    if payload.items.is_empty() {
        return Err(AppError::BadRequest("评分项不能为空".to_string()));
    }

    let task = find_task(&state, task_id).await?;
    let before_task = task.clone();
    if task.task_status.as_deref() != Some("pending_hq_score") {
        return Err(AppError::Conflict("当前任务状态不允许评分".to_string()));
    }

    let existing: Vec<HqScoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "hq_score", value! { "task_id": task_id })
        .await
        .context("failed to query existing hq score")
        .map_err(AppError::Internal)?;
    if !existing.is_empty() {
        return Err(AppError::Conflict("该任务已完成评分，请勿重复提交".to_string()));
    }

    let task_service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(state.rb.as_ref(), "task_service_item", value! { "task_id": task_id })
        .await
        .context("failed to query task service items for scoring")
        .map_err(AppError::Internal)?;

    let mut total = 0i32;
    for item in &payload.items {
        if !(0..=100).contains(&item.score) {
            return Err(AppError::BadRequest("评分范围只能为 0 到 100 分".to_string()));
        }
        let exists_in_task = if task_service_items.is_empty() {
            item.service_type == task.service_type.clone().unwrap_or_default()
        } else {
            task_service_items.iter().any(|service_item| service_item.service_type.as_deref() == Some(item.service_type.as_str()))
        };
        if !exists_in_task {
            return Err(AppError::BadRequest("评分环节与任务环节不一致".to_string()));
        }
        total += item.score;
    }
    let average = total / payload.items.len() as i32;
    let service_level = calc_service_level(average).to_string();

    HqScoreEntity::insert(
        state.rb.as_ref(),
        &HqScoreEntity {
            id: None,
            task_id: Some(task_id),
            scorer_id: Some(auth_user.user_id),
            total_score: Some(average),
            service_level: Some(service_level.clone()),
            score_remark: payload.score_remark.clone(),
            created_at: None,
        },
    )
    .await
    .context("failed to create hq score")
    .map_err(AppError::Internal)?;

    let hq_score: HqScoreEntity = db::select_one_by_map_compat(state.rb.as_ref(), "hq_score", value! { "task_id": task_id })
        .await
        .context("failed to fetch created hq score")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow!("hq score create succeeded but fetch failed")))?;
    let hq_score_id = hq_score.id.unwrap_or_default();

    for item in payload.items {
        HqScoreItemEntity::insert(
            state.rb.as_ref(),
            &HqScoreItemEntity {
                id: None,
                hq_score_id: Some(hq_score_id),
                task_id: Some(task_id),
                service_type: Some(item.service_type),
                score: Some(item.score),
                deduction_reason: item.deduction_reason,
                created_at: None,
            },
        )
        .await
        .context("failed to create hq score item")
        .map_err(AppError::Internal)?;
    }

    TaskEntity::update_by_map(
        state.rb.as_ref(),
        &TaskEntity {
            score_status: Some("scored".to_string()),
            task_status: Some("completed".to_string()),
            ..Default::default()
        },
        value! { "id": task_id },
    )
    .await
    .context("failed to update task after hq score")
    .map_err(AppError::Internal)?;

    for service_item in task_service_items {
        if let Some(id) = service_item.id {
            TaskServiceItemEntity::update_by_map(
                state.rb.as_ref(),
                &TaskServiceItemEntity {
                    service_status: Some("completed".to_string()),
                    progress_percent: Some(100),
                    completed_at: Some(chrono::Utc::now().naive_utc()),
                    ..Default::default()
                },
                value! { "id": id },
            )
            .await
            .context("failed to update task service item after score")
            .map_err(AppError::Internal)?;
        }
    }

    let after_task = find_task(&state, task_id).await?;
    if let Some(technician_id) = task.current_technician_id {
        let _ = notify::create_notice(
            &state,
            technician_id,
            "hq_score_done",
            task_id,
            "总部评分已完成",
            "您的任务已完成总部评分，请查看结果",
        )
        .await;
    }
    if let Some(dealer_user_id) = task.dealer_user_id {
        let _ = notify::create_notice(
            &state,
            dealer_user_id,
            "hq_score_done",
            task_id,
            "任务已完成",
            "任务总部评分已完成，流程已结束",
        )
        .await;
    }
    audit::write_audit_log(
        &state,
        "task",
        task_id,
        "hq_score",
        auth_user.user_id,
        primary_role(&auth_user),
        Some(&before_task),
        Some(&after_task),
        payload.score_remark.clone(),
    )
    .await?;

    Ok(Json(ApiResponse::success(ReviewActionResponse {
        task_id,
        task_status: "completed".to_string(),
    })))
}

async fn find_task(state: &AppState, task_id: i64) -> Result<TaskEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "task", value! { "id": task_id })
        .await
        .context("failed to query task by id")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("任务不存在或已删除".to_string()))
}

async fn ensure_dealer_can_review_task(
    state: &AppState,
    reviewer_id: i64,
    task: &TaskEntity,
) -> Result<(), AppError> {
    let relations: Vec<UserStoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_store", value! { "user_id": reviewer_id })
        .await
        .context("failed to query reviewer store relations")
        .map_err(AppError::Internal)?;
    let task_store_id = task.store_id.ok_or_else(|| AppError::Internal(anyhow!("task.store_id missing")))?;
    if relations.into_iter().any(|item| item.store_id == Some(task_store_id)) {
        return Ok(());
    }
    Err(AppError::Forbidden("您只能操作当前权限范围内的数据".to_string()))
}

fn calc_service_level(score: i32) -> &'static str {
    match score {
        90..=100 => "excellent",
        80..=89 => "good",
        60..=79 => "pass",
        _ => "improve",
    }
}

fn primary_role(auth_user: &auth::CurrentAuthUser) -> &str {
    auth_user.roles.first().map(|item| item.as_str()).unwrap_or("unknown")
}
