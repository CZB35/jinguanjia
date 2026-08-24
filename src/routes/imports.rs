use std::{
    collections::{BTreeSet, HashMap},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, anyhow};
use axum::{
    Json,
    body::Body,
    extract::{Multipart, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bcrypt::hash;
use calamine::{Reader, open_workbook_auto};
use chrono::Utc;
use encoding_rs::GBK;
use rbs::value;
use rust_xlsxwriter::{Color, Format, FormatAlign, FormatBorder, Workbook};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use crate::{
    audit, auth, db,
    error::AppError,
    models::{
        customer::CustomerEntity, customer_address::CustomerAddressEntity,
        customer_source::CustomerSourceEntity, hq_score::HqScoreEntity,
        hq_score_item::HqScoreItemEntity, role::RoleEntity, store::StoreEntity, task::TaskEntity,
        technician_skill::TechnicianSkillEntity, user::UserEntity, user_role::UserRoleEntity,
        user_store::UserStoreEntity,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_file_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_file_base64: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImportTemplateQuery {
    pub role_code: Option<String>,
    pub import_type: Option<String>,
}

#[derive(Debug)]
struct ImportedUserRow {
    name: String,
    mobile: String,
    password: String,
    role_codes: Vec<String>,
    store_refs: Vec<String>,
    status: String,
}

#[derive(Debug)]
struct ImportedTechnicianRow {
    name: String,
    mobile: String,
    password: String,
    id_card: String,
    status: String,
    emergency_contact_name: String,
    emergency_contact_mobile: String,
    service_types: Vec<String>,
    store_ref: String,
}

#[derive(Debug)]
struct ImportedCustomerRow {
    name: String,
    mobile: String,
    source: String,
    store_ref: Option<String>,
    province: Option<String>,
    city: Option<String>,
    district: Option<String>,
    address: String,
    remark: String,
}

#[derive(Debug)]
struct ImportFailureDetail {
    row_no: usize,
    row_values: Vec<String>,
    reason: String,
}

#[derive(Debug, Default)]
struct StoreLookup {
    by_id: HashMap<i64, StoreEntity>,
    ids_by_name: HashMap<String, Vec<i64>>,
    ids_by_code: HashMap<String, Vec<i64>>,
}

#[derive(Debug)]
struct ImportedStoreRow {
    store_name: String,
    code: String,
    province: Option<String>,
    city: Option<String>,
    district: Option<String>,
    address: String,
    contact_name: String,
    contact_mobile: String,
    business_hours: String,
    remark: String,
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

    let mut csv = String::from("\u{feff}账号ID,姓名,手机号,状态\n");
    for user in users {
        csv.push_str(&format!(
            "{},{},{},{}\n",
            user.id.unwrap_or_default(),
            user.name.unwrap_or_default(),
            user.mobile.unwrap_or_default(),
            export_user_status_text(user.status.as_deref())
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
        HeaderValue::from_static("attachment; filename=\"账号导出.csv\""),
    );
    Ok(response)
}

fn export_user_status_text(status: Option<&str>) -> &'static str {
    match status.unwrap_or("enabled") {
        "enabled" | "active" => "启用",
        "disabled" | "inactive" => "禁用",
        _ => "未知",
    }
}

pub async fn download_user_import_template(
    State(state): State<AppState>,
    Query(query): Query<ImportTemplateQuery>,
) -> Result<Response, AppError> {
    let import_type = query
        .import_type
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    if import_type == "customer" {
        let customer_source_notes = load_customer_source_template_notes(&state).await?;
        let mut instructions = vec![
            "客户来源建议填写中文名称，例如：手工录入。".to_string(),
            "所属门店填写门店名称或门店编码，不要填写数据库ID。".to_string(),
            "如不填写客户来源，系统默认按手工录入处理。".to_string(),
        ];
        for note in customer_source_notes {
            instructions.push(note);
        }
        let instruction_refs = instructions.iter().map(String::as_str).collect::<Vec<_>>();
        return build_template_workbook_response(
            "customers-import-template.xlsx",
            &[
                "客户姓名*",
                "客户手机号*",
                "客户来源",
                "所属门店",
                "省",
                "市",
                "区",
                "详细地址",
                "备注",
            ],
            &[
                "示例客户",
                "13800001111",
                "手工录入",
                "西湖直营店",
                "浙江省",
                "杭州市",
                "西湖区",
                "文三路1号",
                "上门前请联系",
            ],
            &instruction_refs,
        );
    }
    if import_type == "store" {
        return build_template_workbook_response(
            "stores-import-template.xlsx",
            &[
                "门店名称*",
                "门店编码",
                "省",
                "市",
                "区",
                "详细地址",
                "联系人",
                "联系电话",
                "营业时间",
                "备注",
                "状态",
            ],
            &[
                "示例门店",
                "STORE-001",
                "浙江省",
                "杭州市",
                "西湖区",
                "文三路1号",
                "张三",
                "13800002222",
                "09:00-18:00",
                "西湖区门店",
                "启用",
            ],
            &[
                "状态可填写：启用、禁用。",
                "状态可留空，留空时默认按启用导入。",
                "门店编码可留空；如填写则建议保持唯一。",
                "门店名称和门店编码都建议保持唯一。",
            ],
        );
    }
    let normalized_role_code = query.role_code.as_deref().map(normalize_role_code_alias);
    if import_type == "technician" || normalized_role_code.as_deref() == Some("technician") {
        return build_technician_import_template_response();
    }
    let (file_name, headers, sample, instructions) =
        if normalized_role_code.as_deref() == Some("dealer") {
            (
                "dealers-import-template.xlsx",
                vec!["姓名*", "手机号*", "密码", "所属门店*", "状态"],
                vec!["示例经销商", "13800009999", "123456", "西湖直营店", "启用"],
                vec![
                    "经销商账号必须且只能绑定一个门店。",
                    "密码可留空，留空时默认 123456。",
                    "所属门店请填写门店名称或门店编码，不要填写数据库ID。",
                    "状态可留空，留空时默认按启用导入。",
                ],
            )
        } else {
            (
                "users-import-template.xlsx",
                vec!["姓名*", "手机号*", "密码", "角色编码*", "所属门店", "状态"],
                vec![
                    "示例用户",
                    "13800009999",
                    "123456",
                    "hq_reviewer",
                    "",
                    "启用",
                ],
                vec![
                    "角色编码可填写：super_admin、hq_reviewer。",
                    "密码可留空，留空时默认 123456。",
                    "如角色需要绑定门店，请填写门店名称或门店编码，不要填写数据库ID。",
                    "多个门店可用 | 分隔。",
                    "状态可留空，留空时默认按启用导入。",
                ],
            )
        };

    build_template_workbook_response(file_name, &headers, &sample, &instructions)
}

pub async fn download_technician_import_template() -> Result<Response, AppError> {
    build_technician_import_template_response()
}

fn build_technician_import_template_response() -> Result<Response, AppError> {
    build_template_workbook_response(
        "technicians-import-template.xlsx",
        &[
            "师傅姓名*",
            "手机号*",
            "密码",
            "身份证号",
            "状态",
            "紧急联系人",
            "紧急联系电话",
            "专业技能*",
            "所属门店*",
        ],
        &[
            "示例师傅",
            "13800008888",
            "123456",
            "",
            "启用",
            "李四",
            "13800007777",
            "量尺、安装",
            "西湖直营店",
        ],
        &[
            "师傅导入固定写入 technician 角色，不需要填写角色编码。",
            "密码可留空，留空时默认 123456。",
            "专业技能可填写：量尺、安装、熨烫、除尘除螨；多个技能用 、 或 | 分隔。",
            "所属门店请填写门店名称或门店编码，不要填写数据库ID。",
            "状态可填写：启用、禁用；留空时默认启用。",
            "如果手机号账号已存在，导入会补齐师傅身份、门店、专业技能和可选信息。",
        ],
    )
}

pub async fn download_score_import_template() -> Result<Response, AppError> {
    build_template_workbook_response(
        "scores-import-template.xlsx",
        &[
            "客户手机号*",
            "服务环节*",
            "总部评分*",
            "扣分原因",
            "评分备注",
        ],
        &[
            "18838962531",
            "除尘除螨",
            "95",
            "现场照片不完整",
            "复核后导入",
        ],
        &[
            "客户手机号用于匹配待评分任务；同一手机号同一服务环节只匹配到一条可评分任务时才会导入。",
            "为兼容旧模板，文件中如仍带任务ID或任务编号列，后端会优先按任务ID或任务编号匹配。",
            "服务环节可填写：量尺、安装、熨烫、除尘除螨；带“服务”两个字也兼容。",
            "总部评分必须是0到100之间的整数。",
            "再次导入同一任务同一服务环节时，会覆盖原总部评分。",
            "总部评分只写入总部核验评分，不会写入客户评价或经销商评价。",
            "总部评分只更新核验状态，不会把任务状态改为已完成。",
        ],
    )
}

pub async fn import_dealers(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<ApiResponse<ImportResult>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let upload = read_import_multipart(multipart).await?;
    let mut rows = parse_import_rows(&upload)?;
    for row in &mut rows {
        row.role_codes = vec!["dealer".to_string()];
    }
    import_user_rows(&state, rows, Some(vec!["dealer".to_string()])).await
}

pub async fn import_technicians(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<ApiResponse<ImportResult>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let upload = read_import_multipart(multipart).await?;
    let rows = parse_technician_import_rows(&upload)?;
    import_technician_rows(&state, rows).await
}

pub async fn import_customers(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<ApiResponse<ImportResult>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let auth_user = auth::authenticate(&state, &headers)?;
    let upload = read_import_multipart(multipart).await?;
    let rows = parse_customer_import_rows(&upload)?;
    let mut success_count = 0usize;
    let store_lookup = load_store_lookup(&state).await?;
    let mut failures = Vec::new();

    for (index, row) in rows.into_iter().enumerate() {
        let row_no = index + 2;
        if row.name.trim().is_empty() || row.mobile.trim().is_empty() {
            failures.push(ImportFailureDetail {
                row_no,
                row_values: customer_row_values(&row),
                reason: "客户姓名或手机号不能为空".to_string(),
            });
            continue;
        }
        let exists: Vec<CustomerEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "customer",
            value! { "mobile": row.mobile.as_str() },
        )
        .await
        .context("failed to query existing customer during import")
        .map_err(AppError::Internal)?;
        if !exists.is_empty() {
            failures.push(ImportFailureDetail {
                row_no,
                row_values: customer_row_values(&row),
                reason: format!("客户手机号已存在: {}", row.mobile),
            });
            continue;
        }

        let primary_store_id =
            match resolve_optional_store_id_from_lookup(&store_lookup, row.store_ref.as_deref()) {
                Ok(store_id) => store_id,
                Err(AppError::BadRequest(message)) => {
                    failures.push(ImportFailureDetail {
                        row_no,
                        row_values: customer_row_values(&row),
                        reason: message,
                    });
                    continue;
                }
                Err(error) => return Err(error),
            };

        let source = resolve_customer_source_value_for_import(&state, &row.source).await?;
        if let Err(error) = CustomerEntity::insert(
            state.rb.as_ref(),
            &CustomerEntity {
                id: None,
                name: Some(row.name.clone()),
                mobile: Some(row.mobile.clone()),
                source: Some(source),
                remark: normalize_optional_text(Some(row.remark.as_str())),
                primary_store_id,
                created_by: Some(auth_user.user_id),
                created_at: None,
                updated_at: None,
                deleted_at: None,
            },
        )
        .await
        {
            tracing::warn!(
                error = ?error,
                row_no,
                mobile = %row.mobile,
                "failed to create imported customer"
            );
            failures.push(ImportFailureDetail {
                row_no,
                row_values: customer_row_values(&row),
                reason: "客户写入失败，请检查手机号、门店和表结构".to_string(),
            });
            continue;
        }

        let customer: CustomerEntity = match db::select_one_by_map_compat(
            state.rb.as_ref(),
            "customer",
            value! { "mobile": row.mobile.as_str() },
        )
        .await
        {
            Ok(Some(customer)) => customer,
            Ok(None) => {
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: customer_row_values(&row),
                    reason: "客户写入后读取失败".to_string(),
                });
                continue;
            }
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    row_no,
                    mobile = %row.mobile,
                    "failed to fetch imported customer"
                );
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: customer_row_values(&row),
                    reason: "客户写入后读取失败".to_string(),
                });
                continue;
            }
        };

        if !row.address.trim().is_empty() {
            if let Err(error) = CustomerAddressEntity::insert(
                state.rb.as_ref(),
                &CustomerAddressEntity {
                    id: None,
                    customer_id: customer.id,
                    province: row.province.clone(),
                    city: row.city.clone(),
                    district: row.district.clone(),
                    address: Some(row.address.clone()),
                    is_default: Some(1),
                    created_at: None,
                },
            )
            .await
            {
                tracing::warn!(
                    error = ?error,
                    row_no,
                    customer_id = ?customer.id,
                    "failed to create imported customer address"
                );
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: customer_row_values(&row),
                    reason: "客户已创建，但地址写入失败".to_string(),
                });
                continue;
            }
        }
        success_count += 1;
    }

    Ok(Json(ApiResponse::success(build_import_result(
        success_count,
        failures,
        "customers-import-errors.xlsx",
        &[
            "Excel行号",
            "客户姓名",
            "客户手机号",
            "客户来源",
            "所属门店",
            "省",
            "市",
            "区",
            "详细地址",
            "备注",
            "失败原因",
        ],
    )?)))
}

