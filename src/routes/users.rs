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
    auth, db,
    error::AppError,
    models::{
        role::RoleEntity, store::StoreEntity, task::TaskEntity, user::UserEntity,
        user_role::UserRoleEntity, user_store::UserStoreEntity,
    },
    response::ApiResponse,
    state::AppState,
};

const BACKEND_ROLE_CODES: &[&str] = &["super_admin", "hq_reviewer"];

#[derive(Debug, Deserialize)]
pub struct UserListQuery {
    pub account: Option<String>,
    pub mobile: Option<String>,
    pub name: Option<String>,
    pub status: Option<String>,
    pub role_code: Option<String>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct DealerListQuery {
    pub mobile: Option<String>,
    pub name: Option<String>,
    pub status: Option<String>,
    pub store_id: Option<i64>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub account: Option<String>,
    #[serde(alias = "username")]
    pub username: Option<String>,
    pub name: String,
    pub mobile: String,
    pub password: String,
    pub role_codes: Vec<String>,
    pub store_ids: Option<Vec<i64>>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDealerRequest {
    pub name: String,
    pub mobile: String,
    pub password: Option<String>,
    pub store_ids: Option<Vec<i64>>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub account: Option<String>,
    #[serde(alias = "username")]
    pub username: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub role_codes: Option<Vec<String>>,
    pub store_ids: Option<Vec<i64>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDealerRequest {
    pub name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
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
    pub account: String,
    pub mobile: String,
    pub status: String,
    pub status_code: String,
    pub status_text: String,
    pub roles: Vec<String>,
    pub role_names_text: String,
    pub store_names: Vec<String>,
    pub technician_count: usize,
    pub customer_count: usize,
    pub completed_task_count: usize,
}

#[derive(Debug, Serialize)]
pub struct UserListResponse {
    pub items: Vec<UserItem>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, Serialize)]
pub struct UserDetailResponse {
    pub id: i64,
    pub name: String,
    pub account: String,
    pub mobile: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub status: String,
    pub status_code: String,
    pub status_text: String,
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
        if let Some(account) = &query.account
            && !user.account.as_deref().unwrap_or_default().contains(account)
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
        let backend_roles = roles
            .iter()
            .filter(|role| BACKEND_ROLE_CODES.contains(&role.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if backend_roles.is_empty() {
            continue;
        }
        if let Some(role_code) = &query.role_code
            && !backend_roles
                .iter()
                .any(|role| role == &normalize_role_code_alias(role_code))
        {
            continue;
        }
        let user_stores: Vec<UserStoreEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "user_store",
            value! { "user_id": user_id },
        )
        .await
        .context("failed to query user stores in list_users")
        .map_err(AppError::Internal)?;
        let store_ids: Vec<i64> = user_stores
            .into_iter()
            .filter_map(|item| item.store_id)
            .collect();
        let store_names = if store_ids.is_empty() {
            Vec::new()
        } else {
            db::select_by_map_compat::<StoreEntity>(
                state.rb.as_ref(),
                "store",
                value! { "id": store_ids.clone() },
            )
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
            account: user.account.clone().unwrap_or_default(),
            mobile: user.mobile.unwrap_or_default(),
            status: user_status_text(user.status.as_deref()).to_string(),
            status_code: user.status.clone().unwrap_or_else(|| "enabled".to_string()),
            status_text: user_status_text(user.status.as_deref()).to_string(),
            roles: backend_roles,
            role_names_text: role_names_text(&roles),
            store_names,
            technician_count: stats.technician_count,
            customer_count: stats.customer_count,
            completed_task_count: stats.completed_task_count,
        });
    }
    let total = items.len();
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).max(1);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let items = items.into_iter().skip(start).take(page_size).collect::<Vec<_>>();

    Ok(Json(ApiResponse::success(UserListResponse {
        items,
        total,
        page,
        page_size,
    })))
}

