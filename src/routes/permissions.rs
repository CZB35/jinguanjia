use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    auth, db,
    error::AppError,
    models::{
        permission::PermissionEntity, role::RoleEntity, role_permission::RolePermissionEntity,
        user_role::UserRoleEntity,
    },
    response::ApiResponse,
    state::AppState,
};

const HIDDEN_BUSINESS_ROLE_CODES: &[&str] = &["dealer", "technician"];

#[derive(Debug, Serialize)]
pub struct RoleItem {
    pub id: i64,
    pub role_code: String,
    pub role_name: String,
    pub name: String,
    pub remark: Option<String>,
    pub status: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub permission_ids: Vec<i64>,
    pub permission_keys: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RoleListResponse {
    pub items: Vec<RoleItem>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, Deserialize)]
pub struct RoleListQuery {
    pub page: Option<usize>,
    pub page_size: Option<usize>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PermissionTreeItem {
    pub id: i64,
    pub key: String,
    pub title: String,
    pub permission_code: String,
    pub permission_name: String,
    pub permission_type: String,
    pub parent_id: Option<i64>,
    pub sort_no: i32,
    pub children: Vec<PermissionTreeItem>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRolePermissionsRequest {
    pub permission_ids: Option<Vec<i64>>,
    pub permission_keys: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRoleRequest {
    pub role_code: Option<String>,
    pub role_name: Option<String>,
    pub name: Option<String>,
    pub permission_ids: Option<Vec<i64>>,
    pub permission_keys: Option<Vec<String>>,
    pub remark: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    pub role_code: Option<String>,
    pub role_name: Option<String>,
    pub name: Option<String>,
    pub permission_ids: Option<Vec<i64>>,
    pub permission_keys: Option<Vec<String>>,
    pub remark: Option<String>,
    pub status: Option<String>,
}

pub async fn get_roles(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RoleListQuery>,
) -> Result<Json<ApiResponse<RoleListResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let items = load_role_items(&state, None).await?;
    let total = items.len();
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).max(1);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let items = items.into_iter().skip(start).take(page_size).collect::<Vec<_>>();
    Ok(Json(ApiResponse::success(RoleListResponse {
        items,
        total,
        page,
        page_size,
    })))
}

pub async fn get_role_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(role_id): Path<i64>,
) -> Result<Json<ApiResponse<RoleItem>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let item = load_role_items(&state, Some(role_id))
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("角色不存在".to_string()))?;
    Ok(Json(ApiResponse::success(item)))
}

pub async fn create_role(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateRoleRequest>,
) -> Result<Json<ApiResponse<RoleItem>>, AppError> {
    ensure_admin_access(&state, &headers)?;

    let role_name = normalize_role_name(payload.role_name.as_deref().or(payload.name.as_deref()))?;
    let role_code = match normalize_optional_text(payload.role_code.as_deref()) {
        Some(code) => code,
        None => generate_role_code(&state).await?,
    };
    let status = normalize_role_status(payload.status.as_deref())?;
    let permission_ids =
        resolve_permission_ids(&state, payload.permission_ids, payload.permission_keys).await?;
    ensure_backend_role_code(&role_code)?;

    ensure_role_name_unique(&state, &role_name, None).await?;
    ensure_role_code_unique(&state, &role_code, None).await?;

    RoleEntity::insert(
        state.rb.as_ref(),
        &RoleEntity {
            id: None,
            role_code: Some(role_code.clone()),
            role_name: Some(role_name),
            remark: normalize_optional_owned(payload.remark),
            status: Some(status),
            created_at: None,
            updated_at: None,
        },
    )
    .await
    .context("failed to create role")
    .map_err(AppError::Internal)?;

    let role = db::select_one_by_map_compat::<RoleEntity>(
        state.rb.as_ref(),
        "role",
        value! { "role_code": role_code.as_str() },
    )
    .await
    .context("failed to fetch created role")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::Internal(anyhow::anyhow!("role create succeeded but fetch failed")))?;
    let role_id = role.id.unwrap_or_default();
    replace_role_permissions(&state, role_id, &permission_ids).await?;

    get_role_detail(State(state), headers, Path(role_id)).await
}

