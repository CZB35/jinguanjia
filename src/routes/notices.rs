use anyhow::Context;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::Utc;
use rbs::value;
use serde::Serialize;

use crate::{
    auth, db, error::AppError, models::notice::NoticeEntity, response::ApiResponse, state::AppState,
};

#[derive(Debug, Serialize)]
pub struct NoticeItem {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub notice_type: String,
    pub biz_type: String,
    pub biz_id: i64,
    pub is_read: bool,
    pub created_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NoticeListResponse {
    pub items: Vec<NoticeItem>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, serde::Deserialize)]
pub struct NoticeListQuery {
    pub page: Option<usize>,
    pub page_size: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct UnreadCountResponse {
    pub unread_count: usize,
}

pub async fn list_notices(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NoticeListQuery>,
) -> Result<Json<ApiResponse<NoticeListResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let notices: Vec<NoticeEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "notice",
        value! { "receiver_id": auth_user.user_id },
    )
    .await
    .context("failed to query notices")
    .map_err(AppError::Internal)?;

    let items = notices
        .into_iter()
        .map(|item| NoticeItem {
            id: item.id.unwrap_or_default(),
            title: item.title.unwrap_or_default(),
            content: item.content.unwrap_or_default(),
            notice_type: item.notice_type.unwrap_or_else(|| "site".to_string()),
            biz_type: item.biz_type.unwrap_or_default(),
            biz_id: item.biz_id.unwrap_or_default(),
            is_read: item.is_read.unwrap_or_default() == 1,
            created_at: item
                .created_at
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string()),
        })
        .collect::<Vec<_>>();

    let total = items.len();
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).max(1);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let items = items.into_iter().skip(start).take(page_size).collect::<Vec<_>>();
    Ok(Json(ApiResponse::success(NoticeListResponse {
        items,
        total,
        page,
        page_size,
    })))
}

pub async fn unread_count(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<UnreadCountResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let notices: Vec<NoticeEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "notice",
        value! { "receiver_id": auth_user.user_id, "is_read": 0 },
    )
    .await
    .context("failed to query unread notices")
    .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(UnreadCountResponse {
        unread_count: notices.len(),
    })))
}

pub async fn read_notice(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(notice_id): Path<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let notice = find_notice(&state, notice_id).await?;
    if notice.receiver_id != Some(auth_user.user_id) {
        return Err(AppError::Forbidden(
            "您只能操作当前权限范围内的数据".to_string(),
        ));
    }

    NoticeEntity::update_by_map(
        state.rb.as_ref(),
        &NoticeEntity {
            is_read: Some(1),
            read_at: Some(Utc::now().naive_utc()),
            ..Default::default()
        },
        value! { "id": notice_id },
    )
    .await
    .context("failed to mark notice as read")
    .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn read_all_notices(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let notices: Vec<NoticeEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "notice",
        value! { "receiver_id": auth_user.user_id, "is_read": 0 },
    )
    .await
    .context("failed to query unread notices for read-all")
    .map_err(AppError::Internal)?;

    for notice in notices {
        if let Some(id) = notice.id {
            NoticeEntity::update_by_map(
                state.rb.as_ref(),
                &NoticeEntity {
                    is_read: Some(1),
                    read_at: Some(Utc::now().naive_utc()),
                    ..Default::default()
                },
                value! { "id": id },
            )
            .await
            .context("failed to mark notice as read in bulk")
            .map_err(AppError::Internal)?;
        }
    }

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

async fn find_notice(state: &AppState, notice_id: i64) -> Result<NoticeEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "notice", value! { "id": notice_id })
        .await
        .context("failed to query notice by id")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("通知不存在".to_string()))
}
