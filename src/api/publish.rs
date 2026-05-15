//! PUT /api/v1/packages
//!
//! Body format (matches cargo's publish wire format):
//!   [u32 LE: JSON metadata length]
//!   [JSON metadata bytes]
//!   [u32 LE: tarball length]
//!   [tarball bytes]

use std::sync::Arc;

use axum::{body::Bytes, extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{auth::AuthToken, AppState};
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
struct PublishMeta {
    name: String,
    vers: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    license: Option<String>,
}

pub async fn publish(
    State(state): State<Arc<AppState>>,
    _auth: AuthToken,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    let (meta, tarball) =
        parse_body(&body).map_err(|e| ApiError::bad_request(e.to_string()))?;

    if meta.name.is_empty() || meta.vers.is_empty() {
        return Err(ApiError::bad_request("`name` and `vers` are required"));
    }

    // Reject duplicate versions up-front (the UNIQUE constraint catches races).
    if let Some((_, versions)) = state.db.get_package(&meta.name).await? {
        if versions.iter().any(|v| v.version == meta.vers) {
            return Err(ApiError::conflict(format!(
                "`{}@{}` already exists",
                meta.name, meta.vers
            )));
        }
    }

    let checksum = hex::encode(Sha256::digest(tarball));

    state
        .storage
        .save(&meta.name, &meta.vers, tarball)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state
        .db
        .publish_version(&meta.name, meta.description.as_deref(), &meta.vers, &checksum)
        .await?;

    tracing::info!("published {}@{}", meta.name, meta.vers);

    Ok(Json(json!({
        "ok": true,
        "warnings": { "invalid_categories": [], "invalid_badges": [], "other": [] }
    })))
}

fn parse_body(data: &[u8]) -> anyhow::Result<(PublishMeta, &[u8])> {
    if data.len() < 4 {
        anyhow::bail!("request body too short");
    }
    let json_len = u32::from_le_bytes(data[..4].try_into().unwrap()) as usize;
    let json_end = 4 + json_len;
    if data.len() < json_end + 4 {
        anyhow::bail!("request body truncated before tarball");
    }
    let meta: PublishMeta = serde_json::from_slice(&data[4..json_end])
        .map_err(|e| anyhow::anyhow!("invalid metadata JSON: {e}"))?;
    let tar_len = u32::from_le_bytes(data[json_end..json_end + 4].try_into().unwrap()) as usize;
    let tar_start = json_end + 4;
    if data.len() < tar_start + tar_len {
        anyhow::bail!("request body truncated in tarball data");
    }
    Ok((meta, &data[tar_start..tar_start + tar_len]))
}
