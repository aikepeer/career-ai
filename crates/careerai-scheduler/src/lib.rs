//! Scheduler daemon.
//!
//! Wraps `tokio-cron-scheduler`. Per-source cadence from config, graceful
//! shutdown on SIGTERM, per-source error isolation. Implemented in M6.
