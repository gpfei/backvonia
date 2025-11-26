mod app_state;
mod config;
mod error;
mod middleware;
mod models;
mod routes;
mod services;
mod utils;

use app_state::AppState;
use config::Config;
use routes::create_router;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,backvonia=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting Talevonia Backend (backvonia)");

    // Load configuration
    let config = Config::load()?;

    tracing::info!(
        "Loaded configuration - Server: {}:{}",
        config.server.host,
        config.server.port
    );

    // Initialize application state
    let state = AppState::new(config.clone()).await?;

    tracing::info!("Initialized application state");

    // Create router
    let app = create_router(state);

    // Create server address
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("Server listening on {}", addr);

    // Start server
    axum::serve(listener, app).await?;

    Ok(())
}
