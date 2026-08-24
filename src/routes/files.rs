use std::{collections::HashMap, io::Write, path::Path, str::FromStr};

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
use font8x8::{BASIC_FONTS, UnicodeFonts};
use image::{ImageFormat, Rgba};
use imageproc::{
    drawing::{draw_filled_rect_mut, draw_text_mut, text_size},
    rect::Rect,
};
use rbs::value;
use serde::Serialize;
use tokio::fs;
use tokio::process::Command;
use uuid::Uuid;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::{
    auth, db,
    error::AppError,
    models::{
        attachment::AttachmentEntity, customer::CustomerEntity,
        customer_address::CustomerAddressEntity, evidence::EvidenceEntity, store::StoreEntity,
        task::TaskEntity, task_execution::TaskExecutionEntity,
        task_service_item::TaskServiceItemEntity, user_store::UserStoreEntity,
    },
    response::ApiResponse,
    state::AppState,
    storage,
};

const MAX_EVIDENCE_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const WATERMARK_BOX_WIDTH_RATIO: f32 = 0.33;
const MAX_EVIDENCE_VIDEO_BYTES: usize = 50 * 1024 * 1024;
const MAX_EVIDENCE_IMAGES_PER_EXECUTION: usize = 10;
const LOCATION_ADDRESS_FIELD_KEYS: &[&str] = &[
    "location_address",
    "locationAddress",
    "location_text",
    "locationText",
    "current_address",
    "currentAddress",
    "formatted_address",
    "formattedAddress",
    "address",
];

#[derive(Debug, Serialize)]
pub struct FileItem {
    pub id: i64,
    pub source: String,
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
    #[serde(
        alias = "locationAddress",
        alias = "location_text",
        alias = "locationText",
        alias = "current_address",
        alias = "currentAddress",
        alias = "formatted_address",
        alias = "formattedAddress",
        alias = "address"
    )]
    pub location_address: Option<String>,
}

