//! Serializable interview-preparation sheet types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrepSheet {
    pub application_id: String,
    pub job_title: String,
    pub company: String,
    pub jd_summary: String,
    pub likely_topics: Vec<String>,
    pub behavioral_questions: Vec<String>,
    pub bullet_to_keyword: Vec<BulletKeyword>,
    #[serde(default)]
    pub company_news: Vec<String>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BulletKeyword {
    pub bullet: String,
    pub keywords: Vec<String>,
}
