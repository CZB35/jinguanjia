use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EvidenceEntity {
    pub id: Option<i64>,
    pub task_id: Option<i64>,
    pub execution_id: Option<i64>,
    pub uploader_id: Option<i64>,
    pub evidence_type: Option<String>,
    pub file_name: Option<String>,
    pub file_url: Option<String>,
    pub file_size: Option<i64>,
    pub mime_type: Option<String>,
    pub watermark_text: Option<String>,
    pub location_address: Option<String>,
    pub longitude: Option<f64>,
    pub latitude: Option<f64>,
    pub shot_at: Option<NaiveDateTime>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(EvidenceEntity {}, "evidence");
