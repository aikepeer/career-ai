//! Hosted PostgreSQL migrations module.
//! SQL files are embedded and applied at startup.
//! Each migration is idempotent and uses expand/contract pattern.

pub const MIGRATION_001: &str = include_str!("migrations/001_hosted_init.sql");
pub const MIGRATION_002: &str = include_str!("migrations/002_rls_policies.sql");
pub const MIGRATION_003: &str = include_str!("migrations/003_external_actions.sql");
pub const MIGRATION_004: &str = include_str!("migrations/004_entitlements_billing.sql");
pub const MIGRATION_005: &str = include_str!("migrations/005_jobs_outbox.sql");
pub const MIGRATION_006: &str = include_str!("migrations/006_audit_chain.sql");

/// All hosted migrations in order.
pub fn all_migrations() -> Vec<(&'static str, &'static str)> {
    vec![
        ("001_hosted_init", MIGRATION_001),
        ("002_rls_policies", MIGRATION_002),
        ("003_external_actions", MIGRATION_003),
        ("004_entitlements_billing", MIGRATION_004),
        ("005_jobs_outbox", MIGRATION_005),
        ("006_audit_chain", MIGRATION_006),
    ]
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn all_migrations_have_content() {
        for (name, sql) in all_migrations() {
            assert!(!sql.is_empty(), "migration {name} is empty");
        }
    }

    #[test]
    fn migrations_enable_rls() {
        let rls_sql = MIGRATION_002;
        assert!(
            rls_sql.contains("ENABLE ROW LEVEL SECURITY"),
            "RLS not enabled in migration 002"
        );
    }

    #[test]
    fn init_migration_creates_tenants_table() {
        assert!(MIGRATION_001.contains("CREATE TABLE"));
        assert!(MIGRATION_001.contains("tenants"));
    }

    #[test]
    fn external_actions_migration_creates_state_machine() {
        assert!(MIGRATION_003.contains("external_actions"));
        assert!(MIGRATION_003.contains("created"));
        assert!(MIGRATION_003.contains("approved"));
        assert!(MIGRATION_003.contains("unknown"));
    }

    #[test]
    fn entitlements_migration_creates_usage_reservations() {
        assert!(MIGRATION_004.contains("usage_reservations"));
        assert!(MIGRATION_004.contains("billing_events"));
    }

    #[test]
    fn audit_migration_creates_hash_chain() {
        assert!(MIGRATION_006.contains("security_audit_events"));
        assert!(MIGRATION_006.contains("event_hash"));
        assert!(MIGRATION_006.contains("previous_hash"));
    }
}
