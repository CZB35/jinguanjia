use anyhow::{Context, anyhow};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use bcrypt::hash;
use chrono::{Datelike, Utc};
use rbs::value;
use serde::{Deserialize, Serialize};

use crate::{
    auth, db,
    error::AppError,
    models::{
        hq_score_item::HqScoreItemEntity, role::RoleEntity, task::TaskEntity,
        task_service_item::TaskServiceItemEntity,
        technician_skill::TechnicianSkillEntity, user::UserEntity, user_role::UserRoleEntity,
        user_store::UserStoreEntity,
    },
    response::ApiResponse,
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct TechnicianListQuery {
    pub mobile: Option<String>,
    pub name: Option<String>,
    pub service_type: Option<String>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct AvailableTechnicianQuery {
    pub store_id: i64,
    pub service_type: String,
}

#[derive(Debug, Deserialize)]
pub struct AvailableTechnicianGroupedQuery {
    pub store_id: i64,
    pub service_types: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTechnicianRequest {
    pub name: String,
    pub nickname: Option<String>,
    pub mobile: String,
    #[serde(default)]
    pub password: Option<String>,
    pub avatar_url: Option<String>,
    pub id_card: Option<String>,
    pub emergency_contact_name: Option<String>,
    pub emergency_contact_mobile: Option<String>,
    pub job_no: Option<String>,
    pub store_id: Option<i64>,
    #[serde(default)]
    pub store_ids: Option<Vec<i64>>,
    #[serde(default)]
    pub service_types: Option<Vec<String>>,
    #[serde(default)]
    pub role_names: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTechnicianRequest {
    pub name: Option<String>,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub id_card: Option<String>,
    pub emergency_contact_name: Option<String>,
    pub emergency_contact_mobile: Option<String>,
    pub job_no: Option<String>,
    pub store_id: Option<i64>,
    pub service_types: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTechnicianStatusRequest {
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct TechnicianItem {
    pub id: i64,
    pub name: String,
    pub nickname: Option<String>,
    pub mobile: String,
    pub avatar_url: Option<String>,
    pub job_no: Option<String>,
    pub status: String,
    pub status_code: String,
    pub status_text: String,
    pub role_names: Vec<String>,
    pub store_names: Vec<String>,
    pub work_status: String,
    pub service_types: Vec<String>,
    pub average_score: f64,
    pub completed_order_count: usize,
    pub pending_order_count: usize,
    pub current_task_no: Option<String>,
    pub current_task_status: Option<String>,
    pub score_count: usize,
}

#[derive(Debug, Serialize)]
pub struct TechnicianListResponse {
    pub items: Vec<TechnicianItem>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, Serialize)]
pub struct AvailableTechnicianGroupedResponse {
    pub groups: Vec<AvailableTechnicianGroup>,
}

#[derive(Debug, Serialize)]
pub struct AvailableTechnicianGroup {
    pub service_type: String,
    pub service_type_text: String,
    pub technicians: Vec<TechnicianItem>,
}

#[derive(Debug, Serialize)]
pub struct TechnicianDetailResponse {
    pub id: i64,
    pub name: String,
    pub nickname: Option<String>,
    pub mobile: String,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub job_no: Option<String>,
    pub id_card: Option<String>,
    pub emergency_contact_name: Option<String>,
    pub emergency_contact_mobile: Option<String>,
    pub status: String,
    pub status_code: String,
    pub status_text: String,
    pub store_ids: Vec<i64>,
    pub store_names: Vec<String>,
    pub role_names: Vec<String>,
    pub service_types: Vec<String>,
    pub work_status: String,
    pub average_score: f64,
    pub pending_task_count: usize,
    pub completed_task_count: usize,
    pub current_task_no: Option<String>,
    pub current_task_status: Option<String>,
    pub recent_tasks: Vec<TechnicianTaskItem>,
    pub monthly_trends: Vec<TechnicianTrendPoint>,
}

#[derive(Debug, Serialize)]
pub struct TechnicianPerformanceResponse {
    pub technician_id: i64,
    pub total_tasks: usize,
    pub completed_tasks: usize,
    pub average_score: f64,
}

#[derive(Debug, Serialize)]
pub struct TechnicianTaskItem {
    pub task_id: i64,
    pub task_no: String,
    pub service_type: String,
    pub task_status: String,
    pub customer_id: i64,
    pub appointment_time: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TechnicianTrendPoint {
    pub period: String,
    pub total_tasks: usize,
    pub completed_tasks: usize,
}

#[derive(Clone)]
struct TechnicianTaskScope {
    task: TaskEntity,
    service_item: Option<TaskServiceItemEntity>,
}

struct TechnicianAggregateStats {
    average_score: f64,
    score_count: usize,
    completed_count: usize,
    pending_count: usize,
    current_task_no: Option<String>,
    current_task_status: Option<String>,
    work_status: String,
    recent_tasks: Vec<TechnicianTaskItem>,
}

pub async fn list_technicians(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TechnicianListQuery>,
) -> Result<Json<ApiResponse<TechnicianListResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    let dealer_store_ids = dealer_scope_store_ids(&state, &auth_user).await?;

    let technician_role_id = find_role_id_by_code(&state, "technician").await?;
    let user_roles: Vec<UserRoleEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_role",
        value! { "role_id": technician_role_id },
    )
    .await
    .context("failed to query technician role relations")
    .map_err(AppError::Internal)?;

    let user_ids: Vec<i64> = user_roles
        .into_iter()
        .filter_map(|item| item.user_id)
        .collect();
    let mut items = Vec::new();
    for user_id in user_ids {
        let user = find_user(&state, user_id).await?;
        if let Some(name) = &query.name
            && !user.name.as_deref().unwrap_or_default().contains(name)
        {
            continue;
        }
        if let Some(mobile) = &query.mobile
            && !user.mobile.as_deref().unwrap_or_default().contains(mobile)
        {
            continue;
        }

        let skills: Vec<TechnicianSkillEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "technician_skill",
            value! { "user_id": user_id },
        )
        .await
        .context("failed to query technician skills")
        .map_err(AppError::Internal)?;
        let service_types: Vec<String> = skills
            .into_iter()
            .filter_map(|item| item.service_type)
            .collect();
        if let Some(service_type) = &query.service_type
            && !service_types.iter().any(|item| item == service_type)
        {
            continue;
        }

        let role_names = find_role_names(&state, user_id).await?;
        let store_relations: Vec<UserStoreEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "user_store",
            value! { "user_id": user_id },
        )
        .await
        .context("failed to query technician store relations")
        .map_err(AppError::Internal)?;
        let store_ids: Vec<i64> = store_relations
            .into_iter()
            .filter_map(|item| item.store_id)
            .collect();
        if let Some(dealer_store_ids) = &dealer_store_ids
            && !store_ids
                .iter()
                .any(|store_id| dealer_store_ids.contains(store_id))
        {
            continue;
        }
        let store_names = if store_ids.is_empty() {
            Vec::new()
        } else {
            db::select_by_map_compat::<crate::models::store::StoreEntity>(
                state.rb.as_ref(),
                "store",
                value! { "id": store_ids },
            )
            .await
            .context("failed to query technician store names")
            .map_err(AppError::Internal)?
            .into_iter()
            .filter_map(|item| item.store_name)
            .collect::<Vec<_>>()
        };
        let stats = build_technician_aggregate_stats(&state, user_id).await?;

        items.push(TechnicianItem {
            id: user_id,
            name: user.name.unwrap_or_default(),
            nickname: user.nickname.clone(),
            mobile: user.mobile.unwrap_or_default(),
            avatar_url: user.avatar_url.clone(),
            job_no: user.job_no.clone(),
            status: technician_status_text(user.status.as_deref()).to_string(),
            status_code: user.status.clone().unwrap_or_else(|| "enabled".to_string()),
            status_text: technician_status_text(user.status.as_deref()).to_string(),
            role_names,
            store_names,
            work_status: stats.work_status,
            service_types,
            average_score: stats.average_score,
            completed_order_count: stats.completed_count,
            pending_order_count: stats.pending_count,
            current_task_no: stats.current_task_no,
            current_task_status: stats.current_task_status,
            score_count: stats.score_count,
        });
    }

    let total = items.len();
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).max(1);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let items = items.into_iter().skip(start).take(page_size).collect::<Vec<_>>();
    Ok(Json(ApiResponse::success(TechnicianListResponse {
        items,
        total,
        page,
        page_size,
    })))
}

pub async fn get_technician_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(technician_id): Path<i64>,
) -> Result<Json<ApiResponse<TechnicianDetailResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    ensure_technician_access(&state, &auth_user, technician_id).await?;

    get_technician_detail_inner(&state, technician_id).await
}

