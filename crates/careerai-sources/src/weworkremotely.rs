//! WeWorkRemotely RSS feed.
//!
//! Endpoint: `GET {base}/categories/remote-programming-jobs.rss`
//! Returns XML RSS 2.0 with `<item>` entries.

use std::time::Duration;

use async_trait::async_trait;
use quick_xml::events::Event;
use reqwest::Client;

use crate::base::{RawListing, Source, SourceError};
use crate::util::html_to_text;

const DEFAULT_BASE_URL: &str = "https://weworkremotely.com";

#[derive(Debug)]
pub struct WeWorkRemotelySource {
    base_url: String,
    http: Client,
}

impl Default for WeWorkRemotelySource {
    fn default() -> Self {
        #[allow(clippy::expect_used)]
        let http = Client::builder()
            .user_agent("careerai/0.1 (+https://github.com/justdoGIT/career-ai)")
            .build()
            .expect("build reqwest client");
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            http,
        }
    }
}

impl WeWorkRemotelySource {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

#[async_trait]
impl Source for WeWorkRemotelySource {
    fn name(&self) -> &'static str {
        "weworkremotely"
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        let url = format!(
            "{}/categories/remote-programming-jobs.rss",
            self.base_url.trim_end_matches('/')
        );
        let resp = self
            .http
            .get(&url)
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(SourceError::HttpStatus {
                status: resp.status().as_u16(),
                body: resp.text().await.unwrap_or_default(),
            });
        }
        let body = resp.text().await?;
        parse_rss(&body)
    }
}

fn parse_rss(xml: &str) -> Result<Vec<RawListing>, SourceError> {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut items = Vec::new();
    let mut in_item = false;
    let mut current_tag = String::new();
    let mut title = String::new();
    let mut link = String::new();
    let mut description = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "item" {
                    in_item = true;
                    title.clear();
                    link.clear();
                    description.clear();
                }
                current_tag = name;
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "item" && in_item {
                    in_item = false;
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "item" && in_item {
                    let (company, clean_title) = split_company_from_title(&title);
                    let external_id = link.trim().rsplit('/').next().unwrap_or(&link).to_string();
                    items.push(RawListing {
                        source: "weworkremotely".to_string(),
                        external_id,
                        title: clean_title,
                        company,
                        location: Some("Remote".to_string()),
                        url: link.clone(),
                        description: html_to_text(&description),
                        raw_json: None,
                    });
                    in_item = false;
                }
                current_tag.clear();
            }
            Ok(Event::Text(e)) => {
                if in_item {
                    let text = String::from_utf8_lossy(e.as_ref()).to_string();
                    let text = text
                        .replace("&amp;", "&")
                        .replace("&lt;", "<")
                        .replace("&gt;", ">")
                        .replace("&quot;", "\"")
                        .replace("&apos;", "'");
                    match current_tag.as_str() {
                        "title" => title.push_str(&text),
                        "link" => link.push_str(&text),
                        "description" => description.push_str(&text),
                        _ => {}
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(SourceError::Parse(e.to_string())),
            _ => {}
        }
        buf.clear();
    }

    Ok(items)
}

/// WeWorkRemotely titles are formatted as "Company: Job Title".
fn split_company_from_title(raw: &str) -> (String, String) {
    if let Some(idx) = raw.find(": ") {
        let company = raw[..idx].trim().to_string();
        let title = raw[idx + 2..].trim().to_string();
        (company, title)
    } else {
        (String::new(), raw.trim().to_string())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn parses_rss_feed() {
        let server = MockServer::start().await;
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>We Work Remotely</title>
    <item>
      <title>Acme Corp: Senior Backend Engineer</title>
      <link>https://weworkremotely.com/remote-jobs/123-senior-backend-engineer</link>
      <description>&lt;p&gt;Build scalable APIs in Rust.&lt;/p&gt;</description>
    </item>
    <item>
      <title>Robotics Inc: Embedded Systems Developer</title>
      <link>https://weworkremotely.com/remote-jobs/456-embedded-systems-developer</link>
      <description>&lt;p&gt;Work on RTOS and sensor fusion.&lt;/p&gt;</description>
    </item>
  </channel>
</rss>"#;
        Mock::given(method("GET"))
            .and(path("/categories/remote-programming-jobs.rss"))
            .respond_with(ResponseTemplate::new(200).set_body_string(xml))
            .mount(&server)
            .await;

        let source = WeWorkRemotelySource::new().with_base_url(server.uri());
        let listings = source.discover().await.unwrap();

        assert_eq!(listings.len(), 2);
        assert_eq!(listings[0].company, "Acme Corp");
        assert_eq!(listings[0].title, "Senior Backend Engineer");
        assert_eq!(listings[0].external_id, "123-senior-backend-engineer");
        assert!(listings[0].description.contains("scalable APIs"));
        assert_eq!(listings[1].company, "Robotics Inc");
        assert_eq!(listings[1].title, "Embedded Systems Developer");
    }

    #[tokio::test]
    async fn handles_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/categories/remote-programming-jobs.rss"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let source = WeWorkRemotelySource::new().with_base_url(server.uri());
        let err = source.discover().await.unwrap_err();
        assert!(matches!(err, SourceError::HttpStatus { .. }));
    }

    #[test]
    fn splits_company_from_title() {
        let (company, title) = split_company_from_title("Acme Corp: Senior Engineer");
        assert_eq!(company, "Acme Corp");
        assert_eq!(title, "Senior Engineer");
    }

    #[test]
    fn handles_title_without_company() {
        let (company, title) = split_company_from_title("Just a title");
        assert!(company.is_empty());
        assert_eq!(title, "Just a title");
    }
}