pub async fn import_stores(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<ApiResponse<ImportResult>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let upload = read_import_multipart(multipart).await?;
    let rows = parse_store_import_rows(&upload)?;
    let mut success_count = 0usize;
    let mut failures = Vec::new();

    for (index, row) in rows.into_iter().enumerate() {
        let row_no = index + 2;
        if row.store_name.trim().is_empty() {
            failures.push(ImportFailureDetail {
                row_no,
                row_values: store_row_values(&row),
                reason: "门店名称不能为空".to_string(),
            });
            continue;
        }
        let existing_by_name: Vec<StoreEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "store",
            value! { "store_name": row.store_name.as_str() },
        )
        .await
        .context("failed to query existing store by name during import")
        .map_err(AppError::Internal)?;
        if !existing_by_name.is_empty() {
            failures.push(ImportFailureDetail {
                row_no,
                row_values: store_row_values(&row),
                reason: format!("门店名称已存在: {}", row.store_name),
            });
            continue;
        }
        if !row.code.trim().is_empty() {
            let existing_by_code: Vec<StoreEntity> = db::select_by_map_compat(
                state.rb.as_ref(),
                "store",
                value! { "code": row.code.as_str() },
            )
            .await
            .context("failed to query existing store by code during import")
            .map_err(AppError::Internal)?;
            if !existing_by_code.is_empty() {
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: store_row_values(&row),
                    reason: format!("门店编码已存在: {}", row.code),
                });
                continue;
            }
        }

        let status = match normalize_general_status(Some(row.status.as_str())) {
            Ok(status) => status,
            Err(AppError::BadRequest(message)) => {
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: store_row_values(&row),
                    reason: message,
                });
                continue;
            }
            Err(error) => return Err(error),
        };

        if let Err(error) = StoreEntity::insert(
            state.rb.as_ref(),
            &StoreEntity {
                id: None,
                store_name: Some(row.store_name.clone()),
                code: normalize_optional_text(Some(row.code.as_str())),
                contact_name: normalize_optional_text(Some(row.contact_name.as_str())),
                contact_mobile: normalize_optional_text(Some(row.contact_mobile.as_str())),
                province: row.province.clone(),
                city: row.city.clone(),
                district: row.district.clone(),
                address: normalize_optional_text(Some(row.address.as_str())),
                remark: normalize_optional_text(Some(row.remark.as_str())),
                business_hours: normalize_optional_text(Some(row.business_hours.as_str())),
                status: Some(status),
                created_at: None,
                updated_at: None,
            },
        )
        .await
        {
            tracing::warn!(
                error = ?error,
                row_no,
                store_name = %row.store_name,
                "failed to create imported store"
            );
            failures.push(ImportFailureDetail {
                row_no,
                row_values: store_row_values(&row),
                reason: "门店写入失败，请检查门店编码、状态或表结构".to_string(),
            });
            continue;
        }
        success_count += 1;
    }

    Ok(Json(ApiResponse::success(build_import_result(
        success_count,
        failures,
        "stores-import-errors.xlsx",
        &[
            "Excel行号",
            "门店名称",
            "门店编码",
            "省",
            "市",
            "区",
            "详细地址",
            "联系人",
            "联系电话",
            "营业时间",
            "备注",
            "状态",
            "失败原因",
        ],
    )?)))
}

pub async fn import_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<ApiResponse<ImportResult>>, AppError> {
    ensure_admin_access(&state, &headers)?;
    let upload = read_import_multipart(multipart).await?;
    if upload_defaults_to_technician_import(&upload.fields) {
        let rows = parse_technician_import_rows(&upload)?;
        return import_technician_rows(&state, rows).await;
    }
    let rows = parse_import_rows(&upload)?;
    let default_role_codes = resolve_default_role_codes(&upload.fields);
    import_user_rows(
        &state,
        rows,
        if default_role_codes.is_empty() {
            None
        } else {
            Some(default_role_codes)
        },
    )
    .await
}

async fn import_technician_rows(
    state: &AppState,
    rows: Vec<ImportedTechnicianRow>,
) -> Result<Json<ApiResponse<ImportResult>>, AppError> {
    let mut success_count = 0usize;
    let mut failures = Vec::new();
    let store_lookup = load_store_lookup(state).await?;
    let technician_role_id = match find_role_id_by_code(state, "technician").await? {
        Some(role_id) => role_id,
        None => {
            return Err(AppError::BadRequest(
                "系统缺少 technician 角色，请先检查角色基础数据".to_string(),
            ));
        }
    };

    for (index, mut row) in rows.into_iter().enumerate() {
        let row_no = index + 2;
        row.service_types = dedup_non_empty(row.service_types.into_iter());
        match normalize_import_service_types(&row.service_types) {
            Ok(service_types) => row.service_types = service_types,
            Err(reason) => {
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: technician_row_values(&row),
                    reason,
                });
                continue;
            }
        }
        if row.password.trim().is_empty() {
            row.password = "123456".to_string();
        }
        if row.name.trim().is_empty()
            || row.mobile.trim().is_empty()
            || row.store_ref.trim().is_empty()
            || row.service_types.is_empty()
        {
            failures.push(ImportFailureDetail {
                row_no,
                row_values: technician_row_values(&row),
                reason: "必填字段缺失：师傅姓名、手机号、所属门店、专业技能不能为空".to_string(),
            });
            continue;
        }
        if row.password.trim().len() < 6 {
            failures.push(ImportFailureDetail {
                row_no,
                row_values: technician_row_values(&row),
                reason: "密码长度不能少于6位".to_string(),
            });
            continue;
        }
        let store_id = match resolve_single_store_ref(&store_lookup, &row.store_ref) {
            Ok(store_id) => store_id,
            Err(AppError::BadRequest(message)) => {
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: technician_row_values(&row),
                    reason: message,
                });
                continue;
            }
            Err(error) => return Err(error),
        };
        let status = match normalize_user_status(Some(row.status.as_str())) {
            Ok(status) => status,
            Err(AppError::BadRequest(message)) => {
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: technician_row_values(&row),
                    reason: message,
                });
                continue;
            }
            Err(error) => return Err(error),
        };

        let existing_user: Option<UserEntity> = db::select_one_by_map_compat(
            state.rb.as_ref(),
            "user",
            value! { "mobile": row.mobile.as_str() },
        )
        .await
        .context("failed to query existing technician import user")
        .map_err(AppError::Internal)?;
        let user_id = if let Some(user) = existing_user {
            let Some(user_id) = user.id else {
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: technician_row_values(&row),
                    reason: "已有账号缺少用户ID".to_string(),
                });
                continue;
            };
            let restoring_deleted_user = user.deleted_at.is_some();
            let update_result = if restoring_deleted_user {
                let password_hash = match hash(&row.password, 10) {
                    Ok(item) => item,
                    Err(_) => {
                        failures.push(ImportFailureDetail {
                            row_no,
                            row_values: technician_row_values(&row),
                            reason: "密码处理失败".to_string(),
                        });
                        continue;
                    }
                };
                UserEntity::update_by_map(
                    state.rb.as_ref(),
                    &UserEntity {
                        password_hash: Some(password_hash),
                        name: Some(row.name.clone()),
                        nickname: None,
                        email: None,
                        avatar_url: None,
                        job_no: Some(format!(
                            "SF{:08}",
                            Utc::now().timestamp_millis().rem_euclid(100_000_000)
                        )),
                        id_card: normalize_optional_text(Some(row.id_card.as_str())),
                        emergency_contact_name: normalize_optional_text(Some(
                            row.emergency_contact_name.as_str(),
                        )),
                        emergency_contact_mobile: normalize_optional_text(Some(
                            row.emergency_contact_mobile.as_str(),
                        )),
                        status: Some("enabled".to_string()),
                        last_login_at: None,
                        last_login_ip: None,
                        deleted_at: None,
                        wechat_openid: None,
                        wechat_unionid: None,
                        wechat_subscribe: Some(0),
                        wechat_bind_at: None,
                        wechat_subscribe_updated_at: None,
                        ..Default::default()
                    },
                    value! {
                        "id": user_id,
                        "column": [
                            "password_hash",
                            "name",
                            "nickname",
                            "email",
                            "avatar_url",
                            "job_no",
                            "id_card",
                            "emergency_contact_name",
                            "emergency_contact_mobile",
                            "status",
                            "last_login_at",
                            "last_login_ip",
                            "deleted_at",
                            "wechat_openid",
                            "wechat_unionid",
                            "wechat_subscribe",
                            "wechat_bind_at",
                            "wechat_subscribe_updated_at"
                        ]
                    },
                )
                .await
            } else {
                UserEntity::update_by_map(
                    state.rb.as_ref(),
                    &UserEntity {
                        id_card: normalize_optional_text(Some(row.id_card.as_str())),
                        emergency_contact_name: normalize_optional_text(Some(
                            row.emergency_contact_name.as_str(),
                        )),
                        emergency_contact_mobile: normalize_optional_text(Some(
                            row.emergency_contact_mobile.as_str(),
                        )),
                        ..Default::default()
                    },
                    value! { "id": user_id, "column": ["id_card", "emergency_contact_name", "emergency_contact_mobile"] },
                )
                .await
            };
            if let Err(error) = update_result {
                tracing::warn!(
                    error = ?error,
                    row_no,
                    user_id,
                    "failed to update existing imported technician user"
                );
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: technician_row_values(&row),
                    reason: "已有账号更新失败".to_string(),
                });
                continue;
            }
            if restoring_deleted_user
                && let Err(error) =
                    UserRoleEntity::delete_by_map(state.rb.as_ref(), value! { "user_id": user_id })
                        .await
            {
                tracing::warn!(
                    error = ?error,
                    row_no,
                    user_id,
                    "failed to clear roles for restored imported technician"
                );
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: technician_row_values(&row),
                    reason: "已删除账号恢复失败".to_string(),
                });
                continue;
            }
            if restoring_deleted_user
                && let Err(error) =
                    UserStoreEntity::delete_by_map(state.rb.as_ref(), value! { "user_id": user_id })
                        .await
            {
                tracing::warn!(
                    error = ?error,
                    row_no,
                    user_id,
                    "failed to clear stores for restored imported technician"
                );
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: technician_row_values(&row),
                    reason: "已删除账号恢复失败".to_string(),
                });
                continue;
            }
            user_id
        } else {
            let password_hash = match hash(&row.password, 10) {
                Ok(item) => item,
                Err(_) => {
                    failures.push(ImportFailureDetail {
                        row_no,
                        row_values: technician_row_values(&row),
                        reason: "密码处理失败".to_string(),
                    });
                    continue;
                }
            };
            if let Err(error) = UserEntity::insert(
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
                    job_no: Some(format!(
                        "SF{:08}",
                        Utc::now().timestamp_millis().rem_euclid(100_000_000)
                    )),
                    id_card: normalize_optional_text(Some(row.id_card.as_str())),
                    emergency_contact_name: normalize_optional_text(Some(
                        row.emergency_contact_name.as_str(),
                    )),
                    emergency_contact_mobile: normalize_optional_text(Some(
                        row.emergency_contact_mobile.as_str(),
                    )),
                    status: Some("enabled".to_string()),
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
            {
                tracing::warn!(
                    error = ?error,
                    row_no,
                    mobile = %row.mobile,
                    "failed to insert imported technician user"
                );
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: technician_row_values(&row),
                    reason: "师傅账号写入失败，请检查手机号、状态或表结构".to_string(),
                });
                continue;
            }
            match db::select_one_by_map_compat::<UserEntity>(
                state.rb.as_ref(),
                "user",
                value! { "mobile": row.mobile.as_str() },
            )
            .await
            {
                Ok(Some(user)) => user.id.unwrap_or_default(),
                Ok(None) => {
                    failures.push(ImportFailureDetail {
                        row_no,
                        row_values: technician_row_values(&row),
                        reason: "师傅账号写入后读取失败".to_string(),
                    });
                    continue;
                }
                Err(error) => {
                    tracing::warn!(
                        error = ?error,
                        row_no,
                        mobile = %row.mobile,
                        "failed to fetch imported technician user"
                    );
                    failures.push(ImportFailureDetail {
                        row_no,
                        row_values: technician_row_values(&row),
                        reason: "师傅账号写入后读取失败".to_string(),
                    });
                    continue;
                }
            }
        };

        if let Err(reason) =
            upsert_user_role_status(state, user_id, technician_role_id, &status, &row.name).await
        {
            failures.push(ImportFailureDetail {
                row_no,
                row_values: technician_row_values(&row),
                reason,
            });
            continue;
        }
        if let Err(reason) = replace_user_single_store(state, user_id, store_id).await {
            failures.push(ImportFailureDetail {
                row_no,
                row_values: technician_row_values(&row),
                reason,
            });
            continue;
        }
        if let Err(reason) =
            replace_user_technician_skills(state, user_id, &row.service_types).await
        {
            failures.push(ImportFailureDetail {
                row_no,
                row_values: technician_row_values(&row),
                reason,
            });
            continue;
        }
        auth::bump_user_session_version(state, user_id).await?;
        success_count += 1;
    }

    Ok(Json(ApiResponse::success(build_import_result(
        success_count,
        failures,
        "technicians-import-errors.xlsx",
        &[
            "Excel行号",
            "师傅姓名",
            "手机号",
            "密码",
            "身份证号",
            "状态",
            "紧急联系人",
            "紧急联系电话",
            "专业技能",
            "所属门店",
            "失败原因",
        ],
    )?)))
}

