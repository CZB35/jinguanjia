use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CustomerSourceEntity {
    pub id: Option<i64>,
    pub name: Option<String>,
    pub code: Option<String>,
    pub sort_no: Option<i32>,
    pub remark: Option<String>,
    pub status: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
}

rbatis::crud!(CustomerSourceEntity {}, "customer_source");
