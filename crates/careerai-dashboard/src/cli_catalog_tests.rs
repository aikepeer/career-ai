//! Tests for the operator-facing CLI catalog.

use super::*;

#[test]
fn catalog_covers_every_top_level_cli_command() {
    let cards = catalog();
    let names: Vec<&str> = cards.iter().map(|c| c.name).collect();
    for expected in [
        "init",
        "config generate",
        "profile import",
        "profile show",
        "profile validate",
        "discover",
        "match",
        "run",
        "tailor",
        "render",
        "apply",
        "retry",
        "review",
        "shortlist show",
        "applied",
        "inspect",
        "digest",
        "sources sync",
        "sources discover-web",
        "mcp probe",
        "llm probe",
        "notify test",
        "cookies refresh",
        "daemon",
        "status serve",
        "service install",
        "service status",
        "service uninstall",
    ] {
        assert!(
            names.contains(&expected),
            "catalog missing `{expected}`: {names:?}"
        );
    }
}

#[test]
fn long_running_and_secret_commands_are_cli_only() {
    let cards = catalog();
    for name in [
        "daemon",
        "status serve",
        "service install",
        "cookies refresh",
        "init",
    ] {
        let card = cards
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(card.availability, CliAvailability::CliOnly);
        assert!(card.run_command.is_none());
    }
}

#[test]
fn grouped_preserves_order_and_covers_every_card() {
    let groups = grouped();
    let total: usize = groups.iter().map(|g| g.cards.len()).sum();
    assert_eq!(total, catalog().len());
    let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Setup",
            "Pipeline",
            "Inspect",
            "Sources",
            "Diagnose",
            "Credentials",
            "Long-running",
            "Service",
        ]
    );
}

#[test]
fn pipeline_one_shots_are_runnable_from_dashboard() {
    let cards = catalog();
    for name in ["discover", "match", "run", "review", "llm probe"] {
        let card = cards
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(card.availability, CliAvailability::Runnable);
        assert!(card.run_command.is_some());
    }
}
