use anyhow::Context;
use axum::{Json, extract::{Multipart, Query, State}, http::HeaderMap};
use bcrypt::hash;
use chrono::Datelike;
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    auth, db,
    error::AppError,
    models::{
        permission::PermissionEntity, role::RoleEntity, role_permission::RolePermissionEntity,
        store::StoreEntity, user::UserEntity, user_message_setting::UserMessageSettingEntity,
        user_role::UserRoleEntity, user_store::UserStoreEntity,
    },
    response::ApiResponse,
    state::AppState,
    storage,
    wechat,
};

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    #[serde(default, alias = "user_name", alias = "username")]
    pub name: Option<String>,
    #[serde(default, alias = "nick_name", alias = "nickName")]
    pub nickname: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default, alias = "avatar", alias = "avatarUrl")]
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
    pub nickname: Option<String>,
    pub mobile: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub status: String,
    pub status_text: String,
    pub job_no: String,
    pub role_names: Vec<String>,
    pub store_id: Option<i64>,
    pub store_ids: Vec<i64>,
    pub store_names: Vec<String>,
    pub primary_store_id: Option<i64>,
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

#[derive(Debug, Serialize)]
pub struct AvatarUploadResponse {
    pub file_name: String,
    pub file_url: String,
    pub file_size: i64,
    pub mime_type: String,
}

#[derive(Debug, Serialize)]
pub struct WechatBindUrlResponse {
    pub bind_url: String,
}

#[derive(Debug, Deserialize)]
pub struct WechatServiceAccountCallbackQuery {
    pub code: String,
    pub state: String,
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
    Ok(Json(ApiResponse::success(
        build_profile_response(&state, user).await?,
    )))
}

pub async fn update_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UpdateProfileRequest>,
) -> Result<Json<ApiResponse<ProfileResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let name = normalize_optional_owned(payload.name);
    let nickname = normalize_optional_owned(payload.nickname);
    let email = normalize_optional_owned(payload.email);
    let avatar_url = normalize_optional_owned(payload.avatar_url);
    let mut columns = Vec::new();
    if name.is_some() {
        columns.push("name");
    }
    if nickname.is_some() {
        columns.push("nickname");
    }
    if email.is_some() {
        columns.push("email");
    }
    if avatar_url.is_some() {
        columns.push("avatar_url");
    }

    if columns.is_empty() {
        let user = find_user(&state, auth_user.user_id).await?;
        return Ok(Json(ApiResponse::success(
            build_profile_response(&state, user).await?,
        )));
    }

    UserEntity::update_by_map(
        state.rb.as_ref(),
        &UserEntity {
            name,
            nickname,
            email,
            avatar_url,
            ..Default::default()
        },
        value! { "id": auth_user.user_id, "column": columns },
    )
    .await
    .context("failed to update profile")
    .map_err(AppError::Internal)?;

    let user = find_user(&state, auth_user.user_id).await?;
    Ok(Json(ApiResponse::success(
        build_profile_response(&state, user).await?,
    )))
}

pub async fn update_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UpdatePasswordRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    if payload.mobile.trim().is_empty()
        || payload.code.trim().is_empty()
        || payload.new_password.trim().is_empty()
        || payload.confirm_password.trim().is_empty()
    {
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

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn get_wechat_bind_url(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<WechatBindUrlResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let bind_url = wechat::build_service_account_bind_url(&state, auth_user.user_id).await?;
    Ok(Json(ApiResponse::success(WechatBindUrlResponse { bind_url })))
}

pub async fn get_wechat_binding_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<wechat::WechatBindStatus>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    Ok(Json(ApiResponse::success(
        wechat::get_bind_status(&state, auth_user.user_id).await?,
    )))
}

pub async fn wechat_service_account_callback(
    State(state): State<AppState>,
    Query(query): Query<WechatServiceAccountCallbackQuery>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let claims = wechat::decode_bind_state_token(&state.config, &query.state)
        .map_err(AppError::Internal)?;
    let oauth = wechat::exchange_service_account_oauth_code(&state, &query.code).await?;
    let openid = oauth
        .openid
        .as_deref()
        .ok_or_else(|| AppError::Conflict("微信服务号授权返回缺少openid".to_string()))?;
    wechat::bind_service_account_user(&state, claims.sub, openid, oauth.unionid.as_deref()).await?;

    Ok(Json(ApiResponse::success(serde_json::json!({
        "ok": true,
        "user_id": claims.sub,
        "wechat_bound": true,
        "wechat_openid": openid,
    }))))
}

pub async fn upload_avatar(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<ApiResponse<AvatarUploadResponse>>, AppError> {
    let _auth_user = auth::authenticate(&state, &headers)?;
    let upload = read_single_multipart(multipart).await?;
    validate_avatar_upload(&upload.file_name, upload.content_type.as_deref())?;

    let stored = storage::save_bytes(
        &state,
        "avatars",
        &upload.file_name,
        upload.content_type.as_deref(),
        &upload.bytes,
    )
    .await
    .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(AvatarUploadResponse {
        file_name: stored.original_name,
        file_url: storage::public_url(&stored.relative_path),
        file_size: stored.file_size,
        mime_type: stored.mime_type,
    })))
}