async fn import_user_rows(
    state: &AppState,
    rows: Vec<ImportedUserRow>,
    forced_role_codes: Option<Vec<String>>,
) -> Result<Json<ApiResponse<ImportResult>>, AppError> {
    let mut success_count = 0usize;
    let mut failures = Vec::new();
    let store_lookup = load_store_lookup(state).await?;

    for (index, mut row) in rows.into_iter().enumerate() {
        let row_no = index + 2;
        if row.role_codes.is_empty() {
            row.role_codes = forced_role_codes.clone().unwrap_or_default();
        }
        row.role_codes = dedup_non_empty(
            row.role_codes
                .into_iter()
                .map(|item| normalize_role_code_alias(&item)),
        );
        row.store_refs = dedup_non_empty(row.store_refs.into_iter());
        if row.password.trim().is_empty() {
            row.password = "123456".to_string();
        }
        if row.name.trim().is_empty() || row.mobile.trim().is_empty() {
            failures.push(ImportFailureDetail {
                row_no,
                row_values: user_row_values(&row),
                reason: "必填字段缺失：姓名、手机号不能为空".to_string(),
            });
            continue;
        }
        if row.password.trim().len() < 6 {
            failures.push(ImportFailureDetail {
                row_no,
                row_values: user_row_values(&row),
                reason: "密码长度不能少于6位".to_string(),
            });
            continue;
        }
        if row.role_codes.is_empty() {
            failures.push(ImportFailureDetail {
                row_no,
                row_values: user_row_values(&row),
                reason: "缺少角色编码".to_string(),
            });
            continue;
        }
        if let Err(error) = validate_import_store_binding(&row.role_codes, &row.store_refs) {
            failures.push(ImportFailureDetail {
                row_no,
                row_values: user_row_values(&row),
                reason: error,
            });
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
            failures.push(ImportFailureDetail {
                row_no,
                row_values: user_row_values(&row),
                reason: format!("角色不存在: {}", role_code),
            });
            continue;
        }

        let store_ids = match resolve_store_ids_from_lookup(&store_lookup, &row.store_refs) {
            Ok(store_ids) => store_ids,
            Err(AppError::BadRequest(message)) => {
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: user_row_values(&row),
                    reason: message,
                });
                continue;
            }
            Err(error) => return Err(error),
        };

        let existing_user: Option<UserEntity> = db::select_one_by_map_compat(
            state.rb.as_ref(),
            "user",
            value! { "mobile": row.mobile.as_str() },
        )
        .await
        .context("failed to check existing import user")
        .map_err(AppError::Internal)?;
        if existing_user
            .as_ref()
            .is_some_and(|user| user.deleted_at.is_none())
        {
            failures.push(ImportFailureDetail {
                row_no,
                row_values: user_row_values(&row),
                reason: format!("手机号已存在: {}", row.mobile),
            });
            continue;
        }

        let status = match normalize_user_status(Some(row.status.as_str())) {
            Ok(status) => status,
            Err(AppError::BadRequest(message)) => {
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: user_row_values(&row),
                    reason: message,
                });
                continue;
            }
            Err(error) => return Err(error),
        };

        let password_hash = match hash(&row.password, 10) {
            Ok(item) => item,
            Err(_) => {
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: user_row_values(&row),
                    reason: "密码处理失败".to_string(),
                });
                continue;
            }
        };

        let user_id = if let Some(user) = existing_user {
            let Some(user_id) = user.id else {
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: user_row_values(&row),
                    reason: "已删除账号缺少用户ID".to_string(),
                });
                continue;
            };
            if let Err(error) = UserEntity::update_by_map(
                state.rb.as_ref(),
                &UserEntity {
                    password_hash: Some(password_hash),
                    name: Some(row.name.clone()),
                    nickname: None,
                    email: None,
                    avatar_url: None,
                    job_no: None,
                    id_card: None,
                    emergency_contact_name: None,
                    emergency_contact_mobile: None,
                    status: Some(status.clone()),
                    last_login_at: None,
                    last_login_ip: None,
                    deleted_at: None,
                    wechat_openid: None,
                    wechat_unionid: None,
                    wechat_subscribe: Some(0),
                    wechat_bind_at: None,
                    wechat_subscribe_updated_at: None,
                    ..Default::default()
                },
                value! {
                    "id": user_id,
                    "column": [
                        "password_hash",
                        "name",
                        "nickname",
                        "email",
                        "avatar_url",
                        "job_no",
                        "id_card",
                        "emergency_contact_name",
                        "emergency_contact_mobile",
                        "status",
                        "last_login_at",
                        "last_login_ip",
                        "deleted_at",
                        "wechat_openid",
                        "wechat_unionid",
                        "wechat_subscribe",
                        "wechat_bind_at",
                        "wechat_subscribe_updated_at"
                    ]
                },
            )
            .await
            {
                tracing::warn!(
                    error = ?error,
                    row_no,
                    user_id,
                    "failed to restore imported user"
                );
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: user_row_values(&row),
                    reason: "已删除账号恢复失败".to_string(),
                });
                continue;
            }
            if let Err(error) =
                UserRoleEntity::delete_by_map(state.rb.as_ref(), value! { "user_id": user_id })
                    .await
            {
                tracing::warn!(error = ?error, row_no, user_id, "failed to clear restored user roles");
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: user_row_values(&row),
                    reason: "已删除账号角色恢复失败".to_string(),
                });
                continue;
            }
            if let Err(error) =
                UserStoreEntity::delete_by_map(state.rb.as_ref(), value! { "user_id": user_id })
                    .await
            {
                tracing::warn!(error = ?error, row_no, user_id, "failed to clear restored user stores");
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: user_row_values(&row),
                    reason: "已删除账号门店恢复失败".to_string(),
                });
                continue;
            }
            user_id
        } else {
            if let Err(error) = UserEntity::insert(
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
                    status: Some(status.clone()),
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
            {
                tracing::warn!(
                    error = ?error,
                    row_no,
                    mobile = %row.mobile,
                    "failed to insert imported user"
                );
                failures.push(ImportFailureDetail {
                    row_no,
                    row_values: user_row_values(&row),
                    reason: "账号写入失败，请检查手机号、状态或表结构".to_string(),
                });
                continue;
            }

            match db::select_one_by_map_compat::<UserEntity>(
                state.rb.as_ref(),
                "user",
                value! { "mobile": row.mobile.as_str() },
            )
            .await
            {
                Ok(Some(user)) => user.id.unwrap_or_default(),
                Ok(None) => {
                    failures.push(ImportFailureDetail {
                        row_no,
                        row_values: user_row_values(&row),
                        reason: "账号写入后读取失败".to_string(),
                    });
                    continue;
                }
                Err(error) => {
                    tracing::warn!(
                        error = ?error,
                        row_no,
                        mobile = %row.mobile,
                        "failed to fetch imported user"
                    );
                    failures.push(ImportFailureDetail {
                        row_no,
                        row_values: user_row_values(&row),
                        reason: "账号写入后读取失败".to_string(),
                    });
                    continue;
                }
            }
        };

        let mut binding_failed = None;
        for role_id in resolved_role_ids {
            if let Err(error) = UserRoleEntity::insert(
                state.rb.as_ref(),
                &UserRoleEntity {
                    id: None,
                    user_id: Some(user_id),
                    role_id: Some(role_id),
                    status: Some(status.clone()),
                    display_name: Some(row.name.clone()),
                    created_at: None,
                },
            )
            .await
            {
                tracing::warn!(
                    error = ?error,
                    row_no,
                    user_id,
                    role_id,
                    "failed to bind imported user role"
                );
                binding_failed = Some("账号已创建，但角色绑定失败".to_string());
                break;
            }
        }
        if binding_failed.is_none() {
            for store_id in store_ids {
                if let Err(error) = UserStoreEntity::insert(
                    state.rb.as_ref(),
                    &UserStoreEntity {
                        id: None,
                        user_id: Some(user_id),
                        store_id: Some(store_id),
                        created_at: None,
                    },
                )
                .await
                {
                    tracing::warn!(
                        error = ?error,
                        row_no,
                        user_id,
                        store_id,
                        "failed to bind imported user store"
                    );
                    binding_failed = Some("账号已创建，但门店绑定失败".to_string());
                    break;
                }
            }
        }
        if let Some(reason) = binding_failed {
            failures.push(ImportFailureDetail {
                row_no,
                row_values: user_row_values(&row),
                reason,
            });
            continue;
        }
        success_count += 1;
    }

    Ok(Json(ApiResponse::success(build_import_result(
        success_count,
        failures,
        "users-import-errors.xlsx",
        &[
            "Excel行号",
            "姓名",
            "手机号",
            "密码",
            "角色编码",
            "所属门店",
            "状态",
            "失败原因",
        ],
    )?)))
}

