use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use axum::{
    body::Body,
    extract::{Path as AxumPath, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::Utc;
use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::{Client, Method};
use sha1::Sha1;
use tokio::fs;
use tracing::warn;
use uuid::Uuid;

use crate::{error::AppError, state::AppState};

type HmacSha1 = Hmac<Sha1>;

const OSS_OBJECT_KEY_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

#[derive(Clone, Debug)]
pub struct StoredFile {
    pub original_name: String,
    pub relative_path: String,
    pub mime_type: String,
    pub file_size: i64,
}

pub async fn save_bytes(
    state: &AppState,
    sub_dir: &str,
    original_name: &str,
    content_type: Option<&str>,
    bytes: &[u8],
) -> Result<StoredFile> {
    let ext = Path::new(original_name)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or("bin");
    let filename = format!("{}.{}", Uuid::new_v4(), ext);
    let relative_path = join_relative_path(sub_dir, &filename);
    let mime_type = content_type.map(ToString::to_string).unwrap_or_else(|| {
        mime_guess::from_path(original_name)
            .first_or_octet_stream()
            .to_string()
    });

    if state.config.storage.is_oss() {
        put_oss_object(state, &relative_path, &mime_type, bytes).await?;
    } else {
        write_local_file(state, &relative_path, bytes).await?;
    }

    Ok(StoredFile {
        original_name: original_name.to_string(),
        relative_path,
        mime_type,
        file_size: bytes.len() as i64,
    })
}

pub async fn replace_bytes(
    state: &AppState,
    relative_path: &str,
    content_type: Option<&str>,
    bytes: &[u8],
) -> Result<()> {
    let relative_path = normalize_relative_path(relative_path);
    let mime_type = content_type.map(ToString::to_string).unwrap_or_else(|| {
        mime_guess::from_path(&relative_path)
            .first_or_octet_stream()
            .to_string()
    });

    if state.config.storage.is_oss() {
        put_oss_object(state, &relative_path, &mime_type, bytes).await?;
    } else {
        write_local_file(state, &relative_path, bytes).await?;
    }

    Ok(())
}

pub async fn read_bytes(state: &AppState, relative_path: &str) -> Result<Vec<u8>> {
    let relative_path = normalize_relative_path(relative_path);
    if relative_path.is_empty() {
        return Err(anyhow!("relative path is empty"));
    }

    if state.config.storage.is_oss() {
        if let Ok(bytes) = get_oss_object(state, &relative_path).await {
            return Ok(bytes);
        }
    }

    let absolute_path = full_path(state, &relative_path);
    fs::read(&absolute_path)
        .await
        .with_context(|| format!("failed to read file: {}", absolute_path.display()))
}

pub async fn delete_file(state: &AppState, relative_path: &str) -> Result<()> {
    let relative_path = normalize_relative_path(relative_path);
    if relative_path.is_empty() {
        return Ok(());
    }

    let mut first_error: Option<anyhow::Error> = None;

    if state.config.storage.is_oss() {
        if let Err(err) = delete_oss_object(state, &relative_path).await {
            warn!(path = %relative_path, error = %err, "failed to delete oss object");
            first_error = Some(err);
        }
    }

    let absolute_path = full_path(state, &relative_path);
    if let Err(err) = fs::remove_file(&absolute_path).await
        && err.kind() != std::io::ErrorKind::NotFound
    {
        warn!(
            path = %absolute_path.display(),
            error = %err,
            "failed to delete local upload file"
        );
        if first_error.is_none() {
            first_error = Some(anyhow!(err));
        }
    }

    if let Some(err) = first_error {
        return Err(err);
    }

    Ok(())
}

pub async fn serve_upload(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
) -> Result<Response, AppError> {
    let relative_path = normalize_relative_path(&path);
    if relative_path.is_empty() {
        return Err(AppError::NotFound("文件不存在".to_string()));
    }

    let bytes = read_bytes(&state, &relative_path).await.map_err(|err| {
        let not_found = err
            .downcast_ref::<std::io::Error>()
            .map(|io_err| io_err.kind() == std::io::ErrorKind::NotFound)
            .unwrap_or(false);
        if not_found {
            AppError::NotFound("文件不存在".to_string())
        } else {
            AppError::Internal(err.context("failed to load upload file"))
        }
    })?;
    let mime_type = mime_guess::from_path(&relative_path)
        .first_or_octet_stream()
        .to_string();

    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&mime_type)
            .map_err(|_| AppError::Internal(anyhow!("invalid content type header")))?,
    );
    Ok(response)
}

