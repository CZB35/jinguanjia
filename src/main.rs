mod app;
mod audit;
mod auth;
mod config;
mod db;
mod error;
mod identity;
mod models;
mod notify;
mod response;
mod rich_text;
mod routes;
mod sms;
mod state;
mod storage;
mod wechat;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    app::bootstrap().await
}