pub async fn import_scores(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["hq_reviewer", "super_admin"])?;
    let upload = read_import_multipart(multipart).await?;

    let ext = Path::new(&upload.file_name)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let rows = if looks_like_excel(&upload.bytes) {
        parse_score_excel(&upload.file_name, &upload.bytes)?
    } else if ext == "csv" {
        parse_score_csv(&upload.bytes)?
    } else if ["xls", "xlsx"].contains(&ext.as_str()) {
        parse_score_excel(&upload.file_name, &upload.bytes)?
    } else {
        return Err(AppError::BadRequest(
            "评分导入仅支持 csv/xls/xlsx 文件".to_string(),
        ));
    };

    let mut success_count = 0usize;
    let mut results = Vec::new();

    for (index, row) in rows.into_iter().enumerate() {
        let row_no = index + 2;
        let Some(service_type) = normalize_score_service_type(row.service_type.as_deref()) else {
            push_score_import_failure(
                &mut results,
                row_no,
                &row,
                "服务环节不能为空，且只能填写量尺、安装、熨烫、除尘除螨",
            );
            continue;
        };
        let task = match resolve_score_import_task(&state, &row, &service_type).await? {
            ScoreImportTaskResolution::Matched(task) => task,
            ScoreImportTaskResolution::Failed(reason) => {
                push_score_import_failure(&mut results, row_no, &row, &reason);
                continue;
            }
        };
        let task_id = task.id.unwrap_or_default();
        let service_types = load_task_service_types(&state, &task).await?;
        if !service_types.iter().any(|item| item == &service_type) {
            push_score_import_failure(
                &mut results,
                row_no,
                &row,
                &format!(
                    "服务环节与任务不匹配: 任务{}不包含{}",
                    task_id,
                    score_service_type_text(&service_type)
                ),
            );
            continue;
        }
        let Some(score) = row.score else {
            let reason = if row.score_text.trim().is_empty() {
                "总部评分不能为空"
            } else {
                "总部评分格式错误，只能填写0到100之间的整数"
            };
            push_score_import_failure(&mut results, row_no, &row, reason);
            continue;
        };
        if !(0..=100).contains(&score) {
            push_score_import_failure(&mut results, row_no, &row, "总部评分必须是0到100之间的整数");
            continue;
        }
        if !matches!(
            task.task_status.as_deref(),
            Some("pending_dealer_review") | Some("pending_hq_score") | Some("completed")
        ) {
            push_score_import_failure(
                &mut results,
                row_no,
                &row,
                &format!(
                    "任务状态不允许评分: {}",
                    task.task_status.as_deref().unwrap_or("空")
                ),
            );
            continue;
        }

        let existing_score_item: Option<HqScoreItemEntity> = db::select_one_by_map_compat(
            state.rb.as_ref(),
            "hq_score_item",
            value! { "task_id": task_id, "service_type": service_type.clone() },
        )
        .await
        .context("failed to query existing score item during import")
        .map_err(AppError::Internal)?;
        let overwritten = existing_score_item.is_some();
        if let Some(existing_score_item) = existing_score_item {
            HqScoreItemEntity::delete_by_map(
                state.rb.as_ref(),
                value! { "id": existing_score_item.id.unwrap_or_default() },
            )
            .await
            .context("failed to delete existing imported score item")
            .map_err(AppError::Internal)?;
        }

        let hq_score = upsert_import_hq_score(
            &state,
            task_id,
            auth_user.user_id,
            score,
            row.score_remark.clone(),
        )
        .await?;

        HqScoreItemEntity::insert(
            state.rb.as_ref(),
            &HqScoreItemEntity {
                id: None,
                hq_score_id: hq_score.id,
                task_id: Some(task_id),
                service_type: Some(service_type.clone()),
                score: Some(score),
                deduction_reason: row.deduction_reason.clone(),
                created_at: None,
            },
        )
        .await
        .context("failed to insert imported hq score item")
        .map_err(AppError::Internal)?;
        refresh_import_hq_score_summary(&state, task_id).await?;

        TaskEntity::update_by_map(
            state.rb.as_ref(),
            &TaskEntity {
                score_status: Some("scored".to_string()),
                ..Default::default()
            },
            value! { "id": task_id, "column": ["score_status"] },
        )
        .await
        .context("failed to update task after score import")
        .map_err(AppError::Internal)?;

        success_count += 1;
        results.push(ScoreImportResultRow {
            row_no,
            customer_mobile: row.customer_mobile.clone(),
            matched_task_text: task.task_no.clone().unwrap_or_else(|| task_id.to_string()),
            service_type_text: score_service_type_text(&service_type).to_string(),
            score_text: score.to_string(),
            deduction_reason: row.deduction_reason.clone().unwrap_or_default(),
            score_remark: row.score_remark.clone().unwrap_or_default(),
            result: "成功".to_string(),
            reason: if overwritten {
                "导入成功，已覆盖该任务原总部评分".to_string()
            } else {
                "导入成功".to_string()
            },
        });
    }

    build_score_import_result_response(success_count, results, "scores-import-result.xlsx")
}

#[derive(Debug)]
struct ScoreImportRow {
    task_id: Option<i64>,
    task_id_text: String,
    customer_mobile: String,
    service_type: Option<String>,
    score: Option<i32>,
    score_text: String,
    deduction_reason: Option<String>,
    score_remark: Option<String>,
}

#[derive(Debug)]
struct ScoreImportResultRow {
    row_no: usize,
    customer_mobile: String,
    matched_task_text: String,
    service_type_text: String,
    score_text: String,
    deduction_reason: String,
    score_remark: String,
    result: String,
    reason: String,
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
        .map_err(AppError::from_multipart_error)?
    {
        let name = field.name().map(ToString::to_string).unwrap_or_default();
        if let Some(current_file_name) = field.file_name().map(ToString::to_string) {
            let file_bytes = field
                .bytes()
                .await
                .map_err(AppError::from_multipart_error)?
                .to_vec();
            file_name = Some(current_file_name);
            bytes = Some(file_bytes);
        } else {
            let text = field.text().await.map_err(AppError::from_multipart_error)?;
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

    if looks_like_excel(&upload.bytes) {
        return parse_user_excel(&upload.file_name, &upload.bytes);
    }
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

fn parse_technician_import_rows(
    upload: &UploadField,
) -> Result<Vec<ImportedTechnicianRow>, AppError> {
    let ext = Path::new(&upload.file_name)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if looks_like_excel(&upload.bytes) {
        return parse_generic_excel_rows(
            &upload.file_name,
            &upload.bytes,
            parse_technician_import_row_values,
        );
    }
    if ext == "csv" {
        return parse_generic_csv_rows(&upload.bytes, parse_technician_import_row_values);
    }
    if ["xls", "xlsx"].contains(&ext.as_str()) {
        return parse_generic_excel_rows(
            &upload.file_name,
            &upload.bytes,
            parse_technician_import_row_values,
        );
    }

    Err(AppError::BadRequest(
        "师傅导入仅支持 csv/xls/xlsx 文件".to_string(),
    ))
}

fn parse_user_csv(bytes: &[u8]) -> Result<Vec<ImportedUserRow>, AppError> {
    let content = decode_csv_content(bytes, "用户导入")?;
    let mut lines = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .peekable();
    if matches!(lines.peek(), Some(line) if line.eq_ignore_ascii_case("sep=,")) {
        let _ = lines.next();
    }
    let headers = split_csv_values(
        lines
            .next()
            .ok_or_else(|| AppError::BadRequest("导入文件不能为空".to_string()))?,
    );
    let header_map = build_header_index_map(&headers);
    let mut rows = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let cols = split_csv_values(line);
        rows.push(parse_import_row_values(&header_map, &cols));
    }
    Ok(rows)
}

fn parse_user_excel(file_name: &str, bytes: &[u8]) -> Result<Vec<ImportedUserRow>, AppError> {
    let temp_path = temp_import_file_path(file_name, bytes);
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
        let normalized = normalize_import_header(header);
        if !normalized.is_empty() {
            header_map.insert(normalized, index);
        }
    }
    header_map
}

fn normalize_import_header(header: &str) -> String {
    header
        .trim()
        .trim_end_matches('*')
        .trim()
        .to_ascii_lowercase()
}

fn build_template_workbook_response(
    file_name: &str,
    headers: &[&str],
    sample: &[&str],
    instructions: &[&str],
) -> Result<Response, AppError> {
    let workbook_bytes = build_template_workbook_bytes(headers, sample, instructions)?;
    let mut response = Response::new(Body::from(workbook_bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{}\"", file_name))
            .map_err(|_| AppError::Internal(anyhow!("invalid content disposition header")))?,
    );
    Ok(response)
}

fn build_template_workbook_bytes(
    headers: &[&str],
    sample: &[&str],
    instructions: &[&str],
) -> Result<Vec<u8>, AppError> {
    let mut workbook = Workbook::new();
    let header_format = Format::new()
        .set_bold()
        .set_font_color(Color::White)
        .set_background_color(Color::RGB(0x1F4E78))
        .set_border(FormatBorder::Thin)
        .set_align(FormatAlign::Center)
        .set_align(FormatAlign::VerticalCenter);
    let cell_format = Format::new()
        .set_border(FormatBorder::Thin)
        .set_align(FormatAlign::VerticalCenter);
    let wrap_format = Format::new()
        .set_text_wrap()
        .set_align(FormatAlign::VerticalCenter);

    let sheet = workbook
        .add_worksheet()
        .set_name("导入模板")
        .context("failed to create import template worksheet")
        .map_err(AppError::Internal)?;
    for (index, header_text) in headers.iter().enumerate() {
        sheet
            .write_string_with_format(0, index as u16, *header_text, &header_format)
            .context("failed to write template header")
            .map_err(AppError::Internal)?;
        sheet
            .set_column_width(index as u16, 18)
            .context("failed to set template column width")
            .map_err(AppError::Internal)?;
    }
    for (index, sample_text) in sample.iter().enumerate() {
        sheet
            .write_string_with_format(1, index as u16, *sample_text, &cell_format)
            .context("failed to write template sample")
            .map_err(AppError::Internal)?;
    }
    sheet
        .set_freeze_panes(1, 0)
        .context("failed to freeze template header row")
        .map_err(AppError::Internal)?;

    let note_sheet = workbook
        .add_worksheet()
        .set_name("填写说明")
        .context("failed to create import template note worksheet")
        .map_err(AppError::Internal)?;
    note_sheet
        .write_string_with_format(0, 0, "说明", &header_format)
        .context("failed to write template note header")
        .map_err(AppError::Internal)?;
    note_sheet
        .set_column_width(0, 80)
        .context("failed to set template note width")
        .map_err(AppError::Internal)?;
    for (index, instruction) in instructions.iter().enumerate() {
        note_sheet
            .write_string_with_format((index + 1) as u32, 0, *instruction, &wrap_format)
            .context("failed to write template note")
            .map_err(AppError::Internal)?;
    }

    workbook
        .save_to_buffer()
        .context("failed to render import template workbook")
        .map_err(AppError::Internal)
}

