use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TechnicianSkillEntity {
    pub id: Option<i64>,
    pub user_id: Option<i64>,
    pub service_type: Option<String>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(TechnicianSkillEntity {}, "technician_skill");
