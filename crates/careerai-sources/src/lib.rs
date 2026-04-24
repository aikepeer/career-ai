//! Job-board discovery adapters.
//!
//! One implementation per board behind the [`base::Source`] trait. New
//! boards plug in by implementing `Source`; callers never branch on the
//! concrete type.

pub mod base;
pub mod greenhouse;
pub mod lever;
pub mod remoteok;
pub mod remotive;
pub mod util;

pub use base::{RawListing, Source, SourceError};
pub use greenhouse::GreenhouseSource;
pub use lever::LeverSource;
pub use remoteok::RemoteOkSource;
pub use remotive::RemotiveSource;