fn build_import_result(
    success_count: usize,
    failures: Vec<ImportFailureDetail>,
    error_file_name: &str,
    headers: &[&str],
) -> Result<ImportResult, AppError> {
    let errors = failures
        .iter()
        .map(|item| format!("第{}行：{}", item.row_no, item.reason))
        .collect::<Vec<_>>();
    let failed_count = failures.len();
    let error_file_base64 = if failures.is_empty() {
        None
    } else {
        Some(STANDARD.encode(build_import_failure_workbook_bytes(headers, &failures)?))
    };

    Ok(ImportResult {
        success_count,
        failed_count,
        errors,
        error_file_name: if failed_count == 0 {
            None
        } else {
            Some(error_file_name.to_string())
        },
        error_file_base64,
    })
}

fn build_import_failure_workbook_bytes(
    headers: &[&str],
    failures: &[ImportFailureDetail],
) -> Result<Vec<u8>, AppError> {
    let mut workbook = Workbook::new();
    let header_format = Format::new()
        .set_bold()
        .set_font_color(Color::White)
        .set_background_color(Color::RGB(0xC0504D))
        .set_border(FormatBorder::Thin)
        .set_align(FormatAlign::Center)
        .set_align(FormatAlign::VerticalCenter);
    let cell_format = Format::new()
        .set_border(FormatBorder::Thin)
        .set_align(FormatAlign::VerticalCenter)
        .set_text_wrap();
    let sheet = workbook
        .add_worksheet()
        .set_name("导入失败明细")
        .context("failed to create import error worksheet")
        .map_err(AppError::Internal)?;
    for (index, header_text) in headers.iter().enumerate() {
        sheet
            .write_string_with_format(0, index as u16, *header_text, &header_format)
            .context("failed to write import error header")
            .map_err(AppError::Internal)?;
        sheet
            .set_column_width(index as u16, 18)
            .context("failed to set import error width")
            .map_err(AppError::Internal)?;
    }
    for (row_index, failure) in failures.iter().enumerate() {
        let excel_row = (row_index + 1) as u32;
        let mut values = failure.row_values.clone();
        if values.len() < headers.len() {
            values.resize(headers.len(), String::new());
        }
        if !values.is_empty() {
            values[0] = failure.row_no.to_string();
        }
        if !headers.is_empty() {
            values[headers.len() - 1] = failure.reason.clone();
        }
        for col_index in 0..headers.len() {
            let value = values
                .get(col_index)
                .map(String::as_str)
                .unwrap_or_default();
            sheet
                .write_string_with_format(excel_row, col_index as u16, value, &cell_format)
                .context("failed to write import error cell")
                .map_err(AppError::Internal)?;
        }
    }
    workbook
        .save_to_buffer()
        .context("failed to render import error workbook")
        .map_err(AppError::Internal)
}

fn build_score_import_result_response(
    success_count: usize,
    results: Vec<ScoreImportResultRow>,
    file_name: &str,
) -> Result<Response, AppError> {
    let failed_count = results.iter().filter(|item| item.result == "失败").count();
    let workbook_bytes =
        build_score_import_result_workbook_bytes(success_count, failed_count, &results)?;
    let mut response = Response::new(Body::from(workbook_bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{}\"", file_name))
            .map_err(|_| AppError::Internal(anyhow!("invalid content disposition header")))?,
    );
    response.headers_mut().insert(
        "x-import-success-count",
        HeaderValue::from_str(&success_count.to_string())
            .map_err(|_| AppError::Internal(anyhow!("invalid import success count header")))?,
    );
    response.headers_mut().insert(
        "x-import-failed-count",
        HeaderValue::from_str(&failed_count.to_string())
            .map_err(|_| AppError::Internal(anyhow!("invalid import failed count header")))?,
    );
    Ok(response)
}

fn build_score_import_result_workbook_bytes(
    success_count: usize,
    failed_count: usize,
    results: &[ScoreImportResultRow],
) -> Result<Vec<u8>, AppError> {
    let mut workbook = Workbook::new();
    let title_format = Format::new()
        .set_bold()
        .set_font_size(14.0)
        .set_align(FormatAlign::VerticalCenter);
    let header_format = Format::new()
        .set_bold()
        .set_font_color(Color::White)
        .set_background_color(Color::RGB(0x1F4E78))
        .set_border(FormatBorder::Thin)
        .set_align(FormatAlign::Center)
        .set_align(FormatAlign::VerticalCenter);
    let success_format = Format::new()
        .set_font_color(Color::RGB(0x008000))
        .set_border(FormatBorder::Thin)
        .set_align(FormatAlign::VerticalCenter)
        .set_text_wrap();
    let failure_format = Format::new()
        .set_font_color(Color::RGB(0xC00000))
        .set_border(FormatBorder::Thin)
        .set_align(FormatAlign::VerticalCenter)
        .set_text_wrap();
    let cell_format = Format::new()
        .set_border(FormatBorder::Thin)
        .set_align(FormatAlign::VerticalCenter)
        .set_text_wrap();

    let sheet = workbook
        .add_worksheet()
        .set_name("总部评分导入结果")
        .context("failed to create score import result worksheet")
        .map_err(AppError::Internal)?;

    sheet
        .write_string_with_format(0, 0, "总部评分导入结果", &title_format)
        .context("failed to write score import result title")
        .map_err(AppError::Internal)?;
    sheet
        .write_string_with_format(
            1,
            0,
            format!("成功{}条，失败{}条", success_count, failed_count),
            &cell_format,
        )
        .context("failed to write score import result summary")
        .map_err(AppError::Internal)?;

    let headers = [
        "Excel行号",
        "客户手机号",
        "匹配任务",
        "服务环节",
        "总部评分",
        "扣分原因",
        "评分备注",
        "导入结果",
        "原因",
    ];
    for (index, header_text) in headers.iter().enumerate() {
        sheet
            .write_string_with_format(3, index as u16, *header_text, &header_format)
            .context("failed to write score import result header")
            .map_err(AppError::Internal)?;
        let width = match index {
            1 | 2 | 8 => 28,
            5 | 6 => 24,
            _ => 16,
        };
        sheet
            .set_column_width(index as u16, width)
            .context("failed to set score import result column width")
            .map_err(AppError::Internal)?;
    }

    for (index, item) in results.iter().enumerate() {
        let excel_row = (index + 4) as u32;
        let values = [
            item.row_no.to_string(),
            item.customer_mobile.clone(),
            item.matched_task_text.clone(),
            item.service_type_text.clone(),
            item.score_text.clone(),
            item.deduction_reason.clone(),
            item.score_remark.clone(),
            item.result.clone(),
            item.reason.clone(),
        ];
        let row_format = if item.result == "失败" {
            &failure_format
        } else {
            &success_format
        };
        for (col_index, value) in values.iter().enumerate() {
            let format = if col_index >= 7 {
                row_format
            } else {
                &cell_format
            };
            sheet
                .write_string_with_format(excel_row, col_index as u16, value, format)
                .context("failed to write score import result cell")
                .map_err(AppError::Internal)?;
        }
    }

    workbook
        .save_to_buffer()
        .context("failed to render score import result workbook")
        .map_err(AppError::Internal)
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

fn looks_like_excel(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0])
}

fn temp_import_file_path(file_name: &str, bytes: &[u8]) -> PathBuf {
    let extension = if bytes.starts_with(b"PK\x03\x04") {
        "xlsx"
    } else if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        "xls"
    } else {
        Path::new(file_name)
            .extension()
            .and_then(|item| item.to_str())
            .unwrap_or("tmp")
    };
    std::env::temp_dir().join(format!("{}-import.{}", Uuid::new_v4(), extension))
}

fn decode_csv_content(bytes: &[u8], label: &str) -> Result<String, AppError> {
    let bytes = strip_utf8_bom(bytes);
    if let Ok(content) = std::str::from_utf8(bytes) {
        return Ok(content.to_string());
    }

    if let Some(content) = decode_utf16_bom_csv(bytes)? {
        return Ok(content);
    }

    let (content, _, _) = GBK.decode(bytes);
    let content = content.into_owned();
    if !content.trim().is_empty() {
        return Ok(content);
    }

    let prefix = if label.trim().is_empty() {
        String::new()
    } else {
        label.trim().to_string()
    };
    Err(AppError::BadRequest(format!(
        "{prefix}CSV文件编码不支持，请使用UTF-8、UTF-8 BOM或GBK编码，或直接上传系统下载的xlsx模板"
    )))
}

fn decode_utf16_bom_csv(bytes: &[u8]) -> Result<Option<String>, AppError> {
    let (payload, little_endian) = if let Some(payload) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        (payload, true)
    } else if let Some(payload) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        (payload, false)
    } else {
        return Ok(None);
    };

    if payload.len() % 2 != 0 {
        return Err(AppError::BadRequest(
            "CSV文件编码不完整，请重新导出后再上传".to_string(),
        ));
    }

    let code_units = payload
        .chunks_exact(2)
        .map(|chunk| {
            if little_endian {
                u16::from_le_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_be_bytes([chunk[0], chunk[1]])
            }
        })
        .collect::<Vec<_>>();

    String::from_utf16(&code_units)
        .map(Some)
        .map_err(|_| AppError::BadRequest("CSV文件编码不支持，请另存为UTF-8后上传".to_string()))
}

fn split_csv_values(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                current.push('"');
                let _ = chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                values.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    values.push(current.trim().to_string());
    values
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
        store_refs: split_multi_values(&store_value),
        status: if status.trim().is_empty() {
            "enabled".to_string()
        } else {
            status
        },
    }
}

fn parse_technician_import_row_values(
    header_map: &HashMap<String, usize>,
    cols: &[String],
) -> ImportedTechnicianRow {
    let service_types = value_by_keys(
        header_map,
        cols,
        &[
            "service_types",
            "service_type",
            "专业技能",
            "服务能力",
            "技能",
            "服务类型",
        ],
    );
    let store_ref = value_by_keys(
        header_map,
        cols,
        &["store_id", "store_ids", "门店id", "所属门店", "门店"],
    );
    let status = value_by_keys(header_map, cols, &["status", "状态", "账号状态"]);

    ImportedTechnicianRow {
        name: value_by_keys(header_map, cols, &["name", "姓名", "师傅姓名", "技师姓名"]),
        mobile: value_by_keys(header_map, cols, &["mobile", "手机号", "联系电话", "电话"]),
        password: value_by_keys(header_map, cols, &["password", "密码", "设置密码"]),
        id_card: value_by_keys(header_map, cols, &["id_card", "身份证号", "身份证"]),
        status: if status.trim().is_empty() {
            "enabled".to_string()
        } else {
            status
        },
        emergency_contact_name: value_by_keys(
            header_map,
            cols,
            &["emergency_contact_name", "紧急联系人"],
        ),
        emergency_contact_mobile: value_by_keys(
            header_map,
            cols,
            &["emergency_contact_mobile", "紧急联系电话", "紧急联系人电话"],
        ),
        service_types: split_multi_values(&service_types),
        store_ref,
    }
}

fn parse_customer_import_row_values(
    header_map: &HashMap<String, usize>,
    cols: &[String],
) -> ImportedCustomerRow {
    ImportedCustomerRow {
        name: value_by_keys(header_map, cols, &["customer_name", "name", "客户姓名"]),
        mobile: value_by_keys(
            header_map,
            cols,
            &["customer_mobile", "mobile", "客户手机号", "手机号"],
        ),
        source: value_by_keys(header_map, cols, &["source", "客户来源"]),
        store_ref: normalize_optional_text(Some(
            value_by_keys(
                header_map,
                cols,
                &[
                    "store_id",
                    "store_code",
                    "store_name",
                    "所属门店",
                    "所属门店名称",
                    "所属门店编码",
                    "所属门店id",
                    "门店id",
                    "门店编码",
                    "门店名称",
                ],
            )
            .as_str(),
        )),
        province: normalize_optional_text(Some(
            value_by_keys(header_map, cols, &["province", "省"]).as_str(),
        )),
        city: normalize_optional_text(Some(
            value_by_keys(header_map, cols, &["city", "市"]).as_str(),
        )),
        district: normalize_optional_text(Some(
            value_by_keys(header_map, cols, &["district", "区"]).as_str(),
        )),
        address: value_by_keys(header_map, cols, &["address", "详细地址", "地址"]),
        remark: value_by_keys(header_map, cols, &["remark", "备注"]),
    }
}

