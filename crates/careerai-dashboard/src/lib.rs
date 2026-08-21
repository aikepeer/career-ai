use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use sqlx::SqlitePool;
use tera::Tera;

pub mod chat;
pub mod cli_catalog;
pub mod daemon_health;
pub mod data;
pub mod details;
pub mod error;
pub mod guided;
pub mod handlers;
pub mod llm_health;
pub mod next_steps;
pub mod profile_handler;
pub mod profile_llm;
pub mod routes;
pub mod security;
pub mod view;

pub use error::{DashboardError, Result};

pub(crate) const STYLE_CSS: &str = include_str!("../static/style.css");
pub(crate) const INDEX_TERA: &str = include_str!("../templates/index.tera");
pub(crate) const BUILD_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct ServeOptions {
    pub port: u16,
    pub bind: IpAddr,
    pub refresh_seconds: u32,
    pub pool: SqlitePool,
}

impl ServeOptions {
    pub fn loopback(pool: SqlitePool, port: u16) -> Self {
        Self {
            port,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            refresh_seconds: 60,
            pool,
        }
    }
}

#[derive(Debug)]
pub struct AppState {
    pub pool: SqlitePool,
    pub tera: Arc<Tera>,
    pub refresh_seconds: u32,
    /// Optional bearer token required on every request. When set (via
    /// `CAREERAI_DASHBOARD_TOKEN`), the dashboard becomes an
    /// authenticated endpoint even if it is bound beyond loopback.
    pub auth_token: Option<String>,
}

pub async fn run(opts: ServeOptions) -> Result<()> {
    let addr = SocketAddr::new(opts.bind, opts.port);
    let non_loopback_allowed =
        std::env::var("CAREERAI_DASHBOARD_ALLOW_NON_LOOPBACK").as_deref() == Ok("1");
    validate_bind(opts.bind, non_loopback_allowed)?;
    if !opts.bind.is_loopback() {
        tracing::warn!(
            %addr,
            "dashboard bound to non-loopback address — set CAREERAI_DASHBOARD_TOKEN to \
             require a bearer token before exposing to untrusted networks"
        );
    }

    let tera = build_tera()?;
    let auth_token = std::env::var("CAREERAI_DASHBOARD_TOKEN")
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    let state = Arc::new(AppState {
        pool: opts.pool,
        tera: Arc::new(tera),
        refresh_seconds: opts.refresh_seconds,
        auth_token,
    });
    let app = routes::build(state);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| DashboardError::BindFailed { addr, source })?;
    tracing::info!(%addr, "dashboard listening");
    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Reject a non-loopback bind unless the operator explicitly opted in.
/// Extracted for direct unit testing — the guard is the safety rail that
/// keeps the unauthenticated dashboard off the network by default.
fn validate_bind(bind: IpAddr, non_loopback_allowed: bool) -> Result<()> {
    if !bind.is_loopback() && !non_loopback_allowed {
        return Err(DashboardError::NonLoopbackBind(bind));
    }
    Ok(())
}

fn build_tera() -> Result<Tera> {
    let mut tera = Tera::default();
    // Tera autoescapes templates whose registered name ends in
    // `.html`/`.htm`/`.xml`. `index.tera` would NOT match, so register
    // the template under an `.html` name to guarantee every `{{ }}` of
    // job-board / DB / profile data is HTML-escaped.
    tera.register_filter("js", js_escape_filter);
    tera.add_raw_template("index.html", INDEX_TERA)?;
    Ok(tera)
}

