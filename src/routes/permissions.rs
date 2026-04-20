use anyhow::Context;
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    auth,
    db,
    error::AppError,
    models::{permission::PermissionEntity, role::RoleEntity, role_permission::RolePermissionEntity},
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Serialize)]
pub struct RoleItem {
    pub id: i64,
    pub role_code: String,
    pub role_name: String,
}

#[derive(Debug, Serialize)]
pub struct RoleListResponse {
    pub items: Vec<RoleItem>,
}

#[derive(Debug, Serialize)]
pub struct PermissionTreeItem {
    pub id: i64,
    pub permission_code: String,
    pub permission_name: String,
    pub permission_type: String,
    pub parent_id: Option<i64>,
    pub sort_no: i32,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRolePermissionsRequest {
    pub permission_ids: Vec<i64>,
}

pub async fn get_roles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<RoleListResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let roles: Vec<RoleEntity> = db::select_by_map_compat(state.rb.as_ref(), "role", value! {})
        .await
        .context("failed to query roles")
        .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(RoleListResponse {
        items: roles
            .into_iter()
            .map(|item| RoleItem {
                id: item.id.unwrap_or_default(),
                role_code: item.role_code.unwrap_or_default(),
                role_name: item.role_name.unwrap_or_default(),
            })
            .collect(),
    })))
}

pub async fn get_role_permissions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(role_id): Path<i64>,
) -> Result<Json<ApiResponse<Vec<i64>>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let _ = find_role(&state, role_id).await?;
    let relations: Vec<RolePermissionEntity> = db::select_by_map_compat(state.rb.as_ref(), "role_permission", value! { "role_id": role_id })
        .await
        .context("failed to query role permissions")
        .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(
        relations.into_iter().filter_map(|item| item.permission_id).collect(),
    )))
}

pub async fn update_role_permissions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(role_id): Path<i64>,
    Json(payload): Json<UpdateRolePermissionsRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let _ = find_role(&state, role_id).await?;

    let existing: Vec<RolePermissionEntity> = db::select_by_map_compat(state.rb.as_ref(), "role_permission", value! { "role_id": role_id })
        .await
        .context("failed to query existing role permissions")
        .map_err(AppError::Internal)?;
    for item in existing {
        if let Some(id) = item.id {
            RolePermissionEntity::delete_by_map(state.rb.as_ref(), value! { "id": id })
                .await
                .context("failed to delete role permission")
                .map_err(AppError::Internal)?;
        }
    }

    for permission_id in payload.permission_ids {
        let _ = find_permission(&state, permission_id).await?;
        RolePermissionEntity::insert(
            state.rb.as_ref(),
            &RolePermissionEntity {
                id: None,
                role_id: Some(role_id),
                permission_id: Some(permission_id),
                created_at: None,
            },
        )
        .await
        .context("failed to insert role permission")
        .map_err(AppError::Internal)?;
    }

    Ok(Json(ApiResponse::success(serde_json::json!({ "ok": true }))))
}

pub async fn get_permission_tree(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<Vec<PermissionTreeItem>>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let permissions: Vec<PermissionEntity> = db::select_by_map_compat(state.rb.as_ref(), "permission", value! {})
        .await
        .context("failed to query permission tree")
        .map_err(AppError::Internal)?;

    let mut items = permissions
        .into_iter()
        .map(|item| PermissionTreeItem {
            id: item.id.unwrap_or_default(),
            permission_code: item.permission_code.unwrap_or_default(),
            permission_name: item.permission_name.unwrap_or_default(),
            permission_type: item.permission_type.unwrap_or_default(),
            parent_id: item.parent_id,
            sort_no: item.sort_no.unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    items.sort_by_key(|item| (item.parent_id.unwrap_or_default(), item.sort_no, item.id));

    Ok(Json(ApiResponse::success(items)))
}

async fn find_role(state: &AppState, role_id: i64) -> Result<RoleEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "role", value! { "id": role_id })
        .await
        .context("failed to query role by id")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("角色不存在".to_string()))
}

async fn find_permission(state: &AppState, permission_id: i64) -> Result<PermissionEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "permission", value! { "id": permission_id })
        .await
        .context("failed to query permission by id")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("权限不存在".to_string()))
}

fn ensure_admin_access(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let auth_user = auth::authenticate(state, headers)?;
    auth_user.require_any_role(&["super_admin", "hq_reviewer"])
}
