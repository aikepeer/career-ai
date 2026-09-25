//! Operator-driven employer response transition.

use std::path::Path;

use anyhow::Result;
use careerai_core::state::ListingState;
use careerai_db::queries;

/// Mark an application and its listing as responded, preserving one audit event.
pub async fn mark_responded(root: &Path, application_id: &str, note: Option<&str>) -> Result<()> {
    let pool = crate::open_pool(root).await?;
    let app = queries::find_application_by_id(&pool, application_id).await?;
    if app.state == ListingState::Responded.as_str() {
        return Ok(());
    }
    queries::transition_application_and_listing(
        &pool,
        application_id,
        &app.listing_id,
        ListingState::Responded.as_str(),
        ListingState::Responded,
        note,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_db::models::{NewApplication, NewListing};
    use careerai_db::pool::pool_from_path;
    use careerai_db::queries::applications::create_application;
    use careerai_db::queries::listings::insert_or_ignore;
    #[tokio::test]
    async fn mark_responded_updates_application_listing_and_event() {
        let root = tempfile::tempdir().unwrap();
        let db_path = careerai_core::paths::database_path(root.path());
        let pool = pool_from_path(&db_path).await.unwrap();
        let (listing_id, _) = insert_or_ignore(
            &pool,
            &NewListing {
                source: "greenhouse".into(),
                external_id: "response-1".into(),
                title: "Senior ML Engineer".into(),
                company: "Acme Robotics".into(),
                location: Some("Remote".into()),
                url: "https://example.com/response-1".into(),
                description: "Build embedded LLM perception systems.".into(),
                raw_json: None,
            },
        )
        .await
        .unwrap();
        let app = create_application(
            &pool,
            &NewApplication {
                listing_id: listing_id.clone(),
                profile_hash: "sha256:abc".into(),
                prompt_version: "tailor.v1".into(),
                llm_model: "local".into(),
            },
        )
        .await
        .unwrap();

        mark_responded(root.path(), &app.id, Some("recruiter replied"))
            .await
            .unwrap();

        let updated = queries::find_application_by_id(&pool, &app.id)
            .await
            .unwrap();
        assert_eq!(updated.state, ListingState::Responded.as_str());
        let listing = queries::find_by_id(&pool, &listing_id).await.unwrap();
        assert_eq!(listing.state, ListingState::Responded.as_str());
        let event: (String, Option<String>) = sqlx::query_as(
            "SELECT to_state, note FROM events WHERE listing_id = ? ORDER BY id DESC LIMIT 1",
        )
        .bind(&listing_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(event.0, "responded");
        assert_eq!(event.1.as_deref(), Some("recruiter replied"));
    }
}
