use anyhow::{Context, Result, anyhow};
use axum::http::{HeaderMap, header};
use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use tokio::{runtime::Handle, task::block_in_place};

use crate::{config::AuthConfig, db, error::AppError, models::user::UserEntity, state::AppState};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: i64,
    pub mobile: String,
    pub roles: Vec<String>,
    pub current_role: String,
    pub session_version: i64,
    pub token_kind: String,
    pub iat: i64,
    pub exp: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurrentAuthUser {
    pub user_id: i64,
    pub mobile: String,
    pub roles: Vec<String>,
    pub current_role: String,
    pub session_version: i64,
}

pub fn issue_token_pair(
    config: &AuthConfig,
    user_id: i64,
    mobile: &str,
    roles: Vec<String>,
    current_role: &str,
    session_version: i64,
) -> Result<TokenPair> {
    let now = Utc::now();
    let access_exp = now + Duration::minutes(config.access_token_expire_minutes);
    let refresh_exp = now + Duration::days(config.refresh_token_expire_days);

    let access_claims = JwtClaims {
        sub: user_id,
        mobile: mobile.to_string(),
        roles: roles.clone(),
        current_role: current_role.to_string(),
        session_version,
        token_kind: "access".to_string(),
        iat: now.timestamp(),
        exp: access_exp.timestamp(),
    };

    let refresh_claims = JwtClaims {
        sub: user_id,
        mobile: mobile.to_string(),
        roles,
        current_role: current_role.to_string(),
        session_version,
        token_kind: "refresh".to_string(),
        iat: now.timestamp(),
        exp: refresh_exp.timestamp(),
    };

    Ok(TokenPair {
        access_token: encode_token(config, &access_claims)?,
        refresh_token: encode_token(config, &refresh_claims)?,
        token_type: "Bearer".to_string(),
        expires_in: config.access_token_expire_minutes * 60,
    })
}

pub fn decode_access_token(config: &AuthConfig, token: &str) -> Result<JwtClaims> {
    let claims = decode_token(config, token)?;
    if claims.token_kind != "access" {
        return Err(anyhow!("invalid access token"));
    }
    Ok(claims)
}

pub fn decode_refresh_token(config: &AuthConfig, token: &str) -> Result<JwtClaims> {
    let claims = decode_token(config, token)?;
    if claims.token_kind != "refresh" {
        return Err(anyhow!("invalid refresh token"));
    }
    Ok(claims)
}

pub fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<CurrentAuthUser, AppError> {
    let token = extract_bearer_token(headers)?;
    let claims = decode_access_token(&state.config.auth, &token)
        .map_err(|_| AppError::Unauthorized("登录已失效，请重新登录".to_string()))?;

    let user = block_in_place(|| {
        Handle::current().block_on(db::select_one_by_map_compat::<UserEntity>(
            state.rb.as_ref(),
            "user",
            rbs::value! { "id": claims.sub },
        ))
    })
    .map_err(|_| AppError::Unauthorized("登录已失效，请重新登录".to_string()))?
    .ok_or_else(|| AppError::Unauthorized("登录已失效，请重新登录".to_string()))?;
    if user.deleted_at.is_some() || user.status.as_deref() == Some("disabled") {
        return Err(AppError::Unauthorized("登录已失效，请重新登录".to_string()));
    }
    if user.session_version.unwrap_or(1) != claims.session_version {
        return Err(AppError::Unauthorized("账号权限已变更，请重新登录".to_string()));
    }

    Ok(CurrentAuthUser {
        user_id: claims.sub,
        mobile: claims.mobile,
        roles: claims.roles,
        current_role: claims.current_role,
        session_version: claims.session_version,
    })
}

impl CurrentAuthUser {
    pub fn has_role(&self, role: &str) -> bool {
        self.current_role == role
    }

    pub fn require_any_role(&self, roles: &[&str]) -> Result<(), AppError> {
        if roles.iter().any(|role| self.has_role(role)) {
            return Ok(());
        }
        Err(AppError::Forbidden("您没有权限执行该操作".to_string()))
    }

    pub fn primary_role(&self) -> &str {
        self.current_role.as_str()
    }
}

pub async fn bump_user_session_version(state: &AppState, user_id: i64) -> Result<(), AppError> {
    let user = db::select_one_by_map_compat::<UserEntity>(
        state.rb.as_ref(),
        "user",
        rbs::value! { "id": user_id },
    )
    .await
    .context("failed to query user for session version bump")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::NotFound("用户不存在".to_string()))?;
    let next_version = user.session_version.unwrap_or(1) + 1;
    UserEntity::update_by_map(
        state.rb.as_ref(),
        &UserEntity {
            session_version: Some(next_version),
            ..Default::default()
        },
        rbs::value! { "id": user_id, "column": ["session_version"] },
    )
    .await
    .context("failed to update user session version")
    .map_err(AppError::Internal)?;
    Ok(())
}

fn encode_token(config: &AuthConfig, claims: &JwtClaims) -> Result<String> {
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
    )
    .context("failed to encode jwt")
}

fn decode_token(config: &AuthConfig, token: &str) -> Result<JwtClaims> {
    let validation = Validation::new(Algorithm::HS256);
    let data = decode::<JwtClaims>(
        token,
        &DecodingKey::from_secret(config.jwt_secret.as_bytes()),
        &validation,
    )
    .context("failed to decode jwt")?;
    Ok(data.claims)
}

fn extract_bearer_token(headers: &HeaderMap) -> Result<String, AppError> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("登录已失效，请重新登录".to_string()))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized("登录已失效，请重新登录".to_string()))?;

    if token.trim().is_empty() {
        return Err(AppError::Unauthorized("登录已失效，请重新登录".to_string()));
    }

    Ok(token.to_string())
}