pub async fn get_permissions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<Vec<String>>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let user_roles: Vec<UserRoleEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": auth_user.user_id },
    )
    .await
    .context("failed to query current user roles")
    .map_err(AppError::Internal)?;
    let role_ids: Vec<i64> = user_roles
        .into_iter()
        .filter_map(|item| item.role_id)
        .collect();
    if role_ids.is_empty() {
        return Ok(Json(ApiResponse::success(Vec::new())));
    }

    let roles: Vec<RoleEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "role", value! { "id": role_ids.clone() })
            .await
            .context("failed to query roles for permissions")
            .map_err(AppError::Internal)?;
    let role_ids: Vec<i64> = roles.into_iter().filter_map(|item| item.id).collect();
    let role_permissions: Vec<RolePermissionEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "role_permission",
        value! { "role_id": role_ids },
    )
    .await
    .context("failed to query role permissions")
    .map_err(AppError::Internal)?;
    let permission_ids: Vec<i64> = role_permissions
        .into_iter()
        .filter_map(|item| item.permission_id)
        .collect();
    if permission_ids.is_empty() {
        return Ok(Json(ApiResponse::success(Vec::new())));
    }
    let permissions: Vec<PermissionEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "permission",
        value! { "id": permission_ids },
    )
    .await
    .context("failed to query permissions")
    .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(
        permissions
            .into_iter()
            .filter_map(|item| item.permission_code)
            .collect(),
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
    let existing: Vec<UserMessageSettingEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_message_setting",
        value! { "user_id": user_id },
    )
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

    db::select_one_by_map_compat(
        state.rb.as_ref(),
        "user_message_setting",
        value! { "user_id": user_id },
    )
    .await
    .context("failed to fetch created user message settings")
    .map_err(AppError::Internal)?
    .ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!(
            "message setting create succeeded but fetch failed"
        ))
    })
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

fn normalize_optional_owned(value: Option<String>) -> Option<String> {
    value.and_then(|item| normalize_optional_text(Some(item.as_str())))
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
}

#[derive(Debug)]
struct UploadField {
    file_name: String,
    content_type: Option<String>,
    bytes: Vec<u8>,
}

async fn read_single_multipart(mut multipart: Multipart) -> Result<UploadField, AppError> {
    let field = multipart
        .next_field()
        .await
        .context("failed to read avatar multipart field")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::BadRequest("请上传文件".to_string()))?;

    let file_name = field
        .file_name()
        .map(ToString::to_string)
        .ok_or_else(|| AppError::BadRequest("上传文件名不能为空".to_string()))?;
    let content_type = field.content_type().map(ToString::to_string);
    let bytes = field
        .bytes()
        .await
        .context("failed to read avatar upload bytes")
        .map_err(AppError::Internal)?
        .to_vec();

    Ok(UploadField {
        file_name,
        content_type,
        bytes,
    })
}

fn validate_avatar_upload(file_name: &str, content_type: Option<&str>) -> Result<(), AppError> {
    let ext = std::path::Path::new(file_name)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let content_type = content_type.unwrap_or("");
    let ok = ["jpg", "jpeg", "png", "webp"].contains(&ext.as_str())
        || content_type.starts_with("image/");
    if !ok {
        return Err(AppError::BadRequest("当前文件格式不支持上传".to_string()));
    }
    Ok(())
}

async fn find_user(state: &AppState, user_id: i64) -> Result<UserEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "user", value! { "id": user_id })
        .await
        .context("failed to query current user")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("用户不存在".to_string()))
}

