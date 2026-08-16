//! Web Search Discovery Agent — dynamically discovers emerging job portals
//! and freelance marketplaces (Remote-first -> Freelance/Gig -> Tech/Indian -> Global).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveredPortal {
    pub name: String,
    pub category: String, // "freelance", "remote_first", "tech_niche", "regional"
    pub base_url: String,
    pub priority: u32,
    pub description: String,
}

#[derive(Debug)]
pub struct WebSearchDiscoveryAgent {
    pub default_queries: Vec<String>,
}

impl Default for WebSearchDiscoveryAgent {
    fn default() -> Self {
        Self {
            default_queries: vec![
                "remote embedded systems engineering freelance jobs".into(),
                "ai ml robotics remote contract work".into(),
                "top freelance platforms for embedded linux developers".into(),
                "tech job portals in India for hardware and software".into(),
            ],
        }
    }
}

impl WebSearchDiscoveryAgent {
    #[must_use]
    pub fn curated_seed_portals() -> Vec<DiscoveredPortal> {
        vec![
            DiscoveredPortal {
                name: "Upwork".into(),
                category: "freelance".into(),
                base_url: "https://www.upwork.com".into(),
                priority: 1,
                description: "Global freelance and contract gig marketplace for AI, Embedded, and Web.".into(),
            },
            DiscoveredPortal {
                name: "Toptal".into(),
                category: "freelance".into(),
                base_url: "https://www.toptal.com".into(),
                priority: 1,
                description: "Top 3% freelance network for senior engineers and software architects.".into(),
            },
            DiscoveredPortal {
                name: "Contra".into(),
                category: "freelance".into(),
                base_url: "https://contra.com".into(),
                priority: 1,
                description: "Commission-free freelance network for tech and design creators.".into(),
            },
            DiscoveredPortal {
                name: "Arc.dev".into(),
                category: "remote_first".into(),
                base_url: "https://arc.dev".into(),
                priority: 1,
                description: "Remote developer job search & contract developer matching.".into(),
            },
            DiscoveredPortal {
                name: "Remotive".into(),
                category: "remote_first".into(),
                base_url: "https://remotive.com".into(),
                priority: 2,
                description: "Curated remote tech job board.".into(),
            },
            DiscoveredPortal {
                name: "Wellfound (AngelList)".into(),
                category: "remote_first".into(),
                base_url: "https://wellfound.com".into(),
                priority: 2,
                description: "Startup job postings & remote engineering positions.".into(),
            },
            DiscoveredPortal {
                name: "Naukri".into(),
                category: "regional".into(),
                base_url: "https://www.naukri.com".into(),
                priority: 3,
                description: "Leading Indian tech job search portal.".into(),
            },
            DiscoveredPortal {
                name: "Freelancer.com".into(),
                category: "freelance".into(),
                base_url: "https://www.freelancer.com".into(),
                priority: 1,
                description: "Global freelance and crowdsourcing marketplace for tech projects.".into(),
            },
            DiscoveredPortal {
                name: "Guru".into(),
                category: "freelance".into(),
                base_url: "https://www.guru.com".into(),
                priority: 1,
                description: "Freelance network for software development and engineering contracts.".into(),
            },
            DiscoveredPortal {
                name: "WeWorkRemotely".into(),
                category: "remote_first".into(),
                base_url: "https://weworkremotely.com".into(),
                priority: 2,
                description: "Largest remote work community for software & devops roles.".into(),
            },
            DiscoveredPortal {
                name: "Foundit (Monster India)".into(),
                category: "regional".into(),
                base_url: "https://www.foundit.in".into(),
                priority: 3,
                description: "Major tech & engineering job portal across India and SEA.".into(),
            },
        ]
    }

    /// Discover new job portals and freelance platforms matching target queries.
    #[must_use]
    pub fn discover_portals(&self) -> Vec<DiscoveredPortal> {
        let mut portals = Self::curated_seed_portals();
        // Deduplicate by base_url
        let mut seen = std::collections::HashSet::new();
        portals.retain(|p| seen.insert(p.base_url.clone()));
        portals
    }

    /// Apply discovered portals into `config/local.yaml`.
    pub fn apply_to_config(
        &self,
        config_path: &std::path::Path,
        portals: &[DiscoveredPortal],
    ) -> anyhow::Result<usize> {
        let mut content = if config_path.exists() {
            std::fs::read_to_string(config_path)?
        } else {
            "version: \"1.0\"\nsources:\n".to_string()
        };

        let mut added_count = 0;
        if !content.contains("discovered_portals:") {
            content.push_str("\ndiscovered_portals:\n");
        }

        for portal in portals {
            let entry_snippet = format!("  - name: \"{}\"", portal.name);
            if !content.contains(&entry_snippet) {
                content.push_str(&format!(
                    "  - name: \"{}\"\n    category: \"{}\"\n    base_url: \"{}\"\n    priority: {}\n",
                    portal.name, portal.category, portal.base_url, portal.priority
                ));
                added_count += 1;
            }
        }

        if let Some(parent) = config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        std::fs::write(config_path, content)?;
        Ok(added_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_curated_seed_portals() {
        let portals = WebSearchDiscoveryAgent::curated_seed_portals();
        assert!(!portals.is_empty());
        assert!(portals.iter().any(|p| p.category == "freelance"));
        assert!(portals.iter().any(|p| p.name == "Upwork"));
    }

    #[test]
    fn test_discover_and_apply_to_config() {
        let agent = WebSearchDiscoveryAgent::default();
        let portals = agent.discover_portals();
        assert!(!portals.is_empty());

        let temp_dir = tempfile::tempdir().unwrap();
        let cfg_path = temp_dir.path().join("config").join("local.yaml");

        let added = agent.apply_to_config(&cfg_path, &portals).unwrap();
        assert_eq!(added, portals.len());
        assert!(cfg_path.exists());

        let content = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(content.contains("Upwork"));
        assert!(content.contains("freelance"));

        // Idempotency check
        let re_added = agent.apply_to_config(&cfg_path, &portals).unwrap();
        assert_eq!(re_added, 0);
    }
}