pub async fn list_dealers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<DealerListQuery>,
) -> Result<Json<ApiResponse<UserListResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;

    let users: Vec<UserEntity> = db::select_by_map_compat(state.rb.as_ref(), "user", value! {})
        .await
        .context("failed to query dealers")
        .map_err(AppError::Internal)?;
    let mut items = Vec::new();
    for user in users {
        let user_id = user.id.unwrap_or_default();
        let roles = find_role_codes(&state, user_id).await?;
        if !roles.iter().any(|role| role == "dealer") {
            continue;
        }
        if let Some(mobile) = &query.mobile
            && !user.mobile.as_deref().unwrap_or_default().contains(mobile)
        {
            continue;
        }
        if let Some(name) = &query.name
            && !user.name.as_deref().unwrap_or_default().contains(name)
        {
            continue;
        }
        if let Some(status) = &query.status
            && user.status.as_deref() != Some(status.as_str())
        {
            continue;
        }

        let store_ids = load_user_store_ids(&state, user_id).await?;
        if let Some(store_id) = query.store_id
            && !store_ids.contains(&store_id)
        {
            continue;
        }
        let store_names = if store_ids.is_empty() {
            Vec::new()
        } else {
            db::select_by_map_compat::<StoreEntity>(
                state.rb.as_ref(),
                "store",
                value! { "id": store_ids.clone() },
            )
            .await
            .context("failed to query dealer store names")
            .map_err(AppError::Internal)?
            .into_iter()
            .filter_map(|item| item.store_name)
            .collect::<Vec<_>>()
        };
        let stats = aggregate_store_scope_stats(&state, &store_ids).await?;
        items.push(UserItem {
            id: user_id,
            name: user.name.unwrap_or_default(),
            account: user.mobile.clone().unwrap_or_default(),
            mobile: user.mobile.unwrap_or_default(),
            status: user_status_text(user.status.as_deref()).to_string(),
            status_code: user.status.clone().unwrap_or_else(|| "enabled".to_string()),
            status_text: user_status_text(user.status.as_deref()).to_string(),
            role_names_text: role_names_text(&roles),
            roles,
            store_names,
            technician_count: stats.technician_count,
            customer_count: stats.customer_count,
            completed_task_count: stats.completed_task_count,
        });
    }

    let total = items.len();
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).max(1);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let items = items.into_iter().skip(start).take(page_size).collect::<Vec<_>>();

    Ok(Json(ApiResponse::success(UserListResponse {
        total,
        items,
        page,
        page_size,
    })))
}

pub async fn get_dealer_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<ApiResponse<UserDetailResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    ensure_dealer_user(&state, user_id).await?;
    get_user_detail_inner(&state, user_id).await
}

pub async fn create_dealer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateDealerRequest>,
) -> Result<Json<ApiResponse<UserDetailResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    if payload.name.trim().is_empty() {
        return Err(AppError::BadRequest("请输入经销商名称".to_string()));
    }
    if payload.mobile.trim().is_empty() {
        return Err(AppError::BadRequest("请输入手机号".to_string()));
    }
    let password = payload
        .password
        .clone()
        .filter(|item| !item.trim().is_empty())
        .unwrap_or_else(|| "123456".to_string());
    if password.trim().len() < 6 {
        return Err(AppError::BadRequest("密码长度不能少于6位".to_string()));
    }

    let dealer_roles = vec!["dealer".to_string()];
    let store_ids = validate_user_store_binding(&dealer_roles, payload.store_ids.clone().unwrap_or_default())?;
    let existing = find_user_by_mobile(&state, &payload.mobile).await?;
    let user = if let Some(existing_user) = existing {
        if existing_user.deleted_at.is_some() {
            return Err(AppError::Conflict("该手机号账号已存在且已删除，请联系管理员处理".to_string()));
        }
        existing_user
    } else {
        let password_hash = hash(&password, 10)
            .context("failed to hash dealer password")
            .map_err(AppError::Internal)?;
        UserEntity::insert(
            state.rb.as_ref(),
            &UserEntity {
                id: None,
                mobile: Some(payload.mobile.clone()),
                password_hash: Some(password_hash),
                name: Some(payload.name.clone()),
                status: Some(payload.status.unwrap_or_else(|| "enabled".to_string())),
                ..Default::default()
            },
        )
        .await
        .context("failed to create dealer user")
        .map_err(AppError::Internal)?;
        db::select_one_by_map_compat(
            state.rb.as_ref(),
            "user",
            value! { "mobile": payload.mobile.as_str() },
        )
        .await
        .context("failed to fetch created dealer user")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow!("dealer create succeeded but fetch failed")))?
    };

    let user_id = user.id.unwrap_or_default();
    ensure_specific_role_binding(&state, user_id, "dealer").await?;
    replace_user_stores(&state, user_id, store_ids).await?;
    get_user_detail_inner(&state, user_id).await
}

