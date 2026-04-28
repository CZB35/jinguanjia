use anyhow::Context;
use axum::{
    extract::Multipart,
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{Duration, Utc};
use rbs::value;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth,
    db,
    error::AppError,
    models::{
        customer_review::CustomerReviewEntity,
        customer_review_image::CustomerReviewImageEntity,
        customer_review_link::CustomerReviewLinkEntity,
        hq_score::HqScoreEntity,
        task::TaskEntity,
        task_service_item::TaskServiceItemEntity,
        user::UserEntity,
    },
    notify,
    response::ApiResponse,
    state::AppState,
    storage,
};

#[derive(Debug, Deserialize)]
pub struct SendReviewLinkRequest {
    pub task_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct ReviewFormQuery {
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct SubmitCustomerReviewRequest {
    pub token: String,
    pub star: i32,
    pub tags: Option<Vec<String>>,
    pub content: Option<String>,
    pub image_ids: Option<Vec<i64>>,
}

#[derive(Debug, Serialize)]
pub struct CustomerReviewFormResponse {
    pub task_id: i64,
    pub customer_id: i64,
    pub expired_at: String,
    pub status: String,
    pub technician_name: String,
    pub technician_avatar_url: Option<String>,
    pub technician_avg_score: f64,
    pub service_items_text: String,
    pub service_time: Option<String>,
    pub tag_options: Vec<String>,
    pub images: Vec<CustomerReviewImagePayload>,
}

#[derive(Debug, Serialize)]
pub struct CustomerReviewImagePayload {
    pub id: i64,
    pub file_name: String,
    pub file_url: String,
}

#[derive(Debug, Serialize)]
pub struct CustomerReviewImageUploadResponse {
    pub id: i64,
    pub file_name: String,
    pub file_url: String,
}

#[derive(Debug, Serialize)]
pub struct CustomerReviewDetailResponse {
    pub review_id: i64,
    pub task_id: i64,
    pub customer_id: i64,
    pub star: i32,
    pub tags: Vec<String>,
    pub content: Option<String>,
    pub images: Vec<CustomerReviewImagePayload>,
    pub created_at: Option<String>,
}

pub async fn send_review_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SendReviewLinkRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let task = find_task(&state, payload.task_id).await?;
    let token = Uuid::new_v4().to_string();
    let expired_at = Utc::now() + Duration::days(3);

    CustomerReviewLinkEntity::insert(
        state.rb.as_ref(),
        &CustomerReviewLinkEntity {
            id: None,
            task_id: task.id,
            customer_id: task.customer_id,
            token: Some(token.clone()),
            expired_at: Some(expired_at.naive_utc()),
            status: Some("pending".to_string()),
            sent_at: Some(Utc::now().naive_utc()),
            created_at: None,
        },
    )
    .await
    .context("failed to create customer review link")
    .map_err(AppError::Internal)?;

    if let Some(dealer_user_id) = task.dealer_user_id {
        let _ = notify::create_notice(
            &state,
            dealer_user_id,
            "customer_review_link",
            payload.task_id,
            "客户评价链接已生成",
            "客户评价链接已生成，可发送给客户进行评价",
        )
        .await;
    }

    Ok(Json(ApiResponse::success(serde_json::json!({
        "ok": true,
        "token": token,
        "url": format!("/api/customer-reviews/form?token={}", token)
    }))))
}

pub async fn review_form(
    State(state): State<AppState>,
    Query(query): Query<ReviewFormQuery>,
) -> Result<Json<ApiResponse<CustomerReviewFormResponse>>, AppError> {
    let link = find_link_by_token(&state, &query.token).await?;
    if link.status.as_deref() == Some("used") {
        return Err(AppError::Conflict("该评价链接已使用".to_string()));
    }
    if link.expired_at.map(|dt| dt.and_utc().timestamp()).unwrap_or_default() < Utc::now().timestamp() {
        return Err(AppError::Conflict("该评价链接已过期".to_string()));
    }

    let task = find_task(&state, link.task_id.unwrap_or_default()).await?;
    let technician = if let Some(technician_id) = task.current_technician_id {
        Some(load_user(&state, technician_id).await?)
    } else {
        None
    };
    let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(state.rb.as_ref(), "task_service_item", value! { "task_id": task.id.unwrap_or_default() })
        .await
        .context("failed to query task service items for customer review form")
        .map_err(AppError::Internal)?;
    let images: Vec<CustomerReviewImageEntity> = db::select_by_map_compat(state.rb.as_ref(), "customer_review_image", value! { "review_link_id": link.id.unwrap_or_default() })
        .await
        .context("failed to query customer review images")
        .map_err(AppError::Internal)?;
    let technician_avg_score = if let Some(technician_id) = task.current_technician_id {
        calculate_technician_average_score(&state, technician_id).await?
    } else {
        0.0
    };

    Ok(Json(ApiResponse::success(CustomerReviewFormResponse {
        task_id: link.task_id.unwrap_or_default(),
        customer_id: link.customer_id.unwrap_or_default(),
        expired_at: link.expired_at.map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default(),
        status: link.status.unwrap_or_default(),
        technician_name: technician.as_ref().and_then(|item| item.name.clone()).unwrap_or_default(),
        technician_avatar_url: technician.as_ref().and_then(|item| item.avatar_url.clone()),
        technician_avg_score,
        service_items_text: service_items.into_iter().filter_map(|item| item.service_type).map(service_type_name).collect::<Vec<_>>().join("、"),
        service_time: task.appointment_time.map(|dt| dt.format("%Y-%m-%d %H:%M").to_string()),
        tag_options: vec!["服务专业".to_string(), "准时上门".to_string(), "价格透明".to_string(), "技术熟练".to_string(), "态度友好".to_string(), "清洁到位".to_string()],
        images: images.into_iter().map(|item| CustomerReviewImagePayload {
            id: item.id.unwrap_or_default(),
            file_name: item.file_name.unwrap_or_default(),
            file_url: storage::public_url(&item.file_url.unwrap_or_default()),
        }).collect(),
    })))
}

pub async fn upload_customer_review_image(
    State(state): State<AppState>,
    Query(query): Query<ReviewFormQuery>,
    multipart: Multipart,
) -> Result<Json<ApiResponse<CustomerReviewImageUploadResponse>>, AppError> {
    let link = find_link_by_token(&state, &query.token).await?;
    if link.status.as_deref() == Some("used") {
        return Err(AppError::Conflict("该评价链接已使用".to_string()));
    }

    let mut multipart = multipart;
    let field = multipart
        .next_field()
        .await
        .context("failed to read customer review image field")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::BadRequest("请上传图片".to_string()))?;
    let file_name = field.file_name().map(ToString::to_string).ok_or_else(|| AppError::BadRequest("上传文件名不能为空".to_string()))?;
    let content_type = field.content_type().map(ToString::to_string);
    let bytes = field.bytes().await.context("failed to read customer review image bytes").map_err(AppError::Internal)?;
    let stored = storage::save_bytes(&state, "customer-reviews", &file_name, content_type.as_deref(), &bytes)
        .await
        .map_err(AppError::Internal)?;

