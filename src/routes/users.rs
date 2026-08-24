use anyhow::{Context, anyhow};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use bcrypt::hash;
use chrono::Utc;
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    audit, auth, db,
    error::AppError,
    identity,
    models::{
        role::RoleEntity, store::StoreEntity, task::TaskEntity, user::UserEntity,
        user_role::UserRoleEntity, user_store::UserStoreEntity,
    },
    response::ApiResponse,
    routes::imports::{deserialize_vec_i64_from_mixed, normalize_delete_ids},
    state::AppState,
};

const BACKEND_ROLE_CODES: &[&str] = &["super_admin", "hq_reviewer"];

#[derive(Debug, Deserialize)]
pub struct UserListQuery {
    pub account: Option<String>,
    #[serde(alias = "phone")]
    pub mobile: Option<String>,
    pub name: Option<String>,
    pub status: Option<String>,
    #[serde(alias = "roleCode")]
    pub role_code: Option<String>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct DealerListQuery {
    #[serde(alias = "phone")]
    pub mobile: Option<String>,
    pub name: Option<String>,
    pub status: Option<String>,
    #[serde(alias = "storeId")]
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
    #[serde(alias = "phone")]
    pub mobile: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub store_ids: Option<Vec<i64>>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BatchDeleteDealersRequest {
    #[serde(
        default,
        alias = "ids",
        alias = "user_ids",
        alias = "dealerIds",
        deserialize_with = "deserialize_vec_i64_from_mixed"
    )]
    pub dealer_ids: Vec<i64>,
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
    let status_filter = normalize_optional_user_status(query.status.as_deref())?;

    let mut users: Vec<UserEntity> = db::select_by_map_compat(state.rb.as_ref(), "user", value! {})
        .await
        .context("failed to query users")
        .map_err(AppError::Internal)?;
    users.retain(|user| {
        if user.deleted_at.is_some() {
            return false;
        }
        if let Some(mobile) = &query.mobile
            && !user.mobile.as_deref().unwrap_or_default().contains(mobile)
        {
            return false;
        }
        if let Some(account) = &query.account
            && !user
                .account
                .as_deref()
                .unwrap_or_default()
                .contains(account)
        {
            return false;
        }
        if let Some(name) = &query.name
            && !user.name.as_deref().unwrap_or_default().contains(name)
        {
            return false;
        }
        if let Some(status) = &status_filter
            && canonical_user_status(user.status.as_deref()) != status.as_str()
        {
            return false;
        }
        true
    });