pub async fn update_role(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(role_id): Path<i64>,
    Json(payload): Json<UpdateRoleRequest>,
) -> Result<Json<ApiResponse<RoleItem>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let existing = find_role(&state, role_id).await?;

    let role_name = match payload.role_name.as_deref().or(payload.name.as_deref()) {
        Some(value) => normalize_role_name(Some(value))?,
        None => existing.role_name.clone().unwrap_or_default(),
    };
    let role_code = match normalize_optional_text(payload.role_code.as_deref()) {
        Some(value) => value,
        None => existing.role_code.clone().unwrap_or_default(),
    };
    let status = match payload.status.as_deref() {
        Some(value) => Some(normalize_role_status(Some(value))?),
        None => existing
            .status
            .clone()
            .or_else(|| Some("active".to_string())),
    };
    ensure_backend_role_code(&role_code)?;

    ensure_role_name_unique(&state, &role_name, Some(role_id)).await?;
    ensure_role_code_unique(&state, &role_code, Some(role_id)).await?;

    RoleEntity::update_by_map(
        state.rb.as_ref(),
        &RoleEntity {
            role_code: Some(role_code),
            role_name: Some(role_name),
            remark: payload
                .remark
                .map(|value| normalize_optional_text(Some(value.as_str())))
                .unwrap_or_else(|| existing.remark.clone()),
            status,
            ..Default::default()
        },
        value! { "id": role_id, "column": ["role_code", "role_name", "remark", "status"] },
    )
    .await
    .context("failed to update role")
    .map_err(AppError::Internal)?;

    if payload.permission_ids.is_some() || payload.permission_keys.is_some() {
        let permission_ids =
            resolve_permission_ids(&state, payload.permission_ids, payload.permission_keys).await?;
        replace_role_permissions(&state, role_id, &permission_ids).await?;
    }

    get_role_detail(State(state), headers, Path(role_id)).await
}

pub async fn delete_role(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(role_id): Path<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let _ = find_role(&state, role_id).await?;

    let bound_users = db::select_by_map_compat::<UserRoleEntity>(
        state.rb.as_ref(),
        "user_role",
        value! { "role_id": role_id },
    )
    .await
    .context("failed to query role user bindings")
    .map_err(AppError::Internal)?;
    if !bound_users.is_empty() {
        return Err(AppError::Conflict(
            "当前角色已绑定用户，无法删除".to_string(),
        ));
    }

    let permissions = db::select_by_map_compat::<RolePermissionEntity>(
        state.rb.as_ref(),
        "role_permission",
        value! { "role_id": role_id },
    )
    .await
    .context("failed to query role permissions before delete")
    .map_err(AppError::Internal)?;
    for item in permissions {
        if let Some(id) = item.id {
            RolePermissionEntity::delete_by_map(state.rb.as_ref(), value! { "id": id })
                .await
                .context("failed to delete role permission before role delete")
                .map_err(AppError::Internal)?;
        }
    }

    RoleEntity::delete_by_map(state.rb.as_ref(), value! { "id": role_id })
        .await
        .context("failed to delete role")
        .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn get_role_permissions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(role_id): Path<i64>,
) -> Result<Json<ApiResponse<Vec<i64>>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let _ = find_role(&state, role_id).await?;
    let relations: Vec<RolePermissionEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "role_permission",
        value! { "role_id": role_id },
    )
    .await
    .context("failed to query role permissions")
    .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(
        relations
            .into_iter()
            .filter_map(|item| item.permission_id)
            .collect(),
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
    let permission_ids =
        resolve_permission_ids(&state, payload.permission_ids, payload.permission_keys).await?;
    replace_role_permissions(&state, role_id, &permission_ids).await?;
    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn get_permission_tree(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<Vec<PermissionTreeItem>>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let permissions: Vec<PermissionEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "permission", value! {})
            .await
            .context("failed to query permission tree")
            .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(build_permission_tree(
        permissions,
    ))))
}

