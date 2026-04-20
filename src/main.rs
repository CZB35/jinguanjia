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
mod storage;
mod state;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    app::bootstrap().await
}
