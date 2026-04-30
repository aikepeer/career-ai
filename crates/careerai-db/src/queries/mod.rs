//! Hand-rolled SQL queries split by entity table. No `query!` macro
//! so we avoid the DATABASE_URL/.sqlx-cache compile-time dance for
//! now. Switch to compile-time-checked queries (M3+) once the schema
//! stabilizes.
//!
//! The module structure mirrors the schema:
//!
//! | Module        | Tables touched |
//! |---|---|
//! | `listings`     | `listings`, `events` (the latter only via `transition`) |
//! | `applications` | `applications`, `listings`, `events` (lockstep transitions) |
//! | `linkedin`     | `applications` ⨝ `listings` (LinkedIn drafts review flow) |
//! | `payloads`     | `application_payloads` |
//! | `artifacts`    | `artifacts` |
//! | `events`       | `events` (read-only; writes live next to their triggers) |
//!
//! `pub use submodule::*` keeps the legacy flat path
//! `careerai_db::queries::find_by_id` working unchanged for every
//! caller in the workspace.

pub mod applications;
pub mod applications_sync;
pub mod artifacts;
pub mod events;
pub mod linkedin;
pub mod listings;
pub mod payloads;

pub use applications::{
    create_application, find_application_by_id, find_latest_application_for_listing,
    list_applications_by_state, set_application_state,
};
pub use applications_sync::{
    list_applications_by_state_and_source, transition_application_and_listing,
};
pub use artifacts::{attach_artifact, list_artifacts};
pub use events::events_for;
pub use linkedin::{claim_drafted_application, list_drafted_linkedin};
pub use listings::{
    find_by_external_id, find_by_id, insert_or_ignore, list_by_state, set_score, transition,
};
pub use payloads::{find_payload_by_application_id, write_payload};

#[cfg(test)]
pub(crate) mod test_support {
    use crate::models::{NewApplication, NewListing};

    pub fn fixture(source: &str, ext: &str) -> NewListing {
        NewListing {
            source: source.into(),
            external_id: ext.into(),
            title: "Senior ML Engineer".into(),
            company: "Acme Robotics".into(),
            location: Some("Remote".into()),
            url: format!("https://example.com/{ext}"),
            description: "Build embedded LLM perception systems.".into(),
            raw_json: None,
        }
    }

    pub fn new_app(listing_id: &str) -> NewApplication {
        NewApplication {
            listing_id: listing_id.into(),
            profile_hash: "sha256:abc".into(),
            prompt_version: "tailor.v1".into(),
            llm_model: "claude-3-5-sonnet-20241022".into(),
        }
    }
}