fn parse_store_import_row_values(
    header_map: &HashMap<String, usize>,
    cols: &[String],
) -> ImportedStoreRow {
    ImportedStoreRow {
        store_name: value_by_keys(header_map, cols, &["store_name", "name", "门店名称"]),
        code: value_by_keys(header_map, cols, &["code", "门店编码"]),
        province: normalize_optional_text(Some(
            value_by_keys(header_map, cols, &["province", "省"]).as_str(),
        )),
        city: normalize_optional_text(Some(
            value_by_keys(header_map, cols, &["city", "市"]).as_str(),
        )),
        district: normalize_optional_text(Some(
            value_by_keys(header_map, cols, &["district", "区"]).as_str(),
        )),
        address: value_by_keys(header_map, cols, &["address", "详细地址", "地址"]),
        contact_name: value_by_keys(header_map, cols, &["contact_name", "联系人"]),
        contact_mobile: value_by_keys(
            header_map,
            cols,
            &["contact_mobile", "联系电话", "联系人电话"],
        ),
        business_hours: value_by_keys(header_map, cols, &["business_hours", "营业时间"]),
        remark: value_by_keys(header_map, cols, &["remark", "备注"]),
        status: value_by_keys(header_map, cols, &["status", "状态"]),
    }
}

fn parse_customer_import_rows(upload: &UploadField) -> Result<Vec<ImportedCustomerRow>, AppError> {
    parse_generic_rows(
        upload,
        parse_customer_import_row_values,
        "客户导入仅支持 csv/xls/xlsx 文件",
    )
}

fn parse_store_import_rows(upload: &UploadField) -> Result<Vec<ImportedStoreRow>, AppError> {
    parse_generic_rows(
        upload,
        parse_store_import_row_values,
        "门店导入仅支持 csv/xls/xlsx 文件",
    )
}

fn parse_generic_rows<T>(
    upload: &UploadField,
    mapper: fn(&HashMap<String, usize>, &[String]) -> T,
    invalid_file_message: &str,
) -> Result<Vec<T>, AppError> {
    let ext = Path::new(&upload.file_name)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if looks_like_excel(&upload.bytes) {
        return parse_generic_excel_rows(&upload.file_name, &upload.bytes, mapper);
    }
    if ext == "csv" {
        return parse_generic_csv_rows(&upload.bytes, mapper);
    }
    if ["xls", "xlsx"].contains(&ext.as_str()) {
        return parse_generic_excel_rows(&upload.file_name, &upload.bytes, mapper);
    }
    Err(AppError::BadRequest(invalid_file_message.to_string()))
}

fn parse_generic_csv_rows<T>(
    bytes: &[u8],
    mapper: fn(&HashMap<String, usize>, &[String]) -> T,
) -> Result<Vec<T>, AppError> {
    let content = decode_csv_content(bytes, "")?;
    let mut lines = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .peekable();
    if matches!(lines.peek(), Some(line) if line.eq_ignore_ascii_case("sep=,")) {
        let _ = lines.next();
    }
    let headers = split_csv_values(
        lines
            .next()
            .ok_or_else(|| AppError::BadRequest("导入文件不能为空".to_string()))?,
    );
    let header_map = build_header_index_map(&headers);
    let mut rows = Vec::new();
    for line in lines {
        let cols = split_csv_values(line);
        rows.push(mapper(&header_map, &cols));
    }
    Ok(rows)
}

fn parse_generic_excel_rows<T>(
    file_name: &str,
    bytes: &[u8],
    mapper: fn(&HashMap<String, usize>, &[String]) -> T,
) -> Result<Vec<T>, AppError> {
    let temp_path = temp_import_file_path(file_name, bytes);
    {
        let mut file = std::fs::File::create(&temp_path)
            .map_err(|_| AppError::Internal(anyhow!("failed to create temp import file")))?;
        file.write_all(bytes)
            .map_err(|_| AppError::Internal(anyhow!("failed to write temp import file")))?;
    }
    let result = parse_generic_excel_file(&temp_path, mapper);
    let _ = std::fs::remove_file(&temp_path);
    result
}

fn parse_generic_excel_file<T>(
    path: &Path,
    mapper: fn(&HashMap<String, usize>, &[String]) -> T,
) -> Result<Vec<T>, AppError> {
    let mut workbook = open_workbook_auto(path)
        .map_err(|_| AppError::BadRequest("无法读取导入文件".to_string()))?;
    let sheet_name = workbook
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| AppError::BadRequest("导入文件没有工作表".to_string()))?;
    let range = workbook
        .worksheet_range(&sheet_name)
        .map_err(|_| AppError::BadRequest("读取导入工作表失败".to_string()))?;
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
        rows.push(mapper(&header_map, &cols));
    }
    Ok(rows)
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

fn dedup_non_empty<I>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() || !seen.insert(value.to_string()) {
            continue;
        }
        result.push(value.to_string());
    }
    result
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
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

fn upload_defaults_to_technician_import(fields: &HashMap<String, String>) -> bool {
    fields
        .get("import_type")
        .map(|item| item.trim().eq_ignore_ascii_case("technician") || item.trim() == "师傅")
        .unwrap_or(false)
        || resolve_default_role_codes(fields)
            .into_iter()
            .map(|item| normalize_role_code_alias(&item))
            .any(|item| item == "technician")
}

fn normalize_role_code_alias(role_code: &str) -> String {
    match role_code.trim().to_ascii_lowercase().as_str() {
        "admin" | "role-001" => "super_admin".to_string(),
        "inspection_admin" | "role-002" => "hq_reviewer".to_string(),
        "dealer_manager" | "role-003" => "dealer".to_string(),
        "readonly_admin" | "role-004" => "hq_reviewer".to_string(),
        "technician" | "tech" | "master" | "worker" | "师傅" | "技师" => {
            "technician".to_string()
        }
        _ => role_code.trim().to_string(),
    }
}

fn normalize_import_service_types(values: &[String]) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    for value in values {
        let Some(normalized) = normalize_import_service_type(value) else {
            return Err(format!("不支持的专业技能: {}", value));
        };
        if !result.iter().any(|item| item == &normalized) {
            result.push(normalized);
        }
    }
    Ok(result)
}

fn normalize_import_service_type(value: &str) -> Option<String> {
    let key = value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let normalized = match key.as_str() {
        "measure" | "measurement" | "量尺" | "量尺服务" => "measure",
        "install" | "installation" | "安装" | "安装服务" => "install",
        "ironing" | "iron" | "熨烫" | "熨烫服务" => "ironing",
        "cleaning" | "clean" | "mite_removal" | "除尘除螨" | "除尘除螨服务" | "除尘" | "除螨" => {
            "cleaning"
        }
        _ => return None,
    };
    Some(normalized.to_string())
}

fn validate_import_store_binding(
    role_codes: &[String],
    store_refs: &[String],
) -> Result<(), String> {
    let requires_single_store = role_codes
        .iter()
        .any(|role| role == "dealer" || role == "technician");
    if requires_single_store && store_refs.len() != 1 {
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

fn normalize_general_status(value: Option<&str>) -> Result<String, AppError> {
    let raw = value.unwrap_or("enabled").trim();
    match raw {
        "" | "启用" => Ok("enabled".to_string()),
        "禁用" => Ok("disabled".to_string()),
        _ => match raw.to_ascii_lowercase().as_str() {
            "enabled" | "active" => Ok("enabled".to_string()),
            "disabled" | "inactive" => Ok("disabled".to_string()),
            _ => Err(AppError::BadRequest(
                "状态只能为启用/禁用或enabled/disabled或active/inactive".to_string(),
            )),
        },
    }
}

async fn resolve_customer_source_value_for_import(
    state: &AppState,
    source: &str,
) -> Result<String, AppError> {
    let normalized = source.trim();
    if normalized.is_empty() {
        return Ok("manual".to_string());
    }
    match normalized {
        "手工录入" | "手动录入" | "人工录入" | "后台录入" => {
            return Ok("manual".to_string());
        }
        _ => {}
    }
    let customer_sources: Vec<CustomerSourceEntity> =
        match db::select_by_map_compat(state.rb.as_ref(), "customer_source", value! {}).await {
            Ok(items) => items,
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    "failed to query customer sources during import; using raw source value"
                );
                return Ok(normalized.to_string());
            }
        };
    if let Some(found) = customer_sources.into_iter().find(|item| {
        item.code.as_deref() == Some(normalized) || item.name.as_deref() == Some(normalized)
    }) {
        return Ok(found
            .code
            .or(found.name)
            .unwrap_or_else(|| normalized.to_string()));
    }
    Ok(normalized.to_string())
}

async fn load_store_lookup(state: &AppState) -> Result<StoreLookup, AppError> {
    let stores: Vec<StoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "store", value! {})
        .await
        .context("failed to query stores for import lookup")
        .map_err(AppError::Internal)?;
    let mut lookup = StoreLookup::default();
    for store in stores {
        let Some(store_id) = store.id else {
            continue;
        };
        if let Some(name) = store
            .store_name
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            lookup
                .ids_by_name
                .entry(normalize_store_lookup_key(name))
                .or_default()
                .push(store_id);
        }
        if let Some(code) = store
            .code
            .as_deref()
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            lookup
                .ids_by_code
                .entry(normalize_store_lookup_key(code))
                .or_default()
                .push(store_id);
        }
        lookup.by_id.insert(store_id, store);
    }
    Ok(lookup)
}

fn normalize_store_lookup_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn resolve_optional_store_id_from_lookup(
    store_lookup: &StoreLookup,
    store_ref: Option<&str>,
) -> Result<Option<i64>, AppError> {
    let Some(store_ref) = store_ref.map(str::trim).filter(|item| !item.is_empty()) else {
        return Ok(None);
    };
    let resolved = resolve_single_store_ref(store_lookup, store_ref)?;
    Ok(Some(resolved))
}

fn resolve_store_ids_from_lookup(
    store_lookup: &StoreLookup,
    store_refs: &[String],
) -> Result<Vec<i64>, AppError> {
    let mut resolved = BTreeSet::new();
    for store_ref in store_refs {
        let store_ref = store_ref.trim();
        if store_ref.is_empty() {
            continue;
        }
        resolved.insert(resolve_single_store_ref(store_lookup, store_ref)?);
    }
    Ok(resolved.into_iter().collect())
}

fn resolve_single_store_ref(store_lookup: &StoreLookup, store_ref: &str) -> Result<i64, AppError> {
    if let Ok(store_id) = store_ref.trim().parse::<i64>()
        && store_lookup.by_id.contains_key(&store_id)
    {
        return Ok(store_id);
    }

    let code_key = normalize_store_lookup_key(store_ref);
    if let Some(store_ids) = store_lookup.ids_by_code.get(&code_key) {
        return resolve_unique_store_match(store_ref, store_ids);
    }
    if let Some(store_ids) = store_lookup.ids_by_name.get(&code_key) {
        return resolve_unique_store_match(store_ref, store_ids);
    }

    Err(AppError::BadRequest(format!(
        "所属门店不存在: {}",
        store_ref
    )))
}

fn resolve_unique_store_match(store_ref: &str, store_ids: &[i64]) -> Result<i64, AppError> {
    match store_ids {
        [store_id] => Ok(*store_id),
        [] => Err(AppError::BadRequest(format!(
            "所属门店不存在: {}",
            store_ref
        ))),
        _ => Err(AppError::BadRequest(format!(
            "所属门店匹配到多个门店，请改填门店编码: {}",
            store_ref
        ))),
    }
}

