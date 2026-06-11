use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Utc;
use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rbs::value;
use serde::{Deserialize, Serialize};
use sha1::Sha1;

use crate::{
    db,
    error::AppError,
    models::{system_config::SystemConfigEntity, user::UserEntity, user_message_setting::UserMessageSettingEntity},
    state::AppState,
};

type HmacSha1 = Hmac<Sha1>;

const ALIYUN_PERCENT_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

const LOGIN_CODE_TEMPLATE_CODE: &str = "SMS_506190346";
const CUSTOMER_REVIEW_TEMPLATE_CODE: &str = "SMS_506145373";
const DEALER_HQ_SCORE_TEMPLATE_CODE: &str = "SMS_506265341";
const CUSTOMER_REVIEW_SIGN_NAME: &str = "如鱼得水杭州软装定制";

#[derive(Clone, Debug)]
pub struct SmsTemplate {
    pub template_code: &'static str,
    pub sign_name: &'static str,
}

#[derive(Clone, Debug)]
pub struct CustomerReviewSmsPayload<'a> {
    pub review_link_code: &'a str,
}

#[derive(Clone, Debug, Default)]
pub struct SmsRuntimeConfig {
    pub enabled: bool,
    pub access_key_id: String,
    pub access_key_secret: String,
    pub sign_name: String,
    pub endpoint: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SmsDeliveryResult {
    pub request_id: Option<String>,
    pub biz_id: Option<String>,
    pub code: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AliyunSmsResponse {
    #[serde(rename = "RequestId")]
    request_id: Option<String>,
    #[serde(rename = "BizId")]
    biz_id: Option<String>,
    #[serde(rename = "Code")]
    code: Option<String>,
    #[serde(rename = "Message")]
    message: Option<String>,
}

pub async fn send_login_code_sms(
    state: &AppState,
    mobile: &str,
    code: &str,
) -> Result<Option<SmsDeliveryResult>, AppError> {
    let params = value!({ "code": code });
    send_sms_by_mobile(state, mobile, &login_code_template(state), params).await
}

pub async fn send_customer_review_sms(
    state: &AppState,
    mobile: &str,
    payload: CustomerReviewSmsPayload<'_>,
) -> Result<Option<SmsDeliveryResult>, AppError> {
    if !load_config_bool(state, "sms.switch.customer_review", true).await? {
        tracing::info!(mobile, "customer review sms skipped because platform switch is disabled");
        return Ok(None);
    }
    let params = value! { "code": payload.review_link_code.to_string() };
    send_sms_by_mobile(state, mobile, &customer_review_template(state), params).await
}

pub async fn send_dealer_hq_score_sms(
    state: &AppState,
    receiver_id: i64,
) -> Result<Option<SmsDeliveryResult>, AppError> {
    if !load_config_bool(state, "sms.switch.dealer_notify", true).await? {
        tracing::info!(receiver_id, "dealer notify sms skipped because platform switch is disabled");
        return Ok(None);
    }
    if !user_sms_enabled(state, receiver_id).await? {
        return Ok(None);
    }
    let user = load_user_by_id(state, receiver_id).await?;
    let mobile = user
        .mobile
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .ok_or_else(|| AppError::BadRequest("经销商手机号不存在，无法发送短信".to_string()))?;
    send_sms_by_mobile(
        state,
        mobile,
        &dealer_hq_score_template(state),
        value!({}),
    )
    .await
}

pub fn login_code_template(state: &AppState) -> SmsTemplate {
    SmsTemplate {
        template_code: LOGIN_CODE_TEMPLATE_CODE,
        sign_name: sms_sign_name_fallback(state),
    }
}

pub fn customer_review_template(state: &AppState) -> SmsTemplate {
    SmsTemplate {
        template_code: CUSTOMER_REVIEW_TEMPLATE_CODE,
        sign_name: customer_review_sign_name(state),
    }
}

pub fn dealer_hq_score_template(state: &AppState) -> SmsTemplate {
    SmsTemplate {
        template_code: DEALER_HQ_SCORE_TEMPLATE_CODE,
        sign_name: sms_sign_name_fallback(state),
    }
}

async fn send_sms_by_mobile(
    state: &AppState,
    mobile: &str,
    template: &SmsTemplate,
    template_params: rbs::Value,
) -> Result<Option<SmsDeliveryResult>, AppError> {
    let config = load_runtime_config(state).await?;
    if !config.enabled {
        tracing::info!(
            mobile,
            template_code = template.template_code,
            "sms sending skipped because sms is disabled"
        );
        return Ok(None);
    }
    if config.sign_name.trim().is_empty() && template.sign_name.trim().is_empty() {
        return Err(AppError::Conflict("阿里云短信签名未配置".to_string()));
    }
    if config.access_key_id.trim().is_empty() || config.access_key_secret.trim().is_empty() {
        return Err(AppError::Conflict("阿里云短信配置不完整".to_string()));
    }

    let template_param_json = to_template_param_json(template_params)?;
    let sign_name = if let Some(config_sign_name) = normalize_sign_name(&config.sign_name) {
        config_sign_name
    } else {
        template.sign_name.to_string()
    };

    let request = build_aliyun_sms_request(
        &config,
        mobile,
        &sign_name,
        template.template_code,
        &template_param_json,
    )?;
    let query = request
        .iter()
        .map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    let endpoint = normalize_endpoint(&config.endpoint);
    let url = format!("{endpoint}/?{query}");
    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .context("failed to request aliyun sms")
        .map_err(AppError::Internal)?
        .json::<AliyunSmsResponse>()
        .await
        .context("failed to decode aliyun sms response")
        .map_err(AppError::Internal)?;

    if response.code.as_deref() != Some("OK") {
        return Err(AppError::Conflict(
            response
                .message
                .clone()
                .unwrap_or_else(|| "阿里云短信发送失败".to_string()),
        ));
    }

    Ok(Some(SmsDeliveryResult {
        request_id: response.request_id,
        biz_id: response.biz_id,
        code: response.code,
        message: response.message,
    }))
}

async fn load_runtime_config(state: &AppState) -> Result<SmsRuntimeConfig, AppError> {
    let enabled = load_config_bool(state, "sms.aliyun.enabled", false).await?;
    let access_key_id = load_config_string(state, "sms.aliyun.access_key_id").await?;
    let access_key_secret = load_config_string(state, "sms.aliyun.access_key_secret").await?;
    let sign_name = load_config_string(state, "sms.aliyun.sign_name").await?;
    let endpoint = load_config_string(state, "sms.aliyun.endpoint").await?;
    Ok(SmsRuntimeConfig {
        enabled,
        access_key_id,
        access_key_secret,
        sign_name,
        endpoint: if endpoint.trim().is_empty() {
            "https://dysmsapi.aliyuncs.com/".to_string()
        } else {
            endpoint
        },
    })
}

fn build_aliyun_sms_request(
    config: &SmsRuntimeConfig,
    mobile: &str,
    sign_name: &str,
    template_code: &str,
    template_param_json: &str,
) -> Result<BTreeMap<String, String>> {
    let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let nonce = uuid::Uuid::new_v4().to_string();

    let mut params = BTreeMap::new();
    params.insert("AccessKeyId".to_string(), config.access_key_id.clone());
    params.insert("Action".to_string(), "SendSms".to_string());
    params.insert("Format".to_string(), "JSON".to_string());
    params.insert("PhoneNumbers".to_string(), mobile.to_string());
    params.insert("RegionId".to_string(), "cn-hangzhou".to_string());
    params.insert("SignName".to_string(), sign_name.to_string());
    params.insert("SignatureMethod".to_string(), "HMAC-SHA1".to_string());
    params.insert("SignatureNonce".to_string(), nonce);
    params.insert("SignatureVersion".to_string(), "1.0".to_string());
    params.insert("TemplateCode".to_string(), template_code.to_string());
    params.insert("TemplateParam".to_string(), template_param_json.to_string());
    params.insert("Timestamp".to_string(), timestamp);
    params.insert("Version".to_string(), "2017-05-25".to_string());

    let canonicalized = params
        .iter()
        .map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    let string_to_sign = format!("GET&%2F&{}", percent_encode(&canonicalized));
    let signature = sign_with_hmac_sha1(&format!("{}&", config.access_key_secret), &string_to_sign)?;
    params.insert("Signature".to_string(), signature);
    Ok(params)
}

fn sign_with_hmac_sha1(secret: &str, payload: &str) -> Result<String> {
    let mut mac = HmacSha1::new_from_slice(secret.as_bytes())
        .map_err(|_| anyhow!("failed to init hmac sha1"))?;
    mac.update(payload.as_bytes());
    Ok(STANDARD.encode(mac.finalize().into_bytes()))
}

fn percent_encode(input: &str) -> String {
    utf8_percent_encode(input, ALIYUN_PERCENT_ENCODE_SET).to_string()
}

fn normalize_endpoint(endpoint: &str) -> String {
    endpoint.trim_end_matches('/').to_string()
}

fn normalize_sign_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_matches('"').trim();
    if trimmed.is_empty() || trimmed.chars().all(|ch| ch == '?') {
        return None;
    }
    Some(trimmed.to_string())
}

fn to_template_param_json(value: rbs::Value) -> Result<String, AppError> {
    match value {
        rbs::Value::Map(map) => {
            let mut object = serde_json::Map::new();
            for (key, value) in map {
                let key = key
                    .as_str()
                    .ok_or_else(|| AppError::Internal(anyhow!("invalid sms template key")))?;
                object.insert(key.to_string(), rbs_value_to_json(value));
            }
            Ok(serde_json::Value::Object(object).to_string())
        }
        _ => Err(AppError::Internal(anyhow!("invalid sms template params"))),
    }
}

fn rbs_value_to_json(value: rbs::Value) -> serde_json::Value {
    match value {
        rbs::Value::Null => serde_json::Value::Null,
        rbs::Value::Bool(v) => serde_json::Value::Bool(v),
        rbs::Value::I32(v) => serde_json::Value::from(v),
        rbs::Value::I64(v) => serde_json::Value::from(v),
        rbs::Value::U32(v) => serde_json::Value::from(v),
        rbs::Value::U64(v) => serde_json::Value::from(v),
        rbs::Value::F32(v) => serde_json::Value::from(v as f64),
        rbs::Value::F64(v) => serde_json::Value::from(v),
        rbs::Value::String(v) => serde_json::Value::String(v),
        rbs::Value::Binary(v) => serde_json::Value::String(String::from_utf8_lossy(&v).to_string()),
        rbs::Value::Array(values) => serde_json::Value::Array(values.into_iter().map(rbs_value_to_json).collect()),
        rbs::Value::Map(map) => {
            let mut object = serde_json::Map::new();
            for (key, value) in map {
                let key = key
                    .as_str()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("{key:?}"));
                object.insert(key, rbs_value_to_json(value));
            }
            serde_json::Value::Object(object)
        }
        rbs::Value::Ext(_, ext) => rbs_value_to_json(*ext),
    }
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
    .context("failed to query sms system config")
    .map_err(AppError::Internal)?;
    Ok(item
        .and_then(|item| item.config_value)
        .map(|raw| serde_json::from_str::<String>(&raw).unwrap_or(raw))
        .filter(|item| !item.trim().is_empty()))
}

