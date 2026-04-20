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
    auth,
    db,
    error::AppError,
    models::{
        hq_score::HqScoreEntity,
        role::RoleEntity,
        task::TaskEntity,
        technician_skill::TechnicianSkillEntity,
        user::UserEntity,
        user_role::UserRoleEntity,
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
}

#[derive(Debug, Deserialize)]
pub struct AvailableTechnicianQuery {
    pub store_id: i64,
    pub service_type: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTechnicianRequest {
    pub name: String,
    pub nickname: Option<String>,
    pub mobile: String,
    pub password: String,
    pub avatar_url: Option<String>,
    pub id_card: Option<String>,
    pub emergency_contact_name: Option<String>,
    pub emergency_contact_mobile: Option<String>,
    pub job_no: Option<String>,
    pub store_id: i64,
    pub service_types: Vec<String>,
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

pub async fn list_technicians(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TechnicianListQuery>,
) -> Result<Json<ApiResponse<TechnicianListResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let technician_role_id = find_role_id_by_code(&state, "technician").await?;
    let user_roles: Vec<UserRoleEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_role", value! { "role_id": technician_role_id })
        .await
        .context("failed to query technician role relations")
        .map_err(AppError::Internal)?;

    let user_ids: Vec<i64> = user_roles.into_iter().filter_map(|item| item.user_id).collect();
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

        let skills: Vec<TechnicianSkillEntity> = db::select_by_map_compat(state.rb.as_ref(), "technician_skill", value! { "user_id": user_id })
            .await
            .context("failed to query technician skills")
            .map_err(AppError::Internal)?;
        let service_types: Vec<String> = skills.into_iter().filter_map(|item| item.service_type).collect();
        if let Some(service_type) = &query.service_type
            && !service_types.iter().any(|item| item == service_type)
        {
            continue;
        }

        let role_names = find_role_names(&state, user_id).await?;
        let store_relations: Vec<UserStoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_store", value! { "user_id": user_id })
            .await
            .context("failed to query technician store relations")
            .map_err(AppError::Internal)?;
        let store_ids: Vec<i64> = store_relations.into_iter().filter_map(|item| item.store_id).collect();
        let store_names = if store_ids.is_empty() {
            Vec::new()
        } else {
            db::select_by_map_compat::<crate::models::store::StoreEntity>(state.rb.as_ref(), "store", value! { "id": store_ids })
                .await
                .context("failed to query technician store names")
                .map_err(AppError::Internal)?
                .into_iter()
                .filter_map(|item| item.store_name)
                .collect::<Vec<_>>()
        };
        let tasks: Vec<TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! { "current_technician_id": user_id })
            .await
            .context("failed to query technician tasks")
            .map_err(AppError::Internal)?;
        let average_score = calculate_technician_average_score(&state, user_id).await?;
        let completed_order_count = tasks.iter().filter(|item| item.task_status.as_deref() == Some("completed")).count();
        let pending_order_count = tasks.iter().filter(|item| item.task_status.as_deref() != Some("completed") && item.task_status.as_deref() != Some("closed")).count();
        let current_task = tasks.iter().find(|item| matches!(item.task_status.as_deref(), Some("pending_execute") | Some("executing")));

        items.push(TechnicianItem {
            id: user_id,
            name: user.name.unwrap_or_default(),
            nickname: user.nickname.clone(),
            mobile: user.mobile.unwrap_or_default(),
            avatar_url: user.avatar_url.clone(),
            job_no: user.job_no.clone(),
            status: user.status.unwrap_or_else(|| "enabled".to_string()),
            role_names,
            store_names,
            work_status: if current_task.is_some() { "busy".to_string() } else { "idle".to_string() },
            service_types,
            average_score,
            completed_order_count,
            pending_order_count,
            current_task_no: current_task.and_then(|item| item.task_no.clone()),
            current_task_status: current_task.and_then(|item| item.task_status.clone()),
            score_count: completed_order_count,
        });
    }

    let total = items.len();
    Ok(Json(ApiResponse::success(TechnicianListResponse { items, total })))
}

