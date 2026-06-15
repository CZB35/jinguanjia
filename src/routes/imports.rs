use std::{collections::HashMap, io::Write, path::Path};

use anyhow::{Context, anyhow};
use axum::{
    Json,
    body::Body,
    extract::{Multipart, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use bcrypt::hash;
use calamine::{Reader, open_workbook_auto};
use chrono::Utc;
use rbs::value;
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use crate::{
    audit, auth, db,
    error::AppError,
    models::{
        hq_score::HqScoreEntity, hq_score_item::HqScoreItemEntity, role::RoleEntity,
        task::TaskEntity, user::UserEntity, user_role::UserRoleEntity, user_store::UserStoreEntity,
    },
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct BatchDeleteUsersRequest {
    #[serde(
        default,
        alias = "ids",
        deserialize_with = "deserialize_vec_i64_from_mixed"
    )]
    pub user_ids: Vec<i64>,
}

#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub success_count: usize,
    pub failed_count: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImportTemplateQuery {
    pub role_code: Option<String>,
}

#[derive(Debug)]
struct ImportedUserRow {
    name: String,
    mobile: String,
    password: String,
    role_codes: Vec<String>,
    store_ids: Vec<i64>,
    status: String,
}

pub async fn batch_delete_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<BatchDeleteUsersRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let auth_user = auth::authenticate(&state, &headers)?;
    if payload.user_ids.is_empty() {
        return Err(AppError::BadRequest("user_ids不能为空".to_string()));
    }
    for user_id in payload.user_ids {
        let before_user = db::select_one_by_map_compat::<UserEntity>(
            state.rb.as_ref(),
            "user",
            value! { "id": user_id },
        )
        .await
        .context("failed to query user before batch delete")
        .map_err(AppError::Internal)?;
        UserEntity::update_by_map(
            state.rb.as_ref(),
            &UserEntity {
                deleted_at: Some(Utc::now().naive_utc()),
                ..Default::default()
            },
            value! { "id": user_id },
        )
        .await
        .context("failed to soft delete user in batch")
        .map_err(AppError::Internal)?;
        auth::bump_user_session_version(&state, user_id).await?;
        let after_user = db::select_one_by_map_compat::<UserEntity>(
            state.rb.as_ref(),
            "user",
            value! { "id": user_id },
        )
        .await
        .context("failed to query user after batch delete")
        .map_err(AppError::Internal)?;
        audit::write_audit_log(
            &state,
            "user",
            user_id,
            "batch_delete_user",
            auth_user.user_id,
            auth_user.primary_role(),
            before_user.as_ref(),
            after_user.as_ref(),
            Some("后台批量删除账号".to_string()),
        )
        .await?;
    }

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(user_id): axum::extract::Path<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let auth_user = auth::authenticate(&state, &headers)?;
    let before_user = db::select_one_by_map_compat::<UserEntity>(
        state.rb.as_ref(),
        "user",
        value! { "id": user_id },
    )
    .await
    .context("failed to query user before delete")
    .map_err(AppError::Internal)?;
    UserEntity::update_by_map(
        state.rb.as_ref(),
        &UserEntity {
            deleted_at: Some(Utc::now().naive_utc()),
            ..Default::default()
        },
        value! { "id": user_id },
    )
    .await
    .context("failed to soft delete user")
    .map_err(AppError::Internal)?;
    auth::bump_user_session_version(&state, user_id).await?;
    let after_user = db::select_one_by_map_compat::<UserEntity>(
        state.rb.as_ref(),
        "user",
        value! { "id": user_id },
    )
    .await
    .context("failed to query user after delete")
    .map_err(AppError::Internal)?;
    audit::write_audit_log(
        &state,
        "user",
        user_id,
        "delete_user",
        auth_user.user_id,
        auth_user.primary_role(),
        before_user.as_ref(),
        after_user.as_ref(),
        Some("后台删除账号".to_string()),
    )
    .await?;

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn export_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    ensure_admin_access(&state, &headers)?;
    let users: Vec<UserEntity> = db::select_by_map_compat(state.rb.as_ref(), "user", value! {})
        .await
        .context("failed to query users for export")
        .map_err(AppError::Internal)?;

    let mut csv = String::from("id,name,mobile,status\n");
    for user in users {
        csv.push_str(&format!(
            "{},{},{},{}\n",
            user.id.unwrap_or_default(),
            user.name.unwrap_or_default(),
            user.mobile.unwrap_or_default(),
            user.status.unwrap_or_default()
        ));
    }

    let mut response = Response::new(Body::from(csv));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"users-export.csv\""),
    );
    Ok(response)
}

