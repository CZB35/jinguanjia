use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StoreEntity {
    pub id: Option<i64>,
    pub store_name: Option<String>,
    pub contact_name: Option<String>,
    pub contact_mobile: Option<String>,
    pub province: Option<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub address: Option<String>,
    pub business_hours: Option<String>,
    pub status: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
}

rbatis::crud!(StoreEntity {}, "store");