async fn upsert_user_role_status(
    state: &AppState,
    user_id: i64,
    role_id: i64,
    status: &str,
    display_name: &str,
) -> Result<(), String> {
    let existing: Vec<UserRoleEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": user_id, "role_id": role_id },
    )
    .await
    .map_err(|error| {
        tracing::warn!(
            error = ?error,
            user_id,
            role_id,
            "failed to query imported technician role binding"
        );
        "师傅角色绑定查询失败".to_string()
    })?;

    if existing.is_empty() {
        UserRoleEntity::insert(
            state.rb.as_ref(),
            &UserRoleEntity {
                id: None,
                user_id: Some(user_id),
                role_id: Some(role_id),
                status: Some(status.to_string()),
                display_name: Some(display_name.trim().to_string()),
                created_at: None,
            },
        )
        .await
        .map_err(|error| {
            tracing::warn!(
                error = ?error,
                user_id,
                role_id,
                "failed to insert imported technician role binding"
            );
            "师傅角色绑定失败".to_string()
        })?;
        return Ok(());
    }

    for relation in existing {
        if let Some(id) = relation.id {
            state
                .rb
                .as_ref()
                .exec(
                    "update `user_role` set `status` = ?, `display_name` = ? where `id` = ?",
                    vec![value!(status), value!(display_name.trim()), value!(id)],
                )
                .await
                .map_err(|error| {
                    tracing::warn!(
                        error = ?error,
                        user_id,
                        role_id,
                        relation_id = id,
                        "failed to update imported technician role status"
                    );
                    "师傅角色状态更新失败".to_string()
                })?;
        }
    }
    Ok(())
}

async fn replace_user_single_store(
    state: &AppState,
    user_id: i64,
    store_id: i64,
) -> Result<(), String> {
    let existing: Vec<UserStoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_store",
        value! { "user_id": user_id },
    )
    .await
    .map_err(|error| {
        tracing::warn!(
            error = ?error,
            user_id,
            "failed to query imported technician store bindings"
        );
        "师傅门店绑定查询失败".to_string()
    })?;

    for relation in existing {
        if let Some(id) = relation.id {
            UserStoreEntity::delete_by_map(state.rb.as_ref(), value! { "id": id })
                .await
                .map_err(|error| {
                    tracing::warn!(
                        error = ?error,
                        user_id,
                        relation_id = id,
                        "failed to delete imported technician store binding"
                    );
                    "师傅原门店解绑失败".to_string()
                })?;
        }
    }

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
    .map_err(|error| {
        tracing::warn!(
            error = ?error,
            user_id,
            store_id,
            "failed to insert imported technician store binding"
        );
        "师傅门店绑定失败".to_string()
    })?;
    Ok(())
}

async fn replace_user_technician_skills(
    state: &AppState,
    user_id: i64,
    service_types: &[String],
) -> Result<(), String> {
    let existing: Vec<TechnicianSkillEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "technician_skill",
        value! { "user_id": user_id },
    )
    .await
    .map_err(|error| {
        tracing::warn!(
            error = ?error,
            user_id,
            "failed to query imported technician skills"
        );
        "师傅专业技能查询失败".to_string()
    })?;

    for skill in existing {
        if let Some(id) = skill.id {
            TechnicianSkillEntity::delete_by_map(state.rb.as_ref(), value! { "id": id })
                .await
                .map_err(|error| {
                    tracing::warn!(
                        error = ?error,
                        user_id,
                        skill_id = id,
                        "failed to delete imported technician skill"
                    );
                    "师傅原专业技能删除失败".to_string()
                })?;
        }
    }

    for service_type in service_types {
        TechnicianSkillEntity::insert(
            state.rb.as_ref(),
            &TechnicianSkillEntity {
                id: None,
                user_id: Some(user_id),
                service_type: Some(service_type.clone()),
                created_at: None,
            },
        )
        .await
        .map_err(|error| {
            tracing::warn!(
                error = ?error,
                user_id,
                service_type = %service_type,
                "failed to insert imported technician skill"
            );
            "师傅专业技能写入失败".to_string()
        })?;
    }
    Ok(())
}

async fn load_customer_source_template_notes(state: &AppState) -> Result<Vec<String>, AppError> {
    let mut customer_sources: Vec<CustomerSourceEntity> =
        match db::select_by_map_compat(state.rb.as_ref(), "customer_source", value! {}).await {
            Ok(items) => items,
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    "failed to query customer sources for template; using fallback notes"
                );
                return Ok(vec![
                    "客户来源可填写手工录入，系统会自动识别为 manual。".to_string(),
                ]);
            }
        };
    customer_sources.sort_by_key(|item| {
        (
            item.sort_no.unwrap_or_default(),
            item.id.unwrap_or_default(),
        )
    });
    let labels = customer_sources
        .into_iter()
        .filter_map(|item| {
            let name = item.name.unwrap_or_default();
            let code = item.code.unwrap_or_default();
            if name.trim().is_empty() && code.trim().is_empty() {
                None
            } else if name.trim().is_empty() {
                Some(code)
            } else if code.trim().is_empty() {
                Some(name)
            } else {
                Some(format!("{}（{}）", name, code))
            }
        })
        .collect::<Vec<_>>();
    if labels.is_empty() {
        return Ok(vec![
            "客户来源可填写手工录入，系统会自动识别为 manual。".to_string(),
        ]);
    }
    Ok(vec![format!("当前可用客户来源：{}", labels.join("、"))])
}

fn user_row_values(row: &ImportedUserRow) -> Vec<String> {
    vec![
        String::new(),
        row.name.clone(),
        row.mobile.clone(),
        row.password.clone(),
        row.role_codes.join(" | "),
        row.store_refs.join(" | "),
        row.status.clone(),
        String::new(),
    ]
}

fn customer_row_values(row: &ImportedCustomerRow) -> Vec<String> {
    vec![
        String::new(),
        row.name.clone(),
        row.mobile.clone(),
        row.source.clone(),
        row.store_ref.clone().unwrap_or_default(),
        row.province.clone().unwrap_or_default(),
        row.city.clone().unwrap_or_default(),
        row.district.clone().unwrap_or_default(),
        row.address.clone(),
        row.remark.clone(),
        String::new(),
    ]
}

fn store_row_values(row: &ImportedStoreRow) -> Vec<String> {
    vec![
        String::new(),
        row.store_name.clone(),
        row.code.clone(),
        row.province.clone().unwrap_or_default(),
        row.city.clone().unwrap_or_default(),
        row.district.clone().unwrap_or_default(),
        row.address.clone(),
        row.contact_name.clone(),
        row.contact_mobile.clone(),
        row.business_hours.clone(),
        row.remark.clone(),
        row.status.clone(),
        String::new(),
    ]
}

fn technician_row_values(row: &ImportedTechnicianRow) -> Vec<String> {
    vec![
        String::new(),
        row.name.clone(),
        row.mobile.clone(),
        row.password.clone(),
        row.id_card.clone(),
        row.status.clone(),
        row.emergency_contact_name.clone(),
        row.emergency_contact_mobile.clone(),
        row.service_types.join(" | "),
        row.store_ref.clone(),
        String::new(),
    ]
}

fn parse_score_csv(bytes: &[u8]) -> Result<Vec<ScoreImportRow>, AppError> {
    let content = decode_csv_content(bytes, "评分导入")?;
    let mut lines = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .peekable();
    if matches!(lines.peek(), Some(line) if line.eq_ignore_ascii_case("sep=,")) {
        let _ = lines.next();
    }
    let headers = split_csv_values(
        lines
            .next()
            .ok_or_else(|| AppError::BadRequest("评分导入文件不能为空".to_string()))?,
    );
    let header_map = build_header_index_map(&headers);
    let mut rows = Vec::new();
    for line in lines {
        let cols = split_csv_values(line);
        rows.push(parse_score_import_row_values(&header_map, &cols));
    }
    Ok(rows)
}

fn parse_score_excel(file_name: &str, bytes: &[u8]) -> Result<Vec<ScoreImportRow>, AppError> {
    let temp_path = temp_import_file_path(file_name, bytes);
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

    let mut row_iter = range.rows();
    let headers = row_iter
        .next()
        .ok_or_else(|| AppError::BadRequest("评分导入文件不能为空".to_string()))?
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
        rows.push(parse_score_import_row_values(&header_map, &cols));
    }
    Ok(rows)
}

fn parse_score_import_row_values(
    header_map: &HashMap<String, usize>,
    cols: &[String],
) -> ScoreImportRow {
    let task_id_text = value_by_keys(
        header_map,
        cols,
        &[
            "task_id",
            "任务id",
            "任务ID",
            "任务编号",
            "任务ID/任务编号",
            "任务id/任务编号",
        ],
    );
    let service_type = value_by_keys(header_map, cols, &["service_type", "服务环节", "服务类型"]);
    let score_text = value_by_keys(header_map, cols, &["score", "总部评分", "评分", "分值"]);
    let customer_mobile = value_by_keys(
        header_map,
        cols,
        &[
            "customer_mobile",
            "customerMobile",
            "mobile",
            "客户手机号",
            "手机号",
            "客户手机",
        ],
    );
    ScoreImportRow {
        task_id: task_id_text.trim().parse::<i64>().ok(),
        task_id_text,
        customer_mobile,
        service_type: normalize_optional_text(Some(service_type.as_str())),
        score: parse_score_text(&score_text),
        score_text,
        deduction_reason: normalize_optional_text(Some(
            value_by_keys(header_map, cols, &["deduction_reason", "扣分原因"]).as_str(),
        )),
        score_remark: normalize_optional_text(Some(
            value_by_keys(header_map, cols, &["score_remark", "评分备注", "备注"]).as_str(),
        )),
    }
}

fn parse_score_text(value: &str) -> Option<i32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(score) = trimmed.parse::<i32>() {
        return Some(score);
    }
    let number = trimmed.parse::<f64>().ok()?;
    if number.fract().abs() > f64::EPSILON {
        return None;
    }
    Some(number as i32)
}

fn push_score_import_failure(
    results: &mut Vec<ScoreImportResultRow>,
    row_no: usize,
    row: &ScoreImportRow,
    reason: &str,
) {
    results.push(ScoreImportResultRow {
        row_no,
        customer_mobile: row.customer_mobile.clone(),
        matched_task_text: row.task_id_text.clone(),
        service_type_text: row.service_type.clone().unwrap_or_default(),
        score_text: row.score_text.clone(),
        deduction_reason: row.deduction_reason.clone().unwrap_or_default(),
        score_remark: row.score_remark.clone().unwrap_or_default(),
        result: "失败".to_string(),
        reason: reason.to_string(),
    });
}

enum ScoreImportTaskResolution {
    Matched(TaskEntity),
    Failed(String),
}

async fn resolve_score_import_task(
    state: &AppState,
    row: &ScoreImportRow,
    service_type: &str,
) -> Result<ScoreImportTaskResolution, AppError> {
    if let Some(task_id) = row.task_id {
        let task = db::select_one_by_map_compat::<TaskEntity>(
            state.rb.as_ref(),
            "task",
            value! { "id": task_id },
        )
        .await
        .context("failed to query task by id during score import")
        .map_err(AppError::Internal)?;
        return Ok(task
            .filter(|task| task.deleted_at.is_none())
            .map(ScoreImportTaskResolution::Matched)
            .unwrap_or_else(|| {
                ScoreImportTaskResolution::Failed(format!("任务不存在: {task_id}"))
            }));
    }

    let task_no = row.task_id_text.trim();
    if !task_no.is_empty() {
        let task = db::select_one_by_map_compat::<TaskEntity>(
            state.rb.as_ref(),
            "task",
            value! { "task_no": task_no },
        )
        .await
        .context("failed to query task by task_no during score import")
        .map_err(AppError::Internal)?;
        return Ok(task
            .filter(|task| task.deleted_at.is_none())
            .map(ScoreImportTaskResolution::Matched)
            .unwrap_or_else(|| {
                ScoreImportTaskResolution::Failed(format!("任务不存在: {task_no}"))
            }));
    }

    let mobile = row.customer_mobile.trim();
    if mobile.is_empty() {
        return Ok(ScoreImportTaskResolution::Failed(
            "客户手机号不能为空".to_string(),
        ));
    }

    resolve_score_import_task_by_customer_mobile(state, mobile, service_type).await
}

