use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NoticeEntity {
    pub id: Option<i64>,
    pub receiver_id: Option<i64>,
    pub notice_type: Option<String>,
    pub biz_type: Option<String>,
    pub biz_id: Option<i64>,
    pub title: Option<String>,
    pub content: Option<String>,
    pub is_read: Option<i32>,
    pub read_at: Option<NaiveDateTime>,
    pub send_status: Option<String>,
    pub sent_at: Option<NaiveDateTime>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(NoticeEntity {}, "notice");
