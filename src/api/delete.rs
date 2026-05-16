//! DELETE /api/v1/packages/:name — hard-delete a package (admin only).
//!
//! Removes the package row and all its versions from the database (cascade),
//! then deletes the tarball directory from storage.  This is irreversible and
//! separate from yanking individual versions.

use std::sync::Arc;

use axum::{extract::{Path, Query, State}, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::AdminToken, db::DEFAULT_CHANNEL, AppState};
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
pub struct ChannelParam {
    #[serde(default)]
    channel: Option<String>,
}

pub async fn delete_package(
    _auth: AdminToken,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(params): Query<ChannelParam>,
) -> ApiResult<Json<Value>> {
    let channel = params.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);
    let found = state.db.delete_package(&name, channel).await?;
    if !found {
        return Err(ApiError::not_found(format!("`{name}` not found in channel `{channel}`")));
    }

    // Best-effort: remove tarballs; log but don't fail if the directory is gone.
    if let Err(e) = state.storage.delete_package_dir(&name).await {
        tracing::warn!(name, "failed to remove tarball directory: {e:#}");
    }

    tracing::info!(name, channel, "package hard-deleted by admin");
    Ok(Json(json!({ "ok": true })))
}
