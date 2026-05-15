//! GET /health — liveness + readiness check.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

use crate::AppState;

pub async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db_ok = state.db.ping().await;
    let uptime = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let status = if db_ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (
        status,
        Json(json!({
            "status": if db_ok { "ok" } else { "degraded" },
            "db":     if db_ok { "ok" } else { "error" },
            "time":   uptime,
        })),
    )
}