pub async fn download_user_import_template(
    Query(query): Query<ImportTemplateQuery>,
) -> Result<Response, AppError> {
    let normalized_role_code = query.role_code.as_deref().map(normalize_role_code_alias);
    let (file_name, content) = if normalized_role_code.as_deref() == Some("dealer") {
        (
            "dealers-import-template.csv",
            build_csv_template(
                &["姓名", "手机号", "密码", "所属门店ID", "状态"],
                &["示例经销商", "13800009999", "123456", "1", "启用"],
            ),
        )
    } else {
        (
            "users-import-template.csv",
            build_csv_template(
                &["姓名", "手机号", "密码", "角色编码", "所属门店ID", "状态"],
                &["示例用户", "13800009999", "123456", "dealer", "1", "启用"],
            ),
        )
    };

    let mut response = Response::new(Body::from(content.into_bytes()));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{}\"", file_name))
            .map_err(|_| AppError::Internal(anyhow!("invalid content disposition header")))?,
    );
    Ok(response)
}

pub async fn import_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<ApiResponse<ImportResult>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let upload = read_import_multipart(multipart).await?;
    let rows = parse_import_rows(&upload)?;
    let default_role_codes = resolve_default_role_codes(&upload.fields);
    let mut success_count = 0usize;
    let mut failed_count = 0usize;
    let mut errors = Vec::new();

    for (index, mut row) in rows.into_iter().enumerate() {
        if row.role_codes.is_empty() {
            row.role_codes = default_role_codes.clone();
        }
        row.role_codes = row
            .role_codes
            .into_iter()
            .map(|item| normalize_role_code_alias(&item))
            .collect();
        if row.name.trim().is_empty()
            || row.mobile.trim().is_empty()
            || row.password.trim().is_empty()
        {
            failed_count += 1;
            errors.push(format!("第{}行必填字段缺失", index + 2));
            continue;
        }
        if row.role_codes.is_empty() {
            failed_count += 1;
            errors.push(format!("第{}行缺少角色编码", index + 2));
            continue;
        }
        if let Err(error) = validate_import_store_binding(&row.role_codes, &row.store_ids) {
            failed_count += 1;
            errors.push(format!("第{}行{}", index + 2, error));
            continue;
        }
        let mut resolved_role_ids = Vec::new();
        let mut missing_role = None;
        for role_code in &row.role_codes {
            if let Some(role_id) = find_role_id_by_code(&state, role_code).await? {
                resolved_role_ids.push(role_id);
            } else {
                missing_role = Some(role_code.clone());
                break;
            }
        }
        if let Some(role_code) = missing_role {
            failed_count += 1;
            errors.push(format!("第{}行角色不存在: {}", index + 2, role_code));
            continue;
        }

        let exists: Vec<UserEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "user",
            value! { "mobile": row.mobile.as_str() },
        )
        .await
        .context("failed to check existing import user")
        .map_err(AppError::Internal)?;
        if !exists.is_empty() {
            failed_count += 1;
            errors.push(format!("第{}行手机号已存在: {}", index + 2, row.mobile));
            continue;
        }

        let password_hash = match hash(&row.password, 10) {
            Ok(item) => item,
            Err(_) => {
                failed_count += 1;
                errors.push(format!("第{}行密码处理失败", index + 2));
                continue;
            }
        };

        UserEntity::insert(
            state.rb.as_ref(),
            &UserEntity {
                id: None,
                mobile: Some(row.mobile.clone()),
                account: Some(row.mobile.clone()),
                password_hash: Some(password_hash),
                name: Some(row.name.clone()),
                nickname: None,
                email: None,
                avatar_url: None,
                job_no: None,
                id_card: None,
                emergency_contact_name: None,
                emergency_contact_mobile: None,
                status: Some(normalize_user_status(Some(row.status.as_str()))?),
                session_version: Some(1),
                last_login_at: None,
                last_login_ip: None,
                created_at: None,
                updated_at: None,
                deleted_at: None,
                wechat_openid: None,
                wechat_unionid: None,
                wechat_subscribe: Some(0),
                wechat_bind_at: None,
                wechat_subscribe_updated_at: None,
            },
        )
        .await
        .context("failed to insert imported user")
        .map_err(AppError::Internal)?;

        let user: UserEntity = db::select_one_by_map_compat(
            state.rb.as_ref(),
            "user",
            value! { "mobile": row.mobile.as_str() },
        )
        .await
        .context("failed to fetch imported user")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow!("imported user fetch failed")))?;
        let user_id = user.id.unwrap_or_default();

        for role_id in resolved_role_ids {
            UserRoleEntity::insert(
                state.rb.as_ref(),
                &UserRoleEntity {
                    id: None,
                    user_id: Some(user_id),
                    role_id: Some(role_id),
                    created_at: None,
                },
            )
            .await
            .context("failed to bind imported user role")
            .map_err(AppError::Internal)?;
        }
        for store_id in row.store_ids {
            UserStoreEntity::insert(
                state.rb.as_ref(),
                &UserStoreEntity {
                    id: None,
                    user_id: Some(user_id),
                    store_id: Some(store_id),
                    created_at: None,
                },
            )
            .await
            .context("failed to bind imported user store")
            .map_err(AppError::Internal)?;
        }
        success_count += 1;
    }

    Ok(Json(ApiResponse::success(ImportResult {
        success_count,
        failed_count,
        errors,
    })))
}

