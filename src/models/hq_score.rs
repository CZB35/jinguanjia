use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HqScoreEntity {
    pub id: Option<i64>,
    pub task_id: Option<i64>,
    pub scorer_id: Option<i64>,
    pub total_score: Option<i32>,
    pub service_level: Option<String>,
    pub score_remark: Option<String>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(HqScoreEntity {}, "hq_score");
