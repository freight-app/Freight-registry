use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    Json,
};
use serde_json::json;

use crate::{
    db::{TokenRow, UserRow},
    AppState,
};

/// Extractor that requires a valid, non-expired `Authorization: Bearer <token>`.
pub struct AuthToken {
    pub user:  UserRow,
    #[allow(dead_code)]
    pub token: TokenRow,
}

#[async_trait]
impl FromRequestParts<Arc<AppState>> for AuthToken {
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let raw = bearer_token(parts)
            .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing API token"))?;

        let (token, user) = state
            .db
            .validate_token(&raw)
            .await
            .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?
            .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "invalid or expired API token"))?;

        Ok(AuthToken { user, token })
    }
}

pub fn bearer_token(parts: &Parts) -> Option<String> {
    let auth = parts.headers.get("Authorization")?.to_str().ok()?;
    Some(auth.strip_prefix("Bearer ")?.to_string())
}

fn err(status: StatusCode, detail: &str) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(json!({ "errors": [{ "detail": detail }] })))
}
