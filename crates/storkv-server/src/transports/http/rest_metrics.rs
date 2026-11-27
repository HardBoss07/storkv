//! Prometheus metrics endpoint.

use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response as HttpResponse};

use super::HttpState;

/// Render the Prometheus text exposition for the running server.
pub(super) async fn metrics(State(state): State<HttpState>) -> HttpResponse {
    let body = crate::metrics::render(&state.server.stats, &state.server.db);
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}