pub async fn upload_task_attachment(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<i64>,
    Query(query): Query<EvidenceUploadQuery>,
    multipart: Multipart,
) -> Result<Json<ApiResponse<UploadResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    let task = find_task(&state, task_id).await?;

    let upload = read_single_multipart(multipart).await?;
    let file_bytes = if upload
        .content_type
        .as_deref()
        .map(|item| item.starts_with("image/"))
        .unwrap_or(false)
        || Path::new(&upload.file_name)
            .extension()
            .and_then(|item| item.to_str())
            .map(|item| {
                ["jpg", "jpeg", "png", "webp"].contains(&item.to_ascii_lowercase().as_str())
            })
            .unwrap_or(false)
    {
        let location_address = query
            .location_address
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .or_else(|| parse_optional_text_field(&upload.fields, LOCATION_ADDRESS_FIELD_KEYS))
            .unwrap_or_default();
        let location_context = EvidenceLocationContext {
            location_address,
            customer_address: load_task_customer_address(&state, &task).await?,
        };
        let watermark_text =
            build_evidence_watermark_data(&state, &task, &location_context).await?;
        apply_visible_watermark(&upload.bytes, &watermark_text)
            .unwrap_or_else(|_| upload.bytes.clone())
    } else {
        upload.bytes.clone()
    };
    let stored = storage::save_bytes(
        &state,
        "attachments",
        &upload.file_name,
        upload.content_type.as_deref(),
        &file_bytes,
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

    let mut items = db::select_by_map_compat::<AttachmentEntity>(
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
        source: "attachment".to_string(),
        file_name: item.file_name.unwrap_or_default(),
        file_url: storage::public_url(&item.file_url.unwrap_or_default()),
        file_size: item.file_size.unwrap_or_default(),
        mime_type: item
            .mime_type
            .unwrap_or_else(|| "application/octet-stream".to_string()),
    })
    .collect::<Vec<_>>();

    let evidence_items = db::select_by_map_compat::<EvidenceEntity>(
        state.rb.as_ref(),
        "evidence",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query attachment-compatible evidences")
    .map_err(AppError::Internal)?
    .into_iter()
    .map(|item: EvidenceEntity| FileItem {
        id: item.id.unwrap_or_default(),
        source: "evidence".to_string(),
        file_name: item.file_name.unwrap_or_default(),
        file_url: storage::public_url(&item.file_url.unwrap_or_default()),
        file_size: item.file_size.unwrap_or_default(),
        mime_type: item
            .mime_type
            .unwrap_or_else(|| "application/octet-stream".to_string()),
    })
    .collect::<Vec<_>>();
    items.extend(evidence_items);
    items.sort_by(|left, right| right.id.cmp(&left.id));

    let total = items.len();
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).max(1);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let items = items
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();
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
        source: "evidence".to_string(),
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
    let items = items
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();
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

    let _ = storage::delete_file(&state, attachment.file_url.as_deref().unwrap_or_default()).await;

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

    let _ = storage::delete_file(&state, evidence.file_url.as_deref().unwrap_or_default()).await;

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

    let bytes = storage::read_bytes(&state, attachment.file_url.as_deref().unwrap_or_default())
        .await
        .context("failed to read attachment file")
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
        return Err(AppError::BadRequest(
            "所选任务暂无附件或现场图片".to_string(),
        ));
    }

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buffer);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let mut used_file_names = HashMap::<String, usize>::new();

        for attachment in attachments {
            let bytes = storage::read_bytes(&state, &attachment.file_url)
                .await
                .context("failed to read attachment file for zip")
                .map_err(AppError::Internal)?;
            let base_file_name = if attachment.file_name.trim().is_empty() {
                format!("{}-{}.bin", attachment.source, attachment.id)
            } else {
                attachment.file_name.clone()
            };
            let file_name =
                unique_zip_entry_name(&mut used_file_names, &base_file_name, attachment.task_id);
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
) -> Result<Vec<BatchDownloadFile>, AppError> {
    let mut attachment_map = std::collections::BTreeMap::<(String, i64), BatchDownloadFile>::new();

    for &attachment_id in &payload.attachment_ids {
        let attachment = find_attachment(state, attachment_id).await?;
        ensure_task_access(state, auth_user, attachment.task_id.unwrap_or_default()).await?;
        let id = attachment.id.unwrap_or_default();
        attachment_map.insert(
            ("attachment".to_string(), id),
            BatchDownloadFile {
                id,
                source: "attachment",
                task_id: attachment.task_id,
                file_name: attachment.file_name.unwrap_or_default(),
                file_url: attachment.file_url.unwrap_or_default(),
            },
        );
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
            let id = attachment.id.unwrap_or_default();
            attachment_map.insert(
                ("attachment".to_string(), id),
                BatchDownloadFile {
                    id,
                    source: "attachment",
                    task_id: attachment.task_id,
                    file_name: attachment.file_name.unwrap_or_default(),
                    file_url: attachment.file_url.unwrap_or_default(),
                },
            );
        }

        let evidences: Vec<EvidenceEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "evidence",
            value! { "task_id": payload.task_ids.clone() },
        )
        .await
        .context("failed to query batch download evidences by task ids")
        .map_err(AppError::Internal)?;

        for evidence in evidences {
            let id = evidence.id.unwrap_or_default();
            attachment_map.insert(
                ("evidence".to_string(), id),
                BatchDownloadFile {
                    id,
                    source: "evidence",
                    task_id: evidence.task_id,
                    file_name: evidence.file_name.unwrap_or_default(),
                    file_url: evidence.file_url.unwrap_or_default(),
                },
            );
        }
    }

    let mut items = attachment_map.into_values().collect::<Vec<_>>();
    items.sort_by_key(|item| (item.task_id.unwrap_or_default(), item.source, item.id));
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
    let upload = if evidence_type == "video" {
        read_single_multipart_with_limit_message(multipart, "现场视频大小不能超过50MB").await?
    } else {
        read_single_multipart(multipart).await?
    };
    let query = merge_evidence_upload_query(query, &upload.fields)?;
    let execution = resolve_evidence_execution(&state, &task, auth_user.user_id, &query).await?;
    let execution_id = execution
        .id
        .ok_or_else(|| AppError::Internal(anyhow!("execution.id missing")))?;

    validate_upload(
        &upload.file_name,
        upload.content_type.as_deref(),
        upload.bytes.len(),
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
        if existing.len() >= MAX_EVIDENCE_IMAGES_PER_EXECUTION {
            return Err(AppError::Conflict("上传图片数量已达上限".to_string()));
        }
    }
    let sub_dir = if evidence_type == "image" {
        "evidences/images"
    } else {
        "evidences/videos"
    };
    let location_context = load_evidence_location_context(&state, &task, &query).await?;
    let technician_location_address = normalize_optional_location_address(
        location_context.location_address.as_str(),
        location_context.customer_address.as_str(),
    );
    let watermark = if evidence_type == "image" {
        Some(build_evidence_watermark_data(&state, &task, &location_context).await?)
    } else {
        None
    };
    let file_bytes = if let Some(watermark) = watermark.as_ref() {
        match apply_visible_watermark(&upload.bytes, watermark) {
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
            watermark_text: watermark.map(|item| item.fallback_text),
            location_address: technician_location_address,
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

fn merge_evidence_upload_query(
    mut query: EvidenceUploadQuery,
    fields: &HashMap<String, String>,
) -> Result<EvidenceUploadQuery, AppError> {
    if query.execution_id.is_none() {
        query.execution_id = parse_optional_i64_field(fields, &["execution_id", "executionId"])?;
    }
    if query.service_item_id.is_none() {
        query.service_item_id =
            parse_optional_i64_field(fields, &["service_item_id", "serviceItemId"])?;
    }
    if query.service_type.is_none() {
        query.service_type = parse_optional_text_field(fields, &["service_type", "serviceType"]);
    }
    if query.location_address.is_none() {
        query.location_address = parse_optional_text_field(fields, LOCATION_ADDRESS_FIELD_KEYS);
    }
    Ok(query)
}

fn parse_optional_i64_field(
    fields: &HashMap<String, String>,
    keys: &[&str],
) -> Result<Option<i64>, AppError> {
    let Some(raw) = keys.iter().find_map(|key| fields.get(*key)) else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    i64::from_str(raw)
        .map(Some)
        .map_err(|_| AppError::BadRequest(format!("{}鏍煎紡閿欒", keys[0])))
}

fn parse_optional_text_field(fields: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| fields.get(*key))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Debug)]
struct UploadField {
    file_name: String,
    content_type: Option<String>,
    bytes: Vec<u8>,
    fields: HashMap<String, String>,
}