pub async fn create_technician(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateTechnicianRequest>,
) -> Result<Json<ApiResponse<TechnicianDetailResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let password = payload
        .password
        .clone()
        .filter(|item| !item.trim().is_empty())
        .unwrap_or_else(|| "123456".to_string());
    let requested_store_id = payload.store_id.or_else(|| {
        payload
            .store_ids
            .as_ref()
            .and_then(|item| item.first().copied())
    });
    let store_id =
        resolve_create_technician_store_id(&state, &auth_user, requested_store_id).await?;
    let service_types =
        normalize_service_types(payload.service_types.clone(), payload.role_names.clone())?;

    if payload.name.trim().is_empty() {
        return Err(AppError::BadRequest("请输入技师姓名".to_string()));
    }
    if payload.mobile.trim().is_empty() {
        return Err(AppError::BadRequest("请输入手机号".to_string()));
    }
    if password.trim().len() < 6 {
        return Err(AppError::BadRequest("密码长度不能少于6位".to_string()));
    }
    if service_types.is_empty() {
        return Err(AppError::BadRequest("请至少选择一个服务能力".to_string()));
    }

    let existing_users: Vec<UserEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user",
        value! { "mobile": payload.mobile.as_str() },
    )
    .await
    .context("failed to query existing user by mobile")
    .map_err(AppError::Internal)?;
    let user = if let Some(existing_user) = existing_users.into_iter().next() {
        if existing_user.deleted_at.is_some() {
            return Err(AppError::Conflict("该手机号账号已存在且已删除，请联系管理员处理".to_string()));
        }
        existing_user
    } else {
        let password_hash = hash(&password, 10)
            .context("failed to hash technician password")
            .map_err(AppError::Internal)?;

        UserEntity::insert(
            state.rb.as_ref(),
            &UserEntity {
                id: None,
                mobile: Some(payload.mobile.clone()),
                account: Some(payload.mobile.clone()),
                password_hash: Some(password_hash),
                name: Some(payload.name.clone()),
                nickname: payload.nickname.clone(),
                email: None,
                avatar_url: payload.avatar_url.clone(),
                job_no: payload.job_no.clone().or_else(|| {
                    Some(format!(
                        "SF{:08}",
                        Utc::now().timestamp_millis().rem_euclid(100_000_000)
                    ))
                }),
                id_card: payload.id_card.clone(),
                emergency_contact_name: payload.emergency_contact_name.clone(),
                emergency_contact_mobile: payload.emergency_contact_mobile.clone(),
                status: Some("enabled".to_string()),
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
        .context("failed to create technician user")
        .map_err(AppError::Internal)?;

        db::select_one_by_map_compat(
            state.rb.as_ref(),
            "user",
            value! { "mobile": payload.mobile.as_str() },
        )
        .await
        .context("failed to fetch created technician")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow!("technician create succeeded but fetch failed")))?
    };
    let user_id = user.id.unwrap_or_default();

    ensure_technician_role_binding(&state, user_id).await?;
    upsert_single_store_binding(&state, user_id, store_id).await?;
    replace_technician_skills(&state, user_id, service_types).await?;

    get_technician_detail_inner(&state, user_id).await
}

