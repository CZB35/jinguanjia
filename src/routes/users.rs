use anyhow::{Context, anyhow};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use bcrypt::hash;
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    auth,
    db,
    error::AppError,
    models::{role::RoleEntity, store::StoreEntity, task::TaskEntity, user::UserEntity, user_role::UserRoleEntity, user_store::UserStoreEntity},
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct UserListQuery {
    pub mobile: Option<String>,
    pub name: Option<String>,
    pub status: Option<String>,
    pub role_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub name: String,
    pub mobile: String,
    pub password: String,
    pub role_codes: Vec<String>,
    pub store_ids: Option<Vec<i64>>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub role_codes: Option<Vec<String>>,
    pub store_ids: Option<Vec<i64>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserStatusRequest {
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordRequest {
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct UserItem {
    pub id: i64,
    pub name: String,
    pub mobile: String,
    pub status: String,
    pub roles: Vec<String>,
    pub store_names: Vec<String>,
    pub technician_count: usize,
    pub customer_count: usize,
    pub completed_task_count: usize,
}

#[derive(Debug, Serialize)]
pub struct UserListResponse {
    pub items: Vec<UserItem>,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct UserDetailResponse {
    pub id: i64,
    pub name: String,
    pub mobile: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub status: String,
    pub roles: Vec<String>,
    pub store_ids: Vec<i64>,
    pub store_names: Vec<String>,
    pub technician_count: usize,
    pub customer_count: usize,
    pub completed_task_count: usize,
}

pub async fn list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<UserListQuery>,
) -> Result<Json<ApiResponse<UserListResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;

    let mut users: Vec<UserEntity> = db::select_by_map_compat(state.rb.as_ref(), "user", value! {})
        .await
        .context("failed to query users")
        .map_err(AppError::Internal)?;
    users.retain(|user| {
        if let Some(mobile) = &query.mobile
            && !user.mobile.as_deref().unwrap_or_default().contains(mobile)
        {
            return false;
        }
        if let Some(name) = &query.name
            && !user.name.as_deref().unwrap_or_default().contains(name)
        {
            return false;
        }
        if let Some(status) = &query.status
            && user.status.as_deref() != Some(status.as_str())
        {
            return false;
        }
        true
    });

    let mut items = Vec::with_capacity(users.len());
    for user in users {
        let user_id = user.id.unwrap_or_default();
        let roles = find_role_codes(&state, user_id).await?;
        if let Some(role_code) = &query.role_code
            && !roles.iter().any(|role| role == &normalize_role_code_alias(role_code))
        {
            continue;
        }
        let user_stores: Vec<UserStoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_store", value! { "user_id": user_id })
            .await
            .context("failed to query user stores in list_users")
            .map_err(AppError::Internal)?;
        let store_ids: Vec<i64> = user_stores.into_iter().filter_map(|item| item.store_id).collect();
        let store_names = if store_ids.is_empty() {
            Vec::new()
        } else {
            db::select_by_map_compat::<StoreEntity>(state.rb.as_ref(), "store", value! { "id": store_ids.clone() })
                .await
                .context("failed to query store names in list_users")
                .map_err(AppError::Internal)?
                .into_iter()
                .filter_map(|item| item.store_name)
                .collect::<Vec<_>>()
        };
        let stats = aggregate_store_scope_stats(&state, &store_ids).await?;
        items.push(UserItem {
            id: user_id,
            name: user.name.unwrap_or_default(),
            mobile: user.mobile.unwrap_or_default(),
            status: user.status.unwrap_or_else(|| "enabled".to_string()),
            roles,
            store_names,
            technician_count: stats.technician_count,
            customer_count: stats.customer_count,
            completed_task_count: stats.completed_task_count,
        });
    }
    let total = items.len();

    Ok(Json(ApiResponse::success(UserListResponse { items, total })))
}

pub async fn get_user_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<ApiResponse<UserDetailResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    get_user_detail_inner(&state, user_id).await
}

pub async fn create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateUserRequest>,
) -> Result<Json<ApiResponse<UserDetailResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;

    if payload.name.trim().is_empty() {
        return Err(AppError::BadRequest("请输入用户名".to_string()));
    }
    if payload.mobile.trim().is_empty() {
        return Err(AppError::BadRequest("请输入手机号".to_string()));
    }
    if payload.password.trim().len() < 6 {
        return Err(AppError::BadRequest("密码长度不能少于6位".to_string()));
    }
    if payload.role_codes.is_empty() {
        return Err(AppError::BadRequest("请至少分配一个角色".to_string()));
    }

    let normalized_role_codes = payload.role_codes.iter().map(|item| normalize_role_code_alias(item)).collect::<Vec<_>>();
    let store_ids = validate_user_store_binding(&normalized_role_codes, payload.store_ids.clone().unwrap_or_default())?;

    let exists: Vec<UserEntity> = db::select_by_map_compat(state.rb.as_ref(), "user", value! { "mobile": payload.mobile.as_str() })
        .await
        .context("failed to query existing user by mobile")
        .map_err(AppError::Internal)?;
    if !exists.is_empty() {
        return Err(AppError::Conflict("该手机号账号已存在".to_string()));
    }

    let password_hash = hash(&payload.password, 10)
        .context("failed to hash user password")
        .map_err(AppError::Internal)?;
    UserEntity::insert(
        state.rb.as_ref(),
        &UserEntity {
            id: None,
            mobile: Some(payload.mobile.clone()),
            password_hash: Some(password_hash),
            name: Some(payload.name.clone()),
            nickname: None,
            email: None,
            avatar_url: None,
            job_no: None,
            id_card: None,
            emergency_contact_name: None,
            emergency_contact_mobile: None,
            status: Some(payload.status.unwrap_or_else(|| "enabled".to_string())),
            last_login_at: None,
            last_login_ip: None,
            created_at: None,
            updated_at: None,
            deleted_at: None,
        },
    )
    .await
    .context("failed to create user")
    .map_err(AppError::Internal)?;

    let user: UserEntity = db::select_one_by_map_compat(state.rb.as_ref(), "user", value! { "mobile": payload.mobile.as_str() })
        .await
        .context("failed to fetch created user")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow!("user create succeeded but fetch failed")))?;
    let user_id = user.id.unwrap_or_default();

    replace_user_roles(&state, user_id, normalized_role_codes).await?;
    replace_user_stores(&state, user_id, store_ids).await?;

    get_user_detail_inner(&state, user_id).await
}

