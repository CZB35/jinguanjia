use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RoleEntity {
    pub id: Option<i64>,
    pub role_code: Option<String>,
    pub role_name: Option<String>,
    pub remark: Option<String>,
    pub status: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
}

rbatis::crud!(RoleEntity {}, "role");
