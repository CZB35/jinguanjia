use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskServiceItemEntity {
    pub id: Option<i64>,
    pub task_id: Option<i64>,
    pub service_type: Option<String>,
    pub service_status: Option<String>,
    pub technician_id: Option<i64>,
    pub scheduled_time: Option<NaiveDateTime>,
    pub started_at: Option<NaiveDateTime>,
    pub completed_at: Option<NaiveDateTime>,
    pub progress_percent: Option<i32>,
    pub sort_no: Option<i32>,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
}

rbatis::crud!(TaskServiceItemEntity {}, "task_service_item");