/// Escape a string for safe embedding inside a single-quoted JavaScript
/// string literal within an HTML attribute. Tera's HTML autoescape does
/// NOT protect JS-string context: `&#x27;` is decoded back to `'` by the
/// HTML parser before the inline handler is compiled, so a value like
/// `'); alert(1); //` would break out. This filter emits both
/// JS-string escapes and HTML-attribute-safe entities so the template can
/// apply `| js | safe` without reopening the HTML context.
fn js_escape_filter(
    value: &tera::Value,
    _args: &std::collections::HashMap<String, tera::Value>,
) -> std::result::Result<tera::Value, tera::Error> {
    let s = match value {
        tera::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    tera::to_value(js_escape(&s)).map_err(tera::Error::from)
}

fn js_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            // HTML-attribute delimiters stay entity-encoded so they can
            // never terminate the `onclick="..."` attribute.
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // The JS string is single-quoted; escape the quote and the
            // backslash that introduces escapes.
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let Ok(mut term) = signal(SignalKind::terminate()) else {
            return ctrl_c.await;
        };
        tokio::select! {
            () = ctrl_c => {},
            _ = term.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        ctrl_c.await;
    }
    tracing::info!("dashboard shutdown signal received");
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn js_filter_escapes_js_string_context() {
        assert_eq!(js_escape("plain"), "plain");
        assert_eq!(js_escape("a'b"), "a\\'b");
        assert_eq!(js_escape("a\\b"), "a\\\\b");
        assert_eq!(js_escape("a\"b"), "a&quot;b");
        assert_eq!(js_escape("a<b&c"), "a&lt;b&amp;c");
        assert_eq!(js_escape("a\nb"), "a\\nb");
    }

    #[test]
    fn build_tera_registers_js_filter() {
        let mut tera = build_tera().unwrap();
        let mut ctx = tera::Context::new();
        ctx.insert("id", "x');alert(1);//");
        let rendered = tera
            .render_str(
                r#"<button onclick="openAppModal('{{ id | js | safe }}')">go</button>"#,
                &ctx,
            )
            .unwrap();
        assert!(rendered.contains(r"openAppModal('x\');alert(1);//')"));
        assert!(
            !rendered.contains("</button><script>"),
            "raw JS must not survive"
        );
    }

    #[test]
    fn index_template_is_registered_as_html_for_autoescape() {
        let tera = build_tera().unwrap();
        assert!(
            tera.get_template_names().any(|n| n == "index.html"),
            "index template must be registered under an .html name so Tera autoescapes it"
        );
    }

    #[test]
    fn index_template_exposes_dashboard_cli_parity_controls() {
        for needle in [
            "runCliCommand",
            "Run All",
            "Review Drafted LinkedIn",
            "Tailor",
            "Render",
            "Apply",
            "Retry",
            "Inspect",
            "CLI Commands",
            "guided-path",
            "Do Step",
            "careerai init",
            "careerai daemon",
            "tab-btn-commands",
        ] {
            assert!(
                INDEX_TERA.contains(needle),
                "index.tera must expose `{needle}` for dashboard/CLI parity",
            );
        }
    }

    #[test]
    fn index_template_shows_explorer_listing_ids() {
        // The Explorer table must surface each listing's DB id so the
        // operator can copy it and run `careerai tailor <id>` from the
        // terminal. Both render paths (server-side Tera, client-side
        // refresh) must emit a copyable listing-id cell.
        assert!(
            INDEX_TERA.contains("<th>ID</th>"),
            "explorer table must have a visible ID column header"
        );
        assert!(
            INDEX_TERA.contains("listing-id-cell"),
            "explorer rows must render a copyable listing-id cell"
        );
        // Server-rendered row embeds the id via Tera + the copy helper;
        // the JS refresh path must do the same from /api/v1/explorer.
        assert!(
            INDEX_TERA.contains("copyCliCommand('{{ item.id | js | safe }}'"),
            "server-rendered id cell must be click-to-copy with the Tera id"
        );
        assert!(
            INDEX_TERA.contains("escapeJsString(item.id)"),
            "JS-refresh id cell must embed item.id"
        );
        assert!(
            INDEX_TERA.contains("escapeHtml(item.id)"),
            "JS-refresh id cell must render the id text"
        );
    }

    #[test]
    fn validate_bind_rejects_non_loopback_without_opt_in() {
        let public = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
        assert!(matches!(
            validate_bind(public, false),
            Err(DashboardError::NonLoopbackBind(_))
        ));
    }

    #[test]
    fn validate_bind_allows_non_loopback_with_opt_in() {
        let public = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
        assert!(validate_bind(public, true).is_ok());
    }

    #[test]
    fn validate_bind_allows_loopback_without_opt_in() {
        assert!(validate_bind(IpAddr::V4(Ipv4Addr::LOCALHOST), false).is_ok());
    }

    #[test]
    fn tera_autoescapes_html_named_templates() {
        // Locks the Tera default this crate relies on: `.html`-named
        // templates are autoescaped, while the old `index.tera` name was
        // not.
        let mut tera = Tera::default();
        tera.add_raw_template("x.html", "<h1>{{ v }}</h1>").unwrap();
        let mut ctx = tera::Context::new();
        ctx.insert("v", "<script>alert(1)</script>");
        let rendered = tera.render("x.html", &ctx).unwrap();
        assert!(
            !rendered.contains("<script>"),
            "raw HTML must not survive: {rendered}"
        );
        assert!(
            rendered.contains("&lt;script&gt;"),
            "must be escaped: {rendered}"
        );
    }
}
