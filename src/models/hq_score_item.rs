use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HqScoreItemEntity {
    pub id: Option<i64>,
    pub hq_score_id: Option<i64>,
    pub task_id: Option<i64>,
    pub service_type: Option<String>,
    pub score: Option<i32>,
    pub deduction_reason: Option<String>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(HqScoreItemEntity {}, "hq_score_item");
