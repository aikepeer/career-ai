use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;

use anyhow::{Context, Result};

use careerai_core::config::CoreConfig;
use careerai_dashboard::{run as run_dashboard, ServeOptions};
use careerai_db::pool_from_path;

pub async fn run_serve(
    cwd: &Path,
    cfg: &CoreConfig,
    port_override: Option<u16>,
    bind_override: Option<IpAddr>,
) -> Result<()> {
    let port = port_override.or(cfg.dashboard.port).unwrap_or(8787);
    let bind = bind_override.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let refresh_seconds = cfg.dashboard.refresh_seconds.unwrap_or(60);

    let db_path = cwd.join("data").join("careerai.sqlite");
    let pool = pool_from_path(&db_path)
        .await
        .with_context(|| format!("open db at {}", db_path.display()))?;

    println!("dashboard: http://{bind}:{port}");
    if !bind.is_loopback() {
        eprintln!(
            "WARNING: dashboard bound to non-loopback {bind} — \
             there is NO authentication. Use only on trusted networks."
        );
    }
    println!("press Ctrl-C to stop");

    let opts = ServeOptions {
        port,
        bind,
        refresh_seconds,
        pool,
    };
    run_dashboard(opts).await.context("dashboard server")?;
    Ok(())
}
