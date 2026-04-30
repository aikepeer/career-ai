//! Schema types: [`DiffDoc`], [`DiffOp`], [`OpKind`], [`SummaryOp`],
//! [`BulletPath`], [`Section`], plus the path regex and `BulletPath`
//! parse/format helpers.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{Result, TailorError};

/// One JSON blob emitted by the tailor LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffDoc {
    pub prompt_version: String,
    #[serde(default)]
    pub summary: Option<SummaryOp>,
    pub ops: Vec<DiffOp>,
    pub cover_letter: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "op", deny_unknown_fields)]
pub enum SummaryOp {
    Keep,
    Reword { new_text: String },
}

// Note: no `deny_unknown_fields` on `DiffOp` — it's incompatible with
// `#[serde(flatten)]` on the `kind` field because serde can't tell at
// parse time which fields belong to the flattened subtype. The inner
// `OpKind` variants each carry `deny_unknown_fields`, which is where the
// airtightness actually lives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffOp {
    pub path: String,
    #[serde(flatten)]
    pub kind: OpKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "op", deny_unknown_fields)]
pub enum OpKind {
    Keep,
    Reword { new_text: String },
    Drop,
    MoveBefore { target_path: String },
}

/// Parsed bullet path. Stable equality + hashing for set membership.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BulletPath {
    pub section: Section,
    pub entry_index: usize,
    pub bullet_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Section {
    Experience,
    Projects,
}

impl Section {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Experience => "experience",
            Self::Projects => "projects",
        }
    }
}

pub(crate) fn path_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Safe: hard-coded, tested.
        #[allow(clippy::unwrap_used)]
        Regex::new(r"^(experience|projects)\[(\d+)\]\.bullets\[(\d+)\]$").unwrap()
    })
}

impl BulletPath {
    pub fn parse(s: &str) -> Result<Self> {
        let caps = path_regex()
            .captures(s)
            .ok_or_else(|| TailorError::BadPath(s.to_string()))?;
        let section = match caps
            .get(1)
            .ok_or_else(|| TailorError::BadPath(s.to_string()))?
            .as_str()
        {
            "experience" => Section::Experience,
            "projects" => Section::Projects,
            other => return Err(TailorError::BadPath(other.to_string())),
        };
        let entry_index: usize = caps
            .get(2)
            .ok_or_else(|| TailorError::BadPath(s.to_string()))?
            .as_str()
            .parse()
            .map_err(|_| TailorError::BadPath(s.to_string()))?;
        let bullet_index: usize = caps
            .get(3)
            .ok_or_else(|| TailorError::BadPath(s.to_string()))?
            .as_str()
            .parse()
            .map_err(|_| TailorError::BadPath(s.to_string()))?;
        Ok(Self {
            section,
            entry_index,
            bullet_index,
        })
    }

    #[must_use]
    pub fn format(&self) -> String {
        format!(
            "{}[{}].bullets[{}]",
            self.section.as_str(),
            self.entry_index,
            self.bullet_index
        )
    }
}