pub async fn update_technician(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(technician_id): Path<i64>,
    Json(payload): Json<UpdateTechnicianRequest>,
) -> Result<Json<ApiResponse<TechnicianDetailResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    ensure_technician_access(&state, &auth_user, technician_id).await?;

    let _ = find_user(&state, technician_id).await?;

    UserEntity::update_by_map(
        state.rb.as_ref(),
        &UserEntity {
            name: payload.name.clone(),
            nickname: payload.nickname.clone(),
            email: payload.email.clone(),
            avatar_url: payload.avatar_url.clone(),
            job_no: payload.job_no.clone(),
            id_card: payload.id_card.clone(),
            emergency_contact_name: payload.emergency_contact_name.clone(),
            emergency_contact_mobile: payload.emergency_contact_mobile.clone(),
            ..Default::default()
        },
        value! { "id": technician_id },
    )
    .await
    .context("failed to update technician")
    .map_err(AppError::Internal)?;

    if let Some(service_types) = payload.service_types {
        let existing: Vec<TechnicianSkillEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "technician_skill",
            value! { "user_id": technician_id },
        )
        .await
        .context("failed to query existing technician skills")
        .map_err(AppError::Internal)?;
        for skill in existing {
            if let Some(id) = skill.id {
                TechnicianSkillEntity::delete_by_map(state.rb.as_ref(), value! { "id": id })
                    .await
                    .context("failed to delete technician skill")
                    .map_err(AppError::Internal)?;
            }
        }
        for service_type in service_types {
            TechnicianSkillEntity::insert(
                state.rb.as_ref(),
                &TechnicianSkillEntity {
                    id: None,
                    user_id: Some(technician_id),
                    service_type: Some(service_type),
                    created_at: None,
                },
            )
            .await
            .context("failed to recreate technician skill")
            .map_err(AppError::Internal)?;
        }
    }

    if let Some(store_id) = payload.store_id {
        let store_id = resolve_technician_store_id(&state, &auth_user, store_id).await?;
        let existing_stores: Vec<UserStoreEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "user_store",
            value! { "user_id": technician_id },
        )
        .await
        .context("failed to query existing technician stores")
        .map_err(AppError::Internal)?;
        for relation in existing_stores {
            if let Some(id) = relation.id {
                UserStoreEntity::delete_by_map(state.rb.as_ref(), value! { "id": id })
                    .await
                    .context("failed to delete existing technician store relation")
                    .map_err(AppError::Internal)?;
            }
        }

        UserStoreEntity::insert(
            state.rb.as_ref(),
            &UserStoreEntity {
                id: None,
                user_id: Some(technician_id),
                store_id: Some(store_id),
                created_at: None,
            },
        )
        .await
        .context("failed to bind technician store")
        .map_err(AppError::Internal)?;
    }

    get_technician_detail_inner(&state, technician_id).await
}

