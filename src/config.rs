use std::{env, net::SocketAddr};

use anyhow::{Context, Result, bail};

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub app: AppMeta,
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub auth: AuthConfig,
    pub storage: StorageConfig,
}

#[derive(Clone, Debug)]
pub struct AppMeta {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug)]
pub struct DatabaseConfig {
    pub url: String,
}

#[derive(Clone, Debug)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub access_token_expire_minutes: i64,
    pub refresh_token_expire_days: i64,
}

#[derive(Clone, Debug)]
pub struct StorageConfig {
    pub backend: String,
    pub upload_dir: String,
    pub oss_endpoint: String,
    pub oss_bucket: String,
    pub oss_access_key_id: String,
    pub oss_access_key_secret: String,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let _ = dotenvy::dotenv();

        Ok(Self {
            app: AppMeta {
                name: env_var_or("APP_NAME", "kgj-backend"),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            server: ServerConfig {
                host: env_var_or("SERVER_HOST", "0.0.0.0"),
                port: env_var_or("SERVER_PORT", "3001")
                    .parse()
                    .context("SERVER_PORT must be a valid u16")?,
            },
            database: DatabaseConfig {
                url: env_var_or(
                    "DATABASE_URL",
                    "mysql://root:ChangeMe123!@127.0.0.1:3306/kgj",
                ),
            },
            auth: AuthConfig {
                jwt_secret: env_var_or("JWT_SECRET", "kgj-backend-dev-secret"),
                access_token_expire_minutes: env_var_or("JWT_ACCESS_EXPIRE_MINUTES", "120")
                    .parse()
                    .context("JWT_ACCESS_EXPIRE_MINUTES must be a valid i64")?,
                refresh_token_expire_days: env_var_or("JWT_REFRESH_EXPIRE_DAYS", "7")
                    .parse()
                    .context("JWT_REFRESH_EXPIRE_DAYS must be a valid i64")?,
            },
            storage: StorageConfig {
                backend: env_var_or("STORAGE_BACKEND", "local"),
                upload_dir: env_var_or("UPLOAD_DIR", "uploads"),
                oss_endpoint: env_var_or("OSS_ENDPOINT", ""),
                oss_bucket: env_var_or("OSS_BUCKET", ""),
                oss_access_key_id: env_var_or("OSS_ACCESS_KEY_ID", ""),
                oss_access_key_secret: env_var_or("OSS_ACCESS_KEY_SECRET", ""),
            },
        })
    }
}

impl StorageConfig {
    pub fn is_oss(&self) -> bool {
        self.backend.trim().eq_ignore_ascii_case("oss")
    }

    pub fn validate(&self) -> Result<()> {
        if self.is_oss() {
            if self.oss_endpoint.trim().is_empty() {
                bail!("OSS_ENDPOINT must be set when STORAGE_BACKEND=oss");
            }
            if self.oss_bucket.trim().is_empty() {
                bail!("OSS_BUCKET must be set when STORAGE_BACKEND=oss");
            }
            if self.oss_access_key_id.trim().is_empty() {
                bail!("OSS_ACCESS_KEY_ID must be set when STORAGE_BACKEND=oss");
            }
            if self.oss_access_key_secret.trim().is_empty() {
                bail!("OSS_ACCESS_KEY_SECRET must be set when STORAGE_BACKEND=oss");
            }
        }
        Ok(())
    }
}

impl ServerConfig {
    pub fn socket_addr(&self) -> Result<SocketAddr> {
        format!("{}:{}", self.host, self.port)
            .parse()
            .context("invalid server host or port")
    }
}

fn env_var_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}
