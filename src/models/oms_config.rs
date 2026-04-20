use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OmsConfigEntity {
    pub id: Option<i64>,
    pub base_url: Option<String>,
    pub app_key: Option<String>,
    pub app_secret: Option<String>,
    pub status: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
}

rbatis::crud!(OmsConfigEntity {}, "oms_config");
