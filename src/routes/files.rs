use std::{collections::HashMap, io::Write, path::Path};

use ab_glyph::{FontArc, PxScale};
use anyhow::{Context, anyhow};
use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use chrono::Local;
use image::{ImageFormat, Rgba};
use imageproc::{
    drawing::{draw_filled_rect_mut, draw_text_mut},
    rect::Rect,
};
use font8x8::{BASIC_FONTS, UnicodeFonts};
use rbs::value;
use serde::Serialize;
use tokio::fs;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::{
    auth, db,
    error::AppError,
    models::{
        attachment::AttachmentEntity, customer::CustomerEntity,
        customer_address::CustomerAddressEntity, evidence::EvidenceEntity,
        store::StoreEntity, task::TaskEntity,
        task_execution::TaskExecutionEntity, task_service_item::TaskServiceItemEntity,
        user_store::UserStoreEntity,
    },
    response::ApiResponse,
    state::AppState,
    storage,
};

#[derive(Debug, Serialize)]
pub struct FileItem {
    pub id: i64,
    pub file_name: String,
    pub file_url: String,
    pub file_size: i64,
    pub mime_type: String,
}

#[derive(Debug, Serialize)]
pub struct FileListResponse {
    pub items: Vec<FileItem>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, serde::Deserialize)]
pub struct FileListQuery {
    pub page: Option<usize>,
    pub page_size: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub id: i64,
    pub file_name: String,
    pub file_url: String,
    pub file_size: i64,
    pub mime_type: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct BatchDownloadRequest {
    #[serde(default)]
    pub attachment_ids: Vec<i64>,
    #[serde(default)]
    pub task_ids: Vec<i64>,
}

#[derive(Debug, serde::Deserialize)]
pub struct EvidenceUploadQuery {
    pub execution_id: Option<i64>,
    pub service_item_id: Option<i64>,
    pub service_type: Option<String>,
}

pub async fn upload_task_attachment(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<i64>,
    multipart: Multipart,
) -> Result<Json<ApiResponse<UploadResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    let _ = find_task(&state, task_id).await?;

    let upload = read_single_multipart(multipart).await?;
    let stored = storage::save_bytes(
        &state,
        "attachments",
        &upload.file_name,
        upload.content_type.as_deref(),
        &upload.bytes,
    )
    .await
    .map_err(AppError::Internal)?;

    AttachmentEntity::insert(
        state.rb.as_ref(),
        &AttachmentEntity {
            id: None,
            task_id: Some(task_id),
            uploader_id: Some(auth_user.user_id),
            attachment_type: Some("task_attachment".to_string()),
            file_name: Some(stored.original_name.clone()),
            file_url: Some(stored.relative_path.clone()),
            file_size: Some(stored.file_size),
            mime_type: Some(stored.mime_type.clone()),
            created_at: None,
        },
    )
    .await
    .context("failed to create attachment record")
    .map_err(AppError::Internal)?;

    let item: AttachmentEntity = db::select_one_by_map_compat(
        state.rb.as_ref(),
        "attachment",
        value! { "task_id": task_id, "file_url": stored.relative_path.as_str() },
    )
    .await
    .context("failed to fetch created attachment")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::Internal(anyhow!("attachment create succeeded but fetch failed")))?;

    Ok(Json(ApiResponse::success(UploadResponse {
        id: item.id.unwrap_or_default(),
        file_name: item.file_name.unwrap_or_default(),
        file_url: storage::public_url(&item.file_url.unwrap_or_default()),
        file_size: item.file_size.unwrap_or_default(),
        mime_type: item
            .mime_type
            .unwrap_or_else(|| "application/octet-stream".to_string()),
    })))
}

pub async fn list_task_attachments(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<i64>,
    Query(query): Query<FileListQuery>,
) -> Result<Json<ApiResponse<FileListResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    ensure_task_access(&state, &auth_user, task_id).await?;

    let items = db::select_by_map_compat::<AttachmentEntity>(
        state.rb.as_ref(),
        "attachment",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query attachments")
    .map_err(AppError::Internal)?
    .into_iter()
    .map(|item: AttachmentEntity| FileItem {
        id: item.id.unwrap_or_default(),
        file_name: item.file_name.unwrap_or_default(),
        file_url: storage::public_url(&item.file_url.unwrap_or_default()),
        file_size: item.file_size.unwrap_or_default(),
        mime_type: item
            .mime_type
            .unwrap_or_else(|| "application/octet-stream".to_string()),
    })
    .collect::<Vec<_>>();

    let total = items.len();
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).max(1);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let items = items.into_iter().skip(start).take(page_size).collect::<Vec<_>>();
    Ok(Json(ApiResponse::success(FileListResponse {
        items,
        total,
        page,
        page_size,
    })))
}

