use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SystemConfigEntity {
    pub id: Option<i64>,
    pub config_key: Option<String>,
    pub config_value: Option<String>,
    pub remark: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
}

rbatis::crud!(SystemConfigEntity {}, "system_config");