pub async fn get_technician_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(technician_id): Path<i64>,
) -> Result<Json<ApiResponse<TechnicianDetailResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    get_technician_detail_inner(&state, technician_id).await
}

pub async fn create_technician(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateTechnicianRequest>,
) -> Result<Json<ApiResponse<TechnicianDetailResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    if payload.name.trim().is_empty() {
        return Err(AppError::BadRequest("请输入技师姓名".to_string()));
    }
    if payload.mobile.trim().is_empty() {
        return Err(AppError::BadRequest("请输入手机号".to_string()));
    }
    if payload.password.trim().len() < 6 {
        return Err(AppError::BadRequest("密码长度不能少于6位".to_string()));
    }
    if payload.service_types.is_empty() {
        return Err(AppError::BadRequest("请至少选择一个服务能力".to_string()));
    }

    let exists: Vec<UserEntity> = db::select_by_map_compat(state.rb.as_ref(), "user", value! { "mobile": payload.mobile.as_str() })
        .await
        .context("failed to query existing user by mobile")
        .map_err(AppError::Internal)?;
    if !exists.is_empty() {
        return Err(AppError::Conflict("该手机号账号已存在".to_string()));
    }

    let password_hash = hash(&payload.password, 10)
        .context("failed to hash technician password")
        .map_err(AppError::Internal)?;

    UserEntity::insert(
        state.rb.as_ref(),
        &UserEntity {
            id: None,
            mobile: Some(payload.mobile.clone()),
            password_hash: Some(password_hash),
            name: Some(payload.name.clone()),
            nickname: payload.nickname.clone(),
            email: None,
            avatar_url: payload.avatar_url.clone(),
            job_no: payload.job_no.clone().or_else(|| Some(format!("SF{:08}", Utc::now().timestamp_millis().rem_euclid(100_000_000)))),
            id_card: payload.id_card.clone(),
            emergency_contact_name: payload.emergency_contact_name.clone(),
            emergency_contact_mobile: payload.emergency_contact_mobile.clone(),
            status: Some("enabled".to_string()),
            last_login_at: None,
            last_login_ip: None,
            created_at: None,
            updated_at: None,
            deleted_at: None,
        },
    )
    .await
    .context("failed to create technician user")
    .map_err(AppError::Internal)?;

    let user: UserEntity = db::select_one_by_map_compat(state.rb.as_ref(), "user", value! { "mobile": payload.mobile.as_str() })
        .await
        .context("failed to fetch created technician")
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::Internal(anyhow!("technician create succeeded but fetch failed")))?;
    let user_id = user.id.unwrap_or_default();

    let role_id = find_role_id_by_code(&state, "technician").await?;
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

    UserStoreEntity::insert(
        state.rb.as_ref(),
        &UserStoreEntity {
            id: None,
            user_id: Some(user_id),
            store_id: Some(payload.store_id),
            created_at: None,
        },
    )
    .await
    .context("failed to bind technician store")
    .map_err(AppError::Internal)?;

    for service_type in payload.service_types {
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
        let existing: Vec<TechnicianSkillEntity> = db::select_by_map_compat(state.rb.as_ref(), "technician_skill", value! { "user_id": technician_id })
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

    if payload.status != "enabled" && payload.status != "disabled" {
        return Err(AppError::BadRequest("status只能为enabled或disabled".to_string()));
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

pub async fn get_technician_performance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(technician_id): Path<i64>,
) -> Result<Json<ApiResponse<TechnicianPerformanceResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let _ = find_user(&state, technician_id).await?;
    let tasks: Vec<crate::models::task::TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! { "current_technician_id": technician_id })
        .await
        .context("failed to query technician tasks")
        .map_err(AppError::Internal)?;
    let scores: Vec<HqScoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "hq_score", value! { "scorer_id": technician_id })
        .await
        .context("failed to query technician scores")
        .map_err(AppError::Internal)?;
    let completed_tasks = tasks.iter().filter(|item| item.task_status.as_deref() == Some("completed")).count();
    let average_score = if scores.is_empty() {
        0.0
    } else {
        scores.iter().map(|item| item.total_score.unwrap_or_default() as f64).sum::<f64>() / scores.len() as f64
    };

    Ok(Json(ApiResponse::success(TechnicianPerformanceResponse {
        technician_id,
        total_tasks: tasks.len(),
        completed_tasks,
        average_score,
    })))
}