pub async fn update_technician_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(technician_id): Path<i64>,
    Json(payload): Json<UpdateTechnicianStatusRequest>,
) -> Result<Json<ApiResponse<TechnicianDetailResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    ensure_technician_access(&state, &auth_user, technician_id).await?;

    if payload.status != "enabled" && payload.status != "disabled" {
        return Err(AppError::BadRequest(
            "status只能为enabled或disabled".to_string(),
        ));
    }

    UserEntity::update_by_map(
        state.rb.as_ref(),
        &UserEntity {
            status: Some(payload.status.clone()),
            ..Default::default()
        },
        value! { "id": technician_id },
    )
    .await
    .context("failed to update technician status")
    .map_err(AppError::Internal)?;

    get_technician_detail_inner(&state, technician_id).await
}

pub async fn reset_technician_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(technician_id): Path<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    ensure_technician_access(&state, &auth_user, technician_id).await?;
    let _ = find_user(&state, technician_id).await?;

    let password_hash = hash("123456", 10)
        .context("failed to hash technician reset password")
        .map_err(AppError::Internal)?;
    UserEntity::update_by_map(
        state.rb.as_ref(),
        &UserEntity {
            password_hash: Some(password_hash),
            ..Default::default()
        },
        value! { "id": technician_id },
    )
    .await
    .context("failed to reset technician password")
    .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true, "default_password": "123456" }),
    )))
}

pub async fn get_technician_performance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(technician_id): Path<i64>,
) -> Result<Json<ApiResponse<TechnicianPerformanceResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    ensure_technician_access(&state, &auth_user, technician_id).await?;

    let _ = find_user(&state, technician_id).await?;
    let task_scopes = load_technician_task_scopes(&state, technician_id).await?;
    let stats = build_technician_aggregate_stats(&state, technician_id).await?;

    Ok(Json(ApiResponse::success(TechnicianPerformanceResponse {
        technician_id,
        total_tasks: task_scopes.len(),
        completed_tasks: stats.completed_count,
        average_score: stats.average_score,
    })))
}

pub async fn list_available_technicians(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AvailableTechnicianQuery>,
) -> Result<Json<ApiResponse<Vec<TechnicianItem>>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    ensure_store_access(&state, &auth_user, query.store_id).await?;

    let store_relations = load_store_relations(&state, query.store_id).await?;
    let store_name = load_store_name(&state, query.store_id).await?;

    Ok(Json(ApiResponse::success(
        available_technicians_for_service(
            &state,
            &store_relations,
            &store_name,
            &query.service_type,
        )
        .await?,
    )))
}

pub async fn list_available_technicians_grouped(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AvailableTechnicianGroupedQuery>,
) -> Result<Json<ApiResponse<AvailableTechnicianGroupedResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    ensure_store_access(&state, &auth_user, query.store_id).await?;

    let service_types = parse_service_types_param(&query.service_types)?;
    let store_relations = load_store_relations(&state, query.store_id).await?;
    let store_name = load_store_name(&state, query.store_id).await?;
    let mut groups = Vec::with_capacity(service_types.len());

    for service_type in service_types {
        groups.push(AvailableTechnicianGroup {
            service_type: service_type.clone(),
            service_type_text: service_type_label(&service_type).to_string(),
            technicians: available_technicians_for_service(
                &state,
                &store_relations,
                &store_name,
                &service_type,
            )
            .await?,
        });
    }

    Ok(Json(ApiResponse::success(
        AvailableTechnicianGroupedResponse { groups },
    )))
}

async fn available_technicians_for_service(
    state: &AppState,
    store_relations: &[UserStoreEntity],
    store_name: &str,
    service_type: &str,
) -> Result<Vec<TechnicianItem>, AppError> {
    let mut items = Vec::new();
    for relation in store_relations {
        let Some(user_id) = relation.user_id else {
            continue;
        };
        let user = find_user(state, user_id).await?;
        if user.status.as_deref() != Some("enabled") {
            continue;
        }
        let skills: Vec<TechnicianSkillEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "technician_skill",
            value! { "user_id": user_id },
        )
        .await
        .context("failed to query available technician skills")
        .map_err(AppError::Internal)?;
        let service_types = skills
            .into_iter()
            .filter_map(|item| item.service_type)
            .collect::<Vec<_>>();
        if !service_types.iter().any(|item| item == service_type) {
            continue;
        }
        let stats = build_technician_aggregate_stats(state, user_id).await?;
        items.push(TechnicianItem {
            id: user_id,
            name: user.name.unwrap_or_default(),
            nickname: user.nickname.clone(),
            mobile: user.mobile.unwrap_or_default(),
            avatar_url: user.avatar_url.clone(),
            job_no: user.job_no.clone(),
            status: technician_status_text(user.status.as_deref()).to_string(),
            status_code: user.status.clone().unwrap_or_else(|| "enabled".to_string()),
            status_text: technician_status_text(user.status.as_deref()).to_string(),
            role_names: vec!["技师".to_string()],
            store_names: vec![store_name.to_string()],
            work_status: stats.work_status,
            service_types,
            average_score: stats.average_score,
            completed_order_count: stats.completed_count,
            pending_order_count: stats.pending_count,
            current_task_no: stats.current_task_no,
            current_task_status: stats.current_task_status,
            score_count: stats.score_count,
        });
    }

    Ok(items)
}

