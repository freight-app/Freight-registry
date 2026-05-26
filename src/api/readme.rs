use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::PublishToken, db::DEFAULT_CHANNEL, AppState};
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
pub struct ChannelParam {
    #[serde(default)]
    channel: Option<String>,
}

// ── GET /api/v1/packages/:name/readme ─────────────────────────────────────────

pub async fn get_readme(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Response {
    match state.storage.read_readme(&name).await {
        Some(content) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
            content,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── PUT /api/v1/packages/:name/readme ─────────────────────────────────────────
//
// Replaces (or sets for the first time) the README for a package.
// Body is raw Markdown text (Content-Type: text/markdown or text/plain).
// Caller must be an owner of the package or a registry admin.

pub async fn put_readme(
    State(state): State<Arc<AppState>>,
    auth: PublishToken,
    Path(name): Path<String>,
    Query(params): Query<ChannelParam>,
    body: String,
) -> ApiResult<Json<Value>> {
    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);

    // Admins may update any package's README; owners may only update their own.
    if auth.user.is_admin == 0 {
        match state.db.user_owns_package(auth.user.id, &name, channel).await? {
            Some(true)  => {}
            Some(false) => return Err(ApiError::forbidden(
                format!("you are not an owner of `{name}` in channel `{channel}`"),
            )),
            None => return Err(ApiError::not_found(
                format!("`{name}` not found in channel `{channel}`"),
            )),
        }
    }

    if body.len() > 512 * 1024 {
        return Err(ApiError::bad_request("README exceeds 512 KiB limit"));
    }

    state
        .storage
        .save_readme(&name, body.as_bytes())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state.db.audit(
        Some(auth.user.id),
        "update_readme",
        Some(&name),
        None,
        None,
    );

    Ok(Json(json!({ "ok": true })))
}