    CustomerReviewImageEntity::insert(
        state.rb.as_ref(),
        &CustomerReviewImageEntity {
            id: None,
            review_link_id: link.id,
            review_id: None,
            file_name: Some(stored.original_name.clone()),
            file_url: Some(stored.relative_path.clone()),
            file_size: Some(stored.file_size),
            mime_type: Some(stored.mime_type.clone()),
            created_at: None,
        },
    )
    .await
    .context("failed to save customer review image")
    .map_err(AppError::Internal)?;

    let image: CustomerReviewImageEntity = db::select_one_by_map_compat(state.rb.as_ref(), "customer_review_image", value! { "review_link_id": link.id.unwrap_or_default(), "file_url": stored.relative_path.as_str() })
        .await
        .context("failed to fetch customer review image")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("customer review image fetch failed")))?;

    Ok(Json(ApiResponse::success(CustomerReviewImageUploadResponse {
        id: image.id.unwrap_or_default(),
        file_name: image.file_name.unwrap_or_default(),
        file_url: storage::public_url(&image.file_url.unwrap_or_default()),
    })))
}

pub async fn delete_customer_review_image(
    State(state): State<AppState>,
    Query(query): Query<ReviewFormQuery>,
    Path(image_id): Path<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let link = find_link_by_token(&state, &query.token).await?;
    if link.status.as_deref() == Some("used") {
        return Err(AppError::Conflict("该评价链接已使用".to_string()));
    }

    let image: CustomerReviewImageEntity = db::select_one_by_map_compat(
        state.rb.as_ref(),
        "customer_review_image",
        value! { "id": image_id, "review_link_id": link.id.unwrap_or_default() },
    )
    .await
    .context("failed to query customer review image for delete")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::NotFound("图片不存在".to_string()))?;

    CustomerReviewImageEntity::delete_by_map(state.rb.as_ref(), value! { "id": image_id })
        .await
        .context("failed to delete customer review image")
        .map_err(AppError::Internal)?;

    let path = storage::full_path(&state, image.file_url.as_deref().unwrap_or_default());
    let _ = tokio::fs::remove_file(path).await;

    Ok(Json(ApiResponse::success(serde_json::json!({ "ok": true }))))
}

