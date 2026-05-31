//! GET /api/v1/me/packages — packages owned by the authenticated user

use std::sync::Arc;

use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::{auth::AuthToken, AppState};
use super::ApiResult;

pub async fn my_packages(
    auth: AuthToken,
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<Value>> {
    let packages = state.db.get_packages_by_owner(auth.user.id).await?;
    let out: Vec<_> = packages.iter().map(|p| json!({
        "name":        p.name,
        "channel":     p.channel,
        "description": p.description,
        "license":     p.license,
        "version":     p.latest_version,
    })).collect();
    Ok(Json(json!({ "packages": out })))
}