async fn load_role_items(
    state: &AppState,
    role_id: Option<i64>,
) -> Result<Vec<RoleItem>, AppError> {
    let roles: Vec<RoleEntity> = match role_id {
        Some(role_id) => {
            db::select_by_map_compat(state.rb.as_ref(), "role", value! { "id": role_id })
                .await
                .context("failed to query role by id for role items")
                .map_err(AppError::Internal)?
        }
        None => db::select_by_map_compat(state.rb.as_ref(), "role", value! {})
            .await
            .context("failed to query roles for role items")
            .map_err(AppError::Internal)?,
    };
    let roles = roles
        .into_iter()
        .filter(|role| {
            role.role_code
                .as_deref()
                .map(|code| !HIDDEN_BUSINESS_ROLE_CODES.contains(&code))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();

    let role_ids = roles.iter().filter_map(|item| item.id).collect::<Vec<_>>();
    let relations = if role_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<RolePermissionEntity>(
            state.rb.as_ref(),
            "role_permission",
            value! { "role_id": role_ids.clone() },
        )
        .await
        .context("failed to query role permission relations for role items")
        .map_err(AppError::Internal)?
    };
    let permission_ids = relations
        .iter()
        .filter_map(|item| item.permission_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let permissions = if permission_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<PermissionEntity>(
            state.rb.as_ref(),
            "permission",
            value! { "id": permission_ids },
        )
        .await
        .context("failed to query permissions for role items")
        .map_err(AppError::Internal)?
    };

    let permission_map = permissions
        .into_iter()
        .filter_map(|item| {
            item.id
                .map(|id| (id, item.permission_code.unwrap_or_default()))
        })
        .collect::<BTreeMap<_, _>>();
    let mut role_permission_map = BTreeMap::<i64, Vec<(i64, String)>>::new();
    for relation in relations {
        let Some(role_id) = relation.role_id else {
            continue;
        };
        let Some(permission_id) = relation.permission_id else {
            continue;
        };
        let permission_code = permission_map
            .get(&permission_id)
            .cloned()
            .unwrap_or_default();
        role_permission_map
            .entry(role_id)
            .or_default()
            .push((permission_id, permission_code));
    }

    let mut items = roles
        .into_iter()
        .map(|role| {
            let role_id = role.id.unwrap_or_default();
            let mut permissions = role_permission_map.remove(&role_id).unwrap_or_default();
            permissions.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
            RoleItem {
                id: role_id,
                role_code: role.role_code.clone().unwrap_or_default(),
                role_name: role.role_name.clone().unwrap_or_default(),
                name: role.role_name.unwrap_or_default(),
                remark: role.remark,
                status: role.status.unwrap_or_else(|| "active".to_string()),
                created_at: role.created_at.map(format_datetime),
                updated_at: role.updated_at.map(format_datetime),
                permission_ids: permissions.iter().map(|item| item.0).collect(),
                permission_keys: permissions.into_iter().map(|item| item.1).collect(),
            }
        })
        .collect::<Vec<_>>();
    items.sort_by(|a, b| a.role_code.cmp(&b.role_code).then(a.id.cmp(&b.id)));
    Ok(items)
}

async fn replace_role_permissions(
    state: &AppState,
    role_id: i64,
    permission_ids: &[i64],
) -> Result<(), AppError> {
    let existing: Vec<RolePermissionEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "role_permission",
        value! { "role_id": role_id },
    )
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

    for permission_id in permission_ids {
        let _ = find_permission(state, *permission_id).await?;
        RolePermissionEntity::insert(
            state.rb.as_ref(),
            &RolePermissionEntity {
                id: None,
                role_id: Some(role_id),
                permission_id: Some(*permission_id),
                created_at: None,
            },
        )
        .await
        .context("failed to insert role permission")
        .map_err(AppError::Internal)?;
    }

    Ok(())
}

async fn resolve_permission_ids(
    state: &AppState,
    permission_ids: Option<Vec<i64>>,
    permission_keys: Option<Vec<String>>,
) -> Result<Vec<i64>, AppError> {
    if let Some(permission_ids) = permission_ids {
        return Ok(permission_ids
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect());
    }

    let Some(permission_keys) = permission_keys else {
        return Ok(Vec::new());
    };
    let normalized_keys = permission_keys
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if normalized_keys.is_empty() {
        return Ok(Vec::new());
    }

    let permissions: Vec<PermissionEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "permission",
        value! { "permission_code": normalized_keys.clone() },
    )
    .await
    .context("failed to query permissions by code")
    .map_err(AppError::Internal)?;
    let code_to_id = permissions
        .into_iter()
        .filter_map(|item| item.permission_code.zip(item.id))
        .collect::<BTreeMap<_, _>>();
    let ids = normalized_keys
        .iter()
        .filter_map(|item| code_to_id.get(item).copied())
        .collect::<Vec<_>>();
    if ids.len() != normalized_keys.len() {
        return Err(AppError::BadRequest("存在无效的权限编码".to_string()));
    }
    Ok(ids)
}