pub async fn upload_evidence_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<i64>,
    Query(query): Query<EvidenceUploadQuery>,
    multipart: Multipart,
) -> Result<Json<ApiResponse<UploadResponse>>, AppError> {
    upload_evidence(state, headers, task_id, query, multipart, "image").await
}

pub async fn upload_evidence_video(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<i64>,
    Query(query): Query<EvidenceUploadQuery>,
    multipart: Multipart,
) -> Result<Json<ApiResponse<UploadResponse>>, AppError> {
    upload_evidence(state, headers, task_id, query, multipart, "video").await
}

pub async fn list_task_evidences(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<i64>,
    Query(query): Query<FileListQuery>,
) -> Result<Json<ApiResponse<FileListResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    ensure_task_access(&state, &auth_user, task_id).await?;

    let items = db::select_by_map_compat::<EvidenceEntity>(
        state.rb.as_ref(),
        "evidence",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query evidences")
    .map_err(AppError::Internal)?
    .into_iter()
    .map(|item: EvidenceEntity| FileItem {
        id: item.id.unwrap_or_default(),
        file_name: item.file_name.unwrap_or_default(),
        file_url: storage::public_url(&item.file_url.unwrap_or_default()),
        file_size: item.file_size.unwrap_or_default(),
        mime_type: item
            .mime_type
            .unwrap_or_else(|| "application/octet-stream".to_string()),
    })
    .collect::<Vec<_>>();

    let total = items.len();
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).max(1);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let items = items.into_iter().skip(start).take(page_size).collect::<Vec<_>>();
    Ok(Json(ApiResponse::success(FileListResponse {
        items,
        total,
        page,
        page_size,
    })))
}

pub async fn delete_attachment(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(attachment_id): AxumPath<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let attachment = find_attachment(&state, attachment_id).await?;
    AttachmentEntity::delete_by_map(state.rb.as_ref(), value! { "id": attachment_id })
        .await
        .context("failed to delete attachment")
        .map_err(AppError::Internal)?;

    let path = storage::full_path(&state, attachment.file_url.as_deref().unwrap_or_default());
    let _ = fs::remove_file(path).await;

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn delete_evidence(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(evidence_id): AxumPath<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["technician", "dealer", "hq_reviewer", "super_admin"])?;

    let evidence = find_evidence(&state, evidence_id).await?;
    EvidenceEntity::delete_by_map(state.rb.as_ref(), value! { "id": evidence_id })
        .await
        .context("failed to delete evidence")
        .map_err(AppError::Internal)?;

    let path = storage::full_path(&state, evidence.file_url.as_deref().unwrap_or_default());
    let _ = fs::remove_file(path).await;

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn download_attachment(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(attachment_id): AxumPath<i64>,
) -> Result<Response, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    let attachment = find_attachment(&state, attachment_id).await?;
    ensure_task_access(&state, &auth_user, attachment.task_id.unwrap_or_default()).await?;

    let path = storage::full_path(&state, attachment.file_url.as_deref().unwrap_or_default());
    let bytes = fs::read(&path)
        .await
        .with_context(|| format!("failed to read attachment file: {}", path.display()))
        .map_err(AppError::Internal)?;

    let mime_type = attachment
        .mime_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let file_name = attachment
        .file_name
        .unwrap_or_else(|| "download.bin".to_string());

    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&mime_type)
            .map_err(|_| AppError::Internal(anyhow!("invalid content type header")))?,
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"{}\"",
            sanitize_filename(&file_name)
        ))
        .map_err(|_| AppError::Internal(anyhow!("invalid content disposition header")))?,
    );
    Ok(response)
}

