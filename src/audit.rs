use anyhow::Context;
use serde::Serialize;

use crate::{error::AppError, models::audit_log::AuditLogEntity, state::AppState};

pub async fn write_audit_log<B, A>(
    state: &AppState,
    biz_type: &str,
    biz_id: i64,
    action: &str,
    operator_id: i64,
    operator_role: &str,
    before_value: Option<&B>,
    after_value: Option<&A>,
    remark: Option<String>,
) -> Result<(), AppError>
where
    B: Serialize,
    A: Serialize,
{
    let before_value = before_value
        .map(serde_json::to_string)
        .transpose()
        .context("failed to serialize audit before_value")
        .map_err(AppError::Internal)?;
    let after_value = after_value
        .map(serde_json::to_string)
        .transpose()
        .context("failed to serialize audit after_value")
        .map_err(AppError::Internal)?;

    AuditLogEntity::insert(
        state.rb.as_ref(),
        &AuditLogEntity {
            id: None,
            biz_type: Some(biz_type.to_string()),
            biz_id: Some(biz_id),
            action: Some(action.to_string()),
            operator_id: Some(operator_id),
            operator_role: Some(operator_role.to_string()),
            before_value,
            after_value,
            remark,
            created_at: None,
        },
    )
    .await
    .context("failed to insert audit log")
    .map_err(AppError::Internal)?;

    Ok(())
}