pub async fn import_scores(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<ApiResponse<ImportResult>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["hq_reviewer", "super_admin"])?;
    let upload = read_import_multipart(multipart).await?;

    let ext = Path::new(&upload.file_name)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let rows = if ext == "csv" {
        parse_score_csv(&upload.bytes)?
    } else {
        parse_score_excel(&upload.file_name, &upload.bytes)?
    };

    let mut success_count = 0usize;
    let mut failed_count = 0usize;
    let mut errors = Vec::new();

    for (index, row) in rows.into_iter().enumerate() {
        let task = db::select_one_by_map_compat::<TaskEntity>(
            state.rb.as_ref(),
            "task",
            value! { "id": row.task_id },
        )
        .await
        .context("failed to query task during score import")
        .map_err(AppError::Internal)?;
        let Some(task) = task else {
            failed_count += 1;
            errors.push(format!("第{}行任务不存在: {}", index + 2, row.task_id));
            continue;
        };

        let service_types = load_task_service_types(&state, &task).await?;
        if !service_types.iter().any(|item| item == &row.service_type) {
            failed_count += 1;
            errors.push(format!(
                "第{}行服务环节与任务不匹配: {} / {}",
                index + 2,
                row.task_id,
                row.service_type
            ));
            continue;
        }
        if task.task_status.as_deref() != Some("pending_hq_score") {
            failed_count += 1;
            errors.push(format!(
                "第{}行任务状态不允许评分: {}",
                index + 2,
                row.task_id
            ));
            continue;
        }

        let exists: Vec<HqScoreEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "hq_score",
            value! { "task_id": row.task_id },
        )
        .await
        .context("failed to query existing score during import")
        .map_err(AppError::Internal)?;
        if !exists.is_empty() {
            failed_count += 1;
            errors.push(format!("第{}行任务已评分: {}", index + 2, row.task_id));
            continue;
        }

        let service_level = calc_service_level(row.score).to_string();
        HqScoreEntity::insert(
            state.rb.as_ref(),
            &HqScoreEntity {
                id: None,
                task_id: Some(row.task_id),
                scorer_id: Some(auth_user.user_id),
                total_score: Some(row.score),
                service_level: Some(service_level),
                score_remark: row.score_remark.clone(),
                created_at: None,
            },
        )
        .await
        .context("failed to insert imported score")
        .map_err(AppError::Internal)?;

        let hq_score: HqScoreEntity = db::select_one_by_map_compat(
            state.rb.as_ref(),
            "hq_score",
            value! { "task_id": row.task_id },
        )
        .await
        .context("failed to fetch imported hq score")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow!("imported hq score fetch failed")))?;

        HqScoreItemEntity::insert(
            state.rb.as_ref(),
            &HqScoreItemEntity {
                id: None,
                hq_score_id: hq_score.id,
                task_id: Some(row.task_id),
                service_type: Some(row.service_type.clone()),
                score: Some(row.score),
                deduction_reason: row.deduction_reason.clone(),
                created_at: None,
            },
        )
        .await
        .context("failed to insert imported hq score item")
        .map_err(AppError::Internal)?;

        TaskEntity::update_by_map(
            state.rb.as_ref(),
            &TaskEntity {
                score_status: Some("scored".to_string()),
                task_status: Some("completed".to_string()),
                ..Default::default()
            },
            value! { "id": row.task_id },
        )
        .await
        .context("failed to update task after score import")
        .map_err(AppError::Internal)?;

        success_count += 1;
    }

    Ok(Json(ApiResponse::success(ImportResult {
        success_count,
        failed_count,
        errors,
    })))
}

