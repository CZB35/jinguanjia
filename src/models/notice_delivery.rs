use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NoticeDeliveryEntity {
    pub id: Option<i64>,
    pub notice_id: Option<i64>,
    pub channel: Option<String>,
    pub receiver_id: Option<i64>,
    pub receiver_openid: Option<String>,
    pub biz_type: Option<String>,
    pub biz_id: Option<i64>,
    pub template_code: Option<String>,
    pub payload_json: Option<String>,
    pub status: Option<String>,
    pub attempt_count: Option<i32>,
    pub next_retry_at: Option<NaiveDateTime>,
    pub provider_message_id: Option<String>,
    pub error_message: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    pub sent_at: Option<NaiveDateTime>,
}

rbatis::crud!(NoticeDeliveryEntity {}, "notice_delivery");
