//! Abuse / problem reports.
//!
//! POST  /api/v1/packages/:name/report        — file a report (any authed user)
//! GET   /api/v1/admin/reports[?status=open]  — list reports (admin)
//! PATCH /api/v1/admin/reports/:id            — resolve / dismiss a report (admin)

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::{scope_allows_publish, AuthToken},
    permissions::Permission,
    AppState,
};

use super::{ApiError, ApiResult};

/// Accepted report categories.
const VALID_REASONS: &[&str] = &[
    "malware",
    "security",
    "license",
    "spam",
    "name-squatting",
    "other",
];

/// Terminal states an admin can move a report into.
const VALID_RESOLUTIONS: &[&str] = &["resolved", "dismissed"];

#[derive(Deserialize)]
pub struct ReportReq {
    pub reason: String,
    #[serde(default)]
    pub details: String,
    /// Optional specific version the report concerns.
    #[serde(default)]
    pub version: Option<String>,
}

/// File an abuse/problem report against a package. Any authenticated user may
/// report; admins triage via the endpoints below.
pub async fn file_report(
    auth: AuthToken,
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<ReportReq>,
) -> ApiResult<Json<Value>> {
    if !VALID_REASONS.contains(&req.reason.as_str()) {
        return Err(ApiError::bad_request(format!(
            "invalid reason `{}` — must be one of: {}",
            req.reason,
            VALID_REASONS.join(", ")
        )));
    }
    if req.details.len() > 4000 {
        return Err(ApiError::bad_request("details too long (max 4000 characters)"));
    }
    if !state.db.package_exists(&name).await? {
        return Err(ApiError::not_found(format!("package `{name}` not found")));
    }

    state
        .db
        .create_report(&name, req.version.as_deref(), auth.user.id, &req.reason, &req.details)
        .await?;
    state
        .db
        .audit(Some(auth.user.id), "report", Some(&name), req.version.as_deref(), None);
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct StatusFilter {
    /// Filter by report status (e.g. `open`, `resolved`, `dismissed`).
    pub status: Option<String>,
}

/// List reports for triage. `?status=open` filters; omit for all.
/// Requires the `ViewReports` permission (moderator or admin).
pub async fn list_reports(
    auth: AuthToken,
    State(state): State<Arc<AppState>>,
    Query(filter): Query<StatusFilter>,
) -> ApiResult<Json<Value>> {
    if !auth.user.can(Permission::ViewReports) {
        return Err(ApiError::forbidden("requires moderator or admin"));
    }
    let reports = state.db.list_reports(filter.status.as_deref()).await?;
    let list: Vec<Value> = reports
        .iter()
        .map(|r| {
            json!({
                "id":          r.id,
                "package":     r.package,
                "version":     r.version,
                "reporter_id": r.reporter_id,
                "reason":      r.reason,
                "details":     r.details,
                "status":      r.status,
                "created_at":  r.created_at,
                "resolved_by": r.resolved_by,
                "resolved_at": r.resolved_at,
                "resolution":  r.resolution,
            })
        })
        .collect();
    Ok(Json(json!({ "reports": list })))
}

#[derive(Deserialize)]
pub struct ResolveReq {
    /// `resolved` (action taken) or `dismissed` (no action needed).
    pub status: String,
    /// Free-text note recorded with the resolution.
    #[serde(default)]
    pub note: String,
}

/// Resolve or dismiss a report.
/// Requires the `ResolveReports` permission (moderator or admin) and a
/// non-read-only token.
pub async fn resolve_report(
    auth: AuthToken,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(req): Json<ResolveReq>,
) -> ApiResult<Json<Value>> {
    if !auth.user.can(Permission::ResolveReports) {
        return Err(ApiError::forbidden("requires moderator or admin"));
    }
    if !scope_allows_publish(&auth.token.scope) {
        return Err(ApiError::forbidden("this token has read-only scope"));
    }
    if !VALID_RESOLUTIONS.contains(&req.status.as_str()) {
        return Err(ApiError::bad_request(format!(
            "invalid status `{}` — must be one of: {}",
            req.status,
            VALID_RESOLUTIONS.join(", ")
        )));
    }
    if !state
        .db
        .resolve_report(id, auth.user.id, &req.status, &req.note)
        .await?
    {
        return Err(ApiError::not_found(format!("report #{id} not found")));
    }
    state.db.audit(
        Some(auth.user.id),
        &format!("report-{}", req.status),
        None,
        None,
        None,
    );
    Ok(Json(json!({ "ok": true })))
}