pub async fn list_available_technicians(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AvailableTechnicianQuery>,
) -> Result<Json<ApiResponse<Vec<TechnicianItem>>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;

    let store_relations: Vec<UserStoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_store", value! { "store_id": query.store_id })
        .await
        .context("failed to query available technician store relations")
        .map_err(AppError::Internal)?;
    let store_name = db::select_one_by_map_compat::<crate::models::store::StoreEntity>(state.rb.as_ref(), "store", value! { "id": query.store_id })
        .await
        .context("failed to query available technician store")
        .map_err(AppError::Internal)?
        .and_then(|item| item.store_name)
        .unwrap_or_default();
    let mut items = Vec::new();
    for relation in store_relations {
        let Some(user_id) = relation.user_id else { continue; };
        let user = find_user(&state, user_id).await?;
        if user.status.as_deref() != Some("enabled") {
            continue;
        }
        let skills: Vec<TechnicianSkillEntity> = db::select_by_map_compat(
            state.rb.as_ref(),
            "technician_skill",
            value! { "user_id": user_id, "service_type": query.service_type.as_str() },
        )
        .await
        .context("failed to query available technician skills")
        .map_err(AppError::Internal)?;
        if skills.is_empty() {
            continue;
        }
        items.push(TechnicianItem {
            id: user_id,
            name: user.name.unwrap_or_default(),
            nickname: user.nickname.clone(),
            mobile: user.mobile.unwrap_or_default(),
            avatar_url: user.avatar_url.clone(),
            job_no: user.job_no.clone(),
            status: user.status.unwrap_or_else(|| "enabled".to_string()),
            role_names: vec!["技师".to_string()],
            store_names: vec![store_name.clone()],
            work_status: "idle".to_string(),
            service_types: vec![query.service_type.clone()],
            average_score: 0.0,
            completed_order_count: 0,
            pending_order_count: 0,
            current_task_no: None,
            current_task_status: None,
            score_count: 0,
        });
    }

    Ok(Json(ApiResponse::success(items)))
}

async fn get_technician_detail_inner(
    state: &AppState,
    technician_id: i64,
) -> Result<Json<ApiResponse<TechnicianDetailResponse>>, AppError> {
    let user = find_user(state, technician_id).await?;
    let stores: Vec<UserStoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_store", value! { "user_id": technician_id })
        .await
        .context("failed to query technician stores")
        .map_err(AppError::Internal)?;
    let skills: Vec<TechnicianSkillEntity> = db::select_by_map_compat(state.rb.as_ref(), "technician_skill", value! { "user_id": technician_id })
        .await
        .context("failed to query technician skills")
        .map_err(AppError::Internal)?;
    let tasks: Vec<TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! { "current_technician_id": technician_id })
        .await
        .context("failed to query technician task counts")
        .map_err(AppError::Internal)?;
    let average_score = calculate_technician_average_score(state, technician_id).await?;
    let role_names = find_role_names(state, technician_id).await?;
    let store_ids: Vec<i64> = stores.iter().filter_map(|item| item.store_id).collect();
    let store_names = if store_ids.is_empty() {
        Vec::new()
    } else {
        db::select_by_map_compat::<crate::models::store::StoreEntity>(state.rb.as_ref(), "store", value! { "id": store_ids.clone() })
            .await
            .context("failed to query technician detail store names")
            .map_err(AppError::Internal)?
            .into_iter()
            .filter_map(|item| item.store_name)
            .collect::<Vec<_>>()
    };
    let current_task = tasks.iter().find(|item| matches!(item.task_status.as_deref(), Some("pending_execute") | Some("executing")));
    let work_status = if current_task.is_some() { "busy".to_string() } else { "idle".to_string() };
    let recent_tasks = tasks.iter()
        .take(10)
        .map(|item| TechnicianTaskItem {
            task_id: item.id.unwrap_or_default(),
            task_no: item.task_no.clone().unwrap_or_default(),
            service_type: item.service_type.clone().unwrap_or_default(),
            task_status: item.task_status.clone().unwrap_or_default(),
            customer_id: item.customer_id.unwrap_or_default(),
            appointment_time: item.appointment_time.map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string()),
        })
        .collect::<Vec<_>>();
    let monthly_trends = build_monthly_trends(&tasks);

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
        status: user.status.unwrap_or_else(|| "enabled".to_string()),
        store_ids,
        store_names,
        role_names,
        service_types: skills.into_iter().filter_map(|item| item.service_type).collect(),
        work_status,
        average_score,
        pending_task_count: tasks.iter().filter(|item| item.task_status.as_deref() != Some("completed") && item.task_status.as_deref() != Some("closed")).count(),
        completed_task_count: tasks.iter().filter(|item| item.task_status.as_deref() == Some("completed")).count(),
        current_task_no: current_task.and_then(|item| item.task_no.clone()),
        current_task_status: current_task.and_then(|item| item.task_status.clone()),
        recent_tasks,
        monthly_trends,
    })))
}