pub async fn submit_customer_review(
    State(state): State<AppState>,
    Json(payload): Json<SubmitCustomerReviewRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    if !(1..=5).contains(&payload.star) {
        return Err(AppError::BadRequest("评分范围只能为 1 到 5 星".to_string()));
    }

    let link = find_link_by_token(&state, &payload.token).await?;
    if link.status.as_deref() == Some("used") {
        return Err(AppError::Conflict("该评价链接已使用".to_string()));
    }
    if link.expired_at.map(|dt| dt.and_utc().timestamp()).unwrap_or_default() < Utc::now().timestamp() {
        return Err(AppError::Conflict("该评价链接已过期".to_string()));
    }

    let exists: Vec<CustomerReviewEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "customer_review",
        value! { "task_id": link.task_id.unwrap_or_default() },
    )
    .await
    .context("failed to query existing customer review")
    .map_err(AppError::Internal)?;
    if !exists.is_empty() {
        return Err(AppError::Conflict("该任务已完成客户评价".to_string()));
    }

    let tag_json = payload.tags.as_ref().map(serde_json::to_string).transpose()
        .context("failed to serialize customer review tags")
        .map_err(AppError::Internal)?;
    CustomerReviewEntity::insert(
        state.rb.as_ref(),
        &CustomerReviewEntity {
            id: None,
            task_id: link.task_id,
            customer_id: link.customer_id,
            review_link_id: link.id,
            star: Some(payload.star),
            tag_json,
            content: payload.content.clone(),
            created_at: None,
        },
    )
    .await
    .context("failed to create customer review")
    .map_err(AppError::Internal)?;

    let review: CustomerReviewEntity = db::select_one_by_map_compat(state.rb.as_ref(), "customer_review", value! { "task_id": link.task_id.unwrap_or_default() })
        .await
        .context("failed to fetch created customer review")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("customer review fetch failed")))?;
    if let Some(image_ids) = payload.image_ids {
        for image_id in image_ids {
            let image: CustomerReviewImageEntity = db::select_one_by_map_compat(
                state.rb.as_ref(),
                "customer_review_image",
                value! { "id": image_id, "review_link_id": link.id.unwrap_or_default() },
            )
            .await
            .context("failed to fetch customer review image for bind")
            .map_err(AppError::Internal)?
            .ok_or_else(|| AppError::BadRequest("评价图片不存在".to_string()))?;
            CustomerReviewImageEntity::update_by_map(
                state.rb.as_ref(),
                &CustomerReviewImageEntity {
                    review_id: review.id,
                    ..image.clone()
                },
                value! { "id": image.id.unwrap_or_default(), "review_link_id": link.id.unwrap_or_default(), "column": ["review_id"] },
            )
                .await
                .context("failed to bind customer review image")
                .map_err(AppError::Internal)?;
        }
    }

    CustomerReviewLinkEntity::update_by_map(
        state.rb.as_ref(),
        &CustomerReviewLinkEntity {
            status: Some("used".to_string()),
            ..link.clone()
        },
        value! { "id": link.id.unwrap_or_default(), "column": ["status"] },
    )
    .await
    .context("failed to update customer review link status")
    .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(serde_json::json!({ "ok": true }))))
}

