use std::collections::{BTreeSet, HashMap};

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
    audit, auth, db,
    error::AppError,
    identity,
    models::{
        hq_score_item::HqScoreItemEntity, role::RoleEntity, task::TaskEntity,
        task_service_item::TaskServiceItemEntity, technician_skill::TechnicianSkillEntity,
        user::UserEntity, user_role::UserRoleEntity, user_store::UserStoreEntity,
    },
    response::ApiResponse,
    routes::{
        imports::{deserialize_vec_i64_from_mixed, normalize_delete_ids},
        users::{ensure_not_current_user, soft_delete_managed_user},
    },
    state::AppState,
};

#[derive(Debug, Deserialize)]
pub struct TechnicianListQuery {
    #[serde(alias = "phone")]
    pub mobile: Option<String>,
    pub name: Option<String>,
    pub status: Option<String>,
    #[serde(alias = "serviceType")]
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
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTechnicianRequest {
    #[serde(default, deserialize_with = "super::deserialize_patch_field")]
    pub name: Option<Option<String>>,
    #[serde(
        default,
        alias = "phone",
        deserialize_with = "super::deserialize_patch_field"
    )]
    pub mobile: Option<Option<String>>,
    #[serde(default, deserialize_with = "super::deserialize_patch_field")]
    pub nickname: Option<Option<String>>,
    #[serde(default, deserialize_with = "super::deserialize_patch_field")]
    pub email: Option<Option<String>>,
    #[serde(
        default,
        alias = "avatarUrl",
        deserialize_with = "super::deserialize_patch_field"
    )]
    pub avatar_url: Option<Option<String>>,
    #[serde(
        default,
        alias = "idCard",
        deserialize_with = "super::deserialize_patch_field"
    )]
    pub id_card: Option<Option<String>>,
    #[serde(
        default,
        alias = "emergencyContactName",
        deserialize_with = "super::deserialize_patch_field"
    )]
    pub emergency_contact_name: Option<Option<String>>,
    #[serde(
        default,
        alias = "emergencyContactMobile",
        deserialize_with = "super::deserialize_patch_field"
    )]
    pub emergency_contact_mobile: Option<Option<String>>,
    #[serde(
        default,
        alias = "jobNo",
        deserialize_with = "super::deserialize_patch_field"
    )]
    pub job_no: Option<Option<String>>,
    #[serde(
        default,
        alias = "storeId",
        deserialize_with = "super::deserialize_patch_field"
    )]
    pub store_id: Option<Option<i64>>,
    #[serde(
        default,
        alias = "serviceTypes",
        deserialize_with = "super::deserialize_patch_field"
    )]
    pub service_types: Option<Option<Vec<String>>>,
    #[serde(default, deserialize_with = "super::deserialize_patch_field")]
    pub status: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTechnicianStatusRequest {
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct BatchDeleteTechniciansRequest {
    #[serde(
        default,
        alias = "ids",
        alias = "user_ids",
        alias = "technicianIds",
        deserialize_with = "deserialize_vec_i64_from_mixed"
    )]
    pub technician_ids: Vec<i64>,
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
    pub average_score_text: String,
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
    pub average_score_text: String,
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
    pub average_score_text: String,
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

struct TechnicianListRecord {
    user: UserEntity,
    display_name: String,
    status: String,
    role_names: Vec<String>,
    store_ids: Vec<i64>,
    store_names: Vec<String>,
    service_types: Vec<String>,
}

fn technician_list_record_matches(
    record: &TechnicianListRecord,
    query: &TechnicianListQuery,
    status_filter: Option<&String>,
    dealer_store_ids: Option<&Vec<i64>>,
) -> bool {
    if let Some(name) = &query.name
        && !record.display_name.contains(name)
    {
        return false;
    }
    if let Some(mobile) = &query.mobile
        && !record
            .user
            .mobile
            .as_deref()
            .unwrap_or_default()
            .contains(mobile)
    {
        return false;
    }
    if let Some(status) = status_filter
        && record.status != *status
    {
        return false;
    }
    if let Some(service_type) = &query.service_type
        && !record.service_types.iter().any(|item| item == service_type)
    {
        return false;
    }
    if let Some(dealer_store_ids) = dealer_store_ids
        && !record
            .store_ids
            .iter()
            .any(|store_id| dealer_store_ids.contains(store_id))
    {
        return false;
    }
    true
}

pub async fn list_technicians(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TechnicianListQuery>,
) -> Result<Json<ApiResponse<TechnicianListResponse>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    let dealer_store_ids = dealer_scope_store_ids(&state, &auth_user).await?;
    let status_filter = normalize_optional_technician_status(query.status.as_deref())?;

    let technician_role_id = find_role_id_by_code(&state, "technician").await?;
    let mut records_by_id = load_technician_list_records(&state, technician_role_id)
        .await?
        .into_iter()
        .filter_map(|record| record.user.id.map(|user_id| (user_id, record)))
        .collect::<HashMap<_, _>>();
    records_by_id.retain(|_, record| {
        technician_list_record_matches(
            record,
            &query,
            status_filter.as_ref(),
            dealer_store_ids.as_ref(),
        )
    });
    let mut user_ids = records_by_id.keys().copied().collect::<Vec<_>>();
    user_ids.sort_by(|left, right| right.cmp(left));
    let total = user_ids.len();
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).max(1);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let user_ids = user_ids
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();
    let mut stats_by_user = load_technician_aggregate_stats_batch(&state, &user_ids).await?;
    let mut items = Vec::new();
    for user_id in user_ids {
        let Some(record) = records_by_id.remove(&user_id) else {
            continue;
        };
        let user = record.user;
        let display_name = record.display_name;
        let technician_status = record.status;
        let service_types = record.service_types;
        let role_names = record.role_names;
        let store_names = record.store_names;
        let stats = stats_by_user
            .remove(&user_id)
            .unwrap_or_else(|| aggregate_technician_stats(Vec::new(), Vec::new()));

        items.push(TechnicianItem {
            id: user_id,
            name: display_name,
            nickname: user.nickname.clone(),
            mobile: user.mobile.unwrap_or_default(),
            avatar_url: user.avatar_url.clone(),
            job_no: user.job_no.clone(),
            status: technician_status_text(Some(&technician_status)).to_string(),
            status_code: technician_status.clone(),
            status_text: technician_status_text(Some(&technician_status)).to_string(),
            role_names,
            store_names,
            work_status: stats.work_status,
            service_types,
            average_score: stats.average_score,
            average_score_text: format_score(stats.average_score),
            completed_order_count: stats.completed_count,
            pending_order_count: stats.pending_count,
            current_task_no: stats.current_task_no,
            current_task_status: stats.current_task_status,
            score_count: stats.score_count,
        });
    }
    Ok(Json(ApiResponse::success(TechnicianListResponse {
        items,
        total,
        page,
        page_size,
    })))
}