async fn resolve_score_import_task_by_customer_mobile(
    state: &AppState,
    mobile: &str,
    service_type: &str,
) -> Result<ScoreImportTaskResolution, AppError> {
    let customers: Vec<CustomerEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "customer", value! { "mobile": mobile })
            .await
            .context("failed to query customer by mobile during score import")
            .map_err(AppError::Internal)?;
    let customer_ids = customers
        .iter()
        .filter_map(|customer| customer.id)
        .collect::<Vec<_>>();
    if customer_ids.is_empty() {
        return Ok(ScoreImportTaskResolution::Failed(format!(
            "客户手机号不存在: {mobile}"
        )));
    }

    let mut tasks: Vec<TaskEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task",
        value! { "customer_id": customer_ids },
    )
    .await
    .context("failed to query customer tasks during score import")
    .map_err(AppError::Internal)?;
    tasks.retain(|task| {
        task.deleted_at.is_none()
            && matches!(
                task.task_status.as_deref(),
                Some("pending_dealer_review") | Some("pending_hq_score") | Some("completed")
            )
    });

    let mut matched_tasks = Vec::new();
    for task in tasks {
        let service_types = load_task_service_types(state, &task).await?;
        if service_types.iter().any(|item| item == service_type) {
            matched_tasks.push(task);
        }
    }

    match matched_tasks.len() {
        0 => Ok(ScoreImportTaskResolution::Failed(format!(
            "该客户手机号没有可评分的{}任务: {}",
            score_service_type_text(service_type),
            mobile
        ))),
        1 => Ok(ScoreImportTaskResolution::Matched(
            matched_tasks.into_iter().next().unwrap_or_default(),
        )),
        _ => {
            matched_tasks.sort_by(|left, right| {
                right
                    .appointment_time
                    .cmp(&left.appointment_time)
                    .then_with(|| right.created_at.cmp(&left.created_at))
                    .then_with(|| right.id.cmp(&left.id))
            });
            let task_nos = matched_tasks
                .iter()
                .take(5)
                .map(|task| {
                    task.task_no
                        .clone()
                        .unwrap_or_else(|| task.id.unwrap_or_default().to_string())
                })
                .collect::<Vec<_>>()
                .join("、");
            Ok(ScoreImportTaskResolution::Failed(format!(
                "客户手机号匹配到多条{}任务，请补充任务编号或确认唯一订单: {}",
                score_service_type_text(service_type),
                task_nos
            )))
        }
    }
}

async fn upsert_import_hq_score(
    state: &AppState,
    task_id: i64,
    scorer_id: i64,
    score: i32,
    score_remark: Option<String>,
) -> Result<HqScoreEntity, AppError> {
    if let Some(existing) = db::select_one_by_map_compat::<HqScoreEntity>(
        state.rb.as_ref(),
        "hq_score",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query hq score during import")
    .map_err(AppError::Internal)?
    {
        HqScoreEntity::update_by_map(
            state.rb.as_ref(),
            &HqScoreEntity {
                scorer_id: Some(scorer_id),
                total_score: Some(score),
                service_level: Some(calc_service_level(score).to_string()),
                score_remark,
                ..Default::default()
            },
            value! { "id": existing.id.unwrap_or_default(), "column": ["scorer_id", "total_score", "service_level", "score_remark"] },
        )
        .await
        .context("failed to update hq score during import")
        .map_err(AppError::Internal)?;
        return db::select_one_by_map_compat::<HqScoreEntity>(
            state.rb.as_ref(),
            "hq_score",
            value! { "id": existing.id.unwrap_or_default() },
        )
        .await
        .context("failed to fetch updated hq score during import")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow!("updated hq score fetch failed")));
    }

    HqScoreEntity::insert(
        state.rb.as_ref(),
        &HqScoreEntity {
            id: None,
            task_id: Some(task_id),
            scorer_id: Some(scorer_id),
            total_score: Some(score),
            service_level: Some(calc_service_level(score).to_string()),
            score_remark,
            created_at: None,
        },
    )
    .await
    .context("failed to insert imported score")
    .map_err(AppError::Internal)?;

    db::select_one_by_map_compat::<HqScoreEntity>(
        state.rb.as_ref(),
        "hq_score",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to fetch imported hq score")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::Internal(anyhow!("imported hq score fetch failed")))
}

async fn refresh_import_hq_score_summary(state: &AppState, task_id: i64) -> Result<(), AppError> {
    let score_items: Vec<HqScoreItemEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "hq_score_item",
        value! { "task_id": task_id },
    )
    .await
    .context("failed to query hq score items during import summary")
    .map_err(AppError::Internal)?;
    let scores = score_items
        .iter()
        .filter_map(|item| item.score)
        .collect::<Vec<_>>();
    if scores.is_empty() {
        return Ok(());
    }
    let average = ((scores.iter().sum::<i32>() as f64) / (scores.len() as f64)).round() as i32;
    HqScoreEntity::update_by_map(
        state.rb.as_ref(),
        &HqScoreEntity {
            total_score: Some(average),
            service_level: Some(calc_service_level(average).to_string()),
            ..Default::default()
        },
        value! { "task_id": task_id, "column": ["total_score", "service_level"] },
    )
    .await
    .context("failed to refresh hq score summary during import")
    .map_err(AppError::Internal)?;
    Ok(())
}

fn normalize_score_service_type(value: Option<&str>) -> Option<String> {
    let normalized = value?.trim();
    if normalized.is_empty() {
        return None;
    }
    let key = normalized
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let service_type = match key.as_str() {
        "measure" | "measurement" | "量尺" | "量尺服务" => "measure",
        "install" | "installation" | "安装" | "安装服务" => "install",
        "ironing" | "iron" | "熨烫" | "熨烫服务" => "ironing",
        "cleaning" | "clean" | "mite_removal" | "除尘除螨" | "除尘除螨服务" => "cleaning",
        _ => return None,
    };
    Some(service_type.to_string())
}

fn score_service_type_text(service_type: &str) -> &'static str {
    match service_type {
        "measure" => "量尺服务",
        "install" => "安装服务",
        "ironing" => "熨烫服务",
        "cleaning" => "除尘除螨服务",
        _ => "其他服务",
    }
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

pub(crate) fn deserialize_vec_i64_from_mixed<'de, D>(deserializer: D) -> Result<Vec<i64>, D::Error>
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

pub(crate) fn normalize_delete_ids(ids: Vec<i64>, field_name: &str) -> Result<Vec<i64>, AppError> {
    if ids.is_empty() {
        return Err(AppError::BadRequest(format!("{field_name}不能为空")));
    }
    if ids.iter().any(|id| *id <= 0) {
        return Err(AppError::BadRequest(format!("{field_name}只能包含正整数")));
    }

    Ok(ids
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_ids_are_validated_and_deduplicated() {
        assert_eq!(
            normalize_delete_ids(vec![3, 1, 3], "ids").unwrap(),
            vec![1, 3]
        );
        assert!(normalize_delete_ids(Vec::new(), "ids").is_err());
        assert!(normalize_delete_ids(vec![0, 1], "ids").is_err());
    }

    #[test]
    fn decode_csv_content_accepts_utf8_bom() {
        let content = decode_csv_content(
            ["\u{feff}", "客户姓名,客户手机号\n张三,13800000000\n"]
                .concat()
                .as_bytes(),
            "客户导入",
        )
        .unwrap();

        assert!(content.starts_with("客户姓名"));
        assert!(content.contains("张三"));
    }

    #[test]
    fn decode_csv_content_accepts_gbk_csv() {
        let (encoded, _, _) = GBK.encode("客户姓名,客户手机号\n张三,13800000000\n");
        let content = decode_csv_content(encoded.as_ref(), "客户导入").unwrap();

        assert!(content.starts_with("客户姓名"));
        assert!(content.contains("张三"));
    }

    #[test]
    fn split_csv_values_handles_quotes_and_commas() {
        let values = split_csv_values("张三,13800000000,\"浙江省,杭州市\",\"备注\"\"A\"\"\"")
            .into_iter()
            .collect::<Vec<_>>();

        assert_eq!(
            values,
            vec!["张三", "13800000000", "浙江省,杭州市", "备注\"A\""]
        );
    }

    #[test]
    fn import_header_normalization_ignores_required_star() {
        let headers = vec!["师傅姓名*".to_string(), "手机号 *".to_string()];
        let header_map = build_header_index_map(&headers);

        assert_eq!(header_map.get("师傅姓名"), Some(&0));
        assert_eq!(header_map.get("手机号"), Some(&1));
    }

    #[test]
    fn technician_import_row_does_not_require_role_code() {
        let headers = vec![
            "师傅姓名*".to_string(),
            "手机号*".to_string(),
            "所属门店*".to_string(),
            "专业技能*".to_string(),
        ];
        let header_map = build_header_index_map(&headers);
        let row = parse_technician_import_row_values(
            &header_map,
            &[
                "张师傅".to_string(),
                "13800000000".to_string(),
                "西湖直营店".to_string(),
                "量尺、安装".to_string(),
            ],
        );

        assert_eq!(row.name, "张师傅");
        assert_eq!(row.mobile, "13800000000");
        assert_eq!(row.store_ref, "西湖直营店");
        assert_eq!(row.service_types, vec!["量尺", "安装"]);
    }

    #[test]
    fn technician_service_type_aliases_are_normalized() {
        let service_types =
            normalize_import_service_types(&["量尺".to_string(), "除尘除螨服务".to_string()])
                .unwrap();

        assert_eq!(service_types, vec!["measure", "cleaning"]);
        assert!(normalize_import_service_types(&["未知技能".to_string()]).is_err());
    }

    #[test]
    fn score_import_row_reads_customer_mobile_template() {
        let headers = vec![
            "客户手机号*".to_string(),
            "服务环节*".to_string(),
            "总部评分*".to_string(),
            "扣分原因".to_string(),
            "评分备注".to_string(),
        ];
        let header_map = build_header_index_map(&headers);
        let row = parse_score_import_row_values(
            &header_map,
            &[
                "18838962531".to_string(),
                "除尘除螨".to_string(),
                "95".to_string(),
                "现场照片不完整".to_string(),
                "复核后导入".to_string(),
            ],
        );

        assert_eq!(row.customer_mobile, "18838962531");
        assert_eq!(row.task_id_text, "");
        assert_eq!(row.service_type.as_deref(), Some("除尘除螨"));
        assert_eq!(row.score, Some(95));
    }

    #[test]
    fn score_import_row_keeps_legacy_task_id_template() {
        let headers = vec![
            "任务ID*".to_string(),
            "服务环节*".to_string(),
            "总部评分*".to_string(),
        ];
        let header_map = build_header_index_map(&headers);
        let row = parse_score_import_row_values(
            &header_map,
            &["35".to_string(), "量尺服务".to_string(), "88".to_string()],
        );

        assert_eq!(row.task_id, Some(35));
        assert_eq!(row.task_id_text, "35");
        assert_eq!(row.customer_mobile, "");
        assert_eq!(row.score, Some(88));
    }
}