#[derive(Debug)]
struct ScoreImportRow {
    task_id: i64,
    service_type: String,
    score: i32,
    deduction_reason: Option<String>,
    score_remark: Option<String>,
}

#[derive(Debug)]
struct UploadField {
    file_name: String,
    bytes: Vec<u8>,
    fields: HashMap<String, String>,
}

async fn read_import_multipart(mut multipart: Multipart) -> Result<UploadField, AppError> {
    let mut fields = HashMap::new();
    let mut file_name = None;
    let mut bytes = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .context("failed to read import multipart field")
        .map_err(AppError::Internal)?
    {
        let name = field.name().map(ToString::to_string).unwrap_or_default();
        if let Some(current_file_name) = field.file_name().map(ToString::to_string) {
            let file_bytes = field
                .bytes()
                .await
                .context("failed to read import file bytes")
                .map_err(AppError::Internal)?
                .to_vec();
            file_name = Some(current_file_name);
            bytes = Some(file_bytes);
        } else {
            let text = field
                .text()
                .await
                .context("failed to read import multipart text field")
                .map_err(AppError::Internal)?;
            if !name.is_empty() {
                fields.insert(name, text);
            }
        }
    }

    Ok(UploadField {
        file_name: file_name.ok_or_else(|| AppError::BadRequest("请上传文件".to_string()))?,
        bytes: bytes.ok_or_else(|| AppError::BadRequest("请上传文件".to_string()))?,
        fields,
    })
}

fn parse_import_rows(upload: &UploadField) -> Result<Vec<ImportedUserRow>, AppError> {
    let ext = Path::new(&upload.file_name)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "csv" {
        return parse_user_csv(&upload.bytes);
    }
    if ["xls", "xlsx"].contains(&ext.as_str()) {
        return parse_user_excel(&upload.file_name, &upload.bytes);
    }

    Err(AppError::BadRequest(
        "用户导入仅支持 csv/xls/xlsx 文件".to_string(),
    ))
}