struct BatchDownloadFile {
    id: i64,
    source: &'static str,
    task_id: Option<i64>,
    file_name: String,
    file_url: String,
}

async fn read_single_multipart(multipart: Multipart) -> Result<UploadField, AppError> {
    read_single_multipart_with_limit_message(multipart, "上传文件大小不能超过10MB").await
}

async fn read_single_multipart_with_limit_message(
    mut multipart: Multipart,
    limit_message: &'static str,
) -> Result<UploadField, AppError> {
    let mut fields = HashMap::new();
    let mut file_name = None;
    let mut content_type = None;
    let mut bytes = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| AppError::from_multipart_error_with_limit_message(err, limit_message))?
    {
        let name = field.name().map(ToString::to_string).unwrap_or_default();
        if let Some(current_file_name) = field.file_name().map(ToString::to_string) {
            content_type = field.content_type().map(ToString::to_string);
            file_name = Some(current_file_name);
            bytes = Some(
                field
                    .bytes()
                    .await
                    .map_err(|err| {
                        AppError::from_multipart_error_with_limit_message(err, limit_message)
                    })?
                    .to_vec(),
            );
        } else {
            let text = field.text().await.map_err(|err| {
                AppError::from_multipart_error_with_limit_message(err, limit_message)
            })?;
            if !name.is_empty() {
                fields.insert(name, text);
            }
        }
    }

    Ok(UploadField {
        file_name: file_name.ok_or_else(|| AppError::BadRequest("请上传文件".to_string()))?,
        content_type,
        bytes: bytes.ok_or_else(|| AppError::BadRequest("请上传文件".to_string()))?,
        fields,
    })
}

fn validate_upload(
    file_name: &str,
    content_type: Option<&str>,
    file_size: usize,
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
            if file_size > MAX_EVIDENCE_IMAGE_BYTES {
                return Err(AppError::BadRequest("现场图片大小不能超过10MB".to_string()));
            }
        }
        "video" => {
            let ok =
                ["mp4", "mov", "avi"].contains(&ext.as_str()) || content_type.starts_with("video/");
            if !ok {
                return Err(AppError::BadRequest("当前文件格式不支持上传".to_string()));
            }
            if file_size > MAX_EVIDENCE_VIDEO_BYTES {
                return Err(AppError::BadRequest("现场视频大小不能超过50MB".to_string()));
            }
        }
        _ => {}
    }
    Ok(())
}