async fn build_profile_response(
    state: &AppState,
    user: UserEntity,
) -> Result<ProfileResponse, AppError> {
    let user_id = user.id.unwrap_or_default();
    let user_roles: Vec<UserRoleEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": user_id },
    )
    .await
    .context("failed to query user roles for profile")
    .map_err(AppError::Internal)?;
    let role_ids: Vec<i64> = user_roles
        .into_iter()
        .filter_map(|item| item.role_id)
        .collect();
    let role_names = if role_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<RoleEntity>(
            state.rb.as_ref(),
            "role",
            value! { "id": role_ids.clone() },
        )
        .await
        .context("failed to query roles for profile")
        .map_err(AppError::Internal)?
        .into_iter()
        .filter_map(|item| item.role_name)
        .collect()
    };
    let role_codes = if role_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<RoleEntity>(
            state.rb.as_ref(),
            "role",
            value! { "id": role_ids.clone() },
        )
        .await
        .context("failed to query role codes for profile")
        .map_err(AppError::Internal)?
        .into_iter()
        .filter_map(|item| item.role_code)
        .collect::<Vec<_>>()
    };
    let user_stores: Vec<UserStoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_store",
        value! { "user_id": user_id },
    )
    .await
    .context("failed to query user stores for profile")
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
        .context("failed to query stores for profile")
        .map_err(AppError::Internal)?
        .into_iter()
        .filter_map(|item| item.store_name)
        .collect()
    };
    let primary_store_id = store_ids.first().copied();
    let primary_store_name = store_names.first().cloned();
    let is_technician_only = role_codes.contains(&"technician".to_string())
        && !role_codes.contains(&"dealer".to_string())
        && !role_codes.contains(&"hq_reviewer".to_string())
        && !role_codes.contains(&"super_admin".to_string());
    let now = chrono::Utc::now();
    let (task_count, completed_count, technician_count, customer_count, month_completed_count) =
        if is_technician_only {
            let tasks = db::select_by_map_compat::<crate::models::task::TaskEntity>(
                state.rb.as_ref(),
                "task",
                value! { "current_technician_id": user_id },
            )
            .await
            .context("failed to query technician tasks for profile")
            .map_err(AppError::Internal)?;
            let completed = tasks
                .iter()
                .filter(|item| item.task_status.as_deref() == Some("completed"))
                .count();
            let month_completed = tasks
                .iter()
                .filter(|item| item.task_status.as_deref() == Some("completed"))
                .filter(|item| {
                    item.updated_at
                        .map(|dt| dt.year() == now.year() && dt.month() == now.month())
                        .unwrap_or(false)
                })
                .count();
            let customers = tasks
                .iter()
                .filter_map(|item| item.customer_id)
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            (tasks.len(), completed, 1, customers, month_completed)
        } else if store_ids.is_empty() {
            (0, 0, 0, 0, 0)
        } else {
            let tasks = db::select_by_map_compat::<crate::models::task::TaskEntity>(
                state.rb.as_ref(),
                "task",
                value! { "store_id": store_ids.clone() },
            )
            .await
            .context("failed to query tasks for profile aggregate")
            .map_err(AppError::Internal)?;
            let completed = tasks
                .iter()
                .filter(|item| item.task_status.as_deref() == Some("completed"))
                .count();
            let month_completed = tasks
                .iter()
                .filter(|item| item.task_status.as_deref() == Some("completed"))
                .filter(|item| {
                    item.updated_at
                        .map(|dt| dt.year() == now.year() && dt.month() == now.month())
                        .unwrap_or(false)
                })
                .count();
            let technician_relations = db::select_by_map_compat::<UserStoreEntity>(
                state.rb.as_ref(),
                "user_store",
                value! { "store_id": store_ids.clone() },
            )
            .await
            .context("failed to query technicians for profile aggregate")
            .map_err(AppError::Internal)?;
            let technician_user_ids = technician_relations
                .into_iter()
                .filter_map(|item| item.user_id)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let technicians = count_technician_users(state, &technician_user_ids).await?;
            let customers = tasks
                .iter()
                .filter_map(|item| item.customer_id)
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            (
                tasks.len(),
                completed,
                technicians,
                customers,
                month_completed,
            )
        };
    let completion_rate = if task_count == 0 {
        0.0
    } else {
        (completed_count as f64 / task_count as f64) * 100.0
    };
    let job_no = user.job_no.clone().unwrap_or_else(|| {
        if is_technician_only {
            format!("SF{:08}", user_id)
        } else {
            format!("DL-{:08}", user_id)
        }
    });

    Ok(ProfileResponse {
        id: user_id,
        name: user.name.unwrap_or_default(),
        nickname: user.nickname,
        mobile: user.mobile.unwrap_or_default(),
        email: user.email,
        avatar_url: user.avatar_url,
        status: user.status.clone().unwrap_or_else(|| "enabled".to_string()),
        status_text: profile_status_text(user.status.as_deref()).to_string(),
        job_no,
        role_names,
        store_id: primary_store_id,
        store_ids,
        store_names,
        primary_store_id,
        primary_store_name,
        technician_count,
        customer_count,
        month_completed_count,
        completion_rate,
    })
}

async fn count_technician_users(state: &AppState, user_ids: &[i64]) -> Result<usize, AppError> {
    if user_ids.is_empty() {
        return Ok(0);
    }

    let technician_role = db::select_one_by_map_compat::<RoleEntity>(
        state.rb.as_ref(),
        "role",
        value! { "role_code": "technician" },
    )
    .await
    .context("failed to query technician role for profile")
    .map_err(AppError::Internal)?
    .and_then(|item| item.id)
    .ok_or_else(|| anyhow::anyhow!("role not found: technician"))
    .map_err(AppError::Internal)?;

    let technician_relations = db::select_by_map_compat::<UserRoleEntity>(
        state.rb.as_ref(),
        "user_role",
        value! { "role_id": technician_role, "user_id": user_ids.to_vec() },
    )
    .await
    .context("failed to query technician role relations for profile")
    .map_err(AppError::Internal)?;

    Ok(technician_relations
        .into_iter()
        .filter_map(|item| item.user_id)
        .collect::<std::collections::BTreeSet<_>>()
        .len())
}

fn profile_status_text(status: Option<&str>) -> &'static str {
    match status.unwrap_or("enabled") {
        "enabled" | "active" => "已启用",
        "disabled" | "inactive" => "已禁用",
        _ => "未知状态",
    }
}
