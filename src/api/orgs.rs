use std::sync::Arc;

use axum::{
    extract::{Path, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::AppState;
use super::{ApiError, ApiResult};
use crate::auth::AuthToken;
use crate::validate;

#[derive(Deserialize)]
pub struct CreateOrgBody {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct MemberBody {
    pub username: String,
    #[serde(default = "default_role")]
    pub role: String,
}

fn default_role() -> String { "member".to_string() }

pub async fn list_orgs(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<Value>> {
    let orgs = state.db.list_orgs().await?;
    let list: Vec<Value> = orgs.iter().map(|o| json!({
        "id":          o.id,
        "name":        o.name,
        "description": o.description,
    })).collect();
    Ok(Json(json!({ "orgs": list })))
}

pub async fn create_org(
    State(state): State<Arc<AppState>>,
    auth: AuthToken,
    Json(body): Json<CreateOrgBody>,
) -> ApiResult<Json<Value>> {
    validate::package_name(&body.name)?;

    if let Some(_) = state.db.get_org(&body.name).await? {
        return Err(ApiError::conflict(format!("org `{}` already exists", body.name)));
    }

    let org_id = state.db.create_org(&body.name, body.description.as_deref(), auth.user.id).await?;
    Ok(Json(json!({ "id": org_id, "name": body.name })))
}

pub async fn get_org(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    let org = state
        .db
        .get_org(&name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("org `{name}` not found")))?;

    let members = state.db.list_org_members(&name).await?;
    let members_json: Vec<Value> = members
        .iter()
        .map(|m| json!({ "username": m.username, "role": m.role }))
        .collect();

    Ok(Json(json!({
        "id":          org.id,
        "name":        org.name,
        "description": org.description,
        "members":     members_json,
    })))
}

pub async fn delete_org(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    auth: AuthToken,
) -> ApiResult<Json<Value>> {
    let is_owner = auth.user.is_admin != 0
        || state.db.is_org_owner(&name, auth.user.id).await?;
    if !is_owner {
        return Err(ApiError::forbidden("only org owners can delete an org"));
    }

    if state.db.delete_org(&name).await? {
        Ok(Json(json!({ "deleted": true })))
    } else {
        Err(ApiError::not_found(format!("org `{name}` not found")))
    }
}

pub async fn list_members(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    let _ = state.db.get_org(&name).await?
        .ok_or_else(|| ApiError::not_found(format!("org `{name}` not found")))?;

    let members = state.db.list_org_members(&name).await?;
    let list: Vec<Value> = members
        .iter()
        .map(|m| json!({ "username": m.username, "role": m.role }))
        .collect();

    Ok(Json(json!({ "members": list })))
}

pub async fn add_member(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    auth: AuthToken,
    Json(body): Json<MemberBody>,
) -> ApiResult<Json<Value>> {
    let is_owner = auth.user.is_admin != 0
        || state.db.is_org_owner(&name, auth.user.id).await?;
    if !is_owner {
        return Err(ApiError::forbidden("only org owners can manage members"));
    }

    if body.role != "owner" && body.role != "member" {
        return Err(ApiError::bad_request("role must be 'owner' or 'member'"));
    }

    if !state.db.add_org_member(&name, &body.username, &body.role).await? {
        return Err(ApiError::not_found(format!("org or user not found")));
    }

    Ok(Json(json!({ "added": body.username, "role": body.role })))
}

pub async fn remove_member(
    State(state): State<Arc<AppState>>,
    Path((name, username)): Path<(String, String)>,
    auth: AuthToken,
) -> ApiResult<Json<Value>> {
    let is_owner = auth.user.is_admin != 0
        || state.db.is_org_owner(&name, auth.user.id).await?;
    // Allow members to remove themselves.
    let is_self = auth.user.username.to_lowercase() == username.to_lowercase();

    if !is_owner && !is_self {
        return Err(ApiError::forbidden("only org owners can remove members"));
    }

    if !state.db.remove_org_member(&name, &username).await? {
        return Err(ApiError::not_found("member not found in org"));
    }

    Ok(Json(json!({ "removed": username })))
}

pub async fn set_package_org(
    State(state): State<Arc<AppState>>,
    Path((pkg_name, channel)): Path<(String, String)>,
    auth: AuthToken,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<Value>> {
    let org_name = body.get("org").and_then(|v| v.as_str());

    // Must own the package.
    if auth.user.is_admin == 0 {
        let owns = state.db.user_owns_package(auth.user.id, &pkg_name, &channel).await?;
        if owns != Some(true) {
            return Err(ApiError::forbidden("you do not own this package"));
        }
    }

    // Must be a member of the target org (if setting one).
    if let Some(org) = org_name {
        if auth.user.is_admin == 0 && !state.db.is_org_member(org, auth.user.id).await? {
            return Err(ApiError::forbidden("you are not a member of that org"));
        }
    }

    if !state.db.set_package_org(&pkg_name, &channel, org_name).await? {
        return Err(ApiError::not_found("package or org not found"));
    }

    Ok(Json(json!({ "org": org_name })))
}