pub async fn batch_download_attachments(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<BatchDownloadRequest>,
) -> Result<Response, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    if payload.attachment_ids.is_empty() && payload.task_ids.is_empty() {
        return Err(AppError::BadRequest(
            "attachment_ids和task_ids不能同时为空".to_string(),
        ));
    }

    let attachments = resolve_batch_download_attachments(&state, &auth_user, &payload).await?;
    if attachments.is_empty() {
        return Err(AppError::BadRequest("所选任务暂无附件".to_string()));
    }

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buffer);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let mut used_file_names = HashMap::<String, usize>::new();

        for attachment in attachments {
            let path =
                storage::full_path(&state, attachment.file_url.as_deref().unwrap_or_default());
            let bytes = fs::read(&path)
                .await
                .with_context(|| {
                    format!("failed to read attachment file for zip: {}", path.display())
                })
                .map_err(AppError::Internal)?;
            let base_file_name = attachment
                .file_name
                .clone()
                .unwrap_or_else(|| format!("attachment-{}.bin", attachment.id.unwrap_or_default()));
            let file_name = unique_zip_entry_name(
                &mut used_file_names,
                &base_file_name,
                attachment.task_id,
            );
            zip.start_file(file_name, options)
                .map_err(|_| AppError::Internal(anyhow!("failed to start zip entry")))?;
            zip.write_all(&bytes)
                .map_err(|_| AppError::Internal(anyhow!("failed to write zip entry")))?;
        }

        zip.finish()
            .map_err(|_| AppError::Internal(anyhow!("failed to finish zip archive")))?;
    }

    let mut response = Response::new(Body::from(buffer.into_inner()));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"{}\"",
            build_batch_attachment_filename(&payload)
        ))
        .map_err(|_| AppError::Internal(anyhow!("invalid content disposition header")))?,
    );
    Ok(response)
}

fn build_batch_attachment_filename(payload: &BatchDownloadRequest) -> String {
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    if !payload.task_ids.is_empty() {
        format!("task-attachments-{timestamp}.zip")
    } else {
        format!("attachments-{timestamp}.zip")
    }
}

async fn resolve_batch_download_attachments(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    payload: &BatchDownloadRequest,
) -> Result<Vec<AttachmentEntity>, AppError> {
    let mut attachment_map = std::collections::BTreeMap::new();

    for &attachment_id in &payload.attachment_ids {
        let attachment = find_attachment(state, attachment_id).await?;
        ensure_task_access(state, auth_user, attachment.task_id.unwrap_or_default()).await?;
        attachment_map.insert(attachment.id.unwrap_or_default(), attachment);
    }

    if !payload.task_ids.is_empty() {
        for &task_id in &payload.task_ids {
            ensure_task_access(state, auth_user, task_id).await?;
        }

        let attachments: Vec<AttachmentEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "attachment",
            value! { "task_id": payload.task_ids.clone() },
        )
        .await
        .context("failed to query batch download attachments by task ids")
        .map_err(AppError::Internal)?;

        for attachment in attachments {
            attachment_map.insert(attachment.id.unwrap_or_default(), attachment);
        }
    }

    let mut items = attachment_map.into_values().collect::<Vec<_>>();
    items.sort_by_key(|item| (item.task_id.unwrap_or_default(), item.id.unwrap_or_default()));
    Ok(items)
}

fn unique_zip_entry_name(
    used_file_names: &mut HashMap<String, usize>,
    original_name: &str,
    task_id: Option<i64>,
) -> String {
    let sanitized_original = sanitize_filename(original_name);
    let base_candidate = if let Some(task_id) = task_id {
        sanitize_filename(&format!("task-{task_id}_{sanitized_original}"))
    } else {
        sanitized_original
    };

    if !used_file_names.contains_key(&base_candidate) {
        used_file_names.insert(base_candidate.clone(), 1);
        return base_candidate;
    }

    let extension = Path::new(&base_candidate)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or("")
        .to_string();
    let stem = Path::new(&base_candidate)
        .file_stem()
        .and_then(|item| item.to_str())
        .unwrap_or("attachment")
        .to_string();
    let mut next_count = used_file_names.get(&base_candidate).copied().unwrap_or(1);

    loop {
        let suffix = next_count;
        next_count += 1;
        let candidate = if extension.is_empty() {
            format!("{stem}_{suffix}")
        } else {
            format!("{stem}_{suffix}.{extension}")
        };
        if !used_file_names.contains_key(&candidate) {
            used_file_names.insert(candidate.clone(), 1);
            used_file_names.insert(base_candidate.clone(), next_count);
            return candidate;
        }
    }
}

