mod app;
mod audit;
mod auth;
mod config;
mod db;
mod error;
mod models;
mod notify;
mod response;
mod routes;
mod state;
mod storage;
mod wechat;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    app::bootstrap().await
}