async fn load_technician_list_records(
    state: &AppState,
    technician_role_id: i64,
) -> Result<Vec<TechnicianListRecord>, AppError> {
    let technician_roles: Vec<UserRoleEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "user_role",
        value! { "role_id": technician_role_id },
    )
    .await
    .context("failed to query technician role relations")
    .map_err(AppError::Internal)?;
    let user_ids = technician_roles
        .iter()
        .filter_map(|item| item.user_id)
        .collect::<Vec<_>>();
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }

    let (users, skills, user_roles, user_stores) = tokio::try_join!(
        async {
            db::select_by_map_compat::<UserEntity>(
                state.rb.as_ref(),
                "user",
                value! { "id": user_ids.clone() },
            )
            .await
            .context("failed to batch query technician users")
            .map_err(AppError::Internal)
        },
        async {
            db::select_by_map_compat::<TechnicianSkillEntity>(
                state.rb.as_ref(),
                "technician_skill",
                value! { "user_id": user_ids.clone() },
            )
            .await
            .context("failed to batch query technician skills")
            .map_err(AppError::Internal)
        },
        async {
            db::select_by_map_compat::<UserRoleEntity>(
                state.rb.as_ref(),
                "user_role",
                value! { "user_id": user_ids.clone() },
            )
            .await
            .context("failed to batch query technician roles")
            .map_err(AppError::Internal)
        },
        async {
            db::select_by_map_compat::<UserStoreEntity>(
                state.rb.as_ref(),
                "user_store",
                value! { "user_id": user_ids.clone() },
            )
            .await
            .context("failed to batch query technician stores")
            .map_err(AppError::Internal)
        },
    )?;

    let identity_by_user = technician_roles
        .into_iter()
        .filter_map(|item| item.user_id.map(|user_id| (user_id, item)))
        .collect::<HashMap<_, _>>();
    let mut users_by_id = users
        .into_iter()
        .filter(|item| item.deleted_at.is_none())
        .filter_map(|item| item.id.map(|id| (id, item)))
        .collect::<HashMap<_, _>>();
    let mut skills_by_user = HashMap::<i64, Vec<String>>::new();
    for skill in skills {
        if let (Some(user_id), Some(service_type)) = (skill.user_id, skill.service_type) {
            skills_by_user
                .entry(user_id)
                .or_default()
                .push(service_type);
        }
    }
    let mut role_ids_by_user = HashMap::<i64, Vec<i64>>::new();
    let mut related_role_ids = BTreeSet::new();
    for relation in user_roles {
        if let (Some(user_id), Some(role_id)) = (relation.user_id, relation.role_id) {
            role_ids_by_user.entry(user_id).or_default().push(role_id);
            related_role_ids.insert(role_id);
        }
    }
    let mut store_ids_by_user = HashMap::<i64, Vec<i64>>::new();
    let mut related_store_ids = BTreeSet::new();
    for relation in user_stores {
        if let (Some(user_id), Some(store_id)) = (relation.user_id, relation.store_id) {
            store_ids_by_user.entry(user_id).or_default().push(store_id);
            related_store_ids.insert(store_id);
        }
    }

    let role_ids = related_role_ids.into_iter().collect::<Vec<_>>();
    let store_ids = related_store_ids.into_iter().collect::<Vec<_>>();
    let (roles, stores) = tokio::try_join!(
        async {
            if role_ids.is_empty() {
                Ok(Vec::new())
            } else {
                db::select_by_map_compat::<RoleEntity>(
                    state.rb.as_ref(),
                    "role",
                    value! { "id": role_ids },
                )
                .await
                .context("failed to batch query technician role names")
                .map_err(AppError::Internal)
            }
        },
        async {
            if store_ids.is_empty() {
                Ok(Vec::new())
            } else {
                db::select_by_map_compat::<crate::models::store::StoreEntity>(
                    state.rb.as_ref(),
                    "store",
                    value! { "id": store_ids },
                )
                .await
                .context("failed to batch query technician store names")
                .map_err(AppError::Internal)
            }
        },
    )?;
    let role_name_by_id = roles
        .into_iter()
        .filter_map(|item| Some((item.id?, item.role_name?)))
        .collect::<HashMap<_, _>>();
    let store_name_by_id = stores
        .into_iter()
        .filter_map(|item| Some((item.id?, item.store_name?)))
        .collect::<HashMap<_, _>>();

    Ok(user_ids
        .into_iter()
        .filter_map(|user_id| {
            let user = users_by_id.remove(&user_id)?;
            let role_names = role_ids_by_user
                .remove(&user_id)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|role_id| role_name_by_id.get(&role_id).cloned())
                .collect();
            let store_ids = store_ids_by_user.remove(&user_id).unwrap_or_default();
            let store_names = store_ids
                .iter()
                .filter_map(|store_id| store_name_by_id.get(store_id).cloned())
                .collect();
            Some(TechnicianListRecord {
                display_name: identity::relation_display_name(
                    identity_by_user.get(&user_id),
                    user.name.as_deref(),
                ),
                user,
                status: identity_by_user
                    .get(&user_id)
                    .map(|item| canonical_technician_status(item.status.as_deref()))
                    .unwrap_or_else(|| "enabled".to_string()),
                role_names,
                store_ids,
                store_names,
                service_types: skills_by_user.remove(&user_id).unwrap_or_default(),
            })
        })
        .collect())
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

    let password = resolve_initial_password(payload.password.as_deref());
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
    let normalized_status = normalize_technician_status(payload.status.as_deref())?;

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
    let user_existed_before = !existing_users.is_empty();
    let user = if let Some(existing_user) = existing_users.into_iter().next() {
        if existing_user.deleted_at.is_some() {
            let user_id = existing_user
                .id
                .ok_or_else(|| AppError::Internal(anyhow!("deleted technician user.id missing")))?;
            let password_hash = hash(&password, 10)
                .context("failed to hash restored technician password")
                .map_err(AppError::Internal)?;
            let job_no = payload.job_no.clone().or_else(|| {
                Some(format!(
                    "SF{:08}",
                    Utc::now().timestamp_millis().rem_euclid(100_000_000)
                ))
            });
            UserEntity::update_by_map(
                state.rb.as_ref(),
                &UserEntity {
                    password_hash: Some(password_hash),
                    name: Some(payload.name.clone()),
                    nickname: payload.nickname.clone(),
                    avatar_url: payload.avatar_url.clone(),
                    job_no,
                    id_card: payload.id_card.clone(),
                    emergency_contact_name: payload.emergency_contact_name.clone(),
                    emergency_contact_mobile: payload.emergency_contact_mobile.clone(),
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
            .context("failed to restore deleted technician user")
            .map_err(AppError::Internal)?;
            UserRoleEntity::delete_by_map(state.rb.as_ref(), value! { "user_id": user_id })
                .await
                .context("failed to clear roles for restored technician user")
                .map_err(AppError::Internal)?;
            UserStoreEntity::delete_by_map(state.rb.as_ref(), value! { "user_id": user_id })
                .await
                .context("failed to clear stores for restored technician user")
                .map_err(AppError::Internal)?;
            db::select_one_by_map_compat(state.rb.as_ref(), "user", value! { "id": user_id })
                .await
                .context("failed to fetch restored technician user")
                .map_err(AppError::Internal)?
                .ok_or_else(|| {
                    AppError::Internal(anyhow!("technician restore succeeded but fetch failed"))
                })?
        } else {
            existing_user
        }
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
        .ok_or_else(|| {
            AppError::Internal(anyhow!("technician create succeeded but fetch failed"))
        })?
    };
    let user_id = user.id.unwrap_or_default();

    ensure_technician_role_binding(&state, user_id).await?;
    identity::update_role_display_name(&state, user_id, "technician", &payload.name).await?;
    update_technician_role_status(&state, user_id, &normalized_status).await?;
    upsert_single_store_binding(&state, user_id, store_id).await?;
    replace_technician_skills(&state, user_id, service_types).await?;
    if user_existed_before {
        auth::bump_user_session_version(&state, user_id).await?;
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
    ensure_technician_access(&state, &auth_user, technician_id).await?;

    let user = find_user(&state, technician_id).await?;
    let role_id = find_role_id_by_code(&state, "technician").await?;
    let role_relation = db::select_one_by_map_compat::<UserRoleEntity>(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": technician_id, "role_id": role_id },
    )
    .await
    .context("failed to query technician role before update")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::NotFound("师傅角色不存在".to_string()))?;
    let current_status = canonical_technician_status(role_relation.status.as_deref());
    let current_name = identity::relation_display_name(Some(&role_relation), user.name.as_deref());
    let existing_skills: Vec<TechnicianSkillEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "technician_skill",
        value! { "user_id": technician_id },
    )
    .await
    .context("failed to query technician skills before update")
    .map_err(AppError::Internal)?;
    let existing_store_ids = load_user_store_ids(&state, technician_id).await?;
    // Full admin edits are form snapshots; their serializer omits cleared ID/contact inputs.
    let is_full_admin_form = is_full_admin_technician_form(&payload);

    let next_name =
        resolve_required_patch_text(&payload.name, Some(&current_name), "请输入师傅姓名")?;
    let next_mobile =
        resolve_required_patch_text(&payload.mobile, user.mobile.as_deref(), "请输入手机号")?;
    let next_nickname = resolve_optional_patch_text(&payload.nickname, user.nickname.as_deref());
    let next_email = resolve_optional_patch_text(&payload.email, user.email.as_deref());
    let next_avatar_url =
        resolve_optional_patch_text(&payload.avatar_url, user.avatar_url.as_deref());
    let next_id_card = resolve_admin_form_optional_text(
        &payload.id_card,
        user.id_card.as_deref(),
        is_full_admin_form,
    );
    let next_emergency_contact_name = resolve_admin_form_optional_text(
        &payload.emergency_contact_name,
        user.emergency_contact_name.as_deref(),
        is_full_admin_form,
    );
    let next_emergency_contact_mobile = resolve_admin_form_optional_text(
        &payload.emergency_contact_mobile,
        user.emergency_contact_mobile.as_deref(),
        is_full_admin_form,
    );
    let next_job_no = resolve_optional_patch_text(&payload.job_no, user.job_no.as_deref());
    let next_status = resolve_update_technician_status(&payload.status, &current_status)?;
    let next_service_types =
        resolve_update_service_types(&payload.service_types, &existing_skills)?;
    let next_store_id = resolve_update_store_id(&payload.store_id, &existing_store_ids)?;
    let current_service_types = normalize_service_types(
        Some(
            existing_skills
                .iter()
                .filter_map(|item| item.service_type.clone())
                .collect(),
        ),
        None,
    )?;
    let mobile_changed =
        payload.mobile.is_some() && user.mobile.as_deref() != Some(next_mobile.as_str());
    let name_changed = payload.name.is_some() && next_name != current_name;
    let status_changed = payload.status.is_some() && next_status != current_status;
    let skills_changed = payload.service_types.is_some()
        && !same_service_types(&next_service_types, &current_service_types);
    let store_changed =
        payload.store_id.is_some() && existing_store_ids.as_slice() != [next_store_id];
    let should_bump_session = mobile_changed || status_changed || skills_changed || store_changed;

    ensure_technician_mobile_available(&state, technician_id, &next_mobile).await?;
    let next_store_id = resolve_technician_store_id(&state, &auth_user, next_store_id).await?;
    let next_store = db::select_one_by_map_compat::<crate::models::store::StoreEntity>(
        state.rb.as_ref(),
        "store",
        value! { "id": next_store_id },
    )
    .await
    .context("failed to query technician store before update")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::BadRequest("所属门店不存在".to_string()))?;

    let Json(current_response) = get_technician_detail_inner(&state, technician_id).await?;
    let mut response = current_response
        .data
        .ok_or_else(|| AppError::Internal(anyhow!("technician detail response data missing")))?;
    response.name = next_name.clone();
    response.mobile = next_mobile.clone();
    response.nickname = next_nickname.clone();
    response.email = next_email.clone();
    response.avatar_url = next_avatar_url.clone();
    response.job_no = next_job_no.clone();
    response.id_card = next_id_card.clone();
    response.emergency_contact_name = next_emergency_contact_name.clone();
    response.emergency_contact_mobile = next_emergency_contact_mobile.clone();
    response.status = technician_status_text(Some(&next_status)).to_string();
    response.status_code = next_status.clone();
    response.status_text = technician_status_text(Some(&next_status)).to_string();
    response.store_ids = vec![next_store_id];
    response.store_names = next_store.store_name.into_iter().collect();
    response.service_types = next_service_types.clone();

    let current_session_version = user.session_version.unwrap_or(1);
    let next_session_version = if should_bump_session {
        Some(
            current_session_version
                .checked_add(1)
                .ok_or_else(|| AppError::Conflict("用户会话版本已达到上限".to_string()))?,
        )
    } else {
        None
    };
    let tx = state
        .rb
        .acquire_begin()
        .await
        .context("failed to begin technician update transaction")
        .map_err(AppError::Internal)?
        .defer_async(|tx| async move {
            if !tx.done() {
                let _ = tx.rollback().await;
            }
        });

    let write_result: Result<(), AppError> = async {
        let mut user_update = UserEntity::default();
        let mut user_columns = Vec::new();
        if let Some(next_session_version) = next_session_version {
            user_update.session_version = Some(next_session_version);
            user_columns.push("session_version");
        }
        if payload.mobile.is_some() {
            user_update.mobile = Some(next_mobile.clone());
            user_columns.push("mobile");
        }
        if payload.nickname.is_some() {
            user_update.nickname = next_nickname.clone();
            user_columns.push("nickname");
        }
        if payload.email.is_some() {
            user_update.email = next_email.clone();
            user_columns.push("email");
        }
        if payload.avatar_url.is_some() {
            user_update.avatar_url = next_avatar_url.clone();
            user_columns.push("avatar_url");
        }
        if payload.job_no.is_some() {
            user_update.job_no = next_job_no.clone();
            user_columns.push("job_no");
        }
        if is_full_admin_form || payload.id_card.is_some() {
            user_update.id_card = next_id_card.clone();
            user_columns.push("id_card");
        }
        if is_full_admin_form || payload.emergency_contact_name.is_some() {
            user_update.emergency_contact_name = next_emergency_contact_name.clone();
            user_columns.push("emergency_contact_name");
        }
        if is_full_admin_form || payload.emergency_contact_mobile.is_some() {
            user_update.emergency_contact_mobile = next_emergency_contact_mobile.clone();
            user_columns.push("emergency_contact_mobile");
        }

        if !user_columns.is_empty() {
            let condition = if should_bump_session {
                value! {
                    "id": technician_id,
                    "session_version": current_session_version,
                    "column": user_columns
                }
            } else {
                value! { "id": technician_id, "column": user_columns }
            };
            let user_update_result = UserEntity::update_by_map(&tx, &user_update, condition)
                .await
                .map_err(|error| {
                    let message = error.to_string();
                    if message.contains("Duplicate entry") || message.contains("1062") {
                        AppError::Conflict("该手机号账号已存在".to_string())
                    } else {
                        AppError::Internal(anyhow!("failed to update technician user: {error}"))
                    }
                })?;
            if should_bump_session && user_update_result.rows_affected == 0 {
                return Err(AppError::Conflict(
                    "师傅资料已被其他操作更新，请刷新后重试".to_string(),
                ));
            }
        }

        if status_changed || name_changed {
            let mut role_update = UserRoleEntity::default();
            let mut role_columns = Vec::new();
            if status_changed {
                role_update.status = Some(next_status.clone());
                role_columns.push("status");
            }
            if name_changed {
                role_update.display_name = Some(next_name.clone());
                role_columns.push("display_name");
            }
            let role_update_result = UserRoleEntity::update_by_map(
                &tx,
                &role_update,
                value! {
                    "user_id": technician_id,
                    "role_id": role_id,
                    "column": role_columns
                },
            )
            .await
            .context("failed to update technician role profile")
            .map_err(AppError::Internal)?;
            if role_update_result.rows_affected == 0 {
                return Err(AppError::Conflict(
                    "师傅身份资料已被其他操作更新，请刷新后重试".to_string(),
                ));
            }
        }

        if skills_changed {
            TechnicianSkillEntity::delete_by_map(&tx, value! { "user_id": technician_id })
                .await
                .context("failed to delete technician skills")
                .map_err(AppError::Internal)?;
            for service_type in &next_service_types {
                TechnicianSkillEntity::insert(
                    &tx,
                    &TechnicianSkillEntity {
                        id: None,
                        user_id: Some(technician_id),
                        service_type: Some(service_type.clone()),
                        created_at: None,
                    },
                )
                .await
                .context("failed to recreate technician skill")
                .map_err(AppError::Internal)?;
            }
        }

        if store_changed {
            UserStoreEntity::delete_by_map(&tx, value! { "user_id": technician_id })
                .await
                .context("failed to delete existing technician store relations")
                .map_err(AppError::Internal)?;
            UserStoreEntity::insert(
                &tx,
                &UserStoreEntity {
                    id: None,
                    user_id: Some(technician_id),
                    store_id: Some(next_store_id),
                    created_at: None,
                },
            )
            .await
            .context("failed to bind technician store")
            .map_err(AppError::Internal)?;
        }

        Ok(())
    }
    .await;

    if let Err(error) = write_result {
        if let Err(rollback_error) = tx.rollback().await {
            tracing::error!(%rollback_error, "failed to roll back technician update transaction");
        }
        return Err(error);
    }
    tx.commit()
        .await
        .context("failed to commit technician update transaction")
        .map_err(AppError::Internal)?;

    Ok(Json(ApiResponse::success(response)))
}

