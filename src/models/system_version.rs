use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SystemVersionEntity {
    pub id: Option<i64>,
    pub version_no: Option<String>,
    pub version_title: Option<String>,
    pub changelog: Option<String>,
    pub published_at: Option<NaiveDateTime>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(SystemVersionEntity {}, "system_version");