async fn load_store_relations(
    state: &AppState,
    store_id: i64,
) -> Result<Vec<UserStoreEntity>, AppError> {
    db::select_by_map_compat(
        state.rb.as_ref(),
        "user_store",
        value! { "store_id": store_id },
    )
    .await
    .context("failed to query available technician store relations")
    .map_err(AppError::Internal)
}

async fn load_store_name(state: &AppState, store_id: i64) -> Result<String, AppError> {
    Ok(
        db::select_one_by_map_compat::<crate::models::store::StoreEntity>(
            state.rb.as_ref(),
            "store",
            value! { "id": store_id },
        )
        .await
        .context("failed to query available technician store")
        .map_err(AppError::Internal)?
        .and_then(|item| item.store_name)
        .unwrap_or_default(),
    )
}

fn parse_service_types_param(input: &str) -> Result<Vec<String>, AppError> {
    let mut service_types = Vec::new();
    for item in input.split(',') {
        let service_type = item.trim();
        if service_type.is_empty()
            || service_types
                .iter()
                .any(|existing| existing == service_type)
        {
            continue;
        }
        service_types.push(service_type.to_string());
    }

    if service_types.is_empty() {
        return Err(AppError::BadRequest("请选择服务环节".to_string()));
    }

    Ok(service_types)
}

fn service_type_label(service_type: &str) -> &'static str {
    match service_type {
        "measure" => "量尺服务",
        "install" => "安装服务",
        "ironing" => "熨烫服务",
        "cleaning" => "除尘除螨服务",
        _ => "其他服务",
    }
}

fn technician_status_text(status: Option<&str>) -> &'static str {
    match status.unwrap_or("enabled") {
        "enabled" | "active" => "已启用",
        "disabled" | "inactive" => "已禁用",
        _ => "未知状态",
    }
}

async fn get_technician_detail_inner(
    state: &AppState,
    technician_id: i64,
) -> Result<Json<ApiResponse<TechnicianDetailResponse>>, AppError> {
    let user = find_user(state, technician_id).await?;
    let stores: Vec<UserStoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_store",
        value! { "user_id": technician_id },
    )
    .await
    .context("failed to query technician stores")
    .map_err(AppError::Internal)?;
    let skills: Vec<TechnicianSkillEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "technician_skill",
        value! { "user_id": technician_id },
    )
    .await
    .context("failed to query technician skills")
    .map_err(AppError::Internal)?;
    let stats = build_technician_aggregate_stats(state, technician_id).await?;
    let role_names = find_role_names(state, technician_id).await?;
    let store_ids: Vec<i64> = stores.iter().filter_map(|item| item.store_id).collect();
    let store_names = if store_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<crate::models::store::StoreEntity>(
            state.rb.as_ref(),
            "store",
            value! { "id": store_ids.clone() },
        )
        .await
        .context("failed to query technician detail store names")
        .map_err(AppError::Internal)?
        .into_iter()
        .filter_map(|item| item.store_name)
        .collect::<Vec<_>>()
    };
    let task_scopes = load_technician_task_scopes(state, technician_id).await?;
    let monthly_trends = build_monthly_trends(&task_scopes);

    Ok(Json(ApiResponse::success(TechnicianDetailResponse {
        id: user.id.unwrap_or_default(),
        name: user.name.unwrap_or_default(),
        nickname: user.nickname,
        mobile: user.mobile.unwrap_or_default(),
        email: user.email,
        avatar_url: user.avatar_url,
        job_no: user.job_no,
        id_card: user.id_card,
        emergency_contact_name: user.emergency_contact_name,
        emergency_contact_mobile: user.emergency_contact_mobile,
        status: technician_status_text(user.status.as_deref()).to_string(),
        status_code: user.status.clone().unwrap_or_else(|| "enabled".to_string()),
        status_text: technician_status_text(user.status.as_deref()).to_string(),
        store_ids,
        store_names,
        role_names,
        service_types: skills
            .into_iter()
            .filter_map(|item| item.service_type)
            .collect(),
        work_status: stats.work_status,
        average_score: stats.average_score,
        pending_task_count: stats.pending_count,
        completed_task_count: stats.completed_count,
        current_task_no: stats.current_task_no,
        current_task_status: stats.current_task_status,
        recent_tasks: stats.recent_tasks,
        monthly_trends,
    })))
}

async fn find_role_id_by_code(state: &AppState, role_code: &str) -> Result<i64, AppError> {
    db::select_one_by_map_compat::<RoleEntity>(
        state.rb.as_ref(),
        "role",
        value! { "role_code": role_code },
    )
    .await
    .context("failed to query role by code")
    .map_err(AppError::Internal)?
    .and_then(|item| item.id)
    .ok_or_else(|| AppError::Internal(anyhow!("role not found: {role_code}")))
}

