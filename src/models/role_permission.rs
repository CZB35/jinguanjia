use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RolePermissionEntity {
    pub id: Option<i64>,
    pub role_id: Option<i64>,
    pub permission_id: Option<i64>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(RolePermissionEntity {}, "role_permission");
