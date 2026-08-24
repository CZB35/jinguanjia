use anyhow::{Context, anyhow};
use rbs::value;
use std::collections::HashMap;

use crate::{
    db,
    error::AppError,
    models::{role::RoleEntity, user_role::UserRoleEntity},
    state::AppState,
};

pub fn relation_display_name(relation: Option<&UserRoleEntity>, fallback: Option<&str>) -> String {
    relation
        .and_then(|item| item.display_name.as_deref())
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .or_else(|| fallback.map(str::trim).filter(|item| !item.is_empty()))
        .unwrap_or_default()
        .to_string()
}

pub async fn role_display_name(
    state: &AppState,
    user_id: i64,
    role_code: &str,
    fallback: Option<&str>,
) -> Result<String, AppError> {
    let relation = find_role_relation(state, user_id, role_code).await?;
    Ok(relation_display_name(relation.as_ref(), fallback))
}

pub async fn update_role_display_name(
    state: &AppState,
    user_id: i64,
    role_code: &str,
    display_name: &str,
) -> Result<(), AppError> {
    let role_id = find_role_id(state, role_code).await?;
    update_role_display_name_by_role_id(state, user_id, role_id, display_name).await
}

pub async fn update_role_display_name_by_role_id(
    state: &AppState,
    user_id: i64,
    role_id: i64,
    display_name: &str,
) -> Result<(), AppError> {
    let result = UserRoleEntity::update_by_map(
        state.rb.as_ref(),
        &UserRoleEntity {
            display_name: Some(display_name.trim().to_string()),
            ..Default::default()
        },
        value! {
            "user_id": user_id,
            "role_id": role_id,
            "column": ["display_name"]
        },
    )
    .await
    .context("failed to update role display name")
    .map_err(AppError::Internal)?;
    if result.rows_affected == 0 {
        let relation = db::select_one_by_map_compat::<UserRoleEntity>(
            state.rb.as_ref(),
            "user_role",
            value! { "user_id": user_id, "role_id": role_id },
        )
        .await
        .context("failed to verify role relation after display name update")
        .map_err(AppError::Internal)?;
        if relation.is_none() {
            return Err(AppError::Internal(anyhow!(
                "role relation missing while updating display name"
            )));
        }
    }
    Ok(())
}

pub async fn role_display_names(
    state: &AppState,
    user_ids: &[i64],
    role_code: &str,
) -> Result<HashMap<i64, String>, AppError> {
    if user_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let role_id = find_role_id(state, role_code).await?;
    let relations: Vec<UserRoleEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": user_ids.to_vec(), "role_id": role_id },
    )
    .await
    .context("failed to batch query role display names")
    .map_err(AppError::Internal)?;
    Ok(relations
        .into_iter()
        .filter_map(|item| {
            let user_id = item.user_id?;
            let display_name = item.display_name?.trim().to_string();
            (!display_name.is_empty()).then_some((user_id, display_name))
        })
        .collect())
}

async fn find_role_relation(
    state: &AppState,
    user_id: i64,
    role_code: &str,
) -> Result<Option<UserRoleEntity>, AppError> {
    let role_id = find_role_id(state, role_code).await?;
    db::select_one_by_map_compat(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": user_id, "role_id": role_id },
    )
    .await
    .context("failed to query role display name")
    .map_err(AppError::Internal)
}

async fn find_role_id(state: &AppState, role_code: &str) -> Result<i64, AppError> {
    db::select_one_by_map_compat::<RoleEntity>(
        state.rb.as_ref(),
        "role",
        value! { "role_code": role_code },
    )
    .await
    .context("failed to query role for display name")
    .map_err(AppError::Internal)?
    .and_then(|item| item.id)
    .ok_or_else(|| AppError::Internal(anyhow!("role not found: {role_code}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_name_prefers_role_specific_name() {
        let relation = UserRoleEntity {
            display_name: Some("  师傅名称  ".to_string()),
            ..Default::default()
        };

        assert_eq!(
            relation_display_name(Some(&relation), Some("账号名称")),
            "师傅名称"
        );
    }

    #[test]
    fn relation_name_falls_back_for_blank_role_name() {
        let relation = UserRoleEntity {
            display_name: Some("   ".to_string()),
            ..Default::default()
        };

        assert_eq!(
            relation_display_name(Some(&relation), Some("账号名称")),
            "账号名称"
        );
    }
}