async fn ensure_technician_role_binding(state: &AppState, user_id: i64) -> Result<(), AppError> {
    let role_id = find_role_id_by_code(state, "technician").await?;
    let existing: Vec<UserRoleEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": user_id, "role_id": role_id },
    )
    .await
    .context("failed to query existing technician role binding")
    .map_err(AppError::Internal)?;
    if !existing.is_empty() {
        return Ok(());
    }

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
    .context("failed to bind technician role")
    .map_err(AppError::Internal)?;
    Ok(())
}

async fn upsert_single_store_binding(
    state: &AppState,
    user_id: i64,
    store_id: i64,
) -> Result<(), AppError> {
    let existing: Vec<UserStoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_store",
        value! { "user_id": user_id },
    )
    .await
    .context("failed to query existing technician stores")
    .map_err(AppError::Internal)?;

    if existing.iter().any(|item| item.store_id == Some(store_id)) {
        return Ok(());
    }

    if existing.len() > 1 {
        return Err(AppError::Conflict(
            "当前账号绑定了多个门店，请先在后台修正为单门店再添加技师角色".to_string(),
        ));
    }

    for relation in existing {
        if let Some(id) = relation.id {
            UserStoreEntity::delete_by_map(state.rb.as_ref(), value! { "id": id })
                .await
                .context("failed to delete existing technician store relation")
                .map_err(AppError::Internal)?;
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
    .context("failed to bind technician store")
    .map_err(AppError::Internal)?;
    Ok(())
}

async fn replace_technician_skills(
    state: &AppState,
    user_id: i64,
    service_types: Vec<String>,
) -> Result<(), AppError> {
    let existing: Vec<TechnicianSkillEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "technician_skill",
        value! { "user_id": user_id },
    )
    .await
    .context("failed to query existing technician skills")
    .map_err(AppError::Internal)?;
    for skill in existing {
        if let Some(id) = skill.id {
            TechnicianSkillEntity::delete_by_map(state.rb.as_ref(), value! { "id": id })
                .await
                .context("failed to delete technician skill")
                .map_err(AppError::Internal)?;
        }
    }
    for service_type in service_types {
        TechnicianSkillEntity::insert(
            state.rb.as_ref(),
            &TechnicianSkillEntity {
                id: None,
                user_id: Some(user_id),
                service_type: Some(service_type),
                created_at: None,
            },
        )
        .await
        .context("failed to create technician skill")
        .map_err(AppError::Internal)?;
    }
    Ok(())
}

async fn find_role_names(state: &AppState, user_id: i64) -> Result<Vec<String>, AppError> {
    let relations: Vec<UserRoleEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": user_id },
    )
    .await
    .context("failed to query technician role relations")
    .map_err(AppError::Internal)?;
    let role_ids: Vec<i64> = relations
        .into_iter()
        .filter_map(|item| item.role_id)
        .collect();
    if role_ids.is_empty() {
        return Ok(Vec::new());
    }
    let roles: Vec<RoleEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "role", value! { "id": role_ids })
            .await
            .context("failed to query role names")
            .map_err(AppError::Internal)?;
    Ok(roles
        .into_iter()
        .filter_map(|item| item.role_name)
        .collect())
}

async fn build_technician_aggregate_stats(
    state: &AppState,
    technician_id: i64,
) -> Result<TechnicianAggregateStats, AppError> {
    let task_scopes = load_technician_task_scopes(state, technician_id).await?;
    let score_items = load_technician_score_items(state, &task_scopes).await?;
    let average_score = if score_items.is_empty() {
        0.0
    } else {
        score_items
            .iter()
            .map(|item| item.score.unwrap_or_default() as f64)
            .sum::<f64>()
            / score_items.len() as f64
    };

    let completed_count = task_scopes
        .iter()
        .filter(|item| technician_scope_is_completed(technician_scope_status(item)))
        .count();
    let pending_count = task_scopes
        .iter()
        .filter(|item| technician_scope_is_pending(technician_scope_status(item)))
        .count();

    let mut recent_scopes = task_scopes.clone();
    recent_scopes.sort_by_key(technician_scope_appointment_time);
    recent_scopes.reverse();
    let current_task = recent_scopes.iter().find(|item| technician_scope_is_active(technician_scope_status(item)));

    Ok(TechnicianAggregateStats {
        average_score,
        score_count: score_items.len(),
        completed_count,
        pending_count,
        current_task_no: current_task.and_then(|item| item.task.task_no.clone()),
        current_task_status: current_task
            .and_then(|item| technician_scope_status(item).map(ToString::to_string)),
        work_status: if current_task.is_some() {
            "busy".to_string()
        } else {
            "idle".to_string()
        },
        recent_tasks: recent_scopes
            .into_iter()
            .take(10)
            .map(|item| TechnicianTaskItem {
                task_id: item.task.id.unwrap_or_default(),
                task_no: item.task.task_no.clone().unwrap_or_default(),
                service_type: technician_scope_service_type(&item).unwrap_or_default().to_string(),
                task_status: technician_scope_status(&item).unwrap_or_default().to_string(),
                customer_id: item.task.customer_id.unwrap_or_default(),
                appointment_time: technician_scope_appointment_time(&item)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string()),
            })
            .collect(),
    })
}