pub async fn update_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    Json(payload): Json<UpdateUserRequest>,
) -> Result<Json<ApiResponse<UserDetailResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let _ = find_user(&state, user_id).await?;
    let current_role_codes = find_role_codes(&state, user_id).await?;
    let current_store_ids = load_user_store_ids(&state, user_id).await?;
    let next_role_codes = payload
        .role_codes
        .clone()
        .map(|codes| codes.into_iter().map(|item| normalize_role_code_alias(&item)).collect())
        .unwrap_or(current_role_codes);
    let next_store_ids = validate_user_store_binding(
        &next_role_codes,
        payload.store_ids.clone().unwrap_or(current_store_ids),
    )?;

    UserEntity::update_by_map(
        state.rb.as_ref(),
        &UserEntity {
            name: payload.name.clone(),
            email: payload.email.clone(),
            avatar_url: payload.avatar_url.clone(),
            ..Default::default()
        },
        value! { "id": user_id },
    )
    .await
    .context("failed to update user")
    .map_err(AppError::Internal)?;

    if payload.role_codes.is_some() {
        let role_codes = next_role_codes.clone();
        replace_user_roles(&state, user_id, role_codes).await?;
    }
    if payload.store_ids.is_some() || next_store_ids != load_user_store_ids(&state, user_id).await? {
        replace_user_stores(&state, user_id, next_store_ids).await?;
    }

    get_user_detail_inner(&state, user_id).await
}

pub async fn update_user_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    Json(payload): Json<UpdateUserStatusRequest>,
) -> Result<Json<ApiResponse<UserDetailResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    if payload.status != "enabled" && payload.status != "disabled" {
        return Err(AppError::BadRequest("status只能为enabled或disabled".to_string()));
    }

    UserEntity::update_by_map(
        state.rb.as_ref(),
        &UserEntity {
            status: Some(payload.status),
            ..Default::default()
        },
        value! { "id": user_id },
    )
    .await
    .context("failed to update user status")
    .map_err(AppError::Internal)?;

    get_user_detail_inner(&state, user_id).await
}