    users.sort_by(|left, right| {
        right.created_at.cmp(&left.created_at).then_with(|| {
            right
                .id
                .unwrap_or_default()
                .cmp(&left.id.unwrap_or_default())
        })
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
            status_code: canonical_user_status(user.status.as_deref()),
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
    let items = items
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();

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
    let status_filter = normalize_optional_user_status(query.status.as_deref())?;

    let mut users: Vec<UserEntity> = db::select_by_map_compat(state.rb.as_ref(), "user", value! {})
        .await
        .context("failed to query dealers")
        .map_err(AppError::Internal)?;
    users.sort_by(|left, right| {
        right.created_at.cmp(&left.created_at).then_with(|| {
            right
                .id
                .unwrap_or_default()
                .cmp(&left.id.unwrap_or_default())
        })
    });
    let mut items = Vec::new();
    for user in users {
        if user.deleted_at.is_some() {
            continue;
        }
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
        let dealer_name =
            identity::role_display_name(&state, user_id, "dealer", user.name.as_deref()).await?;
        if let Some(name) = &query.name
            && !dealer_name.contains(name)
        {
            continue;
        }
        let dealer_status = user_role_status(&state, user_id, "dealer").await?;
        if let Some(status) = &status_filter
            && dealer_status != *status
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
            name: dealer_name,
            account: user.mobile.clone().unwrap_or_default(),
            mobile: user.mobile.unwrap_or_default(),
            status: user_status_text(Some(&dealer_status)).to_string(),
            status_code: dealer_status.clone(),
            status_text: user_status_text(Some(&dealer_status)).to_string(),
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
    let items = items
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();

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
    let normalized_status = normalize_user_status(payload.status.as_deref())?;

    let dealer_roles = vec!["dealer".to_string()];
    let store_ids =
        validate_user_store_binding(&dealer_roles, payload.store_ids.clone().unwrap_or_default())?;
    let existing = find_user_by_mobile(&state, &payload.mobile).await?;
    let before_user = existing.clone();
    let user_existed_before = existing.is_some();
    let user = if let Some(existing_user) = existing {
        if existing_user.deleted_at.is_some() {
            let user_id = existing_user
                .id
                .ok_or_else(|| AppError::Internal(anyhow!("deleted dealer user.id missing")))?;
            let password_hash = hash(&password, 10)
                .context("failed to hash restored dealer password")
                .map_err(AppError::Internal)?;
            UserEntity::update_by_map(
                state.rb.as_ref(),
                &UserEntity {
                    password_hash: Some(password_hash),
                    name: Some(payload.name.clone()),
                    nickname: None,
                    email: None,
                    avatar_url: None,
                    job_no: None,
                    id_card: None,
                    emergency_contact_name: None,
                    emergency_contact_mobile: None,
                    status: Some("enabled".to_string()),
                    last_login_at: None,
                    last_login_ip: None,
                    deleted_at: None,
                    wechat_openid: None,
                    wechat_unionid: None,
                    wechat_subscribe: Some(0),
                    wechat_bind_at: None,
                    wechat_subscribe_updated_at: None,
                    ..Default::default()
                },
                value! {
                    "id": user_id,
                    "column": [
                        "password_hash",
                        "name",
                        "nickname",
                        "email",
                        "avatar_url",
                        "job_no",
                        "id_card",
                        "emergency_contact_name",
                        "emergency_contact_mobile",
                        "status",
                        "last_login_at",
                        "last_login_ip",
                        "deleted_at",
                        "wechat_openid",
                        "wechat_unionid",
                        "wechat_subscribe",
                        "wechat_bind_at",
                        "wechat_subscribe_updated_at"
                    ]
                },
            )
            .await
            .context("failed to restore deleted dealer user")
            .map_err(AppError::Internal)?;
            UserRoleEntity::delete_by_map(state.rb.as_ref(), value! { "user_id": user_id })
                .await
                .context("failed to clear roles for restored dealer user")
                .map_err(AppError::Internal)?;
            db::select_one_by_map_compat(state.rb.as_ref(), "user", value! { "id": user_id })
                .await
                .context("failed to fetch restored dealer user")
                .map_err(AppError::Internal)?
                .ok_or_else(|| {
                    AppError::Internal(anyhow!("dealer restore succeeded but fetch failed"))
                })?
        } else {
            existing_user
        }
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
                status: Some("enabled".to_string()),
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
    identity::update_role_display_name(&state, user_id, "dealer", &payload.name).await?;
    update_user_role_status(&state, user_id, "dealer", &normalized_status).await?;
    replace_user_stores(&state, user_id, store_ids).await?;
    if user_existed_before {
        auth::bump_user_session_version(&state, user_id).await?;
    }
    let detail = get_user_detail_inner(&state, user_id).await?;
    let after_user = find_user(&state, user_id).await?;
    let auth_user = auth::authenticate(&state, &headers)?;
    audit::write_audit_log(
        &state,
        "user",
        user_id,
        "create_dealer",
        auth_user.user_id,
        auth_user.primary_role(),
        before_user.as_ref(),
        Some(&after_user),
        Some("后台新增经销商".to_string()),
    )
    .await?;
    Ok(detail)
}

pub async fn update_dealer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    Json(payload): Json<UpdateDealerRequest>,
) -> Result<Json<ApiResponse<UserDetailResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    ensure_dealer_user(&state, user_id).await?;
    let auth_user = auth::authenticate(&state, &headers)?;
    let before_user = find_user(&state, user_id).await?;
    let next_mobile = payload
        .mobile
        .as_deref()
        .map(str::trim)
        .map(ToString::to_string);
    if matches!(next_mobile.as_deref(), Some("")) {
        return Err(AppError::BadRequest("请输入手机号".to_string()));
    }
    let mobile_changed =
        next_mobile.as_deref() != before_user.mobile.as_deref() && next_mobile.is_some();
    if mobile_changed
        && let Some(existing) =
            find_user_by_mobile(&state, next_mobile.as_deref().unwrap_or_default()).await?
        && existing.id != Some(user_id)
    {
        return Err(AppError::Conflict("该手机号账号已存在".to_string()));
    }
    let current_store_ids = load_user_store_ids(&state, user_id).await?;
    let dealer_roles = vec!["dealer".to_string()];
    let normalized_status = normalize_optional_user_status(payload.status.as_deref())?;
    let next_store_ids = validate_user_store_binding(
        &dealer_roles,
        payload
            .store_ids
            .clone()
            .unwrap_or_else(|| current_store_ids.clone()),
    )?;

    let mut user_update = UserEntity::default();
    let mut user_columns = Vec::new();
    if mobile_changed {
        user_update.mobile = next_mobile;
        user_columns.push("mobile");
    }
    if let Some(email) = payload.email.clone() {
        user_update.email = Some(email);
        user_columns.push("email");
    }
    if let Some(avatar_url) = payload.avatar_url.clone() {
        user_update.avatar_url = Some(avatar_url);
        user_columns.push("avatar_url");
    }
    if !user_columns.is_empty() {
        UserEntity::update_by_map(
            state.rb.as_ref(),
            &user_update,
            value! { "id": user_id, "column": user_columns },
        )
        .await
        .map_err(|error| {
            let message = error.to_string();
            if message.contains("Duplicate entry") || message.contains("1062") {
                AppError::Conflict("该手机号账号已存在".to_string())
            } else {
                AppError::Internal(anyhow!("failed to update dealer: {error}"))
            }
        })?;
    }

    if let Some(name) = payload.name.as_deref() {
        if name.trim().is_empty() {
            return Err(AppError::BadRequest("请输入经销商名称".to_string()));
        }
        identity::update_role_display_name(&state, user_id, "dealer", name).await?;
    }
    if let Some(status) = &normalized_status {
        update_user_role_status(&state, user_id, "dealer", status).await?;
    }
    let mut should_bump_session = mobile_changed;
    if next_store_ids != current_store_ids {
        replace_user_stores(&state, user_id, next_store_ids).await?;
        should_bump_session = true;
    }
    if should_bump_session {
        auth::bump_user_session_version(&state, user_id).await?;
    }
    let detail = get_user_detail_inner(&state, user_id).await?;
    let after_user = find_user(&state, user_id).await?;
    audit::write_audit_log(
        &state,
        "user",
        user_id,
        "update_dealer",
        auth_user.user_id,
        auth_user.primary_role(),
        Some(&before_user),
        Some(&after_user),
        Some("后台编辑经销商".to_string()),
    )
    .await?;
    Ok(detail)
}

pub async fn delete_dealer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["super_admin", "hq_reviewer"])?;
    ensure_not_current_user(&auth_user, user_id)?;

