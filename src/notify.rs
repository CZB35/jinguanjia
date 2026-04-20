use anyhow::Context;
use chrono::Utc;

use crate::{error::AppError, models::notice::NoticeEntity, state::AppState};

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
    Ok(())
}