async fn apply_video_watermark(
    bytes: &[u8],
    file_name: &str,
    watermark: &EvidenceWatermarkData,
) -> Result<Vec<u8>, AppError> {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or("mp4");
    let temp_dir = std::env::temp_dir().join(format!("kgj-video-watermark-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir)
        .context("failed to create temp dir for video watermark")
        .map_err(AppError::Internal)?;

    let input_path = temp_dir.join(format!("input.{extension}"));
    let output_path = temp_dir.join(format!("output.{extension}"));
    let font_path = temp_dir.join("simhei.ttf");
    let text_path = temp_dir.join("watermark.txt");

    let watermark_text = watermark.info_lines.join("\n");
    fs::write(&input_path, bytes)
        .await
        .context("failed to write temp input video")
        .map_err(AppError::Internal)?;
    fs::write(&font_path, include_bytes!("../assets/fonts/simhei.ttf"))
        .await
        .context("failed to write temp watermark font")
        .map_err(AppError::Internal)?;
    fs::write(&text_path, watermark_text.as_bytes())
        .await
        .context("failed to write temp watermark text")
        .map_err(AppError::Internal)?;

    let drawtext_filter = format!(
        "drawtext=fontfile='{}':textfile='{}':reload=0:fontcolor=black:fontsize=24:line_spacing=8:box=1:boxcolor=0xB8B8B8@0.95:boxborderw=12:x=20:y=h-th-20",
        escape_ffmpeg_filter_path(&font_path),
        escape_ffmpeg_filter_path(&text_path),
    );

    let ffmpeg_bin = resolve_ffmpeg_binary_path();
    let output = Command::new(&ffmpeg_bin)
        .arg("-y")
        .arg("-i")
        .arg(&input_path)
        .arg("-vf")
        .arg(drawtext_filter)
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("veryfast")
        .arg("-crf")
        .arg("23")
        .arg("-c:a")
        .arg("copy")
        .arg(&output_path)
        .output()
        .await
        .map_err(|_| AppError::Internal(anyhow!(
            "服务端未找到可用的ffmpeg，优先检查FFMPEG_PATH，其次确认程序同目录是否存在ffmpeg.exe"
        )))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = fs::remove_file(&input_path).await;
        let _ = fs::remove_file(&output_path).await;
        let _ = fs::remove_file(&font_path).await;
        let _ = fs::remove_file(&text_path).await;
        let _ = fs::remove_dir(&temp_dir).await;
        return Err(AppError::Internal(anyhow!(
            "瑙嗛姘村嵃澶勭悊澶辫触: {}",
            stderr.trim()
        )));
    }

    let watermarked_bytes = fs::read(&output_path)
        .await
        .context("failed to read watermarked video output")
        .map_err(AppError::Internal)?;

    let _ = fs::remove_file(&input_path).await;
    let _ = fs::remove_file(&output_path).await;
    let _ = fs::remove_file(&font_path).await;
    let _ = fs::remove_file(&text_path).await;
    let _ = fs::remove_dir(&temp_dir).await;

    Ok(watermarked_bytes)
}

fn escape_ffmpeg_filter_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}

fn resolve_ffmpeg_binary_path() -> String {
    if let Ok(path) = std::env::var("FFMPEG_PATH") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if let Ok(exe_path) = std::env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
        let bundled = exe_dir.join("ffmpeg.exe");
        if bundled.exists() {
            return bundled.to_string_lossy().to_string();
        }
    }

    "ffmpeg".to_string()
}

fn normalize_optional_location_address(
    location_address: &str,
    customer_address: &str,
) -> Option<String> {
    let location_address = location_address.trim();
    if location_address.is_empty() || location_address == customer_address.trim() {
        None
    } else {
        Some(location_address.to_string())
    }
}