    let dealer = find_user(&state, user_id).await?;
    ensure_dealer_user(&state, user_id).await?;
    soft_delete_managed_user(
        &state,
        &auth_user,
        &dealer,
        "delete_dealer",
        "后台删除经销商",
    )
    .await?;

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn batch_delete_dealers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<BatchDeleteDealersRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["super_admin", "hq_reviewer"])?;
    let dealer_ids = normalize_delete_ids(payload.dealer_ids, "dealer_ids")?;

    let mut dealers = Vec::with_capacity(dealer_ids.len());
    for user_id in dealer_ids {
        ensure_not_current_user(&auth_user, user_id)?;
        let dealer = find_user(&state, user_id).await?;
        ensure_dealer_user(&state, user_id).await?;
        dealers.push(dealer);
    }

    let deleted_count = dealers.len();
    for dealer in &dealers {
        soft_delete_managed_user(
            &state,
            &auth_user,
            dealer,
            "batch_delete_dealer",
            "后台批量删除经销商",
        )
        .await?;
    }

    Ok(Json(ApiResponse::success(serde_json::json!({
        "ok": true,
        "deleted_count": deleted_count
    }))))
}

pub async fn reset_dealer_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    ensure_dealer_user(&state, user_id).await?;
    let auth_user = auth::authenticate(&state, &headers)?;
    let before_user = find_user(&state, user_id).await?;
    let result = reset_password_to_default(&state, user_id).await?;
    let after_user = find_user(&state, user_id).await?;
    audit::write_audit_log(
        &state,
        "user",
        user_id,
        "reset_dealer_password",
        auth_user.user_id,
        auth_user.primary_role(),
        Some(&before_user),
        Some(&after_user),
        Some("后台重置经销商密码".to_string()),
    )
    .await?;
    Ok(result)
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
    let auth_user = auth::authenticate(&state, &headers)?;

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
    let normalized_status = normalize_user_status(payload.status.as_deref())?;

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
            status: Some(normalized_status),
            session_version: Some(1),
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
    let detail = get_user_detail_inner(&state, user_id).await?;
    let after_user = find_user(&state, user_id).await?;
    audit::write_audit_log(
        &state,
        "user",
        user_id,
        "create_user",
        auth_user.user_id,
        auth_user.primary_role(),
        None::<&UserEntity>,
        Some(&after_user),
        Some("后台新增账号".to_string()),
    )
    .await?;
    Ok(detail)
}

