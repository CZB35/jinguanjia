use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CustomerAddressEntity {
    pub id: Option<i64>,
    pub customer_id: Option<i64>,
    pub province: Option<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub address: Option<String>,
    pub is_default: Option<i32>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(CustomerAddressEntity {}, "customer_address");
