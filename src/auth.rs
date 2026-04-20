use anyhow::{anyhow, Context, Result};
use axum::http::{header, HeaderMap};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::{config::AuthConfig, error::AppError, state::AppState};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: i64,
    pub mobile: String,
    pub roles: Vec<String>,
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
}

pub fn issue_token_pair(
    config: &AuthConfig,
    user_id: i64,
    mobile: &str,
    roles: Vec<String>,
) -> Result<TokenPair> {
    let now = Utc::now();
    let access_exp = now + Duration::minutes(config.access_token_expire_minutes);
    let refresh_exp = now + Duration::days(config.refresh_token_expire_days);

    let access_claims = JwtClaims {
        sub: user_id,
        mobile: mobile.to_string(),
        roles: roles.clone(),
        token_kind: "access".to_string(),
        iat: now.timestamp(),
        exp: access_exp.timestamp(),
    };

    let refresh_claims = JwtClaims {
        sub: user_id,
        mobile: mobile.to_string(),
        roles,
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

    Ok(CurrentAuthUser {
        user_id: claims.sub,
        mobile: claims.mobile,
        roles: claims.roles,
    })
}

impl CurrentAuthUser {
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|item| item == role)
    }

    pub fn require_any_role(&self, roles: &[&str]) -> Result<(), AppError> {
        if roles.iter().any(|role| self.has_role(role)) {
            return Ok(());
        }
        Err(AppError::Forbidden("您没有权限执行该操作".to_string()))
    }
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
