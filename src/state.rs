use std::{collections::HashMap, sync::Arc};

use rbatis::RBatis;
use tokio::sync::RwLock;

use crate::{config::AppConfig, wechat::{WechatPublicBindState, WechatRuntime}};

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
    pub wechat_runtime: WechatRuntime,
    pub wechat_public_binds: Arc<RwLock<HashMap<String, WechatPublicBindState>>>,
}

impl AppState {
    pub fn new(config: AppConfig, rb: RBatis) -> Self {
        Self {
            config,
            rb: Arc::new(rb),
            verification_codes: Arc::new(RwLock::new(HashMap::new())),
            wechat_runtime: WechatRuntime::default(),
            wechat_public_binds: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}
