use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CustomerReviewEntity {
    pub id: Option<i64>,
    pub task_id: Option<i64>,
    pub customer_id: Option<i64>,
    pub review_link_id: Option<i64>,
    pub star: Option<i32>,
    pub tag_json: Option<String>,
    pub content: Option<String>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(CustomerReviewEntity {}, "customer_review");