pub fn full_path(state: &AppState, relative_path: &str) -> PathBuf {
    resolve_upload_root(&state.config.storage.upload_dir)
        .join(normalize_relative_path(relative_path))
}

pub fn resolve_upload_root(upload_dir: &str) -> PathBuf {
    let path = PathBuf::from(upload_dir);
    if path.is_absolute() {
        return path;
    }

    if let Ok(exe_path) = std::env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
        if exe_dir
            .file_name()
            .and_then(|item| item.to_str())
            .map(|name| name.eq_ignore_ascii_case("release") || name.eq_ignore_ascii_case("debug"))
            .unwrap_or(false)
            && exe_dir
                .parent()
                .and_then(|item| item.file_name())
                .and_then(|item| item.to_str())
                .map(|name| name.eq_ignore_ascii_case("target"))
                .unwrap_or(false)
            && let Some(project_root) = exe_dir.parent().and_then(|item| item.parent())
        {
            return project_root.join(upload_dir);
        }

        return exe_dir.join(upload_dir);
    }

    path
}

pub fn public_url(relative_path: &str) -> String {
    let normalized = normalize_relative_path(relative_path);
    format!("/uploads/{}", normalized)
}

fn join_relative_path(sub_dir: &str, filename: &str) -> String {
    let sub_dir = normalize_relative_path(sub_dir);
    if sub_dir.is_empty() {
        filename.replace('\\', "/")
    } else {
        format!("{}/{}", sub_dir, filename).replace('\\', "/")
    }
}

fn normalize_relative_path(relative_path: &str) -> String {
    relative_path
        .trim()
        .trim_start_matches('/')
        .replace('\\', "/")
}

async fn write_local_file(state: &AppState, relative_path: &str, bytes: &[u8]) -> Result<()> {
    let absolute_path = full_path(state, relative_path);
    if let Some(parent) = absolute_path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create upload dir: {}", parent.display()))?;
    }
    fs::write(&absolute_path, bytes)
        .await
        .with_context(|| format!("failed to write upload file: {}", absolute_path.display()))
}

fn oss_endpoint_base(state: &AppState) -> Result<String> {
    let endpoint = state
        .config
        .storage
        .oss_endpoint
        .trim()
        .trim_end_matches('/');
    if endpoint.is_empty() {
        return Err(anyhow!("OSS_ENDPOINT is empty"));
    }
    Ok(endpoint.to_string())
}

fn split_endpoint(endpoint: &str) -> (&str, &str) {
    if let Some(host) = endpoint.strip_prefix("https://") {
        ("https", host)
    } else if let Some(host) = endpoint.strip_prefix("http://") {
        ("http", host)
    } else {
        ("https", endpoint)
    }
}

fn oss_canonical_resource(state: &AppState, relative_path: &str) -> Result<String> {
    let bucket = state.config.storage.oss_bucket.trim();
    if bucket.is_empty() {
        return Err(anyhow!("OSS_BUCKET is empty"));
    }
    Ok(format!("/{}/{}", bucket, encode_object_key(relative_path)))
}

fn oss_object_url(state: &AppState, relative_path: &str) -> Result<String> {
    let endpoint = oss_endpoint_base(state)?;
    let bucket = state.config.storage.oss_bucket.trim();
    if bucket.is_empty() {
        return Err(anyhow!("OSS_BUCKET is empty"));
    }
    let (scheme, host) = split_endpoint(&endpoint);
    let host = if host
        .to_ascii_lowercase()
        .starts_with(&format!("{}.", bucket.to_ascii_lowercase()))
    {
        host.to_string()
    } else {
        format!("{}.{}", bucket, host)
    };

    Ok(format!(
        "{}://{}/{}",
        scheme,
        host,
        encode_object_key(relative_path)
    ))
}

