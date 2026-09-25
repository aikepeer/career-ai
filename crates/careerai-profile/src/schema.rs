//! Canonical profile schema.
//!
//! Mirrors `profile.example.yaml`. Serde round-trips to/from YAML; `validator`
//! enforces required-field invariants (currently only non-empty `personal.name`).
//! Intentionally tolerant of empty experience/education/projects so partial
//! imports still round-trip cleanly.

use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::error::{ProfileError, Result};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub personal: Personal,

    #[serde(default)]
    pub summary: String,

    #[serde(
        default,
        alias = "target_roles",
        alias = "target roles",
        alias = "target-roles",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub target_roles: Vec<String>,

    #[serde(default)]
    pub skills: Skills,

    #[serde(default)]
    pub experience: Vec<Experience>,

    #[serde(default)]
    pub education: Vec<Education>,

    #[serde(default)]
    pub projects: Vec<Project>,

    /// Target-role archetypes (career-ops): named role profiles with a
    /// level and a fit class. Gives the tailor prompt concrete north-star
    /// framing instead of bare titles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archetypes: Vec<Archetype>,

    /// Career narrative (career-ops): headline, exit story, superpowers,
    /// and quantified proof points. Included in the tailor prompt so
    /// reworded bullets can draw on the profile's best material.
    #[serde(default, skip_serializing_if = "Narrative::is_empty")]
    pub narrative: Narrative,

    /// Compensation targets (career-ops): target range, minimum, and
    /// location flexibility. Used by `careerai salary --gap`.
    #[serde(default, skip_serializing_if = "Compensation::is_empty")]
    pub compensation: Compensation,
}

/// One target-role archetype (career-ops `target_roles.archetypes`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Archetype {
    pub name: String,
    #[serde(default)]
    pub level: String,
    /// `primary` = dream role, `secondary` = good fit, `adjacent` = stretch.
    #[serde(default)]
    pub fit: String,
}

/// Career narrative (career-ops `narrative` block).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Narrative {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub headline: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub exit_story: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub superpowers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proof_points: Vec<ProofPoint>,
}

impl Narrative {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.headline.is_empty()
            && self.exit_story.is_empty()
            && self.superpowers.is_empty()
            && self.proof_points.is_empty()
    }
}

/// A quantified proof point (career-ops `narrative.proof_points`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofPoint {
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub hero_metric: String,
}

/// Compensation targets (career-ops `compensation` block).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Compensation {
    /// e.g. "$150K-200K"
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target_range: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub currency: String,
    /// Walk-away number, e.g. "$120K".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub minimum: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub location_flexibility: String,
}

impl Compensation {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.target_range.is_empty()
            && self.currency.is_empty()
            && self.minimum.is_empty()
            && self.location_flexibility.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Personal {
    pub name: String,

    #[serde(default)]
    pub email: String,

    #[serde(default)]
    pub phone: String,

    #[serde(default)]
    pub location: String,

    #[serde(default)]
    pub links: Links,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Links {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub github: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub linkedin: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub portfolio: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Skills {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frameworks: Vec<String>,

    #[serde(
        default,
        alias = "devops",
        alias = "devOps",
        alias = "DevOps",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub devops: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub debugging: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protocols: Vec<String>,
}

impl Skills {
    pub fn all_skill_names(&self) -> impl Iterator<Item = &String> {
        self.languages
            .iter()
            .chain(self.platforms.iter())
            .chain(self.frameworks.iter())
            .chain(self.devops.iter())
            .chain(self.tools.iter())
            .chain(self.debugging.iter())
            .chain(self.protocols.iter())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Experience {
    pub title: String,
    pub company: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub location: String,

    /// ISO-like start date, `YYYY-MM` or `YYYY`.
    pub start: String,

    /// Same format as `start`, or the literal `"present"`.
    pub end: String,

    #[serde(default)]
    pub bullets: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Education {
    pub degree: String,
    pub institution: String,

    #[serde(default)]
    pub start: String,

    #[serde(default)]
    pub end: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hobbies: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub achievements: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub name: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,

    #[serde(default)]
    pub bullets: Vec<String>,
}

impl Validate for Personal {
    fn validate(&self) -> std::result::Result<(), validator::ValidationErrors> {
        if self.name.trim().is_empty() {
            let mut errors = validator::ValidationErrors::new();
            let mut err = validator::ValidationError::new("length");
            err.message = Some(std::borrow::Cow::Borrowed("name is required"));
            errors.add("name", err);
            return Err(errors);
        }
        Ok(())
    }
}

impl Validate for Profile {
    fn validate(&self) -> std::result::Result<(), validator::ValidationErrors> {
        if let Err(personal_errs) = self.personal.validate() {
            let mut errors = validator::ValidationErrors::new();
            for (_field, field_errs) in personal_errs.field_errors() {
                for err in field_errs {
                    errors.add("personal", err.clone());
                }
            }
            return Err(errors);
        }
        Ok(())
    }
}

impl Profile {
    /// Validate schema invariants. Wraps `validator` into `ProfileError`.
    pub fn check(&self) -> Result<()> {
        Validate::validate(self).map_err(|e| ProfileError::Validation(e.to_string()))
    }

    /// Serialize to YAML.
    pub fn to_yaml(&self) -> Result<String> {
        Ok(serde_yaml::to_string(self)?)
    }

    /// Parse from YAML text.
    pub fn from_yaml(text: &str) -> Result<Self> {
        Ok(serde_yaml::from_str(text)?)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn validation_rejects_empty_name() {
        let p = Profile::default();
        assert!(p.check().is_err());
    }

    #[test]
    fn validation_accepts_minimal_profile() {
        let mut p = Profile::default();
        p.personal.name = "Alice".into();
        p.check().unwrap();
    }

    #[test]
    fn yaml_roundtrip_preserves_fields() {
        let mut p = Profile::default();
        p.personal.name = "Alice".into();
        p.personal.email = "a@example.com".into();
        p.experience.push(Experience {
            title: "Engineer".into(),
            company: "Acme".into(),
            start: "2022-01".into(),
            end: "present".into(),
            bullets: vec!["shipped things".into()],
            ..Default::default()
        });
        let yaml = p.to_yaml().unwrap();
        let back = Profile::from_yaml(&yaml).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn career_ops_enrichment_roundtrips() {
        let mut p = Profile::default();
        p.personal.name = "Alice".into();
        p.archetypes.push(Archetype {
            name: "AI/ML Engineer".into(),
            level: "Senior".into(),
            fit: "primary".into(),
        });
        p.narrative.headline = "ML engineer turned product builder".into();
        p.narrative.superpowers = vec!["End-to-end ML pipelines".into()];
        p.narrative.proof_points.push(ProofPoint {
            name: "Alpha".into(),
            url: "https://x".into(),
            hero_metric: "-40% latency".into(),
        });
        p.compensation.target_range = "$150K-200K".into();
        p.compensation.minimum = "$120K".into();

        let yaml = p.to_yaml().unwrap();
        let back = Profile::from_yaml(&yaml).unwrap();
        assert_eq!(p, back);
        assert_eq!(back.archetypes[0].fit, "primary");
        assert_eq!(back.narrative.proof_points[0].hero_metric, "-40% latency");
        assert_eq!(back.compensation.target_range, "$150K-200K");
    }

    #[test]
    fn legacy_profile_without_enrichment_still_parses() {
        let yaml = "personal:\n  name: Bob\nskills: {}\nexperience: []\n";
        let p = Profile::from_yaml(yaml).unwrap();
        assert!(p.archetypes.is_empty());
        assert!(p.narrative.is_empty());
        assert!(p.compensation.is_empty());
    }
}
