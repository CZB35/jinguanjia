use anyhow::Context;
use axum::{
    Json,
    extract::State,
    http::HeaderMap,
};
use bcrypt::hash;
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    auth,
    db,
    error::AppError,
    models::{permission::PermissionEntity, role::RoleEntity, role_permission::RolePermissionEntity, store::StoreEntity, user::UserEntity, user_message_setting::UserMessageSettingEntity, user_role::UserRoleEntity, user_store::UserStoreEntity},
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePasswordRequest {
    pub mobile: String,
    pub code: String,
    pub new_password: String,
    pub confirm_password: String,
}

#[derive(Debug, Serialize)]
pub struct ProfileResponse {
    pub id: i64,
    pub name: String,
    pub mobile: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub status: String,
    pub job_no: String,
    pub role_names: Vec<String>,
    pub store_names: Vec<String>,
    pub primary_store_name: Option<String>,
    pub technician_count: usize,
    pub customer_count: usize,
    pub month_completed_count: usize,
    pub completion_rate: f64,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMessageSettingsRequest {
    pub new_task_enabled: bool,
    pub review_reminder_enabled: bool,
    pub sms_enabled: bool,
    pub wechat_enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct MessageSettingsResponse {
    pub new_task_enabled: bool,
    pub review_reminder_enabled: bool,
    pub sms_enabled: bool,
    pub wechat_enabled: bool,
}

pub async fn get_message_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<MessageSettingsResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let setting = load_or_create_setting(&state, auth_user.user_id).await?;
    Ok(Json(ApiResponse::success(to_response(setting))))
}

pub async fn get_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<ProfileResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let user = find_user(&state, auth_user.user_id).await?;
    Ok(Json(ApiResponse::success(build_profile_response(&state, user).await?)))
}

pub async fn update_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UpdateProfileRequest>,
) -> Result<Json<ApiResponse<ProfileResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    UserEntity::update_by_map(
        state.rb.as_ref(),
        &UserEntity {
            name: payload.name,
            email: payload.email,
            avatar_url: payload.avatar_url,
            ..Default::default()
        },
        value! { "id": auth_user.user_id },
    )
    .await
    .context("failed to update profile")
    .map_err(AppError::Internal)?;

    let user = find_user(&state, auth_user.user_id).await?;
    Ok(Json(ApiResponse::success(build_profile_response(&state, user).await?)))
}

pub async fn update_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UpdatePasswordRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    if payload.mobile.trim().is_empty() || payload.code.trim().is_empty() || payload.new_password.trim().is_empty() || payload.confirm_password.trim().is_empty() {
        return Err(AppError::BadRequest("请输入完整密码信息".to_string()));
    }
    if payload.new_password.len() < 6 {
        return Err(AppError::BadRequest("密码长度不能少于6位".to_string()));
    }
    if payload.new_password != payload.confirm_password {
        return Err(AppError::BadRequest("两次输入的新密码不一致".to_string()));
    }

    let user = find_user(&state, auth_user.user_id).await?;
    if user.mobile.as_deref() != Some(payload.mobile.as_str()) {
        return Err(AppError::BadRequest("手机号与当前账号不匹配".to_string()));
    }

    let now = chrono::Utc::now().timestamp();
    let stored = {
        let store = state.verification_codes.read().await;
        store.get(&payload.mobile).cloned()
    }
    .ok_or_else(|| AppError::BadRequest("验证码错误".to_string()))?;
    if stored.expires_at < now {
        return Err(AppError::BadRequest("验证码已过期，请重新获取".to_string()));
    }
    if stored.code != payload.code {
        return Err(AppError::BadRequest("验证码错误".to_string()));
    }

    let new_hash = hash(&payload.new_password, 10)
        .context("failed to hash new password")
        .map_err(AppError::Internal)?;
    UserEntity::update_by_map(
        state.rb.as_ref(),
        &UserEntity {
            password_hash: Some(new_hash),
            ..Default::default()
        },
        value! { "id": auth_user.user_id },
    )
    .await
    .context("failed to update password")
    .map_err(AppError::Internal)?;

    {
        let mut store = state.verification_codes.write().await;
        store.remove(&payload.mobile);
    }

    Ok(Json(ApiResponse::success(serde_json::json!({ "ok": true }))))
}

