use std::{io::Write, path::Path};

use anyhow::{Context, anyhow};
use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use chrono::Utc;
use rbs::value;
use serde::Serialize;
use tokio::fs;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::{
    auth, db,
    error::AppError,
    models::{
        attachment::AttachmentEntity, evidence::EvidenceEntity, task::TaskEntity,
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
    pub attachment_ids: Vec<i64>,
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
    Ok(Json(ApiResponse::success(FileListResponse {
        items,
        total,
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
    Ok(Json(ApiResponse::success(FileListResponse {
        items,
        total,
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
    if payload.attachment_ids.is_empty() {
        return Err(AppError::BadRequest("attachment_ids不能为空".to_string()));
    }

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buffer);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

        for attachment_id in payload.attachment_ids {
            let attachment = find_attachment(&state, attachment_id).await?;
            ensure_task_access(&state, &auth_user, attachment.task_id.unwrap_or_default()).await?;
            let path =
                storage::full_path(&state, attachment.file_url.as_deref().unwrap_or_default());
            let bytes = fs::read(&path)
                .await
                .with_context(|| {
                    format!("failed to read attachment file for zip: {}", path.display())
                })
                .map_err(AppError::Internal)?;
            let file_name = sanitize_filename(
                &attachment
                    .file_name
                    .unwrap_or_else(|| format!("{}.bin", attachment_id)),
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
        HeaderValue::from_static("attachment; filename=\"attachments-batch.zip\""),
    );
    Ok(response)
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
    let stored = storage::save_bytes(
        &state,
        sub_dir,
        &upload.file_name,
        upload.content_type.as_deref(),
        &upload.bytes,
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
            watermark_text: Some(format!("task:{} time:{}", task_id, Utc::now().timestamp())),
            longitude: None,
            latitude: None,
            shot_at: Some(Utc::now().naive_utc()),
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
