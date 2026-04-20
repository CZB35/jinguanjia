use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskAssignmentEntity {
    pub id: Option<i64>,
    pub task_id: Option<i64>,
    pub from_technician_id: Option<i64>,
    pub to_technician_id: Option<i64>,
    pub assign_type: Option<String>,
    pub assign_reason: Option<String>,
    pub assigned_by: Option<i64>,
    pub assigned_at: Option<NaiveDateTime>,
}

rbatis::crud!(TaskAssignmentEntity {}, "task_assignment");
