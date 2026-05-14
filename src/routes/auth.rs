use anyhow::{Context, anyhow};
use axum::{Json, extract::State, http::HeaderMap};
use bcrypt::verify;
use chrono::Utc;
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    auth::{self, TokenPair},
    db,
    error::AppError,
    models::{role::RoleEntity, user::UserEntity, user_role::UserRoleEntity},
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct PasswordLoginRequest {
    pub mobile: String,
    pub password: String,
    pub role_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CodeLoginRequest {
    pub mobile: String,
    pub code: String,
    pub role_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SendCodeRequest {
    pub mobile: String,
}

#[derive(Debug, Deserialize)]
pub struct SwitchRoleRequest {
    pub role_code: String,
}

#[derive(Debug, Deserialize)]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub user: CurrentUserPayload,
}

#[derive(Debug, Serialize)]
pub struct CurrentUserPayload {
    pub id: i64,
    pub name: String,
    pub mobile: String,
    pub avatar_url: Option<String>,
    pub roles: Vec<String>,
    pub current_role: String,
    pub switchable: bool,
    pub status: String,
    pub status_code: String,
    pub status_text: String,
}

pub async fn login_by_password(
    State(state): State<AppState>,
    Json(payload): Json<PasswordLoginRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>, AppError> {
    validate_login_request(&payload)?;

    let user = find_user_by_mobile(&state, &payload.mobile).await?;
    ensure_user_loginable(&user)?;

    let password_hash = user
        .password_hash
        .as_deref()
        .ok_or_else(|| AppError::Unauthorized("账号或密码错误".to_string()))?;

    let password_ok = verify(&payload.password, password_hash)
        .context("failed to verify password")
        .map_err(AppError::Internal)?;
    if !password_ok {
        return Err(AppError::BadRequest("账号或密码错误".to_string()));
    }

    let roles = sort_roles_by_priority(find_role_codes(&state, user.id).await?);
    if roles.is_empty() {
        return Err(AppError::Forbidden(
            "账号未分配角色，请联系管理员".to_string(),
        ));
    }

    let current_role = resolve_current_role(&roles, payload.role_code.as_deref())?;
    let token_pair = auth::issue_token_pair(
        &state.config.auth,
        user.id.unwrap_or_default(),
        user.mobile.as_deref().unwrap_or_default(),
        roles.clone(),
    )
    .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(build_login_response(
        &user,
        roles,
        current_role,
        token_pair,
    )?)))
}

pub async fn refresh_token(
    State(state): State<AppState>,
    Json(payload): Json<RefreshTokenRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>, AppError> {
    if payload.refresh_token.trim().is_empty() {
        return Err(AppError::BadRequest("refresh_token不能为空".to_string()));
    }

    let claims = auth::decode_refresh_token(&state.config.auth, &payload.refresh_token)
        .map_err(|_| AppError::Unauthorized("登录已失效，请重新登录".to_string()))?;

    let user = find_user_by_id(&state, claims.sub).await?;
    ensure_user_loginable(&user)?;

    let roles = sort_roles_by_priority(find_role_codes(&state, user.id).await?);
    let current_role = roles
        .first()
        .cloned()
        .ok_or_else(|| AppError::Forbidden("账号未分配角色，请联系管理员".to_string()))?;
    let token_pair = auth::issue_token_pair(
        &state.config.auth,
        user.id.unwrap_or_default(),
        user.mobile.as_deref().unwrap_or_default(),
        roles.clone(),
    )
    .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(build_login_response(
        &user,
        roles,
        current_role,
        token_pair,
    )?)))
}