fn parse_user_csv(bytes: &[u8]) -> Result<Vec<ImportedUserRow>, AppError> {
    let content = String::from_utf8(strip_utf8_bom(bytes).to_vec())
        .map_err(|_| AppError::BadRequest("用户导入CSV文件必须为UTF-8编码".to_string()))?;
    let mut lines = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .peekable();
    if matches!(lines.peek(), Some(line) if line.eq_ignore_ascii_case("sep=,")) {
        let _ = lines.next();
    }
    let headers = lines
        .next()
        .ok_or_else(|| AppError::BadRequest("导入文件不能为空".to_string()))?
        .split(',')
        .map(|item| item.trim().to_string())
        .collect::<Vec<_>>();
    let header_map = build_header_index_map(&headers);
    let mut rows = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let cols = line
            .split(',')
            .map(|item| item.trim().to_string())
            .collect::<Vec<_>>();
        rows.push(parse_import_row_values(&header_map, &cols));
    }
    Ok(rows)
}

fn parse_user_excel(file_name: &str, bytes: &[u8]) -> Result<Vec<ImportedUserRow>, AppError> {
    let temp_dir = std::env::temp_dir();
    let temp_path = temp_dir.join(format!("{}-{}", Uuid::new_v4(), file_name));
    {
        let mut file = std::fs::File::create(&temp_path)
            .map_err(|_| AppError::Internal(anyhow!("failed to create temp user import file")))?;
        file.write_all(bytes)
            .map_err(|_| AppError::Internal(anyhow!("failed to write temp user import file")))?;
    }
    let result = parse_user_excel_file(&temp_path);
    let _ = std::fs::remove_file(&temp_path);
    result
}

fn parse_user_excel_file(path: &Path) -> Result<Vec<ImportedUserRow>, AppError> {
    let mut workbook = open_workbook_auto(path)
        .map_err(|_| AppError::BadRequest("无法读取用户导入文件".to_string()))?;
    let sheet_name = workbook
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| AppError::BadRequest("用户导入文件没有工作表".to_string()))?;
    let range = workbook
        .worksheet_range(&sheet_name)
        .map_err(|_| AppError::BadRequest("读取用户导入工作表失败".to_string()))?;
    let mut row_iter = range.rows();
    let headers = row_iter
        .next()
        .ok_or_else(|| AppError::BadRequest("导入文件不能为空".to_string()))?
        .iter()
        .map(|item| item.to_string())
        .collect::<Vec<_>>();
    let header_map = build_header_index_map(&headers);
    let mut rows = Vec::new();
    for row in row_iter {
        let cols = row.iter().map(|item| item.to_string()).collect::<Vec<_>>();
        if cols.iter().all(|item| item.trim().is_empty()) {
            continue;
        }
        rows.push(parse_import_row_values(&header_map, &cols));
    }
    Ok(rows)
}

fn build_header_index_map(headers: &[String]) -> HashMap<String, usize> {
    let mut header_map = HashMap::new();
    for (index, header) in headers.iter().enumerate() {
        let normalized = header.trim().to_ascii_lowercase();
        if !normalized.is_empty() {
            header_map.insert(normalized, index);
        }
    }
    header_map
}

