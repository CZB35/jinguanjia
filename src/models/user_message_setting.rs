use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UserMessageSettingEntity {
    pub id: Option<i64>,
    pub user_id: Option<i64>,
    pub new_task_enabled: Option<i32>,
    pub review_reminder_enabled: Option<i32>,
    pub sms_enabled: Option<i32>,
    pub wechat_enabled: Option<i32>,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
}

rbatis::crud!(UserMessageSettingEntity {}, "user_message_setting");
