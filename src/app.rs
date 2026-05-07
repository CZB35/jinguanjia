use std::net::SocketAddr;

use anyhow::Result;
use axum::{Router, http::header, middleware::map_response, response::Response};
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{config::AppConfig, db, routes, state::AppState, storage};

pub async fn bootstrap() -> Result<()> {
    init_tracing();

    let config = AppConfig::from_env()?;
    let upload_root = storage::resolve_upload_root(&config.storage.upload_dir);
    std::fs::create_dir_all(&upload_root)?;
    let rb = db::init_rbatis(&config.database.url).await?;
    let addr = config.server.socket_addr()?;
    let state = AppState::new(config.clone(), rb);
    let app = build_router(state);

    tracing::info!(
        app_name = %config.app.name,
        host = %config.server.host,
        port = config.server.port,
        "backend starting"
    );

    serve(app, addr).await
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("kgj_backend=debug,tower_http=info")),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
}

fn build_router(state: AppState) -> Router {
    let upload_service = ServeDir::new(storage::resolve_upload_root(&state.config.storage.upload_dir));
    Router::new()
        .nest("/api", routes::router())
        .nest_service("/uploads", upload_service)
        .with_state(state)
        .layer(map_response(force_utf8_json))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}

async fn force_utf8_json(mut response: Response) -> Response {
    if let Some(content_type) = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
    {
        if content_type.starts_with("application/json") && !content_type.contains("charset=") {
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json; charset=utf-8"),
            );
        }
    }
    response
}

async fn serve(app: Router, addr: SocketAddr) -> Result<()> {
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
