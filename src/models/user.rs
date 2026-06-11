use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UserEntity {
    pub id: Option<i64>,
    pub mobile: Option<String>,
    pub account: Option<String>,
    pub password_hash: Option<String>,
    pub name: Option<String>,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
    pub wechat_openid: Option<String>,
    pub wechat_unionid: Option<String>,
    pub wechat_subscribe: Option<i32>,
    pub wechat_bind_at: Option<NaiveDateTime>,
    pub wechat_subscribe_updated_at: Option<NaiveDateTime>,
    pub job_no: Option<String>,
    pub id_card: Option<String>,
    pub emergency_contact_name: Option<String>,
    pub emergency_contact_mobile: Option<String>,
    pub status: Option<String>,
    pub last_login_at: Option<NaiveDateTime>,
    pub last_login_ip: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub deleted_at: Option<NaiveDateTime>,
}

rbatis::crud!(UserEntity {}, "user");
