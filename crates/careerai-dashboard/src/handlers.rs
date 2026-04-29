use std::fmt::Write as _;
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
};

use crate::data;
use crate::next_steps;
use crate::view::IndexView;
use crate::AppState;

pub async fn index(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match render_index(&state).await {
        Ok(body) => (StatusCode::OK, Html(body)).into_response(),
        Err(err) => {
            // chain the error so Tera template errors surface their inner cause
            let mut chain = format!("{err}");
            let mut src = std::error::Error::source(&err);
            while let Some(s) = src {
                let _ = write!(chain, " :: {s}");
                src = s.source();
            }
            tracing::error!(error = %chain, "dashboard index render failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(fallback_error_page()),
            )
                .into_response()
        }
    }
}

async fn render_index(state: &AppState) -> crate::error::Result<String> {
    let snap = data::snapshot(&state.pool).await?;
    let next_steps = next_steps::compute(&snap);
    let view = IndexView {
        kpi: snap.kpi,
        columns: snap.columns,
        next_steps,
    };
    let mut ctx = tera::Context::new();
    ctx.insert("view", &view);
    ctx.insert("refresh_seconds", &state.refresh_seconds);
    ctx.insert("css", crate::STYLE_CSS);
    ctx.insert("build_version", crate::BUILD_VERSION);
    Ok(state.tera.render("index.tera", &ctx)?)
}

pub async fn healthz() -> &'static str {
    "ok\n"
}

fn fallback_error_page() -> String {
    "<!doctype html><html><body><h1>career-ai</h1>\
     <p>Data temporarily unavailable. Check daemon logs.</p>\
     </body></html>"
        .to_string()
}
