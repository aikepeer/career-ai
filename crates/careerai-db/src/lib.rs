//! SQLite persistence layer.
//!
//! Owns models, queries, and migrations. No business logic. Schema lives
//! in `migrations/` next to this crate.

pub mod error;
pub mod models;
pub mod pool;
pub mod queries;

pub use error::{DbError, Result};
pub use models::{
    Application, ApplicationPayload, Artifact, Event, Listing, NewApplication, NewArtifact,
    NewListing,
};
pub use pool::{pool_from_path, pool_in_memory};
// Re-exported so downstream crates don't have to depend on sqlx directly.
pub use sqlx::SqlitePool;