fn build_permission_tree(permissions: Vec<PermissionEntity>) -> Vec<PermissionTreeItem> {
    let preferred_roots = ["workspace", "tasks", "users", "settings", "master-data"];
    let has_preferred_tree = permissions.iter().any(|item| {
        item.permission_code
            .as_deref()
            .map(|code| preferred_roots.contains(&code))
            .unwrap_or(false)
    });
    let permissions = if has_preferred_tree {
        permissions
            .into_iter()
            .filter(|item| {
                item.permission_code
                    .as_deref()
                    .map(|code| {
                        preferred_roots
                            .iter()
                            .any(|root| code == *root || code.starts_with(&format!("{}:", root)))
                    })
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>()
    } else {
        permissions
    };

    let mut grouped = BTreeMap::<Option<i64>, Vec<PermissionTreeItem>>::new();
    for permission in permissions {
        let item = PermissionTreeItem {
            id: permission.id.unwrap_or_default(),
            key: permission.permission_code.clone().unwrap_or_default(),
            title: permission.permission_name.clone().unwrap_or_default(),
            permission_code: permission.permission_code.unwrap_or_default(),
            permission_name: permission.permission_name.unwrap_or_default(),
            permission_type: permission.permission_type.unwrap_or_default(),
            parent_id: permission.parent_id,
            sort_no: permission.sort_no.unwrap_or_default(),
            children: Vec::new(),
        };
        grouped.entry(item.parent_id).or_default().push(item);
    }

    fn build_children(
        parent_id: Option<i64>,
        grouped: &mut BTreeMap<Option<i64>, Vec<PermissionTreeItem>>,
    ) -> Vec<PermissionTreeItem> {
        let mut items = grouped.remove(&parent_id).unwrap_or_default();
        items.sort_by_key(|item| (item.sort_no, item.id));
        for item in &mut items {
            item.children = build_children(Some(item.id), grouped);
        }
        items
    }

    let mut roots = build_children(None, &mut grouped);
    regroup_permission_roots(&mut roots);
    for item in &mut roots {
        remap_permission_title(item);
    }
    roots
}

fn regroup_permission_roots(roots: &mut Vec<PermissionTreeItem>) {
    let settings_index = roots.iter().position(|item| item.permission_code == "settings");
    let master_data_index = roots.iter().position(|item| item.permission_code == "master-data");
    let (Some(settings_index), Some(master_data_index)) = (settings_index, master_data_index) else {
        return;
    };

    roots[master_data_index].title = "基础信息".to_string();
    roots[master_data_index].permission_name = "基础信息".to_string();

    let settings_children = std::mem::take(&mut roots[settings_index].children);
    let mut kept_settings_children = Vec::new();
    let mut moved_to_base = Vec::new();

    for child in settings_children {
        match child.permission_code.as_str() {
            "settings:roles" | "settings:permissions" => kept_settings_children.push(child),
            "settings:privacy-policy"
            | "settings:user-agreement"
            | "settings:project-settings"
            | "settings:sms-switch" => moved_to_base.push(remap_setting_child_to_base_info(child)),
            _ => kept_settings_children.push(child),
        }
    }

    roots[settings_index].children = kept_settings_children;
    roots[master_data_index].children.extend(moved_to_base);
    if !roots[master_data_index]
        .children
        .iter()
        .any(|item| item.permission_code == "settings:sms-switch")
    {
        roots[master_data_index].children.push(PermissionTreeItem {
            id: 0,
            key: "settings:sms-switch".to_string(),
            title: "短信开关配置".to_string(),
            permission_code: "settings:sms-switch".to_string(),
            permission_name: "短信开关配置".to_string(),
            permission_type: "menu".to_string(),
            parent_id: None,
            sort_no: 504,
            children: Vec::new(),
        });
    }
    roots[master_data_index]
        .children
        .sort_by_key(|item| (item.sort_no, item.id));
}

fn remap_setting_child_to_base_info(mut item: PermissionTreeItem) -> PermissionTreeItem {
    item.parent_id = None;
    item.sort_no = match item.permission_code.as_str() {
        "settings:project-settings" => 503,
        "settings:sms-switch" => 504,
        "settings:privacy-policy" => 505,
        "settings:user-agreement" => 506,
        _ => item.sort_no,
    };
    item
}

fn remap_permission_title(item: &mut PermissionTreeItem) {
    if item.permission_code == "settings:permissions" {
        item.title = "账号管理".to_string();
        item.permission_name = "账号管理".to_string();
    }
    for child in &mut item.children {
        remap_permission_title(child);
    }
}

async fn find_role(state: &AppState, role_id: i64) -> Result<RoleEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "role", value! { "id": role_id })
        .await
        .context("failed to query role by id")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("角色不存在".to_string()))
}