pub async fn review_detail(
    State(state): State<AppState>,
    Query(query): Query<ReviewFormQuery>,
) -> Result<Json<ApiResponse<CustomerReviewDetailResponse>>, AppError> {
    let link = find_link_by_token(&state, &query.token).await?;
    let review: CustomerReviewEntity = db::select_one_by_map_compat(
        state.rb.as_ref(),
        "customer_review",
        value! { "review_link_id": link.id.unwrap_or_default() },
    )
    .await
    .context("failed to query customer review detail")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::NotFound("评价记录不存在".to_string()))?;

    let images: Vec<CustomerReviewImageEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "customer_review_image",
        value! { "review_link_id": link.id.unwrap_or_default() },
    )
    .await
    .context("failed to query customer review detail images")
    .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(CustomerReviewDetailResponse {
        review_id: review.id.unwrap_or_default(),
        task_id: review.task_id.unwrap_or_default(),
        customer_id: review.customer_id.unwrap_or_default(),
        star: review.star.unwrap_or_default(),
        tags: review
            .tag_json
            .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
            .unwrap_or_default(),
        content: review.content,
        images: images
            .into_iter()
            .map(|item| CustomerReviewImagePayload {
                id: item.id.unwrap_or_default(),
                file_name: item.file_name.unwrap_or_default(),
                file_url: storage::public_url(&item.file_url.unwrap_or_default()),
            })
            .collect(),
        created_at: review.created_at.map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string()),
    })))
}

async fn find_task(state: &AppState, task_id: i64) -> Result<TaskEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "task", value! { "id": task_id })
        .await
        .context("failed to query task for customer review")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("任务不存在或已删除".to_string()))
}

async fn find_link_by_token(state: &AppState, token: &str) -> Result<CustomerReviewLinkEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "customer_review_link", value! { "token": token })
        .await
        .context("failed to query customer review link")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("评价链接不存在".to_string()))
}

async fn load_user(state: &AppState, user_id: i64) -> Result<UserEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "user", value! { "id": user_id })
        .await
        .context("failed to query customer review technician")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("用户不存在".to_string()))
}

fn service_type_name(service_type: String) -> String {
    match service_type.as_str() {
        "measure" => "量尺".to_string(),
        "install" => "安装".to_string(),
        "ironing" => "熨烫".to_string(),
        "cleaning" => "除尘除螨".to_string(),
        _ => service_type,
    }
}

async fn calculate_technician_average_score(state: &AppState, technician_id: i64) -> Result<f64, AppError> {
    let tasks: Vec<TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! { "current_technician_id": technician_id })
        .await
        .context("failed to query technician tasks for customer review score")
        .map_err(AppError::Internal)?;
    let task_ids: Vec<i64> = tasks.into_iter().filter_map(|item| item.id).collect();
    if task_ids.is_empty() {
        return Ok(0.0);
    }

    let scores: Vec<HqScoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "hq_score", value! { "task_id": task_ids })
        .await
        .context("failed to query technician scores for customer review")
        .map_err(AppError::Internal)?;
    if scores.is_empty() {
        return Ok(0.0);
    }

    Ok(scores.iter().map(|item| item.total_score.unwrap_or_default() as f64).sum::<f64>() / scores.len() as f64)
}