pub async fn update_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    Json(payload): Json<UpdateUserRequest>,
) -> Result<Json<ApiResponse<UserDetailResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let auth_user = auth::authenticate(&state, &headers)?;
    ensure_backend_user(&state, user_id).await?;
    let before_user = find_user(&state, user_id).await?;
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
        if account_exists
            .into_iter()
            .any(|item| item.id != Some(user_id))
        {
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

    let mut should_bump_session =
        payload.account.is_some() || payload.username.is_some() || payload.role_codes.is_some();
    if payload.role_codes.is_some() {
        let role_codes = next_role_codes.clone();
        replace_user_roles(&state, user_id, role_codes).await?;
        should_bump_session = true;
    }
    if payload.store_ids.is_some() || next_store_ids != load_user_store_ids(&state, user_id).await?
    {
        replace_user_stores(&state, user_id, next_store_ids).await?;
        should_bump_session = true;
    }
    if should_bump_session {
        auth::bump_user_session_version(&state, user_id).await?;
    }

    let detail = get_user_detail_inner(&state, user_id).await?;
    let after_user = find_user(&state, user_id).await?;
    audit::write_audit_log(
        &state,
        "user",
        user_id,
        "update_user",
        auth_user.user_id,
        auth_user.primary_role(),
        Some(&before_user),
        Some(&after_user),
        Some("后台编辑账号".to_string()),
    )
    .await?;
    Ok(detail)
}

pub async fn update_user_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    Json(payload): Json<UpdateUserStatusRequest>,
) -> Result<Json<ApiResponse<UserDetailResponse>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    ensure_backend_user(&state, user_id).await?;
    let status = normalize_required_user_status(&payload.status)?;

    UserEntity::update_by_map(
        state.rb.as_ref(),
        &UserEntity {
            status: Some(status),
            ..Default::default()
        },
        value! { "id": user_id },
    )
    .await
    .context("failed to update user status")
    .map_err(AppError::Internal)?;
    auth::bump_user_session_version(&state, user_id).await?;

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
    let auth_user = auth::authenticate(&state, &headers)?;
    let before_user = find_user(&state, user_id).await?;
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
    auth::bump_user_session_version(&state, user_id).await?;

    let result = Json(ApiResponse::success(serde_json::json!({ "ok": true })));
    let after_user = find_user(&state, user_id).await?;
    audit::write_audit_log(
        &state,
        "user",
        user_id,
        "reset_user_password",
        auth_user.user_id,
        auth_user.primary_role(),
        Some(&before_user),
        Some(&after_user),
        Some("后台重置账号密码".to_string()),
    )
    .await?;
    Ok(result)
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
    auth::bump_user_session_version(state, user_id).await?;

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true, "default_password": "123456" }),
    )))
}

