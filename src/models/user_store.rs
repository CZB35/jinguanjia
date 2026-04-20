use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UserStoreEntity {
    pub id: Option<i64>,
    pub user_id: Option<i64>,
    pub store_id: Option<i64>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(UserStoreEntity {}, "user_store");
