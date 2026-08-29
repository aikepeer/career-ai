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
}
