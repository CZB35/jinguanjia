use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AttachmentEntity {
    pub id: Option<i64>,
    pub task_id: Option<i64>,
    pub uploader_id: Option<i64>,
    pub attachment_type: Option<String>,
    pub file_name: Option<String>,
    pub file_url: Option<String>,
    pub file_size: Option<i64>,
    pub mime_type: Option<String>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(AttachmentEntity {}, "attachment");
