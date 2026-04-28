use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskEntity {
    pub id: Option<i64>,
    pub task_no: Option<String>,
    pub customer_id: Option<i64>,
    pub customer_address_id: Option<i64>,
    pub store_id: Option<i64>,
    pub dealer_user_id: Option<i64>,
    pub current_technician_id: Option<i64>,
    pub service_type: Option<String>,
    pub task_type: Option<String>,
    pub priority: Option<String>,
    pub appointment_time: Option<NaiveDateTime>,
    pub service_time_slot: Option<String>,
    pub task_desc: Option<String>,
    pub task_remark: Option<String>,
    pub task_status: Option<String>,
    pub review_status: Option<String>,
    pub score_status: Option<String>,
    pub is_overdue: Option<i32>,
    pub latest_assignment_id: Option<i64>,
    pub latest_execution_id: Option<i64>,
    pub source: Option<String>,
    pub created_by: Option<i64>,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub deleted_at: Option<NaiveDateTime>,
}

rbatis::crud!(TaskEntity {}, "task");