pub async fn reset_user_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    Json(payload): Json<ResetPasswordRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    if payload.password.trim().len() < 6 {
        return Err(AppError::BadRequest("密码长度不能少于6位".to_string()));
    }

    let password_hash = hash(&payload.password, 10)
        .context("failed to hash reset password")
        .map_err(AppError::Internal)?;
    UserEntity::update_by_map(
        state.rb.as_ref(),
        &UserEntity {
            password_hash: Some(password_hash),
            ..Default::default()
        },
        value! { "id": user_id },
    )
    .await
    .context("failed to reset user password")
    .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(serde_json::json!({ "ok": true }))))
}

async fn get_user_detail_inner(
    state: &AppState,
    user_id: i64,
) -> Result<Json<ApiResponse<UserDetailResponse>>, AppError> {
    let user = find_user(state, user_id).await?;
    let roles = find_role_codes(state, user_id).await?;
    let stores: Vec<UserStoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_store", value! { "user_id": user_id })
        .await
        .context("failed to query user stores")
        .map_err(AppError::Internal)?;
    let store_ids = stores.iter().filter_map(|item| item.store_id).collect::<Vec<_>>();
    let store_names = if store_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<StoreEntity>(state.rb.as_ref(), "store", value! { "id": store_ids.clone() })
            .await
            .context("failed to query user detail store names")
            .map_err(AppError::Internal)?
            .into_iter()
            .filter_map(|item| item.store_name)
            .collect::<Vec<_>>()
    };
    let stats = aggregate_store_scope_stats(state, &store_ids).await?;

    Ok(Json(ApiResponse::success(UserDetailResponse {
        id: user.id.unwrap_or_default(),
        name: user.name.unwrap_or_default(),
        mobile: user.mobile.unwrap_or_default(),
        email: user.email,
        avatar_url: user.avatar_url,
        status: user.status.unwrap_or_else(|| "enabled".to_string()),
        roles,
        store_ids,
        store_names,
        technician_count: stats.technician_count,
        customer_count: stats.customer_count,
        completed_task_count: stats.completed_task_count,
    })))
}

struct StoreScopeStats {
    technician_count: usize,
    customer_count: usize,
    completed_task_count: usize,
}

async fn aggregate_store_scope_stats(state: &AppState, store_ids: &[i64]) -> Result<StoreScopeStats, AppError> {
    if store_ids.is_empty() {
        return Ok(StoreScopeStats {
            technician_count: 0,
            customer_count: 0,
            completed_task_count: 0,
        });
    }

    let related_tasks = db::select_by_map_compat::<TaskEntity>(state.rb.as_ref(), "task", value! { "store_id": store_ids.to_vec() })
        .await
        .context("failed to query related tasks in store scope")
        .map_err(AppError::Internal)?;
    let store_relations = db::select_by_map_compat::<UserStoreEntity>(state.rb.as_ref(), "user_store", value! { "store_id": store_ids.to_vec() })
        .await
        .context("failed to query store user relations in store scope")
        .map_err(AppError::Internal)?;
    let user_ids = store_relations
        .into_iter()
        .filter_map(|item| item.user_id)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let technician_count = count_technician_users(state, &user_ids).await?;
    let customer_count = related_tasks
        .iter()
        .filter_map(|item| item.customer_id)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let completed_task_count = related_tasks
        .iter()
        .filter(|item| item.task_status.as_deref() == Some("completed"))
        .count();

    Ok(StoreScopeStats {
        technician_count,
        customer_count,
        completed_task_count,
    })
}

async fn count_technician_users(state: &AppState, user_ids: &[i64]) -> Result<usize, AppError> {
    if user_ids.is_empty() {
        return Ok(0);
    }

    let technician_role_id = find_role_id_by_code(state, "technician").await?;
    let technician_relations = db::select_by_map_compat::<UserRoleEntity>(
        state.rb.as_ref(),
        "user_role",
        value! { "role_id": technician_role_id, "user_id": user_ids.to_vec() },
    )
    .await
    .context("failed to query technician role relations in store scope")
    .map_err(AppError::Internal)?;

    Ok(technician_relations
        .into_iter()
        .filter_map(|item| item.user_id)
        .collect::<std::collections::BTreeSet<_>>()
        .len())
}