pub(crate) fn ensure_not_current_user(
    auth_user: &auth::CurrentAuthUser,
    user_id: i64,
) -> Result<(), AppError> {
    if auth_user.user_id == user_id {
        return Err(AppError::BadRequest("不能删除当前登录账号".to_string()));
    }
    Ok(())
}

pub(crate) async fn soft_delete_managed_user(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    user: &UserEntity,
    action: &str,
    remark: &str,
) -> Result<(), AppError> {
    let user_id = user
        .id
        .ok_or_else(|| AppError::Internal(anyhow!("user.id missing")))?;
    UserEntity::update_by_map(
        state.rb.as_ref(),
        &UserEntity {
            deleted_at: Some(Utc::now().naive_utc()),
            ..Default::default()
        },
        value! { "id": user_id, "column": ["deleted_at"] },
    )
    .await
    .context("failed to soft delete managed user")
    .map_err(AppError::Internal)?;
    auth::bump_user_session_version(state, user_id).await?;

    let after_user = db::select_one_by_map_compat::<UserEntity>(
        state.rb.as_ref(),
        "user",
        value! { "id": user_id },
    )
    .await
    .context("failed to query managed user after delete")
    .map_err(AppError::Internal)?;
    audit::write_audit_log(
        state,
        "user",
        user_id,
        action,
        auth_user.user_id,
        auth_user.primary_role(),
        Some(user),
        after_user.as_ref(),
        Some(remark.to_string()),
    )
    .await
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
    let primary_role_status =
        primary_detail_role_status(state, user_id, &roles, user.status.as_deref()).await?;
    let display_name = if roles.iter().any(|role| role == "dealer") {
        identity::role_display_name(state, user_id, "dealer", user.name.as_deref()).await?
    } else {
        user.name.clone().unwrap_or_default()
    };

    Ok(Json(ApiResponse::success(UserDetailResponse {
        id: user.id.unwrap_or_default(),
        name: display_name,
        account: user.account.unwrap_or_default(),
        mobile: user.mobile.unwrap_or_default(),
        email: user.email,
        avatar_url: user.avatar_url,
        status: user_status_text(Some(&primary_role_status)).to_string(),
        status_code: primary_role_status.clone(),
        status_text: user_status_text(Some(&primary_role_status)).to_string(),
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
    .map_err(AppError::Internal)?
    .into_iter()
    .filter(|task| task.deleted_at.is_none())
    .collect::<Vec<_>>();
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

    let technician_ids = technician_relations
        .into_iter()
        .filter_map(|item| item.user_id)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if technician_ids.is_empty() {
        return Ok(0);
    }
    let technicians: Vec<UserEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "user", value! { "id": technician_ids })
            .await
            .context("failed to query technician users in store scope")
            .map_err(AppError::Internal)?;
    Ok(technicians
        .into_iter()
        .filter(|user| user.deleted_at.is_none())
        .count())
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
            status: Some("enabled".to_string()),
            display_name: None,
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
                status: Some("enabled".to_string()),
                display_name: None,
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
    let user: UserEntity =
        db::select_one_by_map_compat(state.rb.as_ref(), "user", value! { "id": user_id })
            .await
            .context("failed to query user by id")
            .map_err(AppError::Internal)?
            .ok_or_else(|| AppError::NotFound("用户不存在或已删除".to_string()))?;
    if user.deleted_at.is_some() {
        return Err(AppError::NotFound("用户不存在或已删除".to_string()));
    }
    Ok(user)
}

async fn find_user_by_mobile(
    state: &AppState,
    mobile: &str,
) -> Result<Option<UserEntity>, AppError> {
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

fn normalize_user_status(raw: Option<&str>) -> Result<String, AppError> {
    let Some(value) = raw.map(str::trim).filter(|item| !item.is_empty()) else {
        return Ok("enabled".to_string());
    };

    match value {
        "启用" | "已启用" => Ok("enabled".to_string()),
        "禁用" | "已禁用" => Ok("disabled".to_string()),
        _ => match value.to_ascii_lowercase().as_str() {
            "enabled" | "active" => Ok("enabled".to_string()),
            "disabled" | "inactive" => Ok("disabled".to_string()),
            _ => Err(AppError::BadRequest(
                "status只能为启用/禁用或enabled/disabled".to_string(),
            )),
        },
    }
}

fn normalize_optional_user_status(raw: Option<&str>) -> Result<Option<String>, AppError> {
    match raw.map(str::trim).filter(|item| !item.is_empty()) {
        Some(value) => normalize_user_status(Some(value)).map(Some),
        None => Ok(None),
    }
}

fn normalize_required_user_status(raw: &str) -> Result<String, AppError> {
    if raw.trim().is_empty() {
        return Err(AppError::BadRequest(
            "status只能为启用/禁用或enabled/disabled".to_string(),
        ));
    }
    normalize_user_status(Some(raw))
}

fn canonical_user_status(raw: Option<&str>) -> String {
    normalize_user_status(raw).unwrap_or_else(|_| "enabled".to_string())
}

async fn primary_detail_role_status(
    state: &AppState,
    user_id: i64,
    roles: &[String],
    user_status: Option<&str>,
) -> Result<String, AppError> {
    if roles.iter().any(|role| role == "dealer") {
        return user_role_status(state, user_id, "dealer").await;
    }
    Ok(canonical_user_status(user_status))
}

async fn user_role_status(
    state: &AppState,
    user_id: i64,
    role_code: &str,
) -> Result<String, AppError> {
    let role_id = find_role_id_by_code(state, role_code).await?;
    let relation = db::select_one_by_map_compat::<UserRoleEntity>(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": user_id, "role_id": role_id },
    )
    .await
    .context("failed to query user role status")
    .map_err(AppError::Internal)?;
    Ok(canonical_user_status(
        relation.and_then(|item| item.status).as_deref(),
    ))
}

async fn update_user_role_status(
    state: &AppState,
    user_id: i64,
    role_code: &str,
    status: &str,
) -> Result<(), AppError> {
    let role_id = find_role_id_by_code(state, role_code).await?;
    state
        .rb
        .as_ref()
        .exec(
            "update `user_role` set `status` = ? where `user_id` = ? and `role_id` = ?",
            vec![value!(status), value!(user_id), value!(role_id)],
        )
        .await
        .context("failed to update user role status")
        .map_err(AppError::Internal)?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dealer_batch_delete_request_accepts_user_ids() {
        let payload: BatchDeleteDealersRequest =
            serde_json::from_value(serde_json::json!({ "user_ids": ["6", 7] })).unwrap();
        assert_eq!(payload.dealer_ids, vec![6, 7]);
    }

    #[test]
    fn dealer_update_accepts_current_frontend_payload() {
        let payload: UpdateDealerRequest = serde_json::from_value(serde_json::json!({
            "mobile": "17374160078",
            "name": "经销商-173",
            "status": "enabled",
            "store_ids": [1]
        }))
        .unwrap();

        assert_eq!(payload.mobile.as_deref(), Some("17374160078"));
        assert_eq!(payload.name.as_deref(), Some("经销商-173"));
        assert_eq!(payload.status.as_deref(), Some("enabled"));
        assert_eq!(payload.store_ids, Some(vec![1]));
    }

    #[test]
    fn user_status_accepts_chinese_and_english_values() {
        assert_eq!(normalize_user_status(Some("启用")).unwrap(), "enabled");
        assert_eq!(normalize_user_status(Some("禁用")).unwrap(), "disabled");
        assert_eq!(normalize_user_status(Some("active")).unwrap(), "enabled");
        assert_eq!(normalize_user_status(Some("inactive")).unwrap(), "disabled");
        assert_eq!(normalize_user_status(None).unwrap(), "enabled");
        assert!(normalize_user_status(Some("unknown")).is_err());
    }
}