fn build_csv_template(headers: &[&str], sample: &[&str]) -> String {
    format!(
        "\u{feff}sep=,\n{}\n{}\n",
        headers.join(","),
        sample.join(",")
    )
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

fn parse_import_row_values(
    header_map: &HashMap<String, usize>,
    cols: &[String],
) -> ImportedUserRow {
    let name = value_by_keys(header_map, cols, &["name", "姓名"]);
    let mobile = value_by_keys(header_map, cols, &["mobile", "手机号", "电话"]);
    let password = value_by_keys(header_map, cols, &["password", "密码"]);
    let role_code = value_by_keys(
        header_map,
        cols,
        &["role_code", "role_codes", "角色编码", "角色"],
    );
    let store_value = value_by_keys(
        header_map,
        cols,
        &["store_id", "store_ids", "门店id", "所属门店", "门店"],
    );
    let status = value_by_keys(header_map, cols, &["status", "状态"]);

    ImportedUserRow {
        name,
        mobile,
        password,
        role_codes: split_multi_values(&role_code),
        store_ids: split_multi_i64_values(&store_value),
        status: if status.trim().is_empty() {
            "enabled".to_string()
        } else {
            status
        },
    }
}

fn value_by_keys(header_map: &HashMap<String, usize>, cols: &[String], keys: &[&str]) -> String {
    for key in keys {
        if let Some(index) = header_map.get(&key.to_ascii_lowercase()) {
            return cols
                .get(*index)
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string();
        }
    }
    String::new()
}

fn split_multi_values(value: &str) -> Vec<String> {
    value
        .split('|')
        .flat_map(|item| item.split('、'))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn split_multi_i64_values(value: &str) -> Vec<i64> {
    value
        .split('|')
        .flat_map(|item| item.split('、'))
        .filter_map(|item| item.trim().parse::<i64>().ok())
        .collect()
}

fn resolve_default_role_codes(fields: &HashMap<String, String>) -> Vec<String> {
    fields
        .get("role_code")
        .map(|item| split_multi_values(item))
        .or_else(|| {
            fields
                .get("role_codes")
                .map(|item| split_multi_values(item))
        })
        .unwrap_or_default()
}

fn normalize_role_code_alias(role_code: &str) -> String {
    match role_code.trim().to_ascii_lowercase().as_str() {
        "admin" | "role-001" => "super_admin".to_string(),
        "inspection_admin" | "role-002" => "hq_reviewer".to_string(),
        "dealer_manager" | "role-003" => "dealer".to_string(),
        "readonly_admin" | "role-004" => "hq_reviewer".to_string(),
        _ => role_code.trim().to_string(),
    }
}

fn validate_import_store_binding(role_codes: &[String], store_ids: &[i64]) -> Result<(), String> {
    let requires_single_store = role_codes
        .iter()
        .any(|role| role == "dealer" || role == "technician");
    if requires_single_store && store_ids.len() != 1 {
        return Err("经销商和师傅账号必须且只能绑定一个门店".to_string());
    }
    Ok(())
}

fn normalize_user_status(value: Option<&str>) -> Result<String, AppError> {
    let raw = value.unwrap_or("enabled").trim();
    match raw {
        "启用" => Ok("enabled".to_string()),
        "禁用" => Ok("disabled".to_string()),
        _ => match raw.to_ascii_lowercase().as_str() {
            "enabled" | "active" => Ok("enabled".to_string()),
            "disabled" | "inactive" => Ok("disabled".to_string()),
            _ => Err(AppError::BadRequest(
                "用户状态只能为启用/禁用或enabled/disabled或active/inactive".to_string(),
            )),
        },
    }
}

fn parse_score_csv(bytes: &[u8]) -> Result<Vec<ScoreImportRow>, AppError> {
    let content = String::from_utf8(bytes.to_vec())
        .map_err(|_| AppError::BadRequest("CSV 文件必须为UTF-8编码".to_string()))?;
    let mut rows = Vec::new();
    for line in content.lines().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() < 3 {
            continue;
        }
        let task_id = cols[0]
            .trim()
            .parse()
            .map_err(|_| AppError::BadRequest("评分导入task_id格式错误".to_string()))?;
        let service_type = cols[1].trim().to_string();
        let score = cols[2]
            .trim()
            .parse()
            .map_err(|_| AppError::BadRequest("评分导入score格式错误".to_string()))?;
        let deduction_reason = cols
            .get(3)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let score_remark = cols
            .get(4)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        rows.push(ScoreImportRow {
            task_id,
            service_type,
            score,
            deduction_reason,
            score_remark,
        });
    }
    Ok(rows)
}

fn parse_score_excel(file_name: &str, bytes: &[u8]) -> Result<Vec<ScoreImportRow>, AppError> {
    let temp_dir = std::env::temp_dir();
    let temp_path = temp_dir.join(format!("{}-{}", Uuid::new_v4(), file_name));
    {
        let mut file = std::fs::File::create(&temp_path)
            .map_err(|_| AppError::Internal(anyhow!("failed to create temp score import file")))?;
        file.write_all(bytes)
            .map_err(|_| AppError::Internal(anyhow!("failed to write temp score import file")))?;
    }
    let result = parse_score_excel_file(&temp_path);
    let _ = std::fs::remove_file(&temp_path);
    result
}

fn parse_score_excel_file(path: &Path) -> Result<Vec<ScoreImportRow>, AppError> {
    let mut workbook = open_workbook_auto(path)
        .map_err(|_| AppError::BadRequest("无法读取评分导入文件".to_string()))?;
    let sheet_name = workbook
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| AppError::BadRequest("评分导入文件没有工作表".to_string()))?;
    let range = workbook
        .worksheet_range(&sheet_name)
        .map_err(|_| AppError::BadRequest("读取评分工作表失败".to_string()))?;

    let mut rows = Vec::new();
    for row in range.rows().skip(1) {
        if row.len() < 3 {
            continue;
        }
        let task_id = row[0]
            .to_string()
            .parse()
            .map_err(|_| AppError::BadRequest("评分导入task_id格式错误".to_string()))?;
        let service_type = row[1].to_string();
        let score = row[2]
            .to_string()
            .parse()
            .map_err(|_| AppError::BadRequest("评分导入score格式错误".to_string()))?;
        let deduction_reason = row
            .get(3)
            .map(|item| item.to_string())
            .filter(|s| !s.is_empty());
        let score_remark = row
            .get(4)
            .map(|item| item.to_string())
            .filter(|s| !s.is_empty());
        rows.push(ScoreImportRow {
            task_id,
            service_type,
            score,
            deduction_reason,
            score_remark,
        });
    }
    Ok(rows)
}

