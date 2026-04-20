use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PermissionEntity {
    pub id: Option<i64>,
    pub permission_code: Option<String>,
    pub permission_name: Option<String>,
    pub permission_type: Option<String>,
    pub parent_id: Option<i64>,
    pub sort_no: Option<i32>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(PermissionEntity {}, "permission");