pub async fn send_login_code(
    State(state): State<AppState>,
    Json(payload): Json<SendCodeRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    if payload.mobile.trim().is_empty() {
        return Err(AppError::BadRequest("请输入手机号".to_string()));
    }

    let user = find_user_by_mobile(&state, &payload.mobile)
        .await
        .map_err(|_| AppError::BadRequest("账号不存在，请联系管理员创建账号".to_string()))?;
    ensure_user_loginable(&user)?;

    let now = Utc::now().timestamp();
    let mut store = state.verification_codes.write().await;
    if let Some(existing) = store.get(&payload.mobile)
        && now - existing.sent_at < 60
    {
        return Err(AppError::BadRequest(
            "验证码发送过于频繁，请稍后重试".to_string(),
        ));
    }

    let code = format!("{:06}", (now.rem_euclid(1_000_000)));
    store.insert(
        payload.mobile.clone(),
        crate::state::VerificationCodeState {
            code: code.clone(),
            expires_at: now + 300,
            sent_at: now,
        },
    );

    Ok(Json(ApiResponse::success(serde_json::json!({
        "ok": true,
        "code": code
    }))))
}

pub async fn login_by_code(
    State(state): State<AppState>,
    Json(payload): Json<CodeLoginRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>, AppError> {
    if payload.mobile.trim().is_empty() {
        return Err(AppError::BadRequest("请输入手机号".to_string()));
    }
    if payload.code.trim().is_empty() {
        return Err(AppError::BadRequest("请输入验证码".to_string()));
    }

    let now = Utc::now().timestamp();
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
    ensure_user_loginable(&user)?;

    let roles = sort_roles_by_priority(find_role_codes(&state, user.id).await?);
    if roles.is_empty() {
        return Err(AppError::Forbidden(
            "账号未分配角色，请联系管理员".to_string(),
        ));
    }
    let current_role = resolve_current_role(&roles, payload.role_code.as_deref())?;
    let token_pair = auth::issue_token_pair(
        &state.config.auth,
        user.id.unwrap_or_default(),
        user.mobile.as_deref().unwrap_or_default(),
        roles.clone(),
    )
    .map_err(AppError::Internal)?;

    {
        let mut store = state.verification_codes.write().await;
        store.remove(&payload.mobile);
    }

    Ok(Json(ApiResponse::success(build_login_response(
        &user,
        roles,
        current_role,
        token_pair,
    )?)))
}

pub async fn current_user(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<CurrentUserPayload>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;

    let user = find_user_by_id(&state, auth_user.user_id).await?;
    ensure_user_loginable(&user)?;

    let roles = sort_roles_by_priority(find_role_codes(&state, user.id).await?);
    let current_role = roles
        .first()
        .cloned()
        .ok_or_else(|| AppError::Forbidden("账号未分配角色，请联系管理员".to_string()))?;

    Ok(Json(ApiResponse::success(CurrentUserPayload {
        id: user.id.unwrap_or_default(),
        name: user.name.unwrap_or_default(),
        mobile: user.mobile.unwrap_or_default(),
        avatar_url: user.avatar_url.clone(),
        switchable: roles.len() > 1,
        roles,
        current_role,
        status: auth_status_text(user.status.as_deref()).to_string(),
        status_code: user.status.clone().unwrap_or_else(|| "enabled".to_string()),
        status_text: auth_status_text(user.status.as_deref()).to_string(),
    })))
}

pub async fn switch_role(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SwitchRoleRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let user = find_user_by_id(&state, auth_user.user_id).await?;
    ensure_user_loginable(&user)?;

    let roles = sort_roles_by_priority(find_role_codes(&state, user.id).await?);
    let current_role = resolve_current_role(&roles, Some(payload.role_code.as_str()))?;
    let token_pair = auth::issue_token_pair(
        &state.config.auth,
        user.id.unwrap_or_default(),
        user.mobile.as_deref().unwrap_or_default(),
        roles.clone(),
    )
    .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(build_login_response(
        &user,
        roles,
        current_role,
        token_pair,
    )?)))
}

pub async fn logout() -> Json<ApiResponse<serde_json::Value>> {
    Json(ApiResponse::success(serde_json::json!({ "ok": true })))
}

fn validate_login_request(payload: &PasswordLoginRequest) -> Result<(), AppError> {
    if payload.mobile.trim().is_empty() {
        return Err(AppError::BadRequest("请输入手机号".to_string()));
    }
    if payload.password.trim().is_empty() {
        return Err(AppError::BadRequest("请输入密码".to_string()));
    }
    Ok(())
}

