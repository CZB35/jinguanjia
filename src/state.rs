use std::{collections::HashMap, sync::Arc};

use rbatis::RBatis;
use tokio::sync::RwLock;

use crate::config::AppConfig;

#[derive(Clone, Debug)]
pub struct VerificationCodeState {
    pub code: String,
    pub expires_at: i64,
    pub sent_at: i64,
}

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub rb: Arc<RBatis>,
    pub verification_codes: Arc<RwLock<HashMap<String, VerificationCodeState>>>,
}

impl AppState {
    pub fn new(config: AppConfig, rb: RBatis) -> Self {
        Self {
            config,
            rb: Arc::new(rb),
            verification_codes: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}
