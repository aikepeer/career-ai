//! careerai-hosted API server binary.
//!
//! Starts the Axum API server with session middleware, ops endpoints,
//! and health/readiness checks. Reads configuration from environment
//! variables.

use std::net::SocketAddr;

use careerai_hosted::api::routes::router;
use careerai_hosted::api::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let bind: SocketAddr = std::env::var("CAREERAI_HOSTED_BIND")
        .unwrap_or_else(|_| "0.0.0.0:3000".to_string())
        .parse()?;

    let master_key_hex = std::env::var("MASTER_KEY").unwrap_or_else(|_| {
        tracing::warn!("MASTER_KEY not set, using insecure default");
        "00".repeat(32)
    });
    let master_key =
        hex::decode(&master_key_hex).map_err(|e| format!("invalid MASTER_KEY hex: {e}"))?;
    let mut key = [0u8; 32];
    if master_key.len() == 32 {
        key.copy_from_slice(&master_key);
    } else {
        tracing::warn!("MASTER_KEY is not 32 bytes, using zeroed key");
    }

    let state = AppState::arc(key);
    let app = router(state);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!("careerai-hosted API listening on {bind}");

    axum::serve(listener, app).await?;

    Ok(())
}