async fn find_role_id_by_code(state: &AppState, role_code: &str) -> Result<Option<i64>, AppError> {
    let role: Option<RoleEntity> =
        db::select_one_by_map_compat(state.rb.as_ref(), "role", value! { "role_code": role_code })
            .await
            .context("failed to query role during import")
            .map_err(AppError::Internal)?;
    Ok(role.and_then(|item| item.id))
}

fn calc_service_level(score: i32) -> &'static str {
    match score {
        90..=100 => "excellent",
        80..=89 => "good",
        60..=79 => "pass",
        _ => "improve",
    }
}

fn ensure_admin_access(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let auth_user = auth::authenticate(state, headers)?;
    auth_user.require_any_role(&["super_admin", "hq_reviewer"])
}

async fn load_task_service_types(
    state: &AppState,
    task: &TaskEntity,
) -> Result<Vec<String>, AppError> {
    let task_id = task.id.unwrap_or_default();
    let items =
        db::select_by_map_compat::<crate::models::task_service_item::TaskServiceItemEntity>(
            state.rb.as_ref(),
            "task_service_item",
            value! { "task_id": task_id },
        )
        .await
        .context("failed to query task service items during score import")
        .map_err(AppError::Internal)?;

    if items.is_empty() {
        return Ok(task.service_type.iter().cloned().collect());
    }

    Ok(items
        .into_iter()
        .filter_map(|item| item.service_type)
        .collect())
}

fn deserialize_vec_i64_from_mixed<'de, D>(deserializer: D) -> Result<Vec<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<serde_json::Value>::deserialize(deserializer)?;
    values
        .into_iter()
        .map(|value| match value {
            serde_json::Value::Number(number) => number
                .as_i64()
                .ok_or_else(|| serde::de::Error::custom("invalid integer value")),
            serde_json::Value::String(text) => text
                .trim()
                .parse::<i64>()
                .map_err(|_| serde::de::Error::custom("invalid integer string")),
            _ => Err(serde::de::Error::custom("expected string or number")),
        })
        .collect()
}
