use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use crate::{auth::AuthToken, db::DEFAULT_CHANNEL, AppState};
use super::{ApiError, ApiResult};

/// PUT /api/v1/packages/:name/:version/docs
///
/// Body: raw msgpack bytes (output of `docify dump`).
/// Requires the caller to be an owner of the package.
pub async fn put_docs(
    auth: AuthToken,
    State(state): State<Arc<AppState>>,
    Path((name, version)): Path<(String, String)>,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    if body.is_empty() {
        return Err(ApiError::bad_request("docs body is empty"));
    }
    // Validate that it is parseable msgpack containing DocItems.
    docify::from_msgpack(&body)
        .map_err(|_| ApiError::bad_request("invalid msgpack — expected docify dump output"))?;

    let (pkg, _versions) = state.db.get_package(&name, DEFAULT_CHANNEL).await?
        .ok_or_else(|| ApiError::not_found(format!("package `{name}` not found")))?;

    if !state.db.user_can_manage_package(&name, &pkg.channel, auth.user.id).await? {
        return Err(ApiError::forbidden("you do not own this package"));
    }

    state.storage.save_docs(&name, &version, &body).await
        .map_err(|e| ApiError::internal(format!("storage error: {e}")))?;

    state.db.audit(Some(auth.user.id), "upload_docs", Some(&name), Some(&version), None);

    Ok(Json(json!({ "ok": true })))
}

/// GET /api/v1/packages/:name/:version/docs
///
/// Returns the docset as a JSON array of DocItem objects.
/// Returns 404 if no docs have been uploaded for this version.
pub async fn get_docs(
    State(state): State<Arc<AppState>>,
    Path((name, version)): Path<(String, String)>,
) -> impl IntoResponse {
    // Resolve "latest" alias
    let resolved_version = if version == "latest" {
        match state.db.get_package(&name, DEFAULT_CHANNEL).await {
            Ok(Some((pkg, _))) => pkg.latest_version.unwrap_or(version),
            _ => version,
        }
    } else {
        version
    };

    let blob = match state.storage.read_docs(&name, &resolved_version).await {
        Some(b) => b,
        None => return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            b"{\"errors\":[{\"detail\":\"no docs for this version\"}]}".to_vec(),
        ).into_response(),
    };

    let items: Vec<docify::DocItem> = match docify::from_msgpack(&blob) {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("docs msgpack decode failed for {name}/{resolved_version}: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, [(header::CONTENT_TYPE, "application/json")],
                b"{\"errors\":[{\"detail\":\"docs data corrupt\"}]}".to_vec()).into_response();
        }
    };

    let json_items: Vec<Value> = items.iter().map(|item| {
        let tags: Vec<Value> = item.tags.iter().map(|t| json!({
            "kind":  format!("{:?}", t.kind),
            "label": t.kind.label(),
            "name":  t.name,
            "text":  t.text,
        })).collect();

        json!({
            "name":      item.name,
            "kind":      item.kind.label(),
            "lang":      item.lang.label(),
            "brief":     item.brief,
            "body":      item.body,
            "signature": item.signature,
            "file":      item.file.to_string_lossy(),
            "line":      item.line,
            "tags":      tags,
            "meta": {
                "template_params": item.meta.template_params,
                "access":          item.meta.access.as_ref().map(|a| format!("{a:?}").to_lowercase()),
                "parent":          item.meta.parent,
                "attrs":           item.meta.attrs,
                "group":           item.meta.group,
            },
        })
    }).collect();

    let body = serde_json::to_vec(&json!({ "items": json_items, "total": json_items.len() }))
        .unwrap_or_default();

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    ).into_response()
}
