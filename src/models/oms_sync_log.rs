use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OmsSyncLogEntity {
    pub id: Option<i64>,
    pub biz_type: Option<String>,
    pub biz_id: Option<i64>,
    pub sync_direction: Option<String>,
    pub request_payload: Option<String>,
    pub response_payload: Option<String>,
    pub sync_status: Option<String>,
    pub error_message: Option<String>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(OmsSyncLogEntity {}, "oms_sync_log");