async fn upload_evidence(
    state: AppState,
    headers: HeaderMap,
    task_id: i64,
    query: EvidenceUploadQuery,
    multipart: Multipart,
    evidence_type: &str,
) -> Result<Json<ApiResponse<UploadResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["technician"])?;
    let task = find_task(&state, task_id).await?;
    if task.task_status.as_deref() != Some("executing") {
        return Err(AppError::Conflict("请先开始执行任务".to_string()));
    }
    let execution = resolve_evidence_execution(&state, &task, auth_user.user_id, &query).await?;
    let execution_id = execution
        .id
        .ok_or_else(|| AppError::Internal(anyhow!("execution.id missing")))?;

    let upload = read_single_multipart(multipart).await?;
    validate_upload(
        &upload.file_name,
        upload.content_type.as_deref(),
        evidence_type,
    )?;

    if evidence_type == "image" {
        let existing: Vec<EvidenceEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "evidence",
            value! { "task_id": task_id, "execution_id": execution_id, "evidence_type": "image" },
        )
        .await
        .context("failed to query existing image evidence")
        .map_err(AppError::Internal)?;
        if existing.len() >= 5 {
            return Err(AppError::Conflict("上传图片数量已达上限".to_string()));
        }
    }

    let sub_dir = if evidence_type == "image" {
        "evidences/images"
    } else {
        "evidences/videos"
    };
    let watermark_text = build_evidence_watermark_data(&state, &task).await?;
    let file_bytes = if evidence_type == "image" {
        match apply_visible_watermark(&upload.bytes, &watermark_text) {
            Ok(bytes) => bytes,
            Err(_) => upload.bytes.clone(),
        }
    } else {
        upload.bytes.clone()
    };
    let stored = storage::save_bytes(
        &state,
        sub_dir,
        &upload.file_name,
        upload.content_type.as_deref(),
        &file_bytes,
    )
    .await
    .map_err(AppError::Internal)?;

    EvidenceEntity::insert(
        state.rb.as_ref(),
        &EvidenceEntity {
            id: None,
            task_id: Some(task_id),
            execution_id: Some(execution_id),
            uploader_id: Some(auth_user.user_id),
            evidence_type: Some(evidence_type.to_string()),
            file_name: Some(stored.original_name.clone()),
            file_url: Some(stored.relative_path.clone()),
            file_size: Some(stored.file_size),
            mime_type: Some(stored.mime_type.clone()),
            watermark_text: Some(watermark_text.fallback_text),
            longitude: None,
            latitude: None,
            shot_at: Some(Local::now().naive_local()),
            created_at: None,
        },
    )
    .await
    .context("failed to create evidence record")
    .map_err(AppError::Internal)?;

    let item: EvidenceEntity = db::select_one_by_map_compat(
        state.rb.as_ref(),
        "evidence",
        value! { "task_id": task_id, "file_url": stored.relative_path.as_str() },
    )
    .await
    .context("failed to fetch created evidence")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::Internal(anyhow!("evidence create succeeded but fetch failed")))?;

    Ok(Json(ApiResponse::success(UploadResponse {
        id: item.id.unwrap_or_default(),
        file_name: item.file_name.unwrap_or_default(),
        file_url: storage::public_url(&item.file_url.unwrap_or_default()),
        file_size: item.file_size.unwrap_or_default(),
        mime_type: item
            .mime_type
            .unwrap_or_else(|| "application/octet-stream".to_string()),
    })))
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
        .context("failed to read multipart field")
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
        .context("failed to read upload bytes")
        .map_err(AppError::Internal)?
        .to_vec();

    Ok(UploadField {
        file_name,
        content_type,
        bytes,
    })
}

fn validate_upload(
    file_name: &str,
    content_type: Option<&str>,
    evidence_type: &str,
) -> Result<(), AppError> {
    let ext = Path::new(file_name)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let content_type = content_type.unwrap_or("");

    match evidence_type {
        "image" => {
            let ok = ["jpg", "jpeg", "png", "webp"].contains(&ext.as_str())
                || content_type.starts_with("image/");
            if !ok {
                return Err(AppError::BadRequest("当前文件格式不支持上传".to_string()));
            }
        }
        "video" => {
            let ok =
                ["mp4", "mov", "avi"].contains(&ext.as_str()) || content_type.starts_with("video/");
            if !ok {
                return Err(AppError::BadRequest("当前文件格式不支持上传".to_string()));
            }
        }
        _ => {}
    }
    Ok(())
}

