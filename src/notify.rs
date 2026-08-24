use anyhow::Context;
use chrono::Utc;
use rbs::value;

use crate::{error::AppError, models::notice::NoticeEntity, state::AppState, wechat};

pub async fn create_notice(
    state: &AppState,
    receiver_id: i64,
    biz_type: &str,
    biz_id: i64,
    title: &str,
    content: &str,
) -> Result<(), AppError> {
    NoticeEntity::insert(
        state.rb.as_ref(),
        &NoticeEntity {
            id: None,
            receiver_id: Some(receiver_id),
            notice_type: Some("site".to_string()),
            biz_type: Some(biz_type.to_string()),
            biz_id: Some(biz_id),
            title: Some(title.to_string()),
            content: Some(content.to_string()),
            is_read: Some(0),
            read_at: None,
            send_status: Some("sent".to_string()),
            sent_at: Some(Utc::now().naive_utc()),
            created_at: None,
        },
    )
    .await
    .context("failed to create notice")
    .map_err(AppError::Internal)?;

    let notice = crate::db::select_one_by_map_compat::<NoticeEntity>(
        state.rb.as_ref(),
        "notice",
        value! { "receiver_id": receiver_id, "biz_type": biz_type, "biz_id": biz_id, "title": title },
    )
    .await
    .context("failed to fetch created notice")
    .map_err(AppError::Internal)?;
    if let Some(notice) = notice {
        let _ = wechat::enqueue_wechat_notice(state, &notice).await;
    }
    Ok(())
}

pub async fn create_notice_with_payload(
    state: &AppState,
    receiver_id: i64,
    biz_type: &str,
    biz_id: i64,
    title: &str,
    content: &str,
    payload: serde_json::Value,
) -> Result<(), AppError> {
    NoticeEntity::insert(
        state.rb.as_ref(),
        &NoticeEntity {
            id: None,
            receiver_id: Some(receiver_id),
            notice_type: Some("site".to_string()),
            biz_type: Some(biz_type.to_string()),
            biz_id: Some(biz_id),
            title: Some(title.to_string()),
            content: Some(content.to_string()),
            is_read: Some(0),
            read_at: None,
            send_status: Some("sent".to_string()),
            sent_at: Some(Utc::now().naive_utc()),
            created_at: None,
        },
    )
    .await
    .context("failed to create notice with payload")
    .map_err(AppError::Internal)?;

    let notice = crate::db::select_one_by_map_compat::<NoticeEntity>(
        state.rb.as_ref(),
        "notice",
        value! { "receiver_id": receiver_id, "biz_type": biz_type, "biz_id": biz_id, "title": title },
    )
    .await
    .context("failed to fetch created notice with payload")
    .map_err(AppError::Internal)?;
    if let Some(notice) = notice {
        let _ = crate::wechat::enqueue_wechat_notice_with_payload(state, &notice, payload).await;
    }
    Ok(())
}