pub async fn update_dealer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    Json(payload): Json<UpdateDealerRequest>,
) -> Result<Json<ApiResponse<UserDetailResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    ensure_dealer_user(&state, user_id).await?;
    let current_store_ids = load_user_store_ids(&state, user_id).await?;
    let dealer_roles = vec!["dealer".to_string()];
    let next_store_ids = validate_user_store_binding(
        &dealer_roles,
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
    .context("failed to update dealer")
    .map_err(AppError::Internal)?;

    if payload.store_ids.is_some() || next_store_ids != load_user_store_ids(&state, user_id).await? {
        replace_user_stores(&state, user_id, next_store_ids).await?;
    }
    get_user_detail_inner(&state, user_id).await
}

pub async fn reset_dealer_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    ensure_dealer_user(&state, user_id).await?;
    reset_password_to_default(&state, user_id).await
}

pub async fn get_user_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<ApiResponse<UserDetailResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    ensure_backend_user(&state, user_id).await?;
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
    let account = payload
        .account
        .as_deref()
        .or(payload.username.as_deref())
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .ok_or_else(|| AppError::BadRequest("请输入账号".to_string()))?
        .to_string();
    if payload.mobile.trim().is_empty() {
        return Err(AppError::BadRequest("请输入手机号".to_string()));
    }
    if payload.password.trim().len() < 6 {
        return Err(AppError::BadRequest("密码长度不能少于6位".to_string()));
    }
    if payload.role_codes.is_empty() {
        return Err(AppError::BadRequest("请至少分配一个角色".to_string()));
    }

    let normalized_role_codes = payload
        .role_codes
        .iter()
        .map(|item| normalize_role_code_alias(item))
        .collect::<Vec<_>>();
    ensure_backend_role_codes_only(&normalized_role_codes)?;
    let store_ids = validate_user_store_binding(
        &normalized_role_codes,
        payload.store_ids.clone().unwrap_or_default(),
    )?;

    let exists: Vec<UserEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user",
        value! { "mobile": payload.mobile.as_str() },
    )
    .await
    .context("failed to query existing user by mobile")
    .map_err(AppError::Internal)?;
    if !exists.is_empty() {
        return Err(AppError::Conflict("该手机号账号已存在".to_string()));
    }
    let account_exists = db::select_one_by_map_compat::<UserEntity>(
        state.rb.as_ref(),
        "user",
        value! { "account": account.as_str() },
    )
    .await
    .context("failed to query existing user by account")
    .map_err(AppError::Internal)?;
    if account_exists.is_some() {
        return Err(AppError::Conflict("该账号已存在".to_string()));
    }

    let password_hash = hash(&payload.password, 10)
        .context("failed to hash user password")
        .map_err(AppError::Internal)?;
    UserEntity::insert(
        state.rb.as_ref(),
        &UserEntity {
            id: None,
            mobile: Some(payload.mobile.clone()),
            account: Some(account.clone()),
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
            wechat_openid: None,
            wechat_unionid: None,
            wechat_subscribe: Some(0),
            wechat_bind_at: None,
            wechat_subscribe_updated_at: None,
        },
    )
    .await
    .context("failed to create user")
    .map_err(AppError::Internal)?;

    let user: UserEntity = db::select_one_by_map_compat(
        state.rb.as_ref(),
        "user",
        value! { "account": account.as_str() },
    )
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
    ensure_backend_user(&state, user_id).await?;
    let current_role_codes = find_role_codes(&state, user_id).await?;
    let current_store_ids = load_user_store_ids(&state, user_id).await?;
    let next_role_codes = payload
        .role_codes
        .clone()
        .map(|codes| {
            codes
                .into_iter()
                .map(|item| normalize_role_code_alias(&item))
                .collect()
        })
        .unwrap_or(current_role_codes);
    ensure_backend_role_codes_only(&next_role_codes)?;
    if let Some(account) = payload
        .account
        .as_deref()
        .or(payload.username.as_deref())
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let account_exists = db::select_by_map_compat::<UserEntity>(
            state.rb.as_ref(),
            "user",
            value! { "account": account },
        )
        .await
        .context("failed to query user account before update")
        .map_err(AppError::Internal)?;
        if account_exists.into_iter().any(|item| item.id != Some(user_id)) {
            return Err(AppError::Conflict("该账号已存在".to_string()));
        }
    }
    let next_store_ids = validate_user_store_binding(
        &next_role_codes,
        payload.store_ids.clone().unwrap_or(current_store_ids),
    )?;

    UserEntity::update_by_map(
        state.rb.as_ref(),
        &UserEntity {
            account: payload.account.clone().or(payload.username.clone()),
            name: payload.name.clone(),
            email: payload.email.clone(),
            avatar_url: payload.avatar_url.clone(),
            ..Default::default()
        },
        value! { "id": user_id, "column": ["account", "name", "email", "avatar_url"] },
    )
    .await
    .context("failed to update user")
    .map_err(AppError::Internal)?;

    if payload.role_codes.is_some() {
        let role_codes = next_role_codes.clone();
        replace_user_roles(&state, user_id, role_codes).await?;
    }
    if payload.store_ids.is_some() || next_store_ids != load_user_store_ids(&state, user_id).await?
    {
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
    ensure_backend_user(&state, user_id).await?;
    if payload.status != "enabled" && payload.status != "disabled" {
        return Err(AppError::BadRequest(
            "status只能为enabled或disabled".to_string(),
        ));
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
    ensure_backend_user(&state, user_id).await?;
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

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

async fn reset_password_to_default(
    state: &AppState,
    user_id: i64,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let password_hash = hash("123456", 10)
        .context("failed to hash default reset password")
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
    .context("failed to reset default password")
    .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true, "default_password": "123456" }),
    )))
}