pub async fn get_permissions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<Vec<String>>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let user_roles: Vec<UserRoleEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_role", value! { "user_id": auth_user.user_id })
        .await
        .context("failed to query current user roles")
        .map_err(AppError::Internal)?;
    let role_ids: Vec<i64> = user_roles.into_iter().filter_map(|item| item.role_id).collect();
    if role_ids.is_empty() {
        return Ok(Json(ApiResponse::success(Vec::new())));
    }

    let roles: Vec<RoleEntity> = db::select_by_map_compat(state.rb.as_ref(), "role", value! { "id": role_ids.clone() })
        .await
        .context("failed to query roles for permissions")
        .map_err(AppError::Internal)?;
    let role_ids: Vec<i64> = roles.into_iter().filter_map(|item| item.id).collect();
    let role_permissions: Vec<RolePermissionEntity> = db::select_by_map_compat(state.rb.as_ref(), "role_permission", value! { "role_id": role_ids })
        .await
        .context("failed to query role permissions")
        .map_err(AppError::Internal)?;
    let permission_ids: Vec<i64> = role_permissions.into_iter().filter_map(|item| item.permission_id).collect();
    if permission_ids.is_empty() {
        return Ok(Json(ApiResponse::success(Vec::new())));
    }
    let permissions: Vec<PermissionEntity> = db::select_by_map_compat(state.rb.as_ref(), "permission", value! { "id": permission_ids })
        .await
        .context("failed to query permissions")
        .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(
        permissions.into_iter().filter_map(|item| item.permission_code).collect(),
    )))
}

pub async fn update_message_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UpdateMessageSettingsRequest>,
) -> Result<Json<ApiResponse<MessageSettingsResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let setting = load_or_create_setting(&state, auth_user.user_id).await?;

    UserMessageSettingEntity::update_by_map(
        state.rb.as_ref(),
        &UserMessageSettingEntity {
            new_task_enabled: Some(bool_to_i32(payload.new_task_enabled)),
            review_reminder_enabled: Some(bool_to_i32(payload.review_reminder_enabled)),
            sms_enabled: Some(bool_to_i32(payload.sms_enabled)),
            wechat_enabled: Some(bool_to_i32(payload.wechat_enabled)),
            ..Default::default()
        },
        value! { "id": setting.id.unwrap_or_default() },
    )
    .await
    .context("failed to update message settings")
    .map_err(AppError::Internal)?;

    let updated = load_or_create_setting(&state, auth_user.user_id).await?;
    Ok(Json(ApiResponse::success(to_response(updated))))
}

async fn load_or_create_setting(
    state: &AppState,
    user_id: i64,
) -> Result<UserMessageSettingEntity, AppError> {
    let existing: Vec<UserMessageSettingEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_message_setting", value! { "user_id": user_id })
        .await
        .context("failed to query user message settings")
        .map_err(AppError::Internal)?;
    if let Some(item) = existing.into_iter().next() {
        return Ok(item);
    }

    UserMessageSettingEntity::insert(
        state.rb.as_ref(),
        &UserMessageSettingEntity {
            id: None,
            user_id: Some(user_id),
            new_task_enabled: Some(1),
            review_reminder_enabled: Some(1),
            sms_enabled: Some(1),
            wechat_enabled: Some(1),
            created_at: None,
            updated_at: None,
        },
    )
    .await
    .context("failed to create default message settings")
    .map_err(AppError::Internal)?;

    db::select_one_by_map_compat(state.rb.as_ref(), "user_message_setting", value! { "user_id": user_id })
        .await
        .context("failed to fetch created user message settings")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("message setting create succeeded but fetch failed")))
}