async fn load_config_bool(state: &AppState, key: &str, default: bool) -> Result<bool, AppError> {
    let value = load_config_optional_string(state, key).await?;
    Ok(match value.as_deref() {
        Some("true") | Some("1") | Some("enabled") | Some("启用") => true,
        Some("false") | Some("0") | Some("disabled") | Some("禁用") => false,
        _ => default,
    })
}

async fn user_sms_enabled(state: &AppState, user_id: i64) -> Result<bool, AppError> {
    let setting = db::select_one_by_map_compat::<UserMessageSettingEntity>(
        state.rb.as_ref(),
        "user_message_setting",
        value! { "user_id": user_id },
    )
    .await
    .context("failed to query sms message setting")
    .map_err(AppError::Internal)?;
    Ok(setting
        .as_ref()
        .map(|item| item.sms_enabled.unwrap_or(1) == 1)
        .unwrap_or(true))
}

async fn load_user_by_id(state: &AppState, user_id: i64) -> Result<UserEntity, AppError> {
    db::select_one_by_map_compat::<UserEntity>(
        state.rb.as_ref(),
        "user",
        value! { "id": user_id },
    )
    .await
    .context("failed to query sms receiver user")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::NotFound("用户不存在，无法发送短信".to_string()))
}

fn sms_sign_name_fallback(_state: &AppState) -> &'static str {
    "如鱼得水高端窗帘"
}

fn customer_review_sign_name(_state: &AppState) -> &'static str {
    CUSTOMER_REVIEW_SIGN_NAME
}
