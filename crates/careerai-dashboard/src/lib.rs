use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use sqlx::SqlitePool;
use tera::Tera;

pub mod daemon_health;
pub mod data;
pub mod error;
pub mod handlers;
pub mod llm_health;
pub mod next_steps;
pub mod routes;
pub mod view;

pub use error::{DashboardError, Result};

pub(crate) const STYLE_CSS: &str = include_str!("../static/style.css");
pub(crate) const INDEX_TERA: &str = include_str!("../templates/index.tera");
pub(crate) const BUILD_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct ServeOptions {
    pub port: u16,
    pub bind: IpAddr,
    pub refresh_seconds: u32,
    pub pool: SqlitePool,
}

impl ServeOptions {
    pub fn loopback(pool: SqlitePool, port: u16) -> Self {
        Self {
            port,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            refresh_seconds: 60,
            pool,
        }
    }
}

#[derive(Debug)]
pub struct AppState {
    pub pool: SqlitePool,
    pub tera: Arc<Tera>,
    pub refresh_seconds: u32,
}

pub async fn run(opts: ServeOptions) -> Result<()> {
    let addr = SocketAddr::new(opts.bind, opts.port);
    if !opts.bind.is_loopback() {
        tracing::warn!(
            %addr,
            "dashboard bound to non-loopback address — there is NO authentication; \
             do not expose to untrusted networks"
        );
    }

    let tera = build_tera()?;
    let state = Arc::new(AppState {
        pool: opts.pool,
        tera: Arc::new(tera),
        refresh_seconds: opts.refresh_seconds,
    });
    let app = routes::build(state);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| DashboardError::BindFailed { addr, source })?;
    tracing::info!(%addr, "dashboard listening");
    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn build_tera() -> Result<Tera> {
    let mut tera = Tera::default();
    tera.add_raw_template("index.tera", INDEX_TERA)?;
    Ok(tera)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let Ok(mut term) = signal(SignalKind::terminate()) else {
            return ctrl_c.await;
        };
        tokio::select! {
            () = ctrl_c => {},
            _ = term.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        ctrl_c.await;
    }
    tracing::info!("dashboard shutdown signal received");
}
