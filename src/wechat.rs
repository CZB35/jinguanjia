use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use chrono::{Duration, Local, Utc};
use rbatis::RBatis;
use rbs::value;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{
    config::AppConfig,
    db,
    error::AppError,
    models::{notice::NoticeEntity, notice_delivery::NoticeDeliveryEntity, system_config::SystemConfigEntity, user::UserEntity},
    state::AppState,
};

#[derive(Clone, Default)]
pub struct WechatRuntime {
    pub access_token: Arc<RwLock<Option<WechatAccessTokenState>>>,
}

#[derive(Clone, Debug)]
pub struct WechatAccessTokenState {
    pub access_token: String,
    pub expires_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WechatBindStateClaims {
    pub sub: i64,
    pub purpose: String,
    pub iat: i64,
    pub exp: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WechatServiceAccountConfig {
    pub appid: String,
    pub secret: String,
    pub bind_redirect_uri: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WechatTemplateConfig {
    pub template_id: String,
    pub page_path: Option<String>,
    pub miniprogram_appid: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WechatNotificationSettings {
    pub enabled: bool,
    pub retry_limit: i32,
    pub retry_interval_seconds: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ServiceAccountOAuthTokenResponse {
    pub access_token: Option<String>,
    pub expires_in: Option<i64>,
    pub refresh_token: Option<String>,
    pub openid: Option<String>,
    pub scope: Option<String>,
    pub unionid: Option<String>,
    pub errcode: Option<i64>,
    pub errmsg: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ServiceAccountAccessTokenResponse {
    pub access_token: Option<String>,
    pub expires_in: Option<i64>,
    pub errcode: Option<i64>,
    pub errmsg: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WechatTemplateSendResponse {
    pub msgid: Option<i64>,
    pub errcode: Option<i64>,
    pub errmsg: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WechatBindStatus {
    pub wechat_bound: bool,
    pub wechat_openid: Option<String>,
    pub wechat_subscribe: bool,
    pub wechat_bind_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WechatTemplateSendPayload {
    touser: String,
    template_id: String,
    url: Option<String>,
    miniprogram: Option<WechatTemplateMiniprogram>,
    data: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WechatTemplateMiniprogram {
    appid: String,
    pagepath: String,
}

pub async fn build_service_account_bind_url(state: &AppState, user_id: i64) -> Result<String, AppError> {
    let config = load_service_account_config(state).await?;
    if config.appid.trim().is_empty() || config.bind_redirect_uri.trim().is_empty() {
        return Err(AppError::Conflict("微信服务号配置不完整".to_string()));
    }
    let state_token = build_bind_state_token(&state.config, user_id).map_err(AppError::Internal)?;
    Ok(format!(
        "https://open.weixin.qq.com/connect/oauth2/authorize?appid={}&redirect_uri={}&response_type=code&scope=snsapi_base&state={}#wechat_redirect",
        urlencoding::encode(&config.appid),
        urlencoding::encode(&config.bind_redirect_uri),
        urlencoding::encode(&state_token),
    ))
}

pub fn build_bind_state_token(config: &AppConfig, user_id: i64) -> Result<String> {
    let now = Utc::now();
    let claims = WechatBindStateClaims {
        sub: user_id,
        purpose: "wechat_service_bind".to_string(),
        iat: now.timestamp(),
        exp: (now + Duration::minutes(10)).timestamp(),
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(config.auth.jwt_secret.as_bytes()),
    )
    .context("failed to encode wechat bind state token")
}

pub fn decode_bind_state_token(config: &AppConfig, token: &str) -> Result<WechatBindStateClaims> {
    let data = jsonwebtoken::decode::<WechatBindStateClaims>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(config.auth.jwt_secret.as_bytes()),
        &jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256),
    )
    .context("failed to decode wechat bind state token")?;
    if data.claims.purpose != "wechat_service_bind" {
        return Err(anyhow!("invalid bind state token"));
    }
    Ok(data.claims)
}

pub async fn exchange_service_account_oauth_code(
    state: &AppState,
    code: &str,
) -> Result<ServiceAccountOAuthTokenResponse, AppError> {
    let config = load_service_account_config(state).await?;
    if config.appid.trim().is_empty() || config.secret.trim().is_empty() {
        return Err(AppError::Conflict("微信服务号配置不完整".to_string()));
    }

    let url = format!(
        "https://api.weixin.qq.com/sns/oauth2/access_token?appid={}&secret={}&code={}&grant_type=authorization_code",
        urlencoding::encode(&config.appid),
        urlencoding::encode(&config.secret),
        urlencoding::encode(code),
    );
    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .context("failed to request service account oauth token")
        .map_err(AppError::Internal)?
        .json::<ServiceAccountOAuthTokenResponse>()
        .await
        .context("failed to decode service account oauth token response")
        .map_err(AppError::Internal)?;
    if response.errcode.unwrap_or_default() != 0 {
        return Err(AppError::Conflict(
            response
                .errmsg
                .unwrap_or_else(|| "微信服务号授权失败".to_string()),
        ));
    }
    Ok(response)
}

pub async fn bind_service_account_user(
    state: &AppState,
    user_id: i64,
    openid: &str,
    unionid: Option<&str>,
) -> Result<(), AppError> {
    UserEntity::update_by_map(
        state.rb.as_ref(),
        &UserEntity {
            wechat_openid: Some(openid.to_string()),
            wechat_unionid: unionid.map(ToString::to_string),
            wechat_subscribe: Some(1),
            wechat_bind_at: Some(Local::now().naive_local()),
            wechat_subscribe_updated_at: Some(Local::now().naive_local()),
            ..Default::default()
        },
        value! {
            "id": user_id,
            "column": ["wechat_openid", "wechat_unionid", "wechat_subscribe", "wechat_bind_at", "wechat_subscribe_updated_at"]
        },
    )
    .await
    .context("failed to bind wechat service account user")
    .map_err(AppError::Internal)?;
    Ok(())
}

pub async fn get_bind_status(state: &AppState, user_id: i64) -> Result<WechatBindStatus, AppError> {
    let user: UserEntity = db::select_one_by_map_compat(state.rb.as_ref(), "user", value! { "id": user_id })
        .await
        .context("failed to query user for wechat bind status")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("用户不存在".to_string()))?;
    Ok(WechatBindStatus {
        wechat_bound: user.wechat_openid.as_deref().map(|item| !item.trim().is_empty()).unwrap_or(false),
        wechat_openid: user.wechat_openid,
        wechat_subscribe: user.wechat_subscribe.unwrap_or(0) == 1,
        wechat_bind_at: user.wechat_bind_at.map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string()),
    })
}

pub async fn enqueue_wechat_notice(
    state: &AppState,
    notice: &NoticeEntity,
) -> Result<(), AppError> {
    let Some(receiver_id) = notice.receiver_id else {
        return Ok(());
    };
    let Some(notice_id) = notice.id else {
        return Ok(());
    };

    let setting = db::select_one_by_map_compat::<crate::models::user_message_setting::UserMessageSettingEntity>(
        state.rb.as_ref(),
        "user_message_setting",
        value! { "user_id": receiver_id },
    )
    .await
    .context("failed to query user message setting for wechat notice")
    .map_err(AppError::Internal)?;
    if setting
        .as_ref()
        .map(|item| item.wechat_enabled.unwrap_or(1) == 0)
        .unwrap_or(false)
    {
        return Ok(());
    }

    let user = db::select_one_by_map_compat::<UserEntity>(
        state.rb.as_ref(),
        "user",
        value! { "id": receiver_id },
    )
    .await
    .context("failed to query receiver user for wechat notice")
    .map_err(AppError::Internal)?;
    let Some(user) = user else {
        return Ok(());
    };
    let Some(openid) = user.wechat_openid.clone().filter(|item| !item.trim().is_empty()) else {
        return Ok(());
    };

    let template_code = notice.biz_type.clone().unwrap_or_default();
    let payload_json = serde_json::json!({
        "title": notice.title.clone().unwrap_or_default(),
        "content": notice.content.clone().unwrap_or_default(),
        "biz_type": notice.biz_type.clone().unwrap_or_default(),
        "biz_id": notice.biz_id.unwrap_or_default(),
    })
    .to_string();

    let existing = db::select_one_by_map_compat::<NoticeDeliveryEntity>(
        state.rb.as_ref(),
        "notice_delivery",
        value! { "notice_id": notice_id, "channel": "wechat" },
    )
    .await
    .context("failed to query existing wechat delivery")
    .map_err(AppError::Internal)?;
    if existing.is_some() {
        return Ok(());
    }

    NoticeDeliveryEntity::insert(
        state.rb.as_ref(),
        &NoticeDeliveryEntity {
            id: None,
            notice_id: Some(notice_id),
            channel: Some("wechat".to_string()),
            receiver_id: Some(receiver_id),
            receiver_openid: Some(openid),
            biz_type: notice.biz_type.clone(),
            biz_id: notice.biz_id,
            template_code: Some(template_code),
            payload_json: Some(payload_json),
            status: Some("pending".to_string()),
            attempt_count: Some(0),
            next_retry_at: Some(Local::now().naive_local()),
            provider_message_id: None,
            error_message: None,
            created_at: None,
            sent_at: None,
        },
    )
    .await
    .context("failed to insert wechat notice delivery")
    .map_err(AppError::Internal)?;

    Ok(())
}

pub async fn start_notice_delivery_worker(state: AppState) {
    let worker_state = state.clone();
    tokio::spawn(async move {
        let interval = std::time::Duration::from_secs(3);
        loop {
            if let Err(error) = process_pending_deliveries(&worker_state).await {
                tracing::error!(error = %error, "wechat notice delivery worker failed");
            }
            tokio::time::sleep(interval).await;
        }
    });
}

async fn process_pending_deliveries(state: &AppState) -> Result<()> {
    let settings = load_notification_settings(state).await?;
    if !settings.enabled {
        return Ok(());
    }

    let deliveries: Vec<NoticeDeliveryEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "notice_delivery",
        value! { "channel": "wechat", "status": "pending" },
    )
    .await
    .context("failed to query pending wechat deliveries")?;

    for delivery in deliveries.into_iter().take(20) {
        let Some(delivery_id) = delivery.id else {
            continue;
        };
        let next_retry_at = delivery.next_retry_at.unwrap_or_else(|| Local::now().naive_local());
        if next_retry_at > Local::now().naive_local() {
            continue;
        }
        update_delivery_status(state.rb.as_ref(), delivery_id, "processing", delivery.attempt_count.unwrap_or(0), None, None).await?;

        let send_result = send_wechat_delivery(state, &delivery).await;
        match send_result {
            Ok(provider_msg_id) => {
                NoticeDeliveryEntity::update_by_map(
                    state.rb.as_ref(),
                    &NoticeDeliveryEntity {
                        status: Some("sent".to_string()),
                        attempt_count: Some(delivery.attempt_count.unwrap_or(0) + 1),
                        provider_message_id: provider_msg_id,
                        error_message: None,
                        sent_at: Some(Local::now().naive_local()),
                        ..Default::default()
                    },
                    value! { "id": delivery_id, "column": ["status", "attempt_count", "provider_message_id", "error_message", "sent_at"] },
                )
                .await
                .context("failed to mark wechat delivery as sent")?;
            }
            Err(error) => {
                let attempt = delivery.attempt_count.unwrap_or(0) + 1;
                let final_status = if attempt >= settings.retry_limit.max(1) {
                    "failed"
                } else {
                    "pending"
                };
                let retry_at = if final_status == "pending" {
                    Some(Local::now().naive_local() + Duration::seconds(settings.retry_interval_seconds.max(30)))
                } else {
                    None
                };
                update_delivery_status(
                    state.rb.as_ref(),
                    delivery_id,
                    final_status,
                    attempt,
                    retry_at,
                    Some(error.to_string()),
                )
                .await?;
            }
        }
    }
    Ok(())
}

async fn send_wechat_delivery(state: &AppState, delivery: &NoticeDeliveryEntity) -> Result<Option<String>> {
    let template_code = delivery.template_code.as_deref().unwrap_or_default();
    let template = load_template_config(state, template_code).await?;
    if template.template_id.trim().is_empty() {
        return Err(anyhow!("wechat template not configured: {template_code}"));
    }
    let access_token = get_service_account_access_token(state).await?;
    let payload: serde_json::Value = serde_json::from_str(delivery.payload_json.as_deref().unwrap_or("{}"))
        .context("failed to parse notice delivery payload json")?;

    let request = WechatTemplateSendPayload {
        touser: delivery.receiver_openid.clone().unwrap_or_default(),
        template_id: template.template_id.clone(),
        url: payload
            .get("biz_id")
            .and_then(|item| item.as_i64())
            .map(|biz_id| format!("https://goldenbutler.friendchenkeji.cn/task/{}", biz_id)),
        miniprogram: template
            .miniprogram_appid
            .clone()
            .zip(template.page_path.clone())
            .map(|(appid, pagepath)| WechatTemplateMiniprogram { appid, pagepath }),
        data: serde_json::json!({
            "first": { "value": payload.get("title").and_then(|v| v.as_str()).unwrap_or_default() },
            "keyword1": { "value": payload.get("content").and_then(|v| v.as_str()).unwrap_or_default() },
            "remark": { "value": payload.get("biz_type").and_then(|v| v.as_str()).unwrap_or_default() },
        }),
    };

    let response = reqwest::Client::new()
        .post(format!(
            "https://api.weixin.qq.com/cgi-bin/message/template/send?access_token={}",
            urlencoding::encode(&access_token)
        ))
        .json(&request)
        .send()
        .await
        .context("failed to request wechat template send")?
        .json::<WechatTemplateSendResponse>()
        .await
        .context("failed to decode wechat template send response")?;
    if response.errcode.unwrap_or_default() != 0 {
        return Err(anyhow!(
            "wechat send failed: {}",
            response.errmsg.unwrap_or_else(|| "unknown error".to_string())
        ));
    }
    Ok(response.msgid.map(|item| item.to_string()))
}

async fn get_service_account_access_token(state: &AppState) -> Result<String> {
    {
        let token = state.wechat_runtime.access_token.read().await;
        if let Some(token) = token.as_ref()
            && token.expires_at > Utc::now().timestamp() + 300
        {
            return Ok(token.access_token.clone());
        }
    }

    let config = load_service_account_config(state).await?;
    if config.appid.trim().is_empty() || config.secret.trim().is_empty() {
        return Err(anyhow!("wechat service account config is incomplete"));
    }
    let url = format!(
        "https://api.weixin.qq.com/cgi-bin/token?grant_type=client_credential&appid={}&secret={}",
        urlencoding::encode(&config.appid),
        urlencoding::encode(&config.secret),
    );
    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .context("failed to request wechat access token")?
        .json::<ServiceAccountAccessTokenResponse>()
        .await
        .context("failed to decode wechat access token response")?;
    if response.errcode.unwrap_or_default() != 0 {
        return Err(anyhow!(
            "wechat access token failed: {}",
            response.errmsg.unwrap_or_else(|| "unknown error".to_string())
        ));
    }
    let access_token = response
        .access_token
        .ok_or_else(|| anyhow!("wechat access_token missing"))?;
    let expires_in = response.expires_in.unwrap_or(7200);
    let mut token = state.wechat_runtime.access_token.write().await;
    *token = Some(WechatAccessTokenState {
        access_token: access_token.clone(),
        expires_at: Utc::now().timestamp() + expires_in,
    });
    Ok(access_token)
}

async fn load_service_account_config(state: &AppState) -> Result<WechatServiceAccountConfig, AppError> {
    let appid = load_config_string(state, "wechat.service_account.appid").await?;
    let secret = load_config_string(state, "wechat.service_account.secret").await?;
    let bind_redirect_uri = load_config_string(state, "wechat.service_account.bind_redirect_uri").await?;
    Ok(WechatServiceAccountConfig {
        appid,
        secret,
        bind_redirect_uri,
    })
}

async fn load_notification_settings(state: &AppState) -> Result<WechatNotificationSettings> {
    let enabled = load_config_bool(state, "wechat.notify.enabled", true).await?;
    let retry_limit = load_config_i32(state, "wechat.notify.retry_limit", 3).await?;
    let retry_interval_seconds = load_config_i64(state, "wechat.notify.retry_interval_seconds", 60).await?;
    Ok(WechatNotificationSettings {
        enabled,
        retry_limit,
        retry_interval_seconds,
    })
}

async fn load_template_config(state: &AppState, template_code: &str) -> Result<WechatTemplateConfig> {
    let template_id = load_config_string(state, &format!("wechat.template.{template_code}.template_id")).await?;
    let page_path = load_config_optional_string(state, &format!("wechat.template.{template_code}.page_path")).await?;
    let miniprogram_appid = load_config_optional_string(state, &format!("wechat.template.{template_code}.miniprogram_appid")).await?;
    Ok(WechatTemplateConfig {
        template_id,
        page_path,
        miniprogram_appid,
    })
}

async fn load_config_string(state: &AppState, key: &str) -> Result<String, AppError> {
    Ok(load_config_optional_string(state, key).await?.unwrap_or_default())
}

async fn load_config_optional_string(state: &AppState, key: &str) -> Result<Option<String>, AppError> {
    let item = db::select_one_by_map_compat::<SystemConfigEntity>(
        state.rb.as_ref(),
        "system_config",
        value! { "config_key": key },
    )
    .await
    .context("failed to query system config")
    .map_err(AppError::Internal)?;
    Ok(item
        .and_then(|item| item.config_value)
        .map(|raw| parse_json_string_value(&raw))
        .filter(|item| !item.trim().is_empty()))
}

async fn load_config_bool(state: &AppState, key: &str, default: bool) -> Result<bool> {
    let value = load_config_optional_string(state, key).await?;
    Ok(match value.as_deref() {
        Some("true") | Some("1") | Some("enabled") | Some("启用") => true,
        Some("false") | Some("0") | Some("disabled") | Some("禁用") => false,
        _ => default,
    })
}

async fn load_config_i32(state: &AppState, key: &str, default: i32) -> Result<i32> {
    let value = load_config_optional_string(state, key).await?;
    Ok(value
        .as_deref()
        .and_then(|item| item.parse::<i32>().ok())
        .unwrap_or(default))
}

async fn load_config_i64(state: &AppState, key: &str, default: i64) -> Result<i64> {
    let value = load_config_optional_string(state, key).await?;
    Ok(value
        .as_deref()
        .and_then(|item| item.parse::<i64>().ok())
        .unwrap_or(default))
}

async fn update_delivery_status(
    rb: &RBatis,
    delivery_id: i64,
    status: &str,
    attempt_count: i32,
    next_retry_at: Option<chrono::NaiveDateTime>,
    error_message: Option<String>,
) -> Result<()> {
    NoticeDeliveryEntity::update_by_map(
        rb,
        &NoticeDeliveryEntity {
            status: Some(status.to_string()),
            attempt_count: Some(attempt_count),
            next_retry_at,
            error_message,
            ..Default::default()
        },
        value! { "id": delivery_id, "column": ["status", "attempt_count", "next_retry_at", "error_message"] },
    )
    .await
    .context("failed to update notice delivery status")?;
    Ok(())
}

fn parse_json_string_value(raw: &str) -> String {
    serde_json::from_str::<String>(raw).unwrap_or_else(|_| raw.to_string())
}
