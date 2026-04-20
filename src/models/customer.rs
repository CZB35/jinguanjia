use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CustomerEntity {
    pub id: Option<i64>,
    pub name: Option<String>,
    pub mobile: Option<String>,
    pub source: Option<String>,
    pub remark: Option<String>,
    pub created_by: Option<i64>,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
}

rbatis::crud!(CustomerEntity {}, "customer");