async fn put_oss_object(
    state: &AppState,
    relative_path: &str,
    mime_type: &str,
    bytes: &[u8],
) -> Result<()> {
    let url = oss_object_url(state, relative_path)?;
    let date = gmt_http_date();
    let authorization = oss_authorization(
        "PUT",
        "",
        mime_type,
        &date,
        "",
        &oss_canonical_resource(state, relative_path)?,
        &state.config.storage.oss_access_key_id,
        &state.config.storage.oss_access_key_secret,
    )?;

    let client = Client::new();
    let response = client
        .request(Method::PUT, &url)
        .header(header::DATE, date)
        .header(header::CONTENT_TYPE, mime_type)
        .header(header::AUTHORIZATION, authorization)
        .body(bytes.to_vec())
        .send()
        .await
        .context("failed to upload oss object")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("oss upload failed: {} {}", status, body));
    }

    Ok(())
}

async fn get_oss_object(state: &AppState, relative_path: &str) -> Result<Vec<u8>> {
    let url = oss_object_url(state, relative_path)?;
    let date = gmt_http_date();
    let authorization = oss_authorization(
        "GET",
        "",
        "",
        &date,
        "",
        &oss_canonical_resource(state, relative_path)?,
        &state.config.storage.oss_access_key_id,
        &state.config.storage.oss_access_key_secret,
    )?;

    let client = Client::new();
    let response = client
        .request(Method::GET, &url)
        .header(header::DATE, date)
        .header(header::AUTHORIZATION, authorization)
        .send()
        .await
        .context("failed to download oss object")?;

    if !response.status().is_success() {
        return Err(anyhow!("oss download failed: {}", response.status()));
    }

    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(Into::into)
}

async fn delete_oss_object(state: &AppState, relative_path: &str) -> Result<()> {
    let url = oss_object_url(state, relative_path)?;
    let date = gmt_http_date();
    let authorization = oss_authorization(
        "DELETE",
        "",
        "",
        &date,
        "",
        &oss_canonical_resource(state, relative_path)?,
        &state.config.storage.oss_access_key_id,
        &state.config.storage.oss_access_key_secret,
    )?;

    let client = Client::new();
    let response = client
        .request(Method::DELETE, &url)
        .header(header::DATE, date)
        .header(header::AUTHORIZATION, authorization)
        .send()
        .await
        .context("failed to delete oss object")?;

    if response.status().is_success() || response.status().is_client_error() {
        return Ok(());
    }

    Err(anyhow!("oss delete failed: {}", response.status()))
}

fn oss_authorization(
    method: &str,
    content_md5: &str,
    content_type: &str,
    date: &str,
    canonicalized_headers: &str,
    canonicalized_resource: &str,
    access_key_id: &str,
    access_key_secret: &str,
) -> Result<String> {
    let string_to_sign = format!(
        "{method}\n{content_md5}\n{content_type}\n{date}\n{canonicalized_headers}{canonicalized_resource}"
    );
    let mut mac = HmacSha1::new_from_slice(access_key_secret.as_bytes())
        .map_err(|_| anyhow!("invalid oss access key secret"))?;
    mac.update(string_to_sign.as_bytes());
    let signature = BASE64_STANDARD.encode(mac.finalize().into_bytes());
    Ok(format!("OSS {}:{}", access_key_id, signature))
}

fn gmt_http_date() -> String {
    Utc::now().format("%a, %d %b %Y %T GMT").to_string()
}

fn encode_object_key(relative_path: &str) -> String {
    relative_path
        .trim()
        .trim_start_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| utf8_percent_encode(segment, OSS_OBJECT_KEY_ENCODE_SET).to_string())
        .collect::<Vec<_>>()
        .join("/")
}