fn ensure_user_loginable(user: &UserEntity) -> Result<(), AppError> {
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

fn resolve_current_role(
    roles: &[String],
    requested_role: Option<&str>,
) -> Result<String, AppError> {
    if let Some(role) = requested_role {
        if roles.iter().any(|item| item == role) {
            return Ok(role.to_string());
        }
        return Err(AppError::Forbidden("当前账号不具备指定角色".to_string()));
    }

    roles
        .first()
        .cloned()
        .ok_or_else(|| AppError::Forbidden("账号未分配角色，请联系管理员".to_string()))
}

fn build_login_response(
    user: &UserEntity,
    roles: Vec<String>,
    current_role: String,
    token_pair: TokenPair,
) -> Result<LoginResponse, AppError> {
    Ok(LoginResponse {
        access_token: token_pair.access_token,
        refresh_token: token_pair.refresh_token,
        token_type: token_pair.token_type,
        expires_in: token_pair.expires_in,
        user: CurrentUserPayload {
            id: user.id.unwrap_or_default(),
            name: user
                .name
                .clone()
                .ok_or_else(|| AppError::Internal(anyhow!("user.name is missing")))?,
            mobile: user
                .mobile
                .clone()
                .ok_or_else(|| AppError::Internal(anyhow!("user.mobile is missing")))?,
            avatar_url: user.avatar_url.clone(),
            switchable: roles.len() > 1,
            roles,
            current_role,
            status: auth_status_text(user.status.as_deref()).to_string(),
            status_code: user.status.clone().unwrap_or_else(|| "enabled".to_string()),
            status_text: auth_status_text(user.status.as_deref()).to_string(),
        },
    })
}

fn auth_status_text(status: Option<&str>) -> &'static str {
    match status.unwrap_or("enabled") {
        "enabled" | "active" => "已启用",
        "disabled" | "inactive" => "已禁用",
        _ => "未知状态",
    }
}

async fn find_user_by_mobile(state: &AppState, mobile: &str) -> Result<UserEntity, AppError> {
    let users: Vec<UserEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "user", value! { "mobile": mobile })
            .await
            .context("failed to query user by mobile")
            .map_err(AppError::Internal)?;

    users
        .into_iter()
        .next()
        .ok_or_else(|| AppError::BadRequest("账号或密码错误".to_string()))
}

async fn find_user_by_id(state: &AppState, user_id: i64) -> Result<UserEntity, AppError> {
    let users: Vec<UserEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "user", value! { "id": user_id })
            .await
            .context("failed to query user by id")
            .map_err(AppError::Internal)?;

    users
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Unauthorized("登录已失效，请重新登录".to_string()))
}

async fn find_role_codes(state: &AppState, user_id: Option<i64>) -> Result<Vec<String>, AppError> {
    let user_id = user_id.ok_or_else(|| AppError::Internal(anyhow!("user.id is missing")))?;
    let user_roles: Vec<UserRoleEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": user_id },
    )
    .await
    .context("failed to query user roles")
    .map_err(AppError::Internal)?;

    let role_ids: Vec<i64> = user_roles
        .into_iter()
        .filter_map(|item| item.role_id)
        .collect();
    if role_ids.is_empty() {
        return Ok(Vec::new());
    }

    let roles: Vec<RoleEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "role", value! { "id": role_ids })
            .await
            .context("failed to query roles")
            .map_err(AppError::Internal)?;

    Ok(roles
        .into_iter()
        .filter_map(|item| item.role_code)
        .collect())
}

fn sort_roles_by_priority(mut roles: Vec<String>) -> Vec<String> {
    roles.sort_by_key(|role| role_priority(role));
    roles.dedup();
    roles
}

fn role_priority(role: &str) -> i32 {
    match role {
        "dealer" => 1,
        "technician" => 2,
        "hq_reviewer" => 3,
        "super_admin" => 4,
        _ => 99,
    }
}