async fn load_technician_task_scopes(
    state: &AppState,
    technician_id: i64,
) -> Result<Vec<TechnicianTaskScope>, AppError> {
    let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_service_item",
        value! { "technician_id": technician_id },
    )
    .await
    .context("failed to query technician service items")
    .map_err(AppError::Internal)?;

    let task_ids_with_service_items = service_items
        .iter()
        .filter_map(|item| item.task_id)
        .collect::<std::collections::BTreeSet<_>>();
    let tasks_by_id = load_tasks_by_ids(
        state,
        task_ids_with_service_items.iter().copied().collect::<Vec<_>>(),
    )
    .await?;

    let mut scopes = Vec::new();
    for service_item in service_items {
        let Some(task_id) = service_item.task_id else {
            continue;
        };
        let Some(task) = tasks_by_id.get(&task_id) else {
            continue;
        };
        scopes.push(TechnicianTaskScope {
            task: task.clone(),
            service_item: Some(service_item),
        });
    }

    let legacy_tasks: Vec<TaskEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task",
        value! { "current_technician_id": technician_id },
    )
    .await
    .context("failed to query legacy technician tasks")
    .map_err(AppError::Internal)?;
    let legacy_task_ids = legacy_tasks
        .iter()
        .filter_map(|item| item.id)
        .collect::<Vec<_>>();
    let task_ids_with_any_service_items = load_task_ids_with_service_items(state, &legacy_task_ids).await?;

    for task in legacy_tasks {
        let Some(task_id) = task.id else {
            continue;
        };
        if task_ids_with_service_items.contains(&task_id)
            || task_ids_with_any_service_items.contains(&task_id)
        {
            continue;
        }
        scopes.push(TechnicianTaskScope {
            task,
            service_item: None,
        });
    }

    Ok(scopes)
}

async fn load_tasks_by_ids(
    state: &AppState,
    task_ids: Vec<i64>,
) -> Result<std::collections::BTreeMap<i64, TaskEntity>, AppError> {
    if task_ids.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }
    let tasks: Vec<TaskEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task",
        value! { "id": task_ids },
    )
    .await
    .context("failed to query technician tasks by ids")
    .map_err(AppError::Internal)?;
    Ok(tasks
        .into_iter()
        .filter_map(|item| item.id.map(|id| (id, item)))
        .collect())
}

async fn load_task_ids_with_service_items(
    state: &AppState,
    task_ids: &[i64],
) -> Result<std::collections::BTreeSet<i64>, AppError> {
    if task_ids.is_empty() {
        return Ok(std::collections::BTreeSet::new());
    }
    let service_items: Vec<TaskServiceItemEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task_service_item",
        value! { "task_id": task_ids.to_vec() },
    )
    .await
    .context("failed to query technician legacy task service items")
    .map_err(AppError::Internal)?;
    Ok(service_items
        .into_iter()
        .filter_map(|item| item.task_id)
        .collect())
}

async fn load_technician_score_items(
    state: &AppState,
    task_scopes: &[TechnicianTaskScope],
) -> Result<Vec<HqScoreItemEntity>, AppError> {
    let task_ids = task_scopes
        .iter()
        .filter_map(|item| item.task.id)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if task_ids.is_empty() {
        return Ok(Vec::new());
    }

    let score_items: Vec<HqScoreItemEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "hq_score_item",
        value! { "task_id": task_ids },
    )
    .await
    .context("failed to query technician score items")
    .map_err(AppError::Internal)?;
    let scope_keys = task_scopes
        .iter()
        .filter_map(|item| {
            Some((
                item.task.id?,
                technician_scope_service_type(item)?.to_string(),
            ))
        })
        .collect::<std::collections::BTreeSet<_>>();

    Ok(score_items
        .into_iter()
        .filter(|item| {
            item.task_id
                .zip(item.service_type.clone())
                .map(|key| scope_keys.contains(&key))
                .unwrap_or(false)
        })
        .collect())
}

fn technician_scope_status(scope: &TechnicianTaskScope) -> Option<&str> {
    scope
        .service_item
        .as_ref()
        .and_then(|item| item.service_status.as_deref())
        .or(scope.task.task_status.as_deref())
}

fn technician_scope_service_type(scope: &TechnicianTaskScope) -> Option<&str> {
    scope
        .service_item
        .as_ref()
        .and_then(|item| item.service_type.as_deref())
        .or(scope.task.service_type.as_deref())
}

fn technician_scope_appointment_time(scope: &TechnicianTaskScope) -> Option<chrono::NaiveDateTime> {
    scope
        .service_item
        .as_ref()
        .and_then(|item| item.scheduled_time)
        .or(scope.task.appointment_time)
}

fn technician_scope_is_completed(status: Option<&str>) -> bool {
    matches!(status, Some("completed") | Some("closed"))
}

fn technician_scope_is_pending(status: Option<&str>) -> bool {
    matches!(
        status,
        Some("pending_assign") | Some("pending_execute") | Some("executing") | Some("pending_dealer_review")
    )
}

