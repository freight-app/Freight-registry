use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    Json,
};
use serde_json::json;

use crate::{db::TokenRow, AppState};

/// Extractor that requires a valid `Authorization: Bearer <token>` header.
/// Rejects the request with 401 if the token is absent or invalid.
pub struct AuthToken(pub TokenRow);

#[async_trait]
impl FromRequestParts<Arc<AppState>> for AuthToken {
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts).ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"errors": [{"detail": "missing API token"}]})),
            )
        })?;

        let row = state
            .db
            .validate_token(&token)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"errors": [{"detail": "internal error"}]})),
                )
            })?
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"errors": [{"detail": "invalid API token"}]})),
                )
            })?;

        Ok(AuthToken(row))
    }
}

pub fn bearer_token(parts: &Parts) -> Option<String> {
    let auth = parts.headers.get("Authorization")?.to_str().ok()?;
    Some(auth.strip_prefix("Bearer ")?.to_string())
}