pub async fn delete_technician(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(technician_id): Path<i64>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    ensure_not_current_user(&auth_user, technician_id)?;

    let technician = find_user(&state, technician_id).await?;
    ensure_technician_user(&state, technician_id).await?;
    ensure_technician_access(&state, &auth_user, technician_id).await?;
    soft_delete_managed_user(
        &state,
        &auth_user,
        &technician,
        "delete_technician",
        "后台删除师傅",
    )
    .await?;

    Ok(Json(ApiResponse::success(
        serde_json::json!({ "ok": true }),
    )))
}

pub async fn batch_delete_technicians(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<BatchDeleteTechniciansRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let auth_user = auth::authenticate(&state, &headers)?;
    auth_user.require_any_role(&["dealer", "hq_reviewer", "super_admin"])?;
    let technician_ids = normalize_delete_ids(payload.technician_ids, "technician_ids")?;

    let mut technicians = Vec::with_capacity(technician_ids.len());
    for technician_id in technician_ids {
        ensure_not_current_user(&auth_user, technician_id)?;
        let technician = find_user(&state, technician_id).await?;
        ensure_technician_user(&state, technician_id).await?;
        ensure_technician_access(&state, &auth_user, technician_id).await?;
        technicians.push(technician);
    }

    let deleted_count = technicians.len();
    for technician in &technicians {
        soft_delete_managed_user(
            &state,
            &auth_user,
            technician,
            "batch_delete_technician",
            "后台批量删除师傅",
        )
        .await?;
    }

    Ok(Json(ApiResponse::success(serde_json::json!({
        "ok": true,
        "deleted_count": deleted_count
    }))))
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

    let status = normalize_required_technician_status(&payload.status)?;

    update_technician_role_status(&state, technician_id, &status).await?;

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
    let before_user = find_user(&state, technician_id).await?;

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
    auth::bump_user_session_version(&state, technician_id).await?;

    let result = Json(ApiResponse::success(
        serde_json::json!({ "ok": true, "default_password": "123456" }),
    ));
    let after_user = find_user(&state, technician_id).await?;
    audit::write_audit_log(
        &state,
        "user",
        technician_id,
        "reset_technician_password",
        auth_user.user_id,
        auth_user.primary_role(),
        Some(&before_user),
        Some(&after_user),
        Some("后台重置师傅密码".to_string()),
    )
    .await?;
    Ok(result)
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
        average_score_text: format_score(stats.average_score),
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
        if !has_technician_role(state, user_id).await? {
            continue;
        }
        let Some(user) = find_user_record(state, user_id).await? else {
            continue;
        };
        if user.deleted_at.is_some() {
            continue;
        }
        let technician_status = technician_role_status(state, user_id).await?;
        if technician_status != "enabled" {
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
        let display_name =
            identity::role_display_name(state, user_id, "technician", user.name.as_deref()).await?;
        items.push(TechnicianItem {
            id: user_id,
            name: display_name,
            nickname: user.nickname.clone(),
            mobile: user.mobile.unwrap_or_default(),
            avatar_url: user.avatar_url.clone(),
            job_no: user.job_no.clone(),
            status: technician_status_text(Some(&technician_status)).to_string(),
            status_code: technician_status.clone(),
            status_text: technician_status_text(Some(&technician_status)).to_string(),
            role_names: vec!["技师".to_string()],
            store_names: vec![store_name.to_string()],
            work_status: stats.work_status,
            service_types,
            average_score: stats.average_score,
            average_score_text: format_score(stats.average_score),
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

fn resolve_initial_password(password: Option<&str>) -> String {
    password
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .unwrap_or("123456")
        .to_string()
}

fn normalize_technician_status(raw: Option<&str>) -> Result<String, AppError> {
    let Some(value) = raw.map(str::trim).filter(|item| !item.is_empty()) else {
        return Ok("enabled".to_string());
    };

    match value {
        "启用" | "已启用" => Ok("enabled".to_string()),
        "禁用" | "已禁用" => Ok("disabled".to_string()),
        _ => match value.to_ascii_lowercase().as_str() {
            "enabled" | "active" => Ok("enabled".to_string()),
            "disabled" | "inactive" => Ok("disabled".to_string()),
            _ => Err(AppError::BadRequest(
                "status只能为启用/禁用或enabled/disabled".to_string(),
            )),
        },
    }
}

fn normalize_optional_technician_status(raw: Option<&str>) -> Result<Option<String>, AppError> {
    match raw.map(str::trim).filter(|item| !item.is_empty()) {
        Some(value) => normalize_technician_status(Some(value)).map(Some),
        None => Ok(None),
    }
}

fn normalize_required_technician_status(raw: &str) -> Result<String, AppError> {
    if raw.trim().is_empty() {
        return Err(AppError::BadRequest(
            "status只能为启用/禁用或enabled/disabled".to_string(),
        ));
    }
    normalize_technician_status(Some(raw))
}

fn canonical_technician_status(raw: Option<&str>) -> String {
    normalize_technician_status(raw).unwrap_or_else(|_| "enabled".to_string())
}

fn resolve_required_patch_text(
    patch: &Option<Option<String>>,
    current: Option<&str>,
    empty_message: &str,
) -> Result<String, AppError> {
    let value = match patch {
        None => current.unwrap_or_default(),
        Some(Some(value)) => value.as_str(),
        Some(None) => return Err(AppError::BadRequest(empty_message.to_string())),
    };
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest(empty_message.to_string()));
    }
    Ok(value.to_string())
}

fn resolve_optional_patch_text(
    patch: &Option<Option<String>>,
    current: Option<&str>,
) -> Option<String> {
    match patch {
        None => current.map(ToString::to_string),
        Some(None) => None,
        Some(Some(value)) => {
            let value = value.trim();
            if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            }
        }
    }
}

fn is_full_admin_technician_form(payload: &UpdateTechnicianRequest) -> bool {
    payload.name.is_some()
        && payload.mobile.is_some()
        && payload.store_id.is_some()
        && payload.service_types.is_some()
        && payload.status.is_some()
}

fn resolve_admin_form_optional_text(
    patch: &Option<Option<String>>,
    current: Option<&str>,
    is_full_admin_form: bool,
) -> Option<String> {
    if !is_full_admin_form || patch.is_some() {
        resolve_optional_patch_text(patch, current)
    } else {
        None
    }
}

fn resolve_update_technician_status(
    patch: &Option<Option<String>>,
    current: &str,
) -> Result<String, AppError> {
    match patch {
        None => normalize_required_technician_status(current),
        Some(Some(value)) if value.trim().is_empty() => {
            normalize_required_technician_status(current)
        }
        Some(Some(value)) => normalize_required_technician_status(value),
        Some(None) => Err(AppError::BadRequest(
            "status只能为启用/禁用或enabled/disabled".to_string(),
        )),
    }
}

fn resolve_update_service_types(
    patch: &Option<Option<Vec<String>>>,
    current: &[TechnicianSkillEntity],
) -> Result<Vec<String>, AppError> {
    let service_types = match patch {
        None => current
            .iter()
            .filter_map(|item| item.service_type.clone())
            .collect(),
        Some(Some(service_types)) => service_types.clone(),
        Some(None) => {
            return Err(AppError::BadRequest("请至少选择一个服务能力".to_string()));
        }
    };
    let service_types = normalize_service_types(Some(service_types), None)?;
    if service_types.is_empty() {
        return Err(AppError::BadRequest("请至少选择一个服务能力".to_string()));
    }
    Ok(service_types)
}

fn same_service_types(left: &[String], right: &[String]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort();
    right.sort();
    left == right
}

fn resolve_update_store_id(
    patch: &Option<Option<i64>>,
    current_store_ids: &[i64],
) -> Result<i64, AppError> {
    match patch {
        Some(Some(store_id)) if *store_id > 0 => Ok(*store_id),
        Some(Some(_)) | Some(None) => Err(AppError::BadRequest("请选择所属门店".to_string())),
        None => match current_store_ids {
            [store_id] => Ok(*store_id),
            [] => Err(AppError::BadRequest("请选择所属门店".to_string())),
            _ => Err(AppError::Conflict(
                "当前师傅绑定了多个门店，请重新选择所属门店".to_string(),
            )),
        },
    }
}

async fn ensure_technician_mobile_available(
    state: &AppState,
    technician_id: i64,
    mobile: &str,
) -> Result<(), AppError> {
    let mobile_users: Vec<UserEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "user", value! { "mobile": mobile })
            .await
            .context("failed to check technician mobile uniqueness")
            .map_err(AppError::Internal)?;
    if mobile_users
        .iter()
        .any(|item| item.id != Some(technician_id))
    {
        return Err(AppError::Conflict("该手机号账号已存在".to_string()));
    }

    Ok(())
}

