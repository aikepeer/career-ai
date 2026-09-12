//! Canonical action protocol (PR 4+5).
//!
//! Every external side effect has one immutable `external_actions` record
//! with a canonical JSON payload digest, state machine transitions, and
//! crash recovery semantics.

pub mod canonical;
pub mod state;

pub use canonical::{canonical_json, payload_digest, CanonicalError};
pub use state::{ActionState, ActionTransition, TransitionError, ExternalAction};
