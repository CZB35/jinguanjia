use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CustomerReviewLinkEntity {
    pub id: Option<i64>,
    pub task_id: Option<i64>,
    pub customer_id: Option<i64>,
    pub token: Option<String>,
    pub expired_at: Option<NaiveDateTime>,
    pub status: Option<String>,
    pub sent_at: Option<NaiveDateTime>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(CustomerReviewLinkEntity {}, "customer_review_link");