async fn replace_user_roles(state: &AppState, user_id: i64, role_codes: Vec<String>) -> Result<(), AppError> {
    let existing: Vec<UserRoleEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_role", value! { "user_id": user_id })
        .await
        .context("failed to query existing user roles")
        .map_err(AppError::Internal)?;
    for item in existing {
        if let Some(id) = item.id {
            UserRoleEntity::delete_by_map(state.rb.as_ref(), value! { "id": id })
                .await
                .context("failed to delete existing user role")
                .map_err(AppError::Internal)?;
        }
    }

    for role_code in role_codes {
        let role_id = find_role_id_by_code(state, &role_code).await?;
        UserRoleEntity::insert(
            state.rb.as_ref(),
            &UserRoleEntity {
                id: None,
                user_id: Some(user_id),
                role_id: Some(role_id),
                created_at: None,
            },
        )
        .await
        .context("failed to bind user role")
        .map_err(AppError::Internal)?;
    }
    Ok(())
}

async fn replace_user_stores(state: &AppState, user_id: i64, store_ids: Vec<i64>) -> Result<(), AppError> {
    let existing: Vec<UserStoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_store", value! { "user_id": user_id })
        .await
        .context("failed to query existing user stores")
        .map_err(AppError::Internal)?;
    for item in existing {
        if let Some(id) = item.id {
            UserStoreEntity::delete_by_map(state.rb.as_ref(), value! { "id": id })
                .await
                .context("failed to delete existing user store")
                .map_err(AppError::Internal)?;
        }
    }

    for store_id in store_ids {
        UserStoreEntity::insert(
            state.rb.as_ref(),
            &UserStoreEntity {
                id: None,
                user_id: Some(user_id),
                store_id: Some(store_id),
                created_at: None,
            },
        )
        .await
        .context("failed to bind user store")
        .map_err(AppError::Internal)?;
    }
    Ok(())
}

async fn load_user_store_ids(state: &AppState, user_id: i64) -> Result<Vec<i64>, AppError> {
    let stores: Vec<UserStoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_store", value! { "user_id": user_id })
        .await
        .context("failed to query user stores")
        .map_err(AppError::Internal)?;

    Ok(stores.into_iter().filter_map(|item| item.store_id).collect())
}

fn validate_user_store_binding(role_codes: &[String], store_ids: Vec<i64>) -> Result<Vec<i64>, AppError> {
    let requires_single_store = role_codes.iter().any(|role| role == "dealer" || role == "technician");

    if !requires_single_store {
        return Ok(store_ids);
    }

    if store_ids.len() != 1 {
        return Err(AppError::BadRequest("经销商和师傅账号必须且只能绑定一个门店".to_string()));
    }

    Ok(store_ids)
}

fn normalize_role_code_alias(role_code: &str) -> String {
    match role_code.trim().to_ascii_lowercase().as_str() {
        "admin" | "role-001" => "super_admin".to_string(),
        "inspection_admin" | "role-002" => "hq_reviewer".to_string(),
        "dealer_manager" | "role-003" => "dealer".to_string(),
        "readonly_admin" | "role-004" => "hq_reviewer".to_string(),
        _ => role_code.trim().to_string(),
    }
}

async fn find_role_codes(state: &AppState, user_id: i64) -> Result<Vec<String>, AppError> {
    let relations: Vec<UserRoleEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_role", value! { "user_id": user_id })
        .await
        .context("failed to query user role relations")
        .map_err(AppError::Internal)?;
    let role_ids: Vec<i64> = relations.into_iter().filter_map(|item| item.role_id).collect();
    if role_ids.is_empty() {
        return Ok(Vec::new());
    }
    let roles: Vec<RoleEntity> = db::select_by_map_compat(state.rb.as_ref(), "role", value! { "id": role_ids })
        .await
        .context("failed to query roles by ids")
        .map_err(AppError::Internal)?;

    Ok(roles.into_iter().filter_map(|item| item.role_code).collect())
}

async fn find_role_id_by_code(state: &AppState, role_code: &str) -> Result<i64, AppError> {
    db::select_one_by_map_compat::<RoleEntity>(state.rb.as_ref(), "role", value! { "role_code": role_code })
        .await
        .context("failed to query role by code")
        .map_err(AppError::Internal)?
        .and_then(|item| item.id)
        .ok_or_else(|| AppError::Internal(anyhow!("role not found: {role_code}")))
}

async fn find_user(state: &AppState, user_id: i64) -> Result<UserEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "user", value! { "id": user_id })
        .await
        .context("failed to query user by id")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("用户不存在".to_string()))
}

fn ensure_admin_access(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let auth_user = auth::authenticate(state, headers)?;
    auth_user.require_any_role(&["super_admin", "hq_reviewer"])
}
