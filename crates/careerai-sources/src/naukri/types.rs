//! Response types and constants for the Naukri API.

use serde::Deserialize;

pub(crate) const DEFAULT_BASE_URL: &str = "https://www.naukri.com";
/// Prefix used to absolutize `jdURL`, which upstream returns as a
/// site-relative path (e.g. `/job-listings-...`).
pub(crate) const NAUKRI_WEB_ORIGIN: &str = "https://www.naukri.com";
pub(crate) const DEFAULT_MAX_RESULTS: usize = 20;

#[derive(Debug, Deserialize)]
pub(crate) struct Payload {
    #[serde(rename = "jobDetails", default)]
    pub(crate) job_details: Vec<JobDetail>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub(crate) struct JobDetail {
    /// Naukri's payload has historically returned `jobId` as both a
    /// string ("280125500001") and a JSON number (280125500001) at
    /// different points in the API lifecycle. Accept either; the
    /// deserializer normalizes to `Option<String>`.
    #[serde(
        rename = "jobId",
        default,
        deserialize_with = "deserialize_string_or_number"
    )]
    pub(crate) job_id: Option<String>,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(rename = "companyName", default)]
    pub(crate) company_name: Option<String>,
    #[serde(default)]
    pub(crate) placeholders: Vec<Placeholder>,
    #[serde(rename = "jdURL", default)]
    pub(crate) jd_url: String,
    #[serde(rename = "jobDescription", default)]
    pub(crate) job_description: Option<String>,
    #[serde(rename = "tagsAndSkills", default)]
    #[allow(dead_code)]
    pub(crate) tags_and_skills: Option<String>,
    #[serde(rename = "createdDate", default)]
    #[allow(dead_code)]
    pub(crate) created_date: Option<i64>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub(crate) struct Placeholder {
    #[serde(rename = "type", default)]
    pub(crate) kind: Option<String>,
    #[serde(default)]
    pub(crate) label: Option<String>,
}

/// Accept JSON `"280125"`, `280125`, or `null` for fields that should
/// land as `Option<String>`. Naukri's API has shipped jobId in both
/// string and integer forms across versions; tolerate both rather than
/// drop a whole page of listings on a serde error.
pub(crate) fn deserialize_string_or_number<'de, D>(
    de: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Option<String>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("string, integer, or null")
        }
        fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Self::Value, E> {
            Ok(Some(v.to_owned()))
        }
        fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Self::Value, E> {
            Ok(Some(v))
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
            Ok(Some(v.to_string()))
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
            Ok(Some(v.to_string()))
        }
        #[allow(clippy::cast_possible_truncation)]
        fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Self::Value, E> {
            if v.is_finite() {
                Ok(Some((v as i64).to_string()))
            } else {
                Err(de::Error::custom("non-finite number for jobId"))
            }
        }
        fn visit_some<D: serde::Deserializer<'de>>(
            self,
            de: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            de.deserialize_any(V)
        }
    }
    de.deserialize_any(V)
}
