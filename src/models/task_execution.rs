use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskExecutionEntity {
    pub id: Option<i64>,
    pub task_id: Option<i64>,
    pub technician_id: Option<i64>,
    pub started_at: Option<NaiveDateTime>,
    pub submitted_at: Option<NaiveDateTime>,
    pub submit_remark: Option<String>,
    pub execution_status: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
}

rbatis::crud!(TaskExecutionEntity {}, "task_execution");
