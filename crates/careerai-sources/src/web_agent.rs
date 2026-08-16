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
        ]
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
}