struct EvidenceWatermarkData {
    info_lines: Vec<String>,
    time_text: String,
    fallback_text: String,
}

async fn build_evidence_watermark_data(
    state: &AppState,
    task: &TaskEntity,
) -> Result<EvidenceWatermarkData, AppError> {
    let store_name = if let Some(store_id) = task.store_id {
        db::select_one_by_map_compat::<StoreEntity>(
            state.rb.as_ref(),
            "store",
            value! { "id": store_id },
        )
        .await
        .context("failed to query store for evidence watermark")
        .map_err(AppError::Internal)?
        .and_then(|item| item.store_name)
        .unwrap_or_default()
    } else {
        String::new()
    };
    let service_address = if let Some(address_id) = task.customer_address_id {
        db::select_one_by_map_compat::<CustomerAddressEntity>(
            state.rb.as_ref(),
            "customer_address",
            value! { "id": address_id },
        )
        .await
        .context("failed to query customer address for evidence watermark")
        .map_err(AppError::Internal)?
        .and_then(|item| item.address)
        .unwrap_or_default()
    } else {
        String::new()
    };
    let (customer_name, customer_mobile) = if let Some(customer_id) = task.customer_id {
        let customer = db::select_one_by_map_compat::<CustomerEntity>(
            state.rb.as_ref(),
            "customer",
            value! { "id": customer_id },
        )
        .await
        .context("failed to query customer for evidence watermark")
        .map_err(AppError::Internal)?;
        (
            customer.as_ref().and_then(|item| item.name.clone()).unwrap_or_default(),
            customer.and_then(|item| item.mobile).unwrap_or_default(),
        )
    } else {
        (String::new(), String::new())
    };

    let service_type_text = match task.service_type.as_deref().unwrap_or_default() {
        "measure" => "量尺",
        "install" => "安装",
        "ironing" => "熨烫",
        "cleaning" => "除尘除螨",
        _ => "服务",
    };
    let masked_mobile = mask_mobile(&customer_mobile);
    let time_text = Local::now().format("%Y-%m-%d %H:%M").to_string();
    let info_lines = vec![
        if store_name.trim().is_empty() { "-".to_string() } else { store_name.trim().to_string() },
        if service_address.trim().is_empty() { "-".to_string() } else { service_address.trim().to_string() },
        format!(
            "客户: {}({})",
            if customer_name.trim().is_empty() { "-" } else { customer_name.trim() },
            masked_mobile
        ),
        format!("服务环节: {}", service_type_text),
    ];
    let fallback_text = format!(
        "门店:{} 地址:{} 客户:{}({}) 服务:{} 时间:{}",
        if store_name.trim().is_empty() { "-" } else { store_name.trim() },
        if service_address.trim().is_empty() { "-" } else { service_address.trim() },
        if customer_name.trim().is_empty() { "-" } else { customer_name.trim() },
        masked_mobile,
        service_type_text,
        time_text
    );

    Ok(EvidenceWatermarkData {
        info_lines,
        time_text,
        fallback_text,
    })
}

fn apply_visible_watermark(
    bytes: &[u8],
    watermark: &EvidenceWatermarkData,
) -> Result<Vec<u8>, anyhow::Error> {
    let format = image::guess_format(bytes).context("failed to guess image format")?;
    let mut image = image::load_from_memory(bytes)
        .context("failed to decode image for watermark")?
        .to_rgba8();
    if let Some(font) = load_watermark_font() {
        draw_chinese_watermark(&mut image, &font, watermark);
    } else {
        let (width, height) = image.dimensions();
        let scale = (width.max(height) / 320).max(1);
        let char_w = 8 * scale;
        let char_h = 8 * scale;
        let padding = 12 * scale;
        let line_height = char_h + 4 * scale;
        let max_chars = ((width.saturating_sub(padding * 2)) / char_w).max(1) as usize;
        let lines = wrap_text(&watermark.fallback_text, max_chars);
        let overlay_height = (lines.len() as u32 * line_height) + padding;
        let overlay_y = height.saturating_sub(overlay_height + padding / 2);

        for y in overlay_y..height {
            for x in 0..width {
                let pixel = image.get_pixel_mut(x, y);
                let alpha = 80u16;
                let bg = [0u8, 0u8, 0u8, 255u8];
                let blended = [
                    (((pixel[0] as u16 * (255 - alpha)) + (bg[0] as u16 * alpha)) / 255) as u8,
                    (((pixel[1] as u16 * (255 - alpha)) + (bg[1] as u16 * alpha)) / 255) as u8,
                    (((pixel[2] as u16 * (255 - alpha)) + (bg[2] as u16 * alpha)) / 255) as u8,
                    255,
                ];
                *pixel = Rgba(blended);
            }
        }

        for (index, line) in lines.iter().enumerate() {
            let y = overlay_y + padding / 2 + index as u32 * line_height;
            draw_text_bitmap(&mut image, padding, y, line, scale, Rgba([255, 255, 255, 255]));
        }
    }

    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image).write_to(&mut out, image_output_format(format))?;
    Ok(out.into_inner())
}

