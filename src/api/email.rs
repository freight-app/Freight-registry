//! GET /api/v1/users/verify-email?token=<token>
//!
//! Marks the user's email as verified. The token is issued at registration (if
//! an email address was provided) and logged to stdout via `tracing::warn!`.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::AppState;
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
pub struct VerifyQuery {
    token: String,
}

pub async fn verify_email(
    State(state): State<Arc<AppState>>,
    Query(params): Query<VerifyQuery>,
) -> ApiResult<Json<Value>> {
    let user_id = state
        .db
        .consume_email_token(&params.token, "verify")
        .await?
        .ok_or_else(|| ApiError::bad_request("invalid or expired verification token"))?;

    state.db.set_email_verified(user_id).await?;
    tracing::info!(user_id, "email verified");

    Ok(Json(json!({ "ok": true, "message": "email verified" })))
}