async fn get_user_detail_inner(
    state: &AppState,
    user_id: i64,
) -> Result<Json<ApiResponse<UserDetailResponse>>, AppError> {
    let user = find_user(state, user_id).await?;
    let roles = find_role_codes(state, user_id).await?;
    let stores: Vec<UserStoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_store",
        value! { "user_id": user_id },
    )
    .await
    .context("failed to query user stores")
    .map_err(AppError::Internal)?;
    let store_ids = stores
        .iter()
        .filter_map(|item| item.store_id)
        .collect::<Vec<_>>();
    let store_names = if store_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<StoreEntity>(
            state.rb.as_ref(),
            "store",
            value! { "id": store_ids.clone() },
        )
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
        account: user.account.unwrap_or_default(),
        mobile: user.mobile.unwrap_or_default(),
        email: user.email,
        avatar_url: user.avatar_url,
        status: user_status_text(user.status.as_deref()).to_string(),
        status_code: user.status.clone().unwrap_or_else(|| "enabled".to_string()),
        status_text: user_status_text(user.status.as_deref()).to_string(),
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

async fn aggregate_store_scope_stats(
    state: &AppState,
    store_ids: &[i64],
) -> Result<StoreScopeStats, AppError> {
    if store_ids.is_empty() {
        return Ok(StoreScopeStats {
            technician_count: 0,
            customer_count: 0,
            completed_task_count: 0,
        });
    }

    let related_tasks = db::select_by_map_compat::<TaskEntity>(
        state.rb.as_ref(),
        "task",
        value! { "store_id": store_ids.to_vec() },
    )
    .await
    .context("failed to query related tasks in store scope")
    .map_err(AppError::Internal)?;
    let store_relations = db::select_by_map_compat::<UserStoreEntity>(
        state.rb.as_ref(),
        "user_store",
        value! { "store_id": store_ids.to_vec() },
    )
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

async fn ensure_specific_role_binding(
    state: &AppState,
    user_id: i64,
    role_code: &str,
) -> Result<(), AppError> {
    let role_id = find_role_id_by_code(state, role_code).await?;
    let existing: Vec<UserRoleEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": user_id, "role_id": role_id },
    )
    .await
    .context("failed to query specific role binding")
    .map_err(AppError::Internal)?;
    if !existing.is_empty() {
        return Ok(());
    }

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
    .context("failed to bind specific role")
    .map_err(AppError::Internal)?;
    Ok(())
}

