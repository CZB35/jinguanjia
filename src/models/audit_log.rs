use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuditLogEntity {
    pub id: Option<i64>,
    pub biz_type: Option<String>,
    pub biz_id: Option<i64>,
    pub action: Option<String>,
    pub operator_id: Option<i64>,
    pub operator_role: Option<String>,
    pub before_value: Option<String>,
    pub after_value: Option<String>,
    pub remark: Option<String>,
    pub created_at: Option<NaiveDateTime>,
}

rbatis::crud!(AuditLogEntity {}, "audit_log");
