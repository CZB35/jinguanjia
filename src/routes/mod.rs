pub mod auth;
pub mod customer_reviews;
pub mod customer_sources;
pub mod customers;
pub mod dashboards;
pub mod files;
pub mod health;
pub mod hq_review;
pub mod imports;
pub mod me;
pub mod notices;
pub mod oms;
pub mod permissions;
pub mod reports;
pub mod reviews;
pub mod standards;
pub mod stores;
pub mod system;
pub mod tasks;
pub mod technician_location;
pub mod technicians;
pub mod users;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{delete, get, patch, post},
};

use crate::state::AppState;

const MAX_50MB_MULTIPART_BODY_BYTES: usize = 51 * 1024 * 1024;

pub(crate) fn deserialize_patch_field<'de, D, T>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize<'de>>::deserialize(deserializer).map(Some)
}

pub(crate) fn parse_multi_query_values(value: Option<&str>) -> Vec<String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };

    if value.starts_with('[')
        && let Ok(values) = serde_json::from_str::<Vec<String>>(value)
    {
        return normalize_multi_query_values(values);
    }

    normalize_multi_query_values(
        value
            .split(|character: char| {
                matches!(character, ',' | ';' | '|') || matches!(character as u32, 0xFF0C | 0x3001)
            })
            .map(ToString::to_string),
    )
}

pub(crate) fn multi_query_value_matches(
    value: Option<&str>,
    mut predicate: impl FnMut(&str) -> bool,
) -> bool {
    let values = parse_multi_query_values(value);
    values.is_empty()
        || values.iter().any(|value| is_all_query_value(value))
        || values.iter().any(|value| predicate(value))
}

fn normalize_multi_query_values(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() || normalized.iter().any(|current| current == value) {
            continue;
        }
        normalized.push(value.to_string());
    }
    normalized
}