fn to_response(item: UserMessageSettingEntity) -> MessageSettingsResponse {
    MessageSettingsResponse {
        new_task_enabled: item.new_task_enabled.unwrap_or(1) == 1,
        review_reminder_enabled: item.review_reminder_enabled.unwrap_or(1) == 1,
        sms_enabled: item.sms_enabled.unwrap_or(1) == 1,
        wechat_enabled: item.wechat_enabled.unwrap_or(1) == 1,
    }
}

fn bool_to_i32(value: bool) -> i32 {
    if value { 1 } else { 0 }
}

async fn find_user(state: &AppState, user_id: i64) -> Result<UserEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "user", value! { "id": user_id })
        .await
        .context("failed to query current user")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("用户不存在".to_string()))
}

async fn build_profile_response(state: &AppState, user: UserEntity) -> Result<ProfileResponse, AppError> {
    let user_id = user.id.unwrap_or_default();
    let user_roles: Vec<UserRoleEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_role", value! { "user_id": user_id })
        .await
        .context("failed to query user roles for profile")
        .map_err(AppError::Internal)?;
    let role_ids: Vec<i64> = user_roles.into_iter().filter_map(|item| item.role_id).collect();
    let role_names = if role_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<RoleEntity>(state.rb.as_ref(), "role", value! { "id": role_ids })
            .await
            .context("failed to query roles for profile")
            .map_err(AppError::Internal)?
            .into_iter()
            .filter_map(|item| item.role_name)
            .collect()
    };
    let user_stores: Vec<UserStoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_store", value! { "user_id": user_id })
        .await
        .context("failed to query user stores for profile")
        .map_err(AppError::Internal)?;
    let store_ids: Vec<i64> = user_stores.into_iter().filter_map(|item| item.store_id).collect();
    let store_names = if store_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<StoreEntity>(state.rb.as_ref(), "store", value! { "id": store_ids.clone() })
            .await
            .context("failed to query stores for profile")
            .map_err(AppError::Internal)?
            .into_iter()
            .filter_map(|item| item.store_name)
            .collect()
    };
    let primary_store_name = store_names.first().cloned();
    let task_count = if store_ids.is_empty() {
        0
    } else {
        db::select_by_map_compat::<crate::models::task::TaskEntity>(state.rb.as_ref(), "task", value! { "store_id": store_ids.clone() })
            .await
            .context("failed to query tasks for profile aggregate")
            .map_err(AppError::Internal)?
            .len()
    };
    let completed_count = if store_ids.is_empty() {
        0
    } else {
        db::select_by_map_compat::<crate::models::task::TaskEntity>(state.rb.as_ref(), "task", value! { "store_id": store_ids.clone(), "task_status": "completed" })
            .await
            .context("failed to query completed tasks for profile aggregate")
            .map_err(AppError::Internal)?
            .len()
    };
    let technician_count = if store_ids.is_empty() {
        0
    } else {
        db::select_by_map_compat::<UserStoreEntity>(state.rb.as_ref(), "user_store", value! { "store_id": store_ids.clone() })
            .await
            .context("failed to query technicians for profile aggregate")
            .map_err(AppError::Internal)?
            .len()
    };
    let customer_count = if store_ids.is_empty() {
        0
    } else {
        db::select_by_map_compat::<crate::models::task::TaskEntity>(state.rb.as_ref(), "task", value! { "store_id": store_ids.clone() })
            .await
            .context("failed to query customer count for profile aggregate")
            .map_err(AppError::Internal)?
            .into_iter()
            .filter_map(|item| item.customer_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    };
    let month_completed_count = completed_count;
    let completion_rate = if task_count == 0 { 0.0 } else { (completed_count as f64 / task_count as f64) * 100.0 };

    Ok(ProfileResponse {
        id: user_id,
        name: user.name.unwrap_or_default(),
        mobile: user.mobile.unwrap_or_default(),
        email: user.email,
        avatar_url: user.avatar_url,
        status: user.status.unwrap_or_else(|| "enabled".to_string()),
        job_no: format!("DL-{:08}", user_id),
        role_names,
        store_names,
        primary_store_name,
        technician_count,
        customer_count,
        month_completed_count,
        completion_rate,
    })
}
