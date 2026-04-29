use std::net::SocketAddr;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DashboardError {
    #[error("failed to bind {addr}: {source}")]
    BindFailed {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },

    #[error("database error: {0}")]
    Db(#[from] careerai_db::error::DbError),

    #[error("template error: {0}")]
    Template(#[from] tera::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, DashboardError>;
