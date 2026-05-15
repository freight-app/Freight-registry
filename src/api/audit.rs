//! GET /api/v1/audit — structured audit log (admin only).
//!
//! Query params:
//!   user   — filter by username
//!   action — filter by action string (login, publish, yank, unyank, register)
//!   since  — Unix timestamp lower bound
//!   until  — Unix timestamp upper bound
//!   limit  — max rows to return (default 100, max 500)

use std::sync::Arc;

use axum::{extract::{Query, State}, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::AdminToken, AppState};
use super::ApiResult;

#[derive(Deserialize)]
pub struct AuditQuery {
    user:   Option<String>,
    action: Option<String>,
    since:  Option<i64>,
    until:  Option<i64>,
    #[serde(default = "default_limit")]
    limit:  i64,
}

fn default_limit() -> i64 { 100 }

pub async fn list_audit(
    _auth: AdminToken,
    State(state): State<Arc<AppState>>,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<Value>> {
    let rows = state
        .db
        .list_audit_log(
            q.user.as_deref(),
            q.action.as_deref(),
            q.since,
            q.until,
            q.limit,
        )
        .await?;

    let entries: Vec<Value> = rows
        .iter()
        .map(|r| json!({
            "id":         r.id,
            "user_id":    r.user_id,
            "username":   r.username,
            "action":     r.action,
            "package":    r.package,
            "version":    r.version,
            "ip_addr":    r.ip_addr,
            "created_at": r.created_at,
        }))
        .collect();

    Ok(Json(json!({ "entries": entries, "count": entries.len() })))
}