async fn replace_user_roles(
    state: &AppState,
    user_id: i64,
    role_codes: Vec<String>,
) -> Result<(), AppError> {
    let existing: Vec<UserRoleEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": user_id },
    )
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

async fn replace_user_stores(
    state: &AppState,
    user_id: i64,
    store_ids: Vec<i64>,
) -> Result<(), AppError> {
    let existing: Vec<UserStoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_store",
        value! { "user_id": user_id },
    )
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
    let stores: Vec<UserStoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_store",
        value! { "user_id": user_id },
    )
    .await
    .context("failed to query user stores")
    .map_err(AppError::Internal)?;

    Ok(stores
        .into_iter()
        .filter_map(|item| item.store_id)
        .collect())
}

fn validate_user_store_binding(
    role_codes: &[String],
    store_ids: Vec<i64>,
) -> Result<Vec<i64>, AppError> {
    let requires_single_store = role_codes
        .iter()
        .any(|role| role == "dealer" || role == "technician");

    if !requires_single_store {
        return Ok(store_ids);
    }

    if store_ids.len() != 1 {
        return Err(AppError::BadRequest(
            "经销商和师傅账号必须且只能绑定一个门店".to_string(),
        ));
    }

    Ok(store_ids)
}

fn ensure_backend_role_codes_only(role_codes: &[String]) -> Result<(), AppError> {
    if role_codes
        .iter()
        .all(|role| BACKEND_ROLE_CODES.contains(&role.as_str()))
    {
        return Ok(());
    }
    Err(AppError::BadRequest(
        "后台账号仅允许分配后台管理角色".to_string(),
    ))
}

async fn ensure_backend_user(state: &AppState, user_id: i64) -> Result<(), AppError> {
    let role_codes = find_role_codes(state, user_id).await?;
    ensure_backend_role_codes_only(&role_codes)
}

async fn ensure_dealer_user(state: &AppState, user_id: i64) -> Result<(), AppError> {
    let role_codes = find_role_codes(state, user_id).await?;
    if role_codes.iter().any(|role| role == "dealer") {
        return Ok(());
    }
    Err(AppError::BadRequest("当前账号不是经销商账号".to_string()))
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
    let relations: Vec<UserRoleEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": user_id },
    )
    .await
    .context("failed to query user role relations")
    .map_err(AppError::Internal)?;
    let role_ids: Vec<i64> = relations
        .into_iter()
        .filter_map(|item| item.role_id)
        .collect();
    if role_ids.is_empty() {
        return Ok(Vec::new());
    }
    let roles: Vec<RoleEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "role", value! { "id": role_ids })
            .await
            .context("failed to query roles by ids")
            .map_err(AppError::Internal)?;

    Ok(roles
        .into_iter()
        .filter_map(|item| item.role_code)
        .collect())
}

async fn find_role_id_by_code(state: &AppState, role_code: &str) -> Result<i64, AppError> {
    db::select_one_by_map_compat::<RoleEntity>(
        state.rb.as_ref(),
        "role",
        value! { "role_code": role_code },
    )
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

async fn find_user_by_mobile(state: &AppState, mobile: &str) -> Result<Option<UserEntity>, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "user", value! { "mobile": mobile })
        .await
        .context("failed to query user by mobile")
        .map_err(AppError::Internal)
}

fn user_status_text(status: Option<&str>) -> &'static str {
    match status.unwrap_or("enabled") {
        "enabled" | "active" => "已启用",
        "disabled" | "inactive" => "已禁用",
        _ => "未知状态",
    }
}

fn role_names_text(role_codes: &[String]) -> String {
    role_codes
        .iter()
        .map(|role| match role.as_str() {
            "super_admin" => "超级管理员",
            "hq_reviewer" => "总部审核人",
            "dealer" => "经销商",
            "technician" => "技师",
            _ => role.as_str(),
        })
        .collect::<Vec<_>>()
        .join("、")
}

fn ensure_admin_access(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let auth_user = auth::authenticate(state, headers)?;
    auth_user.require_any_role(&["super_admin", "hq_reviewer"])
}