async fn find_permission(
    state: &AppState,
    permission_id: i64,
) -> Result<PermissionEntity, AppError> {
    db::select_one_by_map_compat(
        state.rb.as_ref(),
        "permission",
        value! { "id": permission_id },
    )
    .await
    .context("failed to query permission by id")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::NotFound("权限不存在".to_string()))
}

async fn ensure_role_name_unique(
    state: &AppState,
    role_name: &str,
    role_id: Option<i64>,
) -> Result<(), AppError> {
    let roles: Vec<RoleEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "role", value! { "role_name": role_name })
            .await
            .context("failed to query role by name")
            .map_err(AppError::Internal)?;
    if roles.into_iter().any(|item| item.id != role_id) {
        return Err(AppError::Conflict("角色名称已存在".to_string()));
    }
    Ok(())
}

async fn ensure_role_code_unique(
    state: &AppState,
    role_code: &str,
    role_id: Option<i64>,
) -> Result<(), AppError> {
    let roles: Vec<RoleEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "role", value! { "role_code": role_code })
            .await
            .context("failed to query role by code")
            .map_err(AppError::Internal)?;
    if roles.into_iter().any(|item| item.id != role_id) {
        return Err(AppError::Conflict("角色编码已存在".to_string()));
    }
    Ok(())
}

async fn generate_role_code(state: &AppState) -> Result<String, AppError> {
    let roles: Vec<RoleEntity> = db::select_by_map_compat(state.rb.as_ref(), "role", value! {})
        .await
        .context("failed to query roles for role code generation")
        .map_err(AppError::Internal)?;

    let max_number = roles
        .into_iter()
        .filter_map(|item| item.role_code)
        .filter_map(|code| {
            code.strip_prefix("ROLE-")
                .and_then(|value| value.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0);
    Ok(format!("ROLE-{:03}", max_number + 1))
}

fn normalize_role_name(value: Option<&str>) -> Result<String, AppError> {
    normalize_optional_text(value)
        .ok_or_else(|| AppError::BadRequest("角色名称不能为空".to_string()))
}

fn normalize_role_status(value: Option<&str>) -> Result<String, AppError> {
    match value
        .unwrap_or("active")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "active" | "enabled" => Ok("active".to_string()),
        "disabled" | "inactive" => Ok("disabled".to_string()),
        _ => Err(AppError::BadRequest(
            "角色状态只能为active/disabled".to_string(),
        )),
    }
}

fn normalize_optional_owned(value: Option<String>) -> Option<String> {
    value.and_then(|item| normalize_optional_text(Some(item.as_str())))
}

fn ensure_backend_role_code(role_code: &str) -> Result<(), AppError> {
    if HIDDEN_BUSINESS_ROLE_CODES.contains(&role_code) {
        return Err(AppError::BadRequest(
            "业务端角色不允许在后台角色管理中维护".to_string(),
        ));
    }
    Ok(())
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
}

fn format_datetime(value: chrono::NaiveDateTime) -> String {
    value.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn ensure_admin_access(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let auth_user = auth::authenticate(state, headers)?;
    auth_user.require_any_role(&["super_admin", "hq_reviewer"])
}
