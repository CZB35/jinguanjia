use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UserRoleEntity {
    pub id: Option<i64>,
    pub user_id: Option<i64>,
    pub role_id: Option<i64>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(UserRoleEntity {}, "user_role");