async fn get_technician_detail_inner(
    state: &AppState,
    technician_id: i64,
) -> Result<Json<ApiResponse<TechnicianDetailResponse>>, AppError> {
    let user = find_user(state, technician_id).await?;
    let technician_role_id = find_role_id_by_code(state, "technician").await?;
    let technician_relation = db::select_one_by_map_compat::<UserRoleEntity>(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": technician_id, "role_id": technician_role_id },
    )
    .await
    .context("failed to query technician role for detail")
    .map_err(AppError::Internal)?
    .ok_or_else(|| AppError::NotFound("师傅不存在或已删除".to_string()))?;
    let technician_status = canonical_technician_status(technician_relation.status.as_deref());
    let display_name =
        identity::relation_display_name(Some(&technician_relation), user.name.as_deref());
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
        name: display_name,
        nickname: user.nickname,
        mobile: user.mobile.unwrap_or_default(),
        email: user.email,
        avatar_url: user.avatar_url,
        job_no: user.job_no,
        id_card: user.id_card,
        emergency_contact_name: user.emergency_contact_name,
        emergency_contact_mobile: user.emergency_contact_mobile,
        status: technician_status_text(Some(&technician_status)).to_string(),
        status_code: technician_status.clone(),
        status_text: technician_status_text(Some(&technician_status)).to_string(),
        store_ids,
        store_names,
        role_names,
        service_types: skills
            .into_iter()
            .filter_map(|item| item.service_type)
            .collect(),
        work_status: stats.work_status,
        average_score: stats.average_score,
        average_score_text: format_score(stats.average_score),
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

async fn has_technician_role(state: &AppState, user_id: i64) -> Result<bool, AppError> {
    let role_id = find_role_id_by_code(state, "technician").await?;
    let relation = db::select_one_by_map_compat::<UserRoleEntity>(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": user_id, "role_id": role_id },
    )
    .await
    .context("failed to query technician role relation")
    .map_err(AppError::Internal)?;
    Ok(relation.is_some())
}

async fn ensure_technician_user(state: &AppState, user_id: i64) -> Result<(), AppError> {
    if has_technician_role(state, user_id).await? {
        return Ok(());
    }
    Err(AppError::NotFound("师傅不存在或已删除".to_string()))
}

async fn technician_role_status(state: &AppState, user_id: i64) -> Result<String, AppError> {
    let role_id = find_role_id_by_code(state, "technician").await?;
    let relation = db::select_one_by_map_compat::<UserRoleEntity>(
        state.rb.as_ref(),
        "user_role",
        value! { "user_id": user_id, "role_id": role_id },
    )
    .await
    .context("failed to query technician role status")
    .map_err(AppError::Internal)?;
    Ok(canonical_technician_status(
        relation.and_then(|item| item.status).as_deref(),
    ))
}

async fn update_technician_role_status(
    state: &AppState,
    user_id: i64,
    status: &str,
) -> Result<(), AppError> {
    let role_id = find_role_id_by_code(state, "technician").await?;
    state
        .rb
        .as_ref()
        .exec(
            "update `user_role` set `status` = ? where `user_id` = ? and `role_id` = ?",
            vec![value!(status), value!(user_id), value!(role_id)],
        )
        .await
        .context("failed to update technician role status")
        .map_err(AppError::Internal)?;
    Ok(())
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
            status: Some("enabled".to_string()),
            display_name: None,
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

async fn load_technician_aggregate_stats_batch(
    state: &AppState,
    technician_ids: &[i64],
) -> Result<HashMap<i64, TechnicianAggregateStats>, AppError> {
    if technician_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let (assigned_service_items, legacy_tasks) = tokio::try_join!(
        async {
            db::select_by_map_compat::<TaskServiceItemEntity>(
                state.rb.as_ref(),
                "task_service_item",
                value! { "technician_id": technician_ids.to_vec() },
            )
            .await
            .context("failed to batch query technician service items")
            .map_err(AppError::Internal)
        },
        async {
            db::select_by_map_compat::<TaskEntity>(
                state.rb.as_ref(),
                "task",
                value! { "current_technician_id": technician_ids.to_vec() },
            )
            .await
            .context("failed to batch query legacy technician tasks")
            .map_err(AppError::Internal)
        },
    )?;
    let assigned_task_ids = assigned_service_items
        .iter()
        .filter_map(|item| item.task_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let legacy_task_ids = legacy_tasks
        .iter()
        .filter_map(|item| item.id)
        .collect::<Vec<_>>();
    let all_task_ids = assigned_task_ids
        .iter()
        .chain(legacy_task_ids.iter())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let (assigned_tasks, legacy_service_items, score_items) = tokio::try_join!(
        load_tasks_for_technician_stats(state, &assigned_task_ids),
        load_service_items_for_technician_stats(state, &legacy_task_ids),
        load_scores_for_technician_stats(state, &all_task_ids),
    )?;
    let mut tasks_by_id = assigned_tasks
        .into_iter()
        .filter(|item| item.deleted_at.is_none())
        .filter_map(|item| item.id.map(|task_id| (task_id, item)))
        .collect::<HashMap<_, _>>();
    for task in legacy_tasks.iter().filter(|item| item.deleted_at.is_none()) {
        if let Some(task_id) = task.id {
            tasks_by_id.entry(task_id).or_insert_with(|| task.clone());
        }
    }
    let legacy_task_ids_with_service_items = legacy_service_items
        .into_iter()
        .filter_map(|item| item.task_id)
        .collect::<BTreeSet<_>>();
    let mut scopes_by_user = HashMap::<i64, Vec<TechnicianTaskScope>>::new();
    for service_item in assigned_service_items {
        let (Some(technician_id), Some(task_id)) =
            (service_item.technician_id, service_item.task_id)
        else {
            continue;
        };
        let Some(task) = tasks_by_id.get(&task_id) else {
            continue;
        };
        scopes_by_user
            .entry(technician_id)
            .or_default()
            .push(TechnicianTaskScope {
                task: task.clone(),
                service_item: Some(service_item),
            });
    }
    for task in legacy_tasks {
        let (Some(technician_id), Some(task_id)) = (task.current_technician_id, task.id) else {
            continue;
        };
        if task.deleted_at.is_some() || legacy_task_ids_with_service_items.contains(&task_id) {
            continue;
        }
        scopes_by_user
            .entry(technician_id)
            .or_default()
            .push(TechnicianTaskScope {
                task,
                service_item: None,
            });
    }
    let mut scores_by_scope = HashMap::<(i64, String), Vec<HqScoreItemEntity>>::new();
    for score_item in score_items {
        let (Some(task_id), Some(service_type)) =
            (score_item.task_id, score_item.service_type.clone())
        else {
            continue;
        };
        scores_by_scope
            .entry((task_id, service_type))
            .or_default()
            .push(score_item);
    }

    Ok(technician_ids
        .iter()
        .copied()
        .map(|technician_id| {
            let task_scopes = scopes_by_user.remove(&technician_id).unwrap_or_default();
            let score_items = task_scopes
                .iter()
                .filter_map(|scope| {
                    Some((
                        scope.task.id?,
                        technician_scope_service_type(scope)?.to_string(),
                    ))
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .flat_map(|key| scores_by_scope.get(&key).into_iter().flatten().cloned())
                .collect::<Vec<_>>();
            (
                technician_id,
                aggregate_technician_stats(task_scopes, score_items),
            )
        })
        .collect())
}

async fn load_tasks_for_technician_stats(
    state: &AppState,
    task_ids: &[i64],
) -> Result<Vec<TaskEntity>, AppError> {
    if task_ids.is_empty() {
        return Ok(Vec::new());
    }
    db::select_by_map_compat(
        state.rb.as_ref(),
        "task",
        value! { "id": task_ids.to_vec() },
    )
    .await
    .context("failed to batch query assigned technician tasks")
    .map_err(AppError::Internal)
}

async fn load_service_items_for_technician_stats(
    state: &AppState,
    task_ids: &[i64],
) -> Result<Vec<TaskServiceItemEntity>, AppError> {
    if task_ids.is_empty() {
        return Ok(Vec::new());
    }
    db::select_by_map_compat(
        state.rb.as_ref(),
        "task_service_item",
        value! { "task_id": task_ids.to_vec() },
    )
    .await
    .context("failed to batch query legacy task service items")
    .map_err(AppError::Internal)
}

async fn load_scores_for_technician_stats(
    state: &AppState,
    task_ids: &[i64],
) -> Result<Vec<HqScoreItemEntity>, AppError> {
    if task_ids.is_empty() {
        return Ok(Vec::new());
    }
    db::select_by_map_compat(
        state.rb.as_ref(),
        "hq_score_item",
        value! { "task_id": task_ids.to_vec() },
    )
    .await
    .context("failed to batch query technician scores")
    .map_err(AppError::Internal)
}

async fn build_technician_aggregate_stats(
    state: &AppState,
    technician_id: i64,
) -> Result<TechnicianAggregateStats, AppError> {
    let task_scopes = load_technician_task_scopes(state, technician_id).await?;
    let score_items = load_technician_score_items(state, &task_scopes).await?;
    Ok(aggregate_technician_stats(task_scopes, score_items))
}

fn aggregate_technician_stats(
    task_scopes: Vec<TechnicianTaskScope>,
    score_items: Vec<HqScoreItemEntity>,
) -> TechnicianAggregateStats {
    let average_score = if score_items.is_empty() {
        0.0
    } else {
        score_items
            .iter()
            .map(|item| item.score.unwrap_or_default() as f64)
            .sum::<f64>()
            / score_items.len() as f64
    };
    let average_score = round_score(average_score);

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
    let current_task = recent_scopes
        .iter()
        .find(|item| technician_scope_is_active(technician_scope_status(item)));

    TechnicianAggregateStats {
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
                service_type: technician_scope_service_type(&item)
                    .unwrap_or_default()
                    .to_string(),
                task_status: technician_scope_status(&item)
                    .unwrap_or_default()
                    .to_string(),
                customer_id: item.task.customer_id.unwrap_or_default(),
                appointment_time: technician_scope_appointment_time(&item)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string()),
            })
            .collect(),
    }
}

fn round_score(score: f64) -> f64 {
    (score * 100.0).round() / 100.0
}

fn format_score(score: f64) -> String {
    format!("{:.2}", round_score(score))
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
        task_ids_with_service_items
            .iter()
            .copied()
            .collect::<Vec<_>>(),
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

    let mut legacy_tasks: Vec<TaskEntity> = db::select_by_map_compat(
        state.rb.as_ref(),
        "task",
        value! { "current_technician_id": technician_id },
    )
    .await
    .context("failed to query legacy technician tasks")
    .map_err(AppError::Internal)?;
    legacy_tasks.retain(|task| task.deleted_at.is_none());
    let legacy_task_ids = legacy_tasks
        .iter()
        .filter_map(|item| item.id)
        .collect::<Vec<_>>();
    let task_ids_with_any_service_items =
        load_task_ids_with_service_items(state, &legacy_task_ids).await?;

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
    let tasks: Vec<TaskEntity> =
        db::select_by_map_compat(state.rb.as_ref(), "task", value! { "id": task_ids })
            .await
            .context("failed to query technician tasks by ids")
            .map_err(AppError::Internal)?;
    Ok(tasks
        .into_iter()
        .filter(|item| item.deleted_at.is_none())
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
        Some("pending_assign")
            | Some("pending_execute")
            | Some("executing")
            | Some("pending_dealer_review")
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
    let user = find_user_record(state, user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("师傅不存在或已删除".to_string()))?;
    if user.deleted_at.is_some() {
        return Err(AppError::NotFound("师傅不存在或已删除".to_string()));
    }
    Ok(user)
}

async fn find_user_record(state: &AppState, user_id: i64) -> Result<Option<UserEntity>, AppError> {
    db::select_one_by_map_compat(state.rb.as_ref(), "user", value! { "id": user_id })
        .await
        .context("failed to query user by id")
        .map_err(AppError::Internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn technician_list_record_filter_keeps_existing_scope_rules() {
        let record = TechnicianListRecord {
            user: UserEntity {
                name: Some("张师傅".to_string()),
                mobile: Some("13800000000".to_string()),
                ..Default::default()
            },
            display_name: "张师傅".to_string(),
            status: "enabled".to_string(),
            role_names: Vec::new(),
            store_ids: vec![3],
            store_names: vec!["测试门店".to_string()],
            service_types: vec!["install".to_string()],
        };
        let query = TechnicianListQuery {
            mobile: Some("138".to_string()),
            name: Some("张".to_string()),
            status: None,
            service_type: Some("install".to_string()),
            page: None,
            page_size: None,
        };

        assert!(technician_list_record_matches(
            &record,
            &query,
            Some(&"enabled".to_string()),
            Some(&vec![3])
        ));
        assert!(!technician_list_record_matches(
            &record,
            &query,
            Some(&"disabled".to_string()),
            Some(&vec![3])
        ));
    }

    #[test]
    fn batched_technician_stats_keep_counts_and_current_task() {
        let scopes = vec![
            TechnicianTaskScope {
                task: TaskEntity {
                    id: Some(1),
                    task_no: Some("TASK-1".to_string()),
                    ..Default::default()
                },
                service_item: Some(TaskServiceItemEntity {
                    service_type: Some("install".to_string()),
                    service_status: Some("executing".to_string()),
                    ..Default::default()
                }),
            },
            TechnicianTaskScope {
                task: TaskEntity {
                    id: Some(2),
                    ..Default::default()
                },
                service_item: Some(TaskServiceItemEntity {
                    service_type: Some("measure".to_string()),
                    service_status: Some("completed".to_string()),
                    ..Default::default()
                }),
            },
        ];
        let stats = aggregate_technician_stats(
            scopes,
            vec![HqScoreItemEntity {
                score: Some(90),
                ..Default::default()
            }],
        );

        assert_eq!(stats.pending_count, 1);
        assert_eq!(stats.completed_count, 1);
        assert_eq!(stats.current_task_no.as_deref(), Some("TASK-1"));
        assert_eq!(stats.work_status, "busy");
        assert_eq!(stats.average_score, 90.0);
    }

    #[test]
    fn technician_status_accepts_chinese_and_english_values() {
        assert_eq!(
            normalize_technician_status(Some("启用")).unwrap(),
            "enabled"
        );
        assert_eq!(
            normalize_technician_status(Some("禁用")).unwrap(),
            "disabled"
        );
        assert_eq!(
            normalize_technician_status(Some("active")).unwrap(),
            "enabled"
        );
        assert_eq!(
            normalize_technician_status(Some("inactive")).unwrap(),
            "disabled"
        );
        assert_eq!(normalize_technician_status(None).unwrap(), "enabled");
        assert!(normalize_technician_status(Some("unknown")).is_err());
    }

    #[test]
    fn initial_password_defaults_when_missing_or_blank() {
        assert_eq!(resolve_initial_password(None), "123456");
        assert_eq!(resolve_initial_password(Some("  ")), "123456");
        assert_eq!(resolve_initial_password(Some(" secret123 ")), "secret123");
    }

    #[test]
    fn batch_delete_request_accepts_legacy_string_ids() {
        let payload: BatchDeleteTechniciansRequest =
            serde_json::from_value(serde_json::json!({ "ids": ["6", 7] })).unwrap();
        assert_eq!(payload.technician_ids, vec![6, 7]);
    }

    #[test]
    fn update_request_distinguishes_missing_null_and_value() {
        let missing: UpdateTechnicianRequest =
            serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(missing.name, None);
        assert_eq!(missing.mobile, None);
        assert_eq!(missing.nickname, None);
        assert_eq!(missing.store_id, None);
        assert_eq!(missing.service_types, None);
        assert_eq!(missing.status, None);
        assert!(!is_full_admin_technician_form(&missing));

        let explicit_null: UpdateTechnicianRequest = serde_json::from_value(serde_json::json!({
            "name": null,
            "mobile": null,
            "nickname": null,
            "storeId": null,
            "serviceTypes": null,
            "status": null
        }))
        .unwrap();
        assert_eq!(explicit_null.name, Some(None));
        assert_eq!(explicit_null.mobile, Some(None));
        assert_eq!(explicit_null.nickname, Some(None));
        assert_eq!(explicit_null.store_id, Some(None));
        assert_eq!(explicit_null.service_types, Some(None));
        assert_eq!(explicit_null.status, Some(None));

        let values: UpdateTechnicianRequest = serde_json::from_value(serde_json::json!({
            "name": " Alice ",
            "phone": "13800000000",
            "nickname": "  ",
            "storeId": 9,
            "serviceTypes": ["install"],
            "status": "enabled"
        }))
        .unwrap();
        assert_eq!(values.name, Some(Some(" Alice ".to_string())));
        assert_eq!(values.mobile, Some(Some("13800000000".to_string())));
        assert_eq!(values.nickname, Some(Some("  ".to_string())));
        assert_eq!(values.store_id, Some(Some(9)));
        assert_eq!(
            values.service_types,
            Some(Some(vec!["install".to_string()]))
        );
        assert_eq!(values.status, Some(Some("enabled".to_string())));
        assert!(is_full_admin_technician_form(&values));
    }

    #[test]
    fn update_text_validation_rejects_required_blanks_and_clears_optional_blanks() {
        let missing: Option<Option<String>> = None;
        assert_eq!(
            resolve_required_patch_text(&missing, Some(" Existing "), "required").unwrap(),
            "Existing"
        );
        assert!(resolve_required_patch_text(&Some(None), Some("old"), "required").is_err());
        assert!(
            resolve_required_patch_text(&Some(Some("  ".to_string())), Some("old"), "required")
                .is_err()
        );
        assert_eq!(
            resolve_optional_patch_text(&missing, Some("old")),
            Some("old".to_string())
        );
        assert_eq!(resolve_optional_patch_text(&Some(None), Some("old")), None);
        assert_eq!(
            resolve_optional_patch_text(&Some(Some("  ".to_string())), Some("old")),
            None
        );
        assert_eq!(
            resolve_optional_patch_text(&Some(Some(" New ".to_string())), Some("old")),
            Some("New".to_string())
        );
    }

    #[test]
    fn full_admin_form_omission_clears_supported_optional_fields() {
        assert_eq!(
            resolve_admin_form_optional_text(&None, Some("old"), true),
            None
        );
        assert_eq!(
            resolve_admin_form_optional_text(&Some(None), Some("old"), true),
            None
        );
        assert_eq!(
            resolve_admin_form_optional_text(&Some(Some("  ".to_string())), Some("old"), true),
            None
        );
        assert_eq!(
            resolve_admin_form_optional_text(
                &Some(Some(" Contact ".to_string())),
                Some("old"),
                true
            ),
            Some("Contact".to_string())
        );
    }

    #[test]
    fn full_admin_payload_keeps_sent_id_card_and_clears_omitted_contacts() {
        let request: UpdateTechnicianRequest = serde_json::from_value(serde_json::json!({
            "id_card": "412727199608110716",
            "mobile": "13253398605",
            "name": "王小波",
            "service_types": ["install", "ironing", "measure"],
            "status": "enabled",
            "store_id": 3
        }))
        .unwrap();
        let is_full_admin_form = is_full_admin_technician_form(&request);

        assert!(is_full_admin_form);
        assert_eq!(
            resolve_admin_form_optional_text(
                &request.id_card,
                Some("old-id-card"),
                is_full_admin_form,
            ),
            Some("412727199608110716".to_string())
        );
        assert_eq!(
            resolve_admin_form_optional_text(
                &request.emergency_contact_name,
                Some("艾迪"),
                is_full_admin_form,
            ),
            None
        );
        assert_eq!(
            resolve_admin_form_optional_text(
                &request.emergency_contact_mobile,
                Some("16634649876"),
                is_full_admin_form,
            ),
            None
        );
    }

    #[test]
    fn partial_patch_omission_preserves_admin_form_optional_fields() {
        assert_eq!(
            resolve_admin_form_optional_text(&None, Some("old"), false),
            Some("old".to_string())
        );
    }

    #[test]
    fn update_security_fields_are_required_and_skills_are_normalized() {
        let current_skills = vec![TechnicianSkillEntity {
            service_type: Some("measure".to_string()),
            ..Default::default()
        }];
        assert!(resolve_update_service_types(&Some(None), &current_skills).is_err());
        assert!(resolve_update_service_types(&Some(Some(Vec::new())), &current_skills).is_err());
        assert_eq!(
            resolve_update_service_types(
                &Some(Some(vec![
                    " install ".to_string(),
                    "install".to_string(),
                    "cleaning".to_string()
                ])),
                &current_skills
            )
            .unwrap(),
            vec!["install".to_string(), "cleaning".to_string()]
        );
        assert!(resolve_update_store_id(&Some(None), &[1]).is_err());
        assert!(resolve_update_store_id(&Some(Some(0)), &[1]).is_err());
        assert_eq!(resolve_update_store_id(&None, &[7]).unwrap(), 7);
        assert!(resolve_update_technician_status(&Some(None), "enabled").is_err());
        assert_eq!(
            resolve_update_technician_status(&Some(Some("  ".to_string())), "enabled").unwrap(),
            "enabled"
        );
        assert!(same_service_types(
            &["install".to_string(), "measure".to_string()],
            &["measure".to_string(), "install".to_string()]
        ));
    }
}
