use std::sync::Arc;

use argon2::{password_hash::{rand_core::OsRng, SaltString}, Argon2, PasswordHasher};
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

/// Hash a plaintext password with Argon2id. Returns the PHC string.
pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("password hashing failed: {e}"))?
        .to_string();
    Ok(hash)
}

/// Extractor for a valid, non-expired `Authorization: Bearer <token>`.
/// Rejects tokens with `kind = "refresh"` — those may only be used at `/auth/refresh`.
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

        if token.kind == "refresh" {
            return Err(err(
                StatusCode::UNAUTHORIZED,
                "refresh tokens cannot be used for API auth — use POST /api/v1/auth/refresh",
            ));
        }

        Ok(AuthToken { user, token })
    }
}

/// Extractor that additionally requires `is_admin = 1` on the authenticated user.
pub struct AdminToken {
    #[allow(dead_code)]
    pub user:  UserRow,
    #[allow(dead_code)]
    pub token: TokenRow,
}

#[async_trait]
impl FromRequestParts<Arc<AppState>> for AdminToken {
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let auth = AuthToken::from_request_parts(parts, state).await?;
        if auth.user.is_admin == 0 {
            return Err(err(StatusCode::FORBIDDEN, "admin access required"));
        }
        Ok(AdminToken { user: auth.user, token: auth.token })
    }
}

/// Extractor for refresh-token auth. Accepts only tokens with `kind = "refresh"`.
/// Used exclusively by `POST /api/v1/auth/refresh`.
pub struct RefreshTokenAuth {
    pub user:  UserRow,
    #[allow(dead_code)]
    pub token: TokenRow,
}

#[async_trait]
impl FromRequestParts<Arc<AppState>> for RefreshTokenAuth {
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let raw = bearer_token(parts)
            .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing bearer token"))?;

        let (token, user) = state
            .db
            .validate_token(&raw)
            .await
            .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))?
            .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "invalid or expired token"))?;

        if token.kind != "refresh" {
            return Err(err(StatusCode::UNAUTHORIZED, "expected a refresh token"));
        }

        Ok(RefreshTokenAuth { user, token })
    }
}

pub fn bearer_token(parts: &Parts) -> Option<String> {
    let auth = parts.headers.get("Authorization")?.to_str().ok()?;
    Some(auth.strip_prefix("Bearer ")?.to_string())
}

fn err(status: StatusCode, detail: &str) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(json!({ "errors": [{ "detail": detail }] })))
}
