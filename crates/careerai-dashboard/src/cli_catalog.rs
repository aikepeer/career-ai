//! Operator-facing catalog of every `careerai` CLI command.
//!
//! The dashboard Commands tab renders this list. Runnable entries map
//! 1:1 onto the `/api/v1/cli/run` whitelist; long-running or
//! credential-prompting commands are shown as CLI-only so the operator
//! still sees the full surface.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CliAvailability {
    Runnable,
    NeedsId,
    CliOnly,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CliCommandCard {
    pub group: &'static str,
    pub name: &'static str,
    pub argv: &'static str,
    pub summary: &'static str,
    pub hint: &'static str,
    pub availability: CliAvailability,
    /// Value passed to `runCliCommand` when `availability` is Runnable
    /// or NeedsId. `None` for CLI-only entries.
    pub run_command: Option<&'static str>,
}

static CATALOG: &[CliCommandCard] = &[
    CliCommandCard {
        group: "Setup",
        name: "init",
        argv: "careerai init",
        summary: "Scaffold config/, profile/, and .env in the working directory.",
        hint: "One-time workspace bootstrap — run in a terminal so you can review the files it creates.",
        availability: CliAvailability::CliOnly,
        run_command: None,
    },
    CliCommandCard {
        group: "Setup",
        name: "config generate",
        argv: "careerai config generate",
        summary: "Derive discovery keywords and source lists from the candidate profile.",
        hint: "Preview first, then apply. The existing config/local.yaml is backed up.",
        availability: CliAvailability::Runnable,
        run_command: Some("config generate"),
    },
    CliCommandCard {
        group: "Setup",
        name: "profile import",
        argv: "careerai profile import <files>",
        summary: "Ingest a resume PDF/DOCX and optional LinkedIn export ZIP.",
        hint: "Use the Config tab upload form, or run the CLI with --force to overwrite.",
        availability: CliAvailability::CliOnly,
        run_command: None,
    },
    CliCommandCard {
        group: "Setup",
        name: "profile show",
        argv: "careerai profile show",
        summary: "Print the parsed profile.yaml.",
        hint: "Read-only check that ingestion landed the fields you expect.",
        availability: CliAvailability::Runnable,
        run_command: Some("profile show"),
    },
    CliCommandCard {
        group: "Setup",
        name: "profile validate",
        argv: "careerai profile validate",
        summary: "Validate profile.yaml against the schema.",
        hint: "Run after an import or a manual edit before discovering jobs.",
        availability: CliAvailability::Runnable,
        run_command: Some("profile validate"),
    },
    CliCommandCard {
        group: "Pipeline",
        name: "discover",
        argv: "careerai discover [--source X,Y]",
        summary: "Pull new listings from configured job sources. Prints new listing ids.",
        hint: "Step 3 of the guided path. Prefer this before matching.",
        availability: CliAvailability::Runnable,
        run_command: Some("discover"),
    },
    CliCommandCard {
        group: "Pipeline",
        name: "match",
        argv: "careerai match [--tune]",
        summary: "Filter and rank discovered listings against the profile.",
        hint: "Step 4. Shortlisted cards then become ready to tailor.",
        availability: CliAvailability::Runnable,
        run_command: Some("match"),
    },
    CliCommandCard {
        group: "Pipeline",
        name: "run",
        argv: "careerai run [--auto-submit]",
        summary: "Walk match → tailor → render → apply in one shot (dry-run by default).",
        hint: "Use when the funnel already has shortlisted work. Never enables live submit unless you opt in.",
        availability: CliAvailability::Runnable,
        run_command: Some("run"),
    },
    CliCommandCard {
        group: "Pipeline",
        name: "tailor",
        argv: "careerai tailor <listing-id>",
        summary: "LLM-tailor the master resume to one shortlisted listing.",
        hint: "Step 5. Needs a listing id from Explorer or a funnel card.",
        availability: CliAvailability::NeedsId,
        run_command: Some("tailor"),
    },
    CliCommandCard {
        group: "Pipeline",
        name: "render",
        argv: "careerai render <application-id>",
        summary: "Render a tailored application to DOCX + PDF via pandoc.",
        hint: "Step 6. pandoc must be on PATH.",
        availability: CliAvailability::NeedsId,
        run_command: Some("render"),
    },
    CliCommandCard {
        group: "Pipeline",
        name: "apply",
        argv: "careerai apply [--all] [--auto-submit]",
        summary: "Submit prepared applications. Defaults to dry-run.",
        hint: "Step 7. Live submit stays off unless auto_submit is enabled in config.",
        availability: CliAvailability::NeedsId,
        run_command: Some("apply"),
    },
    CliCommandCard {
        group: "Pipeline",
        name: "retry",
        argv: "careerai retry <application-id>",
        summary: "Reset a failed application and retry apply.",
        hint: "Use from Explorer on failed rows, or paste an application id here.",
        availability: CliAvailability::NeedsId,
        run_command: Some("retry"),
    },
    CliCommandCard {
        group: "Pipeline",
        name: "review",
        argv: "careerai review",
        summary: "Review drafted LinkedIn Easy Apply packets.",
        hint: "Opens the Action Center path for human approval.",
        availability: CliAvailability::Runnable,
        run_command: Some("review"),
    },
    CliCommandCard {
        group: "Inspect",
        name: "shortlist show",
        argv: "careerai shortlist show [--limit N]",
        summary: "List currently shortlisted listings.",
        hint: "Read-only snapshot of what match selected.",
        availability: CliAvailability::Runnable,
        run_command: Some("shortlist show"),
    },
    CliCommandCard {
        group: "Inspect",
        name: "applied",
        argv: "careerai applied [--source S] [--limit N]",
        summary: "Show applications submitted so far.",
        hint: "Includes dry-run would_submit events.",
        availability: CliAvailability::Runnable,
        run_command: Some("applied"),
    },
    CliCommandCard {
        group: "Inspect",
        name: "inspect",
        argv: "careerai inspect <application-id>",
        summary: "Dump an application row, state history, and artifacts.",
        hint: "Same data as the dashboard inspect modal.",
        availability: CliAvailability::NeedsId,
        run_command: Some("inspect"),
    },
    CliCommandCard {
        group: "Inspect",
        name: "digest",
        argv: "careerai digest [--since 24h]",
        summary: "Print a daily summary of pipeline activity.",
        hint: "Good end-of-day check after a discover + match cycle.",
        availability: CliAvailability::Runnable,
        run_command: Some("digest"),
    },
    CliCommandCard {
        group: "Sources",
        name: "sources sync",
        argv: "careerai sources sync [--apply]",
        summary: "Preview or merge seeded ATS companies into config/local.yaml.",
        hint: "Default is preview. Pass apply only after reading the diff.",
        availability: CliAvailability::Runnable,
        run_command: Some("sources sync"),
    },
    CliCommandCard {
        group: "Sources",
        name: "sources discover-web",
        argv: "careerai sources discover-web [--apply]",
        summary: "Discover job and freelance portals via the web-search agent.",
        hint: "Preview by default. Apply writes newly found portals into local.yaml.",
        availability: CliAvailability::Runnable,
        run_command: Some("sources discover-web"),
    },
    CliCommandCard {
        group: "Diagnose",
        name: "mcp probe",
        argv: "careerai mcp probe",
        summary: "Probe configured MCP job sources for reachability.",
        hint: "Read-only. Never sends a real query.",
        availability: CliAvailability::Runnable,
        run_command: Some("mcp probe"),
    },
    CliCommandCard {
        group: "Diagnose",
        name: "llm probe",
        argv: "careerai llm probe",
        summary: "Report which LLM backend Auto would pick and ping it.",
        hint: "Use after changing backend, model, or API base in Config.",
        availability: CliAvailability::Runnable,
        run_command: Some("llm probe"),
    },
    CliCommandCard {
        group: "Diagnose",
        name: "notify test",
        argv: "careerai notify test",
        summary: "Fire a synthetic event through every notification channel.",
        hint: "Confirms Slack / Telegram / email / ntfy wiring.",
        availability: CliAvailability::Runnable,
        run_command: Some("notify test"),
    },
    CliCommandCard {
        group: "Credentials",
        name: "cookies refresh",
        argv: "careerai cookies refresh <linkedin|naukri>",
        summary: "Prompt for a session cookie and store it in the OS keyring.",
        hint: "Must run in a terminal — the dashboard never collects cookies.",
        availability: CliAvailability::CliOnly,
        run_command: None,
    },
    CliCommandCard {
        group: "Long-running",
        name: "daemon",
        argv: "careerai daemon",
        summary: "Start the cron-driven scheduler in the foreground.",
        hint: "Keep this in a terminal or a systemd/runit unit, not the dashboard process.",
        availability: CliAvailability::CliOnly,
        run_command: None,
    },
    CliCommandCard {
        group: "Long-running",
        name: "status serve",
        argv: "careerai status serve [--port N]",
        summary: "Start this HTTP dashboard.",
        hint: "Already running — this page is the server.",
        availability: CliAvailability::CliOnly,
        run_command: None,
    },
    CliCommandCard {
        group: "Service",
        name: "service install",
        argv: "careerai service install [--force]",
        summary: "Install the systemd user unit for careerai daemon.",
        hint: "Requires a login session with systemd --user.",
        availability: CliAvailability::CliOnly,
        run_command: None,
    },
    CliCommandCard {
        group: "Service",
        name: "service status",
        argv: "careerai service status",
        summary: "Show systemctl --user status for the careerai unit.",
        hint: "The header daemon pill is the live equivalent on this page.",
        availability: CliAvailability::CliOnly,
        run_command: None,
    },
    CliCommandCard {
        group: "Service",
        name: "service uninstall",
        argv: "careerai service uninstall",
        summary: "Disable, stop, and remove the systemd user unit.",
        hint: "CLI only — destructive to the installed service.",
        availability: CliAvailability::CliOnly,
        run_command: None,
    },
];

pub fn catalog() -> Vec<CliCommandCard> {
    CATALOG.to_vec()
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CliGroup {
    pub name: String,
    pub cards: Vec<CliCommandCard>,
}

/// Catalog cards grouped by their `group` label, preserving first-seen
/// order so the Commands tab follows the catalog's narrative flow.
pub fn grouped() -> Vec<CliGroup> {
    let mut groups: Vec<CliGroup> = Vec::new();
    for card in catalog() {
        match groups.iter_mut().find(|g| g.name == card.group) {
            Some(g) => g.cards.push(card),
            None => groups.push(CliGroup {
                name: card.group.to_string(),
                cards: vec![card],
            }),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
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
}
