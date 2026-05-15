use std::sync::Arc;

use axum::{
    extract::State,
    http::header,
    response::IntoResponse,
};

use crate::AppState;

pub async fn metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if let Ok(stats) = state.db.stats().await {
        state.metrics.packages_count.set(stats.packages);
        state.metrics.versions_count.set(stats.versions);
        state.metrics.users_count.set(stats.users);
        state.metrics.tokens_active.set(stats.tokens_active);
        state.metrics.downloads_db_total.set(stats.downloads_total);
    }

    let body = state.metrics.encode();
    (
        [(header::CONTENT_TYPE, "application/openmetrics-text; version=1.0.0; charset=utf-8")],
        body,
    )
}