pub(crate) fn is_all_query_value(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "all" | "*")
        || matches!(value.trim(), "\u{5168}\u{90e8}" | "\u{4e0d}\u{9650}")
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health::health_check))
        .route("/auth/login/password", post(auth::login_by_password))
        .route("/auth/login/admin", post(auth::login_admin_by_password))
        .route("/auth/login/code", post(auth::login_by_code))
        .route("/auth/code/send", post(auth::send_login_code))
        .route("/auth/refresh", post(auth::refresh_token))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::current_user))
        .route("/auth/switch-role", post(auth::switch_role))
        .route(
            "/me/profile",
            get(me::get_profile).patch(me::update_profile),
        )
        .route("/me/avatar", post(me::upload_avatar))
        .route("/me/wechat-bind-url", get(me::get_wechat_bind_url))
        .route("/me/wechat-binding", get(me::get_wechat_binding_status))
        .route("/wechat/bind-url", post(me::get_public_wechat_bind_url))
        .route("/wechat/bind/code/send", post(me::send_wechat_bind_code))
        .route("/wechat/bind/confirm", post(me::confirm_wechat_bind))
        .route("/me/password", patch(me::update_password))
        .route("/me/permissions", get(me::get_permissions))
        .route(
            "/me/message-settings",
            get(me::get_message_settings).patch(me::update_message_settings),
        )
        .route("/dealer/dashboard", get(dashboards::dealer_dashboard))
        .route(
            "/technician/dashboard",
            get(dashboards::technician_dashboard),
        )
        .route("/hq/dashboard", get(dashboards::hq_dashboard))
        .route("/notices", get(notices::list_notices))
        .route("/notices/unread-count", get(notices::unread_count))
        .route("/notices/{id}/read", patch(notices::read_notice))
        .route("/notices/read-all", patch(notices::read_all_notices))
        .route(
            "/customer-reviews/send-link",
            post(customer_reviews::send_review_link),
        )
        .route("/customer-reviews/form", get(customer_reviews::review_form))
        .route(
            "/customer-reviews/detail",
            get(customer_reviews::review_detail),
        )
        .route(
            "/customer-reviews/images",
            post(customer_reviews::upload_customer_review_image),
        )
        .route(
            "/customer-reviews/images/{id}",
            delete(customer_reviews::delete_customer_review_image),
        )
        .route(
            "/customer-reviews/submit",
            post(customer_reviews::submit_customer_review),
        )
        .route(
            "/customer-sources",
            get(customer_sources::list_customer_sources)
                .post(customer_sources::create_customer_source),
        )
        .route(
            "/customer-sources/{id}",
            get(customer_sources::get_customer_source_detail)
                .patch(customer_sources::update_customer_source)
                .delete(customer_sources::delete_customer_source),
        )
        .route(
            "/stores",
            get(stores::list_stores).post(stores::create_store),
        )
        .route(
            "/stores/{id}",
            get(stores::get_store_detail)
                .patch(stores::update_store_detail)
                .delete(stores::delete_store),
        )
        .route("/stores/{id}/status", patch(stores::update_store_status))
        .route("/users/import", post(imports::import_users))
        .route(
            "/users/import-template",
            get(imports::download_user_import_template),
        )
        .route("/dealers/import", post(imports::import_dealers))
        .route("/technicians/import", post(imports::import_technicians))
        .route(
            "/technicians/import-template",
            get(imports::download_technician_import_template),
        )
        .route("/customers/import", post(imports::import_customers))
        .route("/stores/import", post(imports::import_stores))
        .route("/users/export", get(imports::export_users))
        .route("/users/batch-delete", post(imports::batch_delete_users))
        .route(
            "/customers/batch-delete",
            post(customers::batch_delete_customers),
        )
        .route(
            "/customers",
            get(customers::list_customers).post(customers::create_customer),
        )
        .route(
            "/customers/search-by-mobile",
            get(customers::search_customer_by_mobile),
        )
        .route(
            "/customers/{id}",
            get(customers::get_customer_detail)
                .patch(customers::update_customer)
                .delete(customers::delete_customer),
        )
        .route(
            "/customers/{id}/service-history",
            get(customers::get_customer_service_history),
        )
        .route(
            "/dealers",
            get(users::list_dealers).post(users::create_dealer),
        )
        .route("/dealers/batch-delete", post(users::batch_delete_dealers))
        .route(
            "/dealers/{id}",
            get(users::get_dealer_detail)
                .patch(users::update_dealer)
                .delete(users::delete_dealer),
        )
        .route(
            "/dealers/{id}/reset-password",
            post(users::reset_dealer_password),
        )
        .route(
            "/technicians",
            get(technicians::list_technicians).post(technicians::create_technician),
        )
        .route(
            "/technicians/batch-delete",
            post(technicians::batch_delete_technicians),
        )
        .route(
            "/technicians/available",
            get(technicians::list_available_technicians),
        )
        .route(
            "/technicians/available/grouped",
            get(technicians::list_available_technicians_grouped),
        )
        .route(
            "/technicians/{id}",
            get(technicians::get_technician_detail)
                .patch(technicians::update_technician)
                .delete(technicians::delete_technician),
        )
        .route(
            "/technicians/{id}/performance",
            get(technicians::get_technician_performance),
        )
        .route(
            "/technicians/{id}/status",
            patch(technicians::update_technician_status),
        )
        .route(
            "/technicians/{id}/reset-password",
            post(technicians::reset_technician_password),
        )
        .route("/users", get(users::list_users).post(users::create_user))
        .route(
            "/users/{id}",
            get(users::get_user_detail).patch(users::update_user),
        )
        .route("/users/{id}", delete(imports::delete_user))
        .route("/users/{id}/status", patch(users::update_user_status))
        .route(
            "/users/{id}/reset-password",
            post(users::reset_user_password),
        )
        .route("/tasks", post(tasks::create_task).get(tasks::list_tasks))
        .route("/tasks/batch-delete", post(tasks::batch_delete_tasks))
        .route("/tasks/export", get(tasks::export_tasks))
        .route("/tasks/export/all", get(tasks::export_tasks))
        .route("/tasks/export/selected", post(tasks::export_selected_tasks))
        .route(
            "/tasks/{id}",
            get(tasks::get_task_detail).delete(tasks::delete_task),
        )
        .route("/tasks/{id}/detail", get(tasks::get_task_detail))
        .route(
            "/tasks/{id}/attachments",
            post(files::upload_task_attachment).get(files::list_task_attachments),
        )
        .route("/tasks/{id}/evidences", get(files::list_task_evidences))
        .route(
            "/tasks/{id}/evidences/images",
            post(files::upload_evidence_image),
        )
        .route(
            "/tasks/{id}/evidences/videos",
            post(files::upload_evidence_video)
                .layer(DefaultBodyLimit::max(MAX_50MB_MULTIPART_BODY_BYTES)),
        )
        .route(
            "/attachments/batch-download",
            post(files::batch_download_attachments),
        )
        .route("/tasks/{id}/assign", post(tasks::assign_task))
        .route("/tasks/{id}/reassign", post(tasks::reassign_task))
        .route("/tasks/{id}/start", post(tasks::start_task))
        .route("/tasks/{id}/submit", post(tasks::submit_task))
        .route("/tasks/{id}/close", post(tasks::close_task))
        .route("/tasks/{id}/logs", get(tasks::task_logs))
        .route("/tasks/{id}/dealer-review", post(reviews::dealer_review))
        .route("/tasks/{id}/hq-score", post(reviews::hq_score))
        .route("/hq/review/tasks", get(hq_review::list_review_tasks))
        .route(
            "/hq/review/tasks/pending-and-completed",
            get(hq_review::list_review_pending_and_completed_tasks),
        )
        .route(
            "/hq/review/tasks/pending-execute-and-executing",
            get(hq_review::list_review_pending_execute_and_executing_tasks),
        )
        .route(
            "/hq/review/tasks/export",
            get(hq_review::export_review_tasks),
        )
        .route(
            "/hq/review/tasks/export/all",
            get(hq_review::export_review_tasks),
        )
        .route(
            "/hq/review/tasks/export/selected",
            post(hq_review::export_selected_review_tasks),
        )
        .route(
            "/hq/review/tasks/{id}",
            get(hq_review::get_review_task_detail),
        )
        .route(
            "/hq/review/tasks/{id}/detail",
            get(hq_review::get_review_task_detail),
        )
        .route(
            "/hq/review/reminders",
            get(hq_review::list_review_reminders),
        )
        .route(
            "/hq/review/tasks/{id}/remind",
            post(hq_review::send_review_remind),
        )
        .route("/hq/reports/overview", get(reports::overview))
        .route("/hq/reports/performance", get(reports::performance))
        .route(
            "/hq/reports/quality-distribution",
            get(reports::quality_distribution),
        )
        .route("/hq/reports/export", get(reports::export))
        .route(
            "/hq/scores/import-template",
            get(imports::download_score_import_template),
        )
        .route("/hq/scores/import", post(imports::import_scores))
        .route(
            "/system/configs",
            get(system::get_configs).patch(system::update_configs),
        )
        .route(
            "/system/project-settings",
            get(system::get_project_settings).patch(system::save_project_settings),
        )
        .route(
            "/system/project-settings/upload",
            post(system::upload_project_setting_image)
                .layer(DefaultBodyLimit::max(MAX_50MB_MULTIPART_BODY_BYTES)),
        )
        .route(
            "/system/agreements/user",
            get(system::get_user_agreement).patch(system::save_user_agreement),
        )
        .route(
            "/system/agreements/privacy",
            get(system::get_privacy_policy).patch(system::save_privacy_policy),
        )
        .route(
            "/system/roles",
            get(permissions::get_roles).post(permissions::create_role),
        )
        .route(
            "/system/roles/{id}",
            get(permissions::get_role_detail)
                .patch(permissions::update_role)
                .delete(permissions::delete_role),
        )
        .route(
            "/system/roles/{id}/permissions",
            get(permissions::get_role_permissions).patch(permissions::update_role_permissions),
        )
        .route(
            "/system/permission-tree",
            get(permissions::get_permission_tree),
        )
        .route("/system/version", get(system::get_version))
        .route("/system/version/changelog", get(system::get_changelog))
        .route("/system/operation-logs", get(system::get_operation_logs))
        .route(
            "/system/operation-logs/{id}",
            get(system::get_operation_log_detail),
        )
        .route("/agreements/user", get(system::get_user_agreement))
        .route("/agreements/privacy", get(system::get_privacy_policy))
        .route(
            "/wechat/service-account/callback",
            get(me::wechat_service_account_callback),
        )
        .route(
            "/standards/{serviceType}",
            get(standards::get_standard_by_service_type),
        )
        .route(
            "/oms/config",
            get(oms::get_config).patch(oms::update_config),
        )
        .route("/oms/sync/tasks", post(oms::sync_tasks))
        .route("/oms/sync/logs", get(oms::sync_logs))
        .route("/attachments/{id}", delete(files::delete_attachment))
        .route(
            "/attachments/{id}/download",
            get(files::download_attachment),
        )
        .route("/evidences/{id}", delete(files::delete_evidence))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_query_values_accept_scalar_delimited_and_json_array() {
        assert_eq!(
            parse_multi_query_values(Some("pending_execute")),
            vec!["pending_execute"]
        );
        assert_eq!(
            parse_multi_query_values(Some("pending_execute,executing|completed")),
            vec!["pending_execute", "executing", "completed"]
        );
        assert_eq!(
            parse_multi_query_values(Some(r#"["pending","scored","pending"]"#)),
            vec!["pending", "scored"]
        );
    }

    #[test]
    fn multi_query_all_value_disables_the_filter() {
        assert!(multi_query_value_matches(Some("all"), |_| false));
        assert!(multi_query_value_matches(None, |_| false));
        assert!(multi_query_value_matches(Some("pending,scored"), |value| {
            value == "scored"
        }));
    }
}
