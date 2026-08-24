use anyhow::Context;
use axum::{
    Json,
    extract::{Multipart, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
};
use bcrypt::hash;
use chrono::Datelike;
use rbs::value;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth, db,
    error::AppError,
    identity,
    models::{
        permission::PermissionEntity, role::RoleEntity, role_permission::RolePermissionEntity,
        store::StoreEntity, user::UserEntity, user_message_setting::UserMessageSettingEntity,
        user_role::UserRoleEntity, user_store::UserStoreEntity,
    },
    response::ApiResponse,
    sms,
    state::AppState,
    storage, wechat,
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
    pub status_code: String,
    pub status_text: String,
    pub job_no: String,
    pub role_names: Vec<String>,
    pub role_codes: Vec<String>,
    pub current_role: String,
    pub store_id: Option<i64>,
    pub store_ids: Vec<i64>,
    pub store_names: Vec<String>,
    pub primary_store_id: Option<i64>,
    pub primary_store_name: Option<String>,
    pub technician_count: usize,
    pub customer_count: usize,
    pub month_completed_count: usize,
    pub completion_rate: f64,
    pub service_types: Vec<String>,
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
pub struct PublicWechatBindUrlRequest {
    pub bind_token: Option<String>,
    pub return_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SendWechatBindCodeRequest {
    pub mobile: String,
}

#[derive(Debug, Deserialize)]
pub struct ConfirmWechatBindRequest {
    pub bind_token: String,
    pub mobile: String,
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct PublicWechatBindUrlResponse {
    pub bind_token: String,
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
        build_profile_response(&state, user, &auth_user).await?,
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
    let role_name = if matches!(auth_user.current_role.as_str(), "dealer" | "technician") {
        name.clone()
    } else {
        None
    };
    let account_name = if role_name.is_none() { name } else { None };
    let mut columns = Vec::new();
    if account_name.is_some() {
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

    if columns.is_empty() && role_name.is_none() {
        let user = find_user(&state, auth_user.user_id).await?;
        return Ok(Json(ApiResponse::success(
            build_profile_response(&state, user, &auth_user).await?,
        )));
    }

    if !columns.is_empty() {
        UserEntity::update_by_map(
            state.rb.as_ref(),
            &UserEntity {
                name: account_name,
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
    }
    if let Some(role_name) = role_name {
        identity::update_role_display_name(
            &state,
            auth_user.user_id,
            &auth_user.current_role,
            &role_name,
        )
        .await?;
    }

    let user = find_user(&state, auth_user.user_id).await?;
    Ok(Json(ApiResponse::success(
        build_profile_response(&state, user, &auth_user).await?,
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
    auth::bump_user_session_version(&state, auth_user.user_id).await?;

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
    Ok(Json(ApiResponse::success(WechatBindUrlResponse {
        bind_url,
    })))
}

pub async fn get_public_wechat_bind_url(
    State(state): State<AppState>,
    Json(payload): Json<PublicWechatBindUrlRequest>,
) -> Result<Json<ApiResponse<PublicWechatBindUrlResponse>>, AppError> {
    let bind_token = payload
        .bind_token
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    if let Some(return_url) = payload.return_url.as_deref() {
        wechat::save_public_bind_return_url(&state, &bind_token, return_url).await;
    }
    let bind_url = wechat::build_public_bind_url(&state, &bind_token).await?;
    Ok(Json(ApiResponse::success(PublicWechatBindUrlResponse {
        bind_token,
        bind_url,
    })))
}

pub async fn send_wechat_bind_code(
    State(state): State<AppState>,
    Json(payload): Json<SendWechatBindCodeRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    if payload.mobile.trim().is_empty() {
        return Err(AppError::BadRequest("请输入手机号".to_string()));
    }

    let user = find_user_by_mobile(&state, &payload.mobile).await?;
    ensure_user_enabled(&user)?;

    let now = chrono::Utc::now().timestamp();
    {
        let store = state.verification_codes.read().await;
        if let Some(existing) = store.get(&payload.mobile)
            && now - existing.sent_at < 60
        {
            return Err(AppError::BadRequest(
                "验证码发送过于频繁，请稍后重试".to_string(),
            ));
        }
    }

    let code = format!("{:06}", (now.rem_euclid(1_000_000)));
    sms::send_login_code_sms(&state, &payload.mobile, &code).await?;

    let mut store = state.verification_codes.write().await;
    store.insert(
        payload.mobile.clone(),
        crate::state::VerificationCodeState {
            code,
            expires_at: now + 300,
            sent_at: now,
        },
    );

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn confirm_wechat_bind(
    State(state): State<AppState>,
    Json(payload): Json<ConfirmWechatBindRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    if payload.bind_token.trim().is_empty()
        || payload.mobile.trim().is_empty()
        || payload.code.trim().is_empty()
    {
        return Err(AppError::BadRequest("请填写完整绑定信息".to_string()));
    }

    let bind_state = wechat::load_public_bind_state(&state, &payload.bind_token).await?;
    let openid = bind_state.openid.trim().to_string();
    if openid.is_empty() {
        return Err(AppError::Conflict("请先在微信内完成授权".to_string()));
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

    let user = find_user_by_mobile(&state, &payload.mobile).await?;
    ensure_user_enabled(&user)?;
    let user_id = user.id.unwrap_or_default();
    wechat::bind_service_account_user(&state, user_id, &openid, bind_state.unionid.as_deref())
        .await?;
    wechat::clear_public_bind_state(&state, &payload.bind_token).await;

    {
        let mut store = state.verification_codes.write().await;
        store.remove(&payload.mobile);
    }

    Ok(Json(ApiResponse::success(serde_json::json!({
        "ok": true,
        "user_id": user_id,
        "wechat_bound": true
    }))))
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
) -> Result<Response, AppError> {
    let oauth_state = if let Some(item) = wechat::load_oauth_state(&state, &query.state).await {
        Some(item)
    } else {
        None
    };
    let oauth = wechat::exchange_service_account_oauth_code(&state, &query.code).await?;
    let openid = oauth
        .openid
        .as_deref()
        .ok_or_else(|| AppError::Conflict("微信服务号授权返回缺少openid".to_string()))?;

    if let Some(oauth_state) = oauth_state {
        wechat::clear_oauth_state(&state, &query.state).await;
        if oauth_state.purpose == "wechat_service_public_bind" {
            let bind_token = oauth_state
                .bind_token
                .clone()
                .filter(|item| !item.trim().is_empty())
                .ok_or_else(|| AppError::Conflict("微信绑定态缺少bind_token".to_string()))?;
            wechat::save_public_bind_state(&state, &bind_token, openid, oauth.unionid.as_deref())
                .await?;
            if let Some(return_url) = wechat::load_public_bind_return_url(&state, &bind_token).await
            {
                let separator = if return_url.contains('?') { '&' } else { '?' };
                let redirect_url = format!(
                    "{}{}bind_token={}&authorized=true",
                    return_url,
                    separator,
                    urlencoding::encode(&bind_token)
                );
                return Ok(Redirect::temporary(&redirect_url).into_response());
            }
            return Ok(Json(ApiResponse::success(serde_json::json!({
                "ok": true,
                "bind_token": bind_token,
                "wechat_bound": false,
                "authorized": true,
                "next_action": "confirm_mobile"
            })))
            .into_response());
        }

        let user_id = oauth_state
            .user_id
            .ok_or_else(|| AppError::Conflict("微信绑定态缺少用户信息".to_string()))?;
        wechat::bind_service_account_user(&state, user_id, openid, oauth.unionid.as_deref())
            .await?;

        return Ok(Json(ApiResponse::success(serde_json::json!({
            "ok": true,
            "user_id": user_id,
            "wechat_bound": true,
            "wechat_openid": openid,
        })))
        .into_response());
    }

    let claims =
        wechat::decode_bind_state_token(&state.config, &query.state).map_err(AppError::Internal)?;

    if claims.purpose == "wechat_service_public_bind" {
        let bind_token = claims
            .bind_token
            .clone()
            .filter(|item| !item.trim().is_empty())
            .ok_or_else(|| AppError::Conflict("微信绑定态缺少bind_token".to_string()))?;
        wechat::save_public_bind_state(&state, &bind_token, openid, oauth.unionid.as_deref())
            .await?;
        if let Some(return_url) = wechat::load_public_bind_return_url(&state, &bind_token).await {
            let separator = if return_url.contains('?') { '&' } else { '?' };
            let redirect_url = format!(
                "{}{}bind_token={}&authorized=true",
                return_url,
                separator,
                urlencoding::encode(&bind_token)
            );
            return Ok(Redirect::temporary(&redirect_url).into_response());
        }
        return Ok(Json(ApiResponse::success(serde_json::json!({
            "ok": true,
            "bind_token": bind_token,
            "wechat_bound": false,
            "authorized": true,
            "next_action": "confirm_mobile"
        })))
        .into_response());
    }

    let user_id = claims
        .sub
        .ok_or_else(|| AppError::Conflict("微信绑定态缺少用户信息".to_string()))?;
    wechat::bind_service_account_user(&state, user_id, openid, oauth.unionid.as_deref()).await?;

    Ok(Json(ApiResponse::success(serde_json::json!({
        "ok": true,
        "user_id": user_id,
        "wechat_bound": true,
        "wechat_openid": openid,
    })))
    .into_response())
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
    let role_ids: Vec<i64> = roles
        .into_iter()
        .filter(|item| {
            role_matches_current_scope(item.role_code.as_deref(), auth_user.current_role.as_str())
        })
        .filter_map(|item| item.id)
        .collect();
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
        return Ok(Json(ApiResponse::success(
            default_permissions_for_current_role(auth_user.current_role.as_str()),
        )));
    }
    let permissions: Vec<PermissionEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "permission",
        value! { "id": permission_ids },
    )
    .await
    .context("failed to query permissions")
    .map_err(AppError::Internal)?;

    let permission_codes = permissions
        .into_iter()
        .filter_map(|item| item.permission_code)
        .collect::<Vec<_>>();
    if permission_codes.is_empty() {
        return Ok(Json(ApiResponse::success(
            default_permissions_for_current_role(auth_user.current_role.as_str()),
        )));
    }

    Ok(Json(ApiResponse::success(permission_codes)))
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

async fn find_user_by_mobile(state: &AppState, mobile: &str) -> Result<UserEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "user", value! { "mobile": mobile })
        .await
        .context("failed to query user by mobile for wechat bind")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::BadRequest("账号不存在，请联系管理员创建账号".to_string()))
}

fn ensure_user_enabled(user: &UserEntity) -> Result<(), AppError> {
    if user.deleted_at.is_some() {
        return Err(AppError::NotFound("用户不存在或已删除".to_string()));
    }
    if user.status.as_deref() == Some("disabled") {
        return Err(AppError::Forbidden(
            "账号已被禁用，请联系管理员".to_string(),
        ));
    }
    Ok(())
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

fn role_matches_current_scope(role_code: Option<&str>, current_role: &str) -> bool {
    let Some(role_code) = role_code else {
        return false;
    };
    if current_role == "dealer" || current_role == "technician" {
        return role_code == "dealer" || role_code == "technician";
    }
    if current_role == "super_admin" || current_role == "hq_reviewer" {
        return role_code == "super_admin" || role_code == "hq_reviewer";
    }
    role_code == current_role
}

fn default_permissions_for_current_role(current_role: &str) -> Vec<String> {
    match current_role {
        "technician" => vec![
            "workspace",
            "tasks",
            "tasks:list",
            "tasks:records",
            "tasks:evaluation",
            "measure",
            "install",
            "ironing",
            "cleaning",
            "users",
            "customers",
            "customers:list",
            "users:customers",
            "settings",
            "settings:project-settings",
            "master-data",
            "master-data:stores",
            "master-data:sources",
        ]
        .into_iter()
        .map(ToString::to_string)
        .collect(),
        "dealer" => vec![
            "workspace",
            "tasks",
            "tasks:list",
            "tasks:list:create",
            "tasks:records",
            "tasks:evaluation",
            "measure",
            "install",
            "ironing",
            "cleaning",
            "users",
            "customers",
            "customers:list",
            "users:customers",
            "users:staff",
            "users:dealers",
            "master-data",
            "master-data:stores",
            "master-data:sources",
            "settings",
            "settings:project-settings",
        ]
        .into_iter()
        .map(ToString::to_string)
        .collect(),
        _ => Vec::new(),
    }
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
        .map_err(AppError::from_multipart_error)?
        .ok_or_else(|| AppError::BadRequest("请上传文件".to_string()))?;

    let file_name = field
        .file_name()
        .map(ToString::to_string)
        .ok_or_else(|| AppError::BadRequest("上传文件名不能为空".to_string()))?;
    let content_type = field.content_type().map(ToString::to_string);
    let bytes = field
        .bytes()
        .await
        .map_err(AppError::from_multipart_error)?
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
    auth_user: &auth::CurrentAuthUser,
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
    let current_role = auth_user.current_role.clone();
    let display_name =
        identity::role_display_name(state, user_id, &current_role, user.name.as_deref()).await?;
    let is_technician_only = current_role == "technician";
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
            .map_err(AppError::Internal)?
            .into_iter()
            .filter(|task| task.deleted_at.is_none())
            .collect::<Vec<_>>();
            let completed = tasks
                .iter()
                .filter(|item| is_profile_completed_status(item.task_status.as_deref(), true))
                .count();
            let month_completed = tasks
                .iter()
                .filter(|item| is_profile_completed_status(item.task_status.as_deref(), true))
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
            .map_err(AppError::Internal)?
            .into_iter()
            .filter(|task| task.deleted_at.is_none())
            .collect::<Vec<_>>();
            let completed = tasks
                .iter()
                .filter(|item| is_profile_completed_status(item.task_status.as_deref(), false))
                .count();
            let month_completed = tasks
                .iter()
                .filter(|item| is_profile_completed_status(item.task_status.as_deref(), false))
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
    let service_types = if current_role == "technician" {
        db::select_by_map_compat::<crate::models::technician_skill::TechnicianSkillEntity>(
            state.rb.as_ref(),
            "technician_skill",
            value! { "user_id": user_id },
        )
        .await
        .context("failed to query technician skills for profile")
        .map_err(AppError::Internal)?
        .into_iter()
        .filter_map(|item| item.service_type)
        .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    Ok(ProfileResponse {
        id: user_id,
        name: display_name,
        nickname: user.nickname,
        mobile: user.mobile.unwrap_or_default(),
        email: user.email,
        avatar_url: user.avatar_url,
        status: profile_status_text(user.status.as_deref()).to_string(),
        status_code: user.status.clone().unwrap_or_else(|| "enabled".to_string()),
        status_text: profile_status_text(user.status.as_deref()).to_string(),
        job_no,
        role_names,
        role_codes,
        current_role,
        store_id: primary_store_id,
        store_ids,
        store_names,
        primary_store_id,
        primary_store_name,
        technician_count,
        customer_count,
        month_completed_count,
        completion_rate,
        service_types,
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

fn is_profile_completed_status(status: Option<&str>, _technician_only: bool) -> bool {
    matches!(
        status,
        Some("pending_hq_score") | Some("completed") | Some("closed")
    )
}