pub async fn refresh_execution_evidence_watermarks(
    state: &AppState,
    task: &TaskEntity,
    execution_id: i64,
    location_address: &str,
) -> Result<(), AppError> {
    let location_address = location_address.trim();
    if location_address.is_empty() {
        return Ok(());
    }

    let evidences: Vec<EvidenceEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "evidence",
        value! { "task_id": task.id.unwrap_or_default(), "execution_id": execution_id },
    )
    .await
    .context("failed to query evidence for watermark refresh")
    .map_err(AppError::Internal)?;

    if evidences.is_empty() {
        return Ok(());
    }

    let location_context = EvidenceLocationContext {
        location_address: location_address.to_string(),
        customer_address: load_task_customer_address(state, task).await?,
    };
    let watermark = build_evidence_watermark_data(state, task, &location_context).await?;

    for evidence in evidences {
        let evidence_id = evidence.id.unwrap_or_default();
        let evidence_type = evidence.evidence_type.as_deref().unwrap_or_default();
        if evidence_type == "video" {
            EvidenceEntity::update_by_map(
                state.rb.as_ref(),
                &EvidenceEntity {
                    location_address: Some(location_address.to_string()),
                    ..Default::default()
                },
                value! { "id": evidence_id, "column": ["location_address"] },
            )
            .await
            .context("failed to update video evidence location")
            .map_err(AppError::Internal)?;
            continue;
        }
        if evidence_type != "image" {
            continue;
        }
        if evidence
            .watermark_text
            .as_deref()
            .map(|text| text.contains(location_address))
            .unwrap_or(false)
        {
            continue;
        }

        let Some(relative_path) = evidence.file_url.as_deref() else {
            continue;
        };
        let original_bytes = storage::read_bytes(state, relative_path)
            .await
            .context("failed to read evidence for watermark refresh")
            .map_err(AppError::Internal)?;
        let refreshed_bytes =
            apply_visible_watermark(&original_bytes, &watermark).map_err(AppError::Internal)?;

        storage::replace_bytes(
            state,
            relative_path,
            evidence.mime_type.as_deref(),
            &refreshed_bytes,
        )
        .await
        .context("failed to write refreshed evidence watermark")
        .map_err(AppError::Internal)?;
        EvidenceEntity::update_by_map(
            state.rb.as_ref(),
            &EvidenceEntity {
                watermark_text: Some(watermark.fallback_text.clone()),
                location_address: Some(location_address.to_string()),
                ..Default::default()
            },
            value! { "id": evidence_id, "column": ["watermark_text", "location_address"] },
        )
        .await
        .context("failed to update refreshed evidence watermark text")
        .map_err(AppError::Internal)?;
    }

    Ok(())
}

struct EvidenceWatermarkData {
    info_lines: Vec<String>,
    fallback_text: String,
}

struct EvidenceLocationContext {
    location_address: String,
    customer_address: String,
}

async fn build_evidence_watermark_data(
    state: &AppState,
    task: &TaskEntity,
    location_context: &EvidenceLocationContext,
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
    let service_address = if location_context.location_address.trim().is_empty() {
        location_context.customer_address.clone()
    } else {
        location_context.location_address.clone()
    };
    let customer_address = location_context.customer_address.clone();
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
            customer
                .as_ref()
                .and_then(|item| item.name.clone())
                .unwrap_or_default(),
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
    let display_mobile = if customer_mobile.trim().is_empty() {
        "-".to_string()
    } else {
        customer_mobile.trim().to_string()
    };
    let time_text = Local::now().format("%Y-%m-%d %H:%M").to_string();
    let display_name = if customer_name.trim().is_empty() {
        "-"
    } else {
        customer_name.trim()
    };
    let info_lines = vec![
        if store_name.trim().is_empty() {
            "-".to_string()
        } else {
            store_name.trim().to_string()
        },
        if service_address.trim().is_empty() {
            "-".to_string()
        } else {
            service_address.trim().to_string()
        },
        format!("手机号: {}", display_mobile),
        format!("姓名: {}", display_name),
        format!("服务环节: {}", service_type_text),
        format!("时间: {}", time_text),
    ];
    let fallback_text = format!(
        "门店:{} 地址:{} 客户地址:{} 客户:{}({}) 服务:{} 时间:{}",
        if store_name.trim().is_empty() {
            "-"
        } else {
            store_name.trim()
        },
        if service_address.trim().is_empty() {
            "-"
        } else {
            service_address.trim()
        },
        if customer_address.trim().is_empty() {
            "-"
        } else {
            customer_address.trim()
        },
        display_name,
        display_mobile,
        service_type_text,
        time_text
    );

    Ok(EvidenceWatermarkData {
        info_lines,
        fallback_text,
    })
}