fn draw_chinese_watermark(
    image: &mut image::RgbaImage,
    font: &FontArc,
    watermark: &EvidenceWatermarkData,
) {
    let width = image.width();
    let scale_value = ((width as f32) / 48.0).clamp(18.0, 28.0);
    let text_scale = PxScale::from(scale_value);
    let time_scale = PxScale::from((scale_value - 2.0).max(16.0));
    let padding = 18i32;
    let line_gap = 8i32;
    let line_height = scale_value as i32 + line_gap;
    let box_width = ((width as f32) * 0.62) as u32;
    let box_height = (padding * 2 + watermark.info_lines.len() as i32 * line_height) as u32;

    draw_filled_rect_mut(
        image,
        Rect::at(padding, padding).of_size(box_width.max(260), box_height.max(120)),
        Rgba([0, 0, 0, 160]),
    );

    for (index, line) in watermark.info_lines.iter().enumerate() {
        draw_text_mut(
            image,
            Rgba([255, 255, 255, 255]),
            padding + 12,
            padding + 10 + index as i32 * line_height,
            text_scale,
            font,
            line,
        );
    }

    draw_text_mut(
        image,
        Rgba([255, 255, 255, 235]),
        width.saturating_sub((watermark.time_text.chars().count() as u32 * (scale_value as u32 / 2 + 9)) + 20) as i32,
        padding + 6,
        time_scale,
        font,
        &watermark.time_text,
    );
}

fn load_watermark_font() -> Option<FontArc> {
    let bytes = include_bytes!("../assets/fonts/simhei.ttf");
    FontArc::try_from_slice(bytes).ok()
}

fn mask_mobile(mobile: &str) -> String {
    let mobile = mobile.trim();
    if mobile.len() >= 7 {
        format!("{}****{}", &mobile[..3], &mobile[mobile.len().saturating_sub(4)..])
    } else if mobile.is_empty() {
        "-".to_string()
    } else {
        mobile.to_string()
    }
}

fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let chars = text.chars().collect::<Vec<_>>();
    chars
        .chunks(max_chars.max(1))
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
}

fn draw_text_bitmap(
    image: &mut image::RgbaImage,
    start_x: u32,
    start_y: u32,
    text: &str,
    scale: u32,
    color: Rgba<u8>,
) {
    let mut cursor_x = start_x;
    for ch in text.chars() {
        if let Some(glyph) = BASIC_FONTS.get(ch) {
            for (row, bits) in glyph.iter().enumerate() {
                for col in 0..8u32 {
                    if (bits >> col) & 1 == 1 {
                        for sy in 0..scale {
                            for sx in 0..scale {
                                let x = cursor_x + (7 - col) * scale + sx;
                                let y = start_y + row as u32 * scale + sy;
                                if x < image.width() && y < image.height() {
                                    image.put_pixel(x, y, color);
                                }
                            }
                        }
                    }
                }
            }
        }
        cursor_x += 8 * scale;
    }
}

fn image_output_format(format: ImageFormat) -> image::ImageFormat {
    match format {
        ImageFormat::Png => ImageFormat::Png,
        ImageFormat::Jpeg => ImageFormat::Jpeg,
        ImageFormat::WebP => ImageFormat::WebP,
        _ => ImageFormat::Png,
    }
}

