use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ServiceStandardEntity {
    pub id: Option<i64>,
    pub service_type: Option<String>,
    pub title: Option<String>,
    pub content: Option<String>,
    pub example_file_url: Option<String>,
    pub status: Option<String>,
    pub version_no: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
}

rbatis::crud!(ServiceStandardEntity {}, "service_standard");