async fn find_role_id_by_code(state: &AppState, role_code: &str) -> Result<i64, AppError> {
    db::select_one_by_map_compat::<RoleEntity>(state.rb.as_ref(), "role", value! { "role_code": role_code })
        .await
        .context("failed to query role by code")
        .map_err(AppError::Internal)?
        .and_then(|item| item.id)
        .ok_or_else(|| AppError::Internal(anyhow!("role not found: {role_code}")))
}

async fn find_role_names(state: &AppState, user_id: i64) -> Result<Vec<String>, AppError> {
    let relations: Vec<UserRoleEntity> = db::select_by_map_compat(state.rb.as_ref(), "user_role", value! { "user_id": user_id })
        .await
        .context("failed to query technician role relations")
        .map_err(AppError::Internal)?;
    let role_ids: Vec<i64> = relations.into_iter().filter_map(|item| item.role_id).collect();
    if role_ids.is_empty() {
        return Ok(Vec::new());
    }
    let roles: Vec<RoleEntity> = db::select_by_map_compat(state.rb.as_ref(), "role", value! { "id": role_ids })
        .await
        .context("failed to query role names")
        .map_err(AppError::Internal)?;
    Ok(roles.into_iter().filter_map(|item| item.role_name).collect())
}

async fn calculate_technician_average_score(state: &AppState, technician_id: i64) -> Result<f64, AppError> {
    let tasks: Vec<TaskEntity> = db::select_by_map_compat(state.rb.as_ref(), "task", value! { "current_technician_id": technician_id })
        .await
        .context("failed to query technician tasks for score calc")
        .map_err(AppError::Internal)?;
    let task_ids: Vec<i64> = tasks.into_iter().filter_map(|item| item.id).collect();
    if task_ids.is_empty() {
        return Ok(0.0);
    }

    let scores: Vec<HqScoreEntity> = db::select_by_map_compat(state.rb.as_ref(), "hq_score", value! { "task_id": task_ids })
        .await
        .context("failed to query hq scores for technician score calc")
        .map_err(AppError::Internal)?;
    if scores.is_empty() {
        return Ok(0.0);
    }

    Ok(scores.iter().map(|item| item.total_score.unwrap_or_default() as f64).sum::<f64>() / scores.len() as f64)
}

fn build_monthly_trends(tasks: &[TaskEntity]) -> Vec<TechnicianTrendPoint> {
    let now = Utc::now();
    let mut points = Vec::new();
    for offset in (0..6).rev() {
        let month = now - chrono::Duration::days(30 * offset);
        let total_tasks = tasks
            .iter()
            .filter(|item| item.appointment_time.map(|dt| dt.year() == month.year() && dt.month() == month.month()).unwrap_or(false))
            .count();
        let completed_tasks = tasks
            .iter()
            .filter(|item| item.appointment_time.map(|dt| dt.year() == month.year() && dt.month() == month.month()).unwrap_or(false))
            .filter(|item| item.task_status.as_deref() == Some("completed"))
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