fn technician_scope_is_active(status: Option<&str>) -> bool {
    matches!(status, Some("pending_execute") | Some("executing"))
}

async fn ensure_technician_access(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    technician_id: i64,
) -> Result<(), AppError> {
    let Some(dealer_store_ids) = dealer_scope_store_ids(state, auth_user).await? else {
        return Ok(());
    };

    let technician_store_ids = load_user_store_ids(state, technician_id).await?;
    if technician_store_ids
        .iter()
        .any(|store_id| dealer_store_ids.contains(store_id))
    {
        return Ok(());
    }

    Err(AppError::Forbidden(
        "您只能查看或维护所属门店下的师傅".to_string(),
    ))
}

async fn ensure_store_access(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    store_id: i64,
) -> Result<(), AppError> {
    let Some(dealer_store_ids) = dealer_scope_store_ids(state, auth_user).await? else {
        return Ok(());
    };

    if dealer_store_ids.contains(&store_id) {
        return Ok(());
    }

    Err(AppError::Forbidden("您只能选择所属门店".to_string()))
}

async fn resolve_technician_store_id(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    store_id: i64,
) -> Result<i64, AppError> {
    ensure_store_access(state, auth_user, store_id).await?;
    Ok(store_id)
}

async fn dealer_scope_store_ids(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
) -> Result<Option<Vec<i64>>, AppError> {
    if !auth_user.has_role("dealer")
        || auth_user.has_role("hq_reviewer")
        || auth_user.has_role("super_admin")
    {
        return Ok(None);
    }

    Ok(Some(load_user_store_ids(state, auth_user.user_id).await?))
}

async fn resolve_create_technician_store_id(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
    requested_store_id: Option<i64>,
) -> Result<i64, AppError> {
    if auth_user.has_role("dealer")
        && !auth_user.has_role("hq_reviewer")
        && !auth_user.has_role("super_admin")
    {
        return dealer_owned_store_id(state, auth_user).await;
    }

    let store_id =
        requested_store_id.ok_or_else(|| AppError::BadRequest("请选择所属门店".to_string()))?;
    resolve_technician_store_id(state, auth_user, store_id).await
}

async fn dealer_owned_store_id(
    state: &AppState,
    auth_user: &auth::CurrentAuthUser,
) -> Result<i64, AppError> {
    let store_ids = load_user_store_ids(state, auth_user.user_id).await?;

    match store_ids.as_slice() {
        [store_id] => Ok(*store_id),
        [] => Err(AppError::Forbidden("当前经销商账号未绑定门店".to_string())),
        _ => Err(AppError::Conflict(
            "当前经销商账号绑定了多个门店，请先在后台修正为单门店".to_string(),
        )),
    }
}

async fn load_user_store_ids(state: &AppState, user_id: i64) -> Result<Vec<i64>, AppError> {
    let stores: Vec<UserStoreEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_store",
        value! { "user_id": user_id },
    )
    .await
    .context("failed to query user store relations")
    .map_err(AppError::Internal)?;

    Ok(stores
        .into_iter()
        .filter_map(|item| item.store_id)
        .collect())
}

fn normalize_service_types(
    service_types: Option<Vec<String>>,
    role_names: Option<Vec<String>>,
) -> Result<Vec<String>, AppError> {
    let source = service_types.or(role_names).unwrap_or_default();
    let mut result = Vec::new();

    for item in source {
        let normalized = match item.trim() {
            "measure" | "量尺" => "measure",
            "install" | "安装" => "install",
            "ironing" | "熨烫" => "ironing",
            "cleaning" | "除尘除螨" | "除尘" | "除螨" => "cleaning",
            _ => return Err(AppError::BadRequest(format!("不支持的服务能力: {}", item))),
        };

        if !result.iter().any(|existing| existing == normalized) {
            result.push(normalized.to_string());
        }
    }

    Ok(result)
}

fn build_monthly_trends(tasks: &[TechnicianTaskScope]) -> Vec<TechnicianTrendPoint> {
    let now = Utc::now();
    let mut points = Vec::new();
    for offset in (0..6).rev() {
        let month = now - chrono::Duration::days(30 * offset);
        let total_tasks = tasks
            .iter()
            .filter(|item| {
                technician_scope_appointment_time(item)
                    .map(|dt| dt.year() == month.year() && dt.month() == month.month())
                    .unwrap_or(false)
            })
            .count();
        let completed_tasks = tasks
            .iter()
            .filter(|item| {
                technician_scope_appointment_time(item)
                    .map(|dt| dt.year() == month.year() && dt.month() == month.month())
                    .unwrap_or(false)
            })
            .filter(|item| technician_scope_is_completed(technician_scope_status(item)))
            .count();
        points.push(TechnicianTrendPoint {
            period: format!("{}-{:02}", month.year(), month.month()),
            total_tasks,
            completed_tasks,
        });
    }
    points
}

async fn find_user(state: &AppState, user_id: i64) -> Result<UserEntity, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "user", value! { "id": user_id })
        .await
        .context("failed to query user by id")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound("技师不存在".to_string()))
}