async fn resolve_evidence_execution(
    state: &AppState,
    task: &TaskEntity,
    technician_id: i64,
    query: &EvidenceUploadQuery,
) -> Result<TaskExecutionEntity, AppError> {
    let task_id = task
        .id
        .ok_or_else(|| AppError::Internal(anyhow!("task.id missing")))?;
    let mut executions: Vec<TaskExecutionEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_execution",
        value! { "task_id": task_id, "technician_id": technician_id },
    )
    .await
    .context("failed to query technician executions for evidence")
    .map_err(AppError::Internal)?;

    if let Some(execution_id) = query.execution_id {
        let execution = executions
            .into_iter()
            .find(|item| item.id == Some(execution_id))
            .ok_or_else(|| AppError::Forbidden("执行记录与当前任务不匹配".to_string()))?;
        validate_evidence_execution(state, &execution, technician_id).await?;
        return Ok(execution);
    }

    if let Some(service_item_id) = query.service_item_id {
        executions.retain(|item| item.service_item_id == Some(service_item_id));
    }
    if let Some(service_type) = query
        .service_type
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        executions.retain(|item| item.service_type.as_deref() == Some(service_type));
    }
    executions.retain(|item| item.execution_status.as_deref() == Some("executing"));

    match executions.as_slice() {
        [execution] => {
            let execution = execution.clone();
            validate_evidence_execution(state, &execution, technician_id).await?;
            Ok(execution)
        }
        [] => Err(AppError::Conflict("任务尚未生成执行记录".to_string())),
        _ => Err(AppError::BadRequest(
            "当前任务存在多个执行环节，请传 execution_id 或 service_item_id".to_string(),
        )),
    }
}

async fn validate_evidence_execution(
    state: &AppState,
    execution: &TaskExecutionEntity,
    technician_id: i64,
) -> Result<(), AppError> {
    if execution.technician_id != Some(technician_id)
        || execution.execution_status.as_deref() != Some("executing")
    {
        return Err(AppError::Forbidden("执行记录与当前任务不匹配".to_string()));
    }
    if let Some(service_item_id) = execution.service_item_id {
        let service_item = db::select_one_by_map_compat::<TaskServiceItemEntity>(
            state.rb.as_ref(),
            "task_service_item",
            value! { "id": service_item_id },
        )
        .await
        .context("failed to query execution service item")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::BadRequest("服务环节不存在".to_string()))?;
        if service_item.technician_id != Some(technician_id)
            || service_item.service_status.as_deref() != Some("executing")
        {
            return Err(AppError::Forbidden(
                "您不是该服务环节当前执行技师".to_string(),
            ));
        }
    }
    Ok(())
}

async fn ensure_task_access(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    task_id: i64,
) -> Result<(), AppError> {
    let task = find_task(state, task_id).await?;
    if auth_user.has_role("hq_reviewer") || auth_user.has_role("super_admin") {
        return Ok(());
    }
    if auth_user.has_role("technician") && task.current_technician_id == Some(auth_user.user_id) {
        return Ok(());
    }
    if auth_user.has_role("technician") {
        let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "task_service_item",
            value! { "task_id": task_id, "technician_id": auth_user.user_id },
        )
        .await
        .context("failed to query technician task service item access")
        .map_err(AppError::Internal)?;
        if !service_items.is_empty() {
            return Ok(());
        }
    }
    if auth_user.has_role("dealer") {
        let relations: Vec<UserStoreEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "user_store",
            value! { "user_id": auth_user.user_id },
        )
        .await
        .context("failed to query dealer store relations")
        .map_err(AppError::Internal)?;
        if relations
            .into_iter()
            .any(|item| item.store_id == task.store_id)
        {
            return Ok(());
        }
    }
    Err(AppError::Forbidden(
        "您只能操作当前权限范围内的数据".to_string(),
    ))
}

async fn find_task(state: &AppState, task_id: i64) -> Result<TaskEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "task", value! { "id": task_id })
        .await
        .context("failed to query task")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("任务不存在或已删除".to_string()))
}

async fn find_attachment(
    state: &AppState,
    attachment_id: i64,
) -> Result<AttachmentEntity, AppError> {
    db::select_one_by_map_compat(
        state.rb.as_ref(),
        "attachment",
        value! { "id": attachment_id },
    )
    .await
    .context("failed to query attachment")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::NotFound("文件不存在或已失效".to_string()))
}

async fn find_evidence(state: &AppState, evidence_id: i64) -> Result<EvidenceEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "evidence", value! { "id": evidence_id })
        .await
        .context("failed to query evidence")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("文件不存在或已失效".to_string()))
}

fn sanitize_filename(file_name: &str) -> String {
    file_name.replace(['\r', '\n', '"'], "_")
}
