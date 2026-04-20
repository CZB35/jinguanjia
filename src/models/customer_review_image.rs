use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CustomerReviewImageEntity {
    pub id: Option<i64>,
    pub review_link_id: Option<i64>,
    pub review_id: Option<i64>,
    pub file_name: Option<String>,
    pub file_url: Option<String>,
    pub file_size: Option<i64>,
    pub mime_type: Option<String>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(CustomerReviewImageEntity {}, "customer_review_image");