async fn load_evidence_location_context(
    state: &AppState,
    task: &TaskEntity,
    query: &EvidenceUploadQuery,
) -> Result<EvidenceLocationContext, AppError> {
    let customer_address = load_task_customer_address(state, task).await?;
    Ok(EvidenceLocationContext {
        location_address: query
            .location_address
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .unwrap_or_default(),
        customer_address,
    })
}

async fn load_task_customer_address(
    state: &AppState,
    task: &TaskEntity,
) -> Result<String, AppError> {
    let Some(address_id) = task.customer_address_id else {
        return Ok(String::new());
    };
    Ok(db::select_one_by_map_compat::<CustomerAddressEntity>(
        state.rb.as_ref(),
        "customer_address",
        value! { "id": address_id },
    )
    .await
    .context("failed to query customer address for evidence watermark")
    .map_err(AppError::Internal)?
    .and_then(|item| item.address)
    .map(|item| item.trim().to_string())
    .filter(|item| !item.is_empty())
    .unwrap_or_default())
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
        let box_width = watermark_box_width(width, padding);
        let max_chars = ((box_width.saturating_sub(padding)) / char_w).max(1) as usize;
        let lines = wrap_text(&watermark.fallback_text, max_chars);
        let overlay_height = (lines.len() as u32 * line_height) + padding * 2;
        let overlay_x = padding;
        let overlay_y = height.saturating_sub(overlay_height + padding);
        draw_filled_rect_mut(
            &mut image,
            Rect::at(overlay_x as i32, overlay_y as i32).of_size(box_width, overlay_height),
            Rgba([184, 184, 184, 242]),
        );
        for (index, line) in lines.iter().enumerate() {
            let y = overlay_y + padding / 2 + index as u32 * line_height;
            draw_text_bitmap(
                &mut image,
                overlay_x + padding / 2,
                y,
                line,
                scale,
                Rgba([0, 0, 0, 255]),
            );
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
    let height = image.height();
    let scale_value = ((width as f32) / 48.0).clamp(18.0, 28.0);
    let padding = 18i32;
    let line_gap = 8i32;
    let line_height = scale_value as i32 + line_gap;
    let box_width = watermark_box_width(width, padding as u32);
    let text_scale_value = fit_watermark_text_scale(
        &watermark.info_lines,
        font,
        scale_value,
        box_width.saturating_sub((padding * 2) as u32 + 24) as f32,
    );
    let text_scale = PxScale::from(text_scale_value);
    let box_height = (padding * 2 + watermark.info_lines.len() as i32 * line_height) as u32;
    let box_x = padding;
    let box_y = height.saturating_sub(box_height + padding as u32) as i32;

    draw_filled_rect_mut(
        image,
        Rect::at(box_x, box_y).of_size(box_width, box_height.max(120)),
        Rgba([184, 184, 184, 242]),
    );

    for (index, line) in watermark.info_lines.iter().enumerate() {
        draw_text_mut(
            image,
            Rgba([0, 0, 0, 255]),
            box_x + 12,
            box_y + 10 + index as i32 * line_height,
            text_scale,
            font,
            line,
        );
    }
}

fn watermark_box_width(width: u32, padding: u32) -> u32 {
    (((width as f32) * WATERMARK_BOX_WIDTH_RATIO) as u32)
        .max(160)
        .min(width.saturating_sub(padding.saturating_mul(2)))
}

fn fit_watermark_text_scale(
    lines: &[String],
    font: &FontArc,
    initial_scale: f32,
    available_width: f32,
) -> f32 {
    let initial = PxScale::from(initial_scale);
    let widest = lines
        .iter()
        .map(|line| text_size(initial, font, line).0 as f32)
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or_default();
    if widest <= available_width || widest <= 0.0 {
        return initial_scale;
    }
    (initial_scale * available_width / widest).max(12.0)
}

fn load_watermark_font() -> Option<FontArc> {
    let bytes = include_bytes!("../assets/fonts/simhei.ttf");
    FontArc::try_from_slice(bytes).ok()
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
    let task: TaskEntity =
        db::select_one_by_map_compat(state.rb.as_ref(), "task", value! { "id": task_id })
            .await
            .context("failed to query task")
            .map_err(AppError::Internal)?
            .ok_or_else(|| AppError::NotFound("任务不存在或已删除".to_string()))?;
    if task.deleted_at.is_some() {
        return Err(AppError::NotFound("任务不存在或已删除".to_string()));
    }
    Ok(task)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{AppConfig, AppMeta, AuthConfig, DatabaseConfig, ServerConfig, StorageConfig},
        models::task::TaskEntity,
    };
    use rbatis::RBatis;

    fn test_state() -> AppState {
        AppState::new(
            AppConfig {
                app: AppMeta {
                    name: "kgj-backend-test".to_string(),
                    version: "test".to_string(),
                },
                server: ServerConfig {
                    host: "127.0.0.1".to_string(),
                    port: 0,
                },
                database: DatabaseConfig { url: String::new() },
                auth: AuthConfig {
                    jwt_secret: "test".to_string(),
                    access_token_expire_minutes: 1,
                    refresh_token_expire_days: 1,
                },
                storage: StorageConfig {
                    backend: "local".to_string(),
                    upload_dir: "uploads".to_string(),
                    oss_endpoint: String::new(),
                    oss_bucket: String::new(),
                    oss_access_key_id: String::new(),
                    oss_access_key_secret: String::new(),
                },
            },
            RBatis::new(),
        )
    }

    fn test_task() -> TaskEntity {
        TaskEntity {
            service_type: Some("cleaning".to_string()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn photo_watermark_uses_current_location_and_falls_back_for_blank() {
        let state = test_state();
        let task = test_task();
        let current_address = "临颍县荣耀授权专卖店(漯河临颍金地时代广场店)";
        let customer_address = "客户地址测试";

        let current = build_evidence_watermark_data(
            &state,
            &task,
            &EvidenceLocationContext {
                location_address: current_address.to_string(),
                customer_address: customer_address.to_string(),
            },
        )
        .await
        .expect("current location watermark should build");
        assert_eq!(current.info_lines[1], current_address);
        assert_eq!(
            current
                .info_lines
                .last()
                .map(|line| line.starts_with("时间: ")),
            Some(true)
        );
        assert!(current.fallback_text.contains(current_address));

        let blank = build_evidence_watermark_data(
            &state,
            &task,
            &EvidenceLocationContext {
                location_address: "   ".to_string(),
                customer_address: customer_address.to_string(),
            },
        )
        .await
        .expect("blank location watermark should build");
        assert_eq!(blank.info_lines[1], customer_address);

        let image = image::RgbaImage::from_pixel(640, 480, Rgba([240, 240, 240, 255]));
        let mut source = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut source, ImageFormat::Png)
            .expect("test image should encode");
        let output = apply_visible_watermark(&source.into_inner(), &current)
            .expect("photo watermark should render");
        let output_image = image::load_from_memory(&output)
            .expect("watermarked image should decode")
            .to_rgba8();
        assert_eq!(output_image.get_pixel(30, 30), &Rgba([240, 240, 240, 255]));
        assert_ne!(
            output_image.get_pixel(30, output_image.height().saturating_sub(40)),
            &Rgba([240, 240, 240, 255])
        );
    }

    #[test]
    fn photo_watermark_box_uses_one_third_of_the_image_width() {
        assert_eq!(watermark_box_width(1449, 18), 478);
        assert_eq!(watermark_box_width(640, 18), 211);

        let mut image = image::RgbaImage::from_pixel(1449, 760, Rgba([240, 240, 240, 255]));
        let font = load_watermark_font().expect("watermark font should load");
        let watermark = EvidenceWatermarkData {
            info_lines: vec![
                "location".to_string(),
                "address".to_string(),
                "mobile".to_string(),
                "name".to_string(),
                "service".to_string(),
                "time".to_string(),
            ],
            fallback_text: String::new(),
        };
        draw_chinese_watermark(&mut image, &font, &watermark);

        assert_ne!(image.get_pixel(490, 735), &Rgba([240, 240, 240, 255]));
        assert_eq!(image.get_pixel(520, 735), &Rgba([240, 240, 240, 255]));
    }
}
