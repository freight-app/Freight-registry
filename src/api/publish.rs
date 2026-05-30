//! PUT /api/v1/packages
//!
//! Body format (matches cargo's publish wire format):
//!   [u32 LE: JSON metadata length]
//!   [JSON metadata bytes]
//!   [u32 LE: tarball length]
//!   [tarball bytes]

use std::collections::HashMap;
use std::sync::Arc;

use axum::{body::Bytes, extract::{ConnectInfo, State}, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;

use crate::{auth::PublishToken, db::DEFAULT_CHANNEL, validate, AppState};
use super::{ApiError, ApiResult};

#[derive(Deserialize)]
struct PublishMeta {
    name: String,
    vers: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    license: Option<String>,
    /// Channel to publish to (default: "stable").
    #[serde(default)]
    channel: Option<String>,
    /// Upstream source archive URL for "metadata-only" packages.
    /// When set the server does not store a tarball; `/download` issues a 302 redirect.
    #[serde(default)]
    upstream_url: Option<String>,
    /// Foreign build system required to compile this package ("cmake", "make", "meson", …).
    #[serde(default)]
    build_system: Option<String>,
    /// Platform support expression (e.g. "!uwp & !arm"). Uses freight boolean syntax.
    #[serde(default)]
    supports: Option<String>,
    /// Pre-computed dependency map for metadata-only packages (name → version constraint).
    #[serde(default)]
    deps: Option<serde_json::Value>,
}

pub async fn publish(
    State(state): State<Arc<AppState>>,
    auth: PublishToken,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    if state.limiters.write.check_key(&addr.ip()).is_err() {
        return Err(ApiError::too_many_requests());
    }

    let (meta, tarball) =
        parse_body(&body).map_err(|e| ApiError::bad_request(e.to_string()))?;

    validate::package_name(&meta.name)?;
    validate::version(&meta.vers)?;

    let channel = meta.channel.as_deref().unwrap_or(DEFAULT_CHANNEL);
    validate::channel_name(channel)?;

    match state.db.user_owns_package(auth.user.id, &meta.name, channel).await? {
        None => {
            // Package doesn't exist yet — this will be a new package.
            // Enforce the per-user limit for non-admins.
            if auth.user.is_admin == 0 {
                if let Some(limit) = state.max_packages_per_user {
                    let owned = state.db.count_owned_packages(auth.user.id).await?;
                    if owned >= limit as i64 {
                        return Err(ApiError::forbidden(format!(
                            "package limit reached: you own {owned} package(s) \
                             (max {limit} per user)"
                        )));
                    }
                }
            }
        }
        Some(true) => {}
        Some(false) => {
            return Err(ApiError::forbidden(format!(
                "you are not an owner of `{}` in channel `{channel}`", meta.name
            )));
        }
    }

    if let Some((_, versions)) = state.db.get_package(&meta.name, channel).await? {
        if versions.iter().any(|v| v.version == meta.vers) {
            return Err(ApiError::conflict(format!(
                "`{}@{}` already exists in channel `{channel}`",
                meta.name, meta.vers
            )));
        }
    }

    let is_metadata_only = meta.upstream_url.is_some();

    // For metadata-only packages (upstream_url set) the client sends an empty tarball.
    // For regular packages we require a valid gzip archive.
    if !is_metadata_only && (tarball.len() < 2 || tarball[0] != 0x1f || tarball[1] != 0x8b) {
        return Err(ApiError::bad_request("tarball is not a valid gzip archive"));
    }

    let checksum = if is_metadata_only {
        String::new()
    } else {
        hex::encode(Sha256::digest(tarball))
    };

    let dependencies = if is_metadata_only {
        meta.deps
            .as_ref()
            .and_then(|d| serde_json::to_string(d).ok())
            .unwrap_or_else(|| "{}".to_string())
    } else {
        extract_dependencies(tarball)
    };
    let keywords = if is_metadata_only {
        None
    } else {
        extract_keywords(tarball)
    };
    let readme = if is_metadata_only {
        None
    } else {
        extract_file(tarball, "README.md")
            .or_else(|| extract_file(tarball, "readme.md"))
            .or_else(|| extract_file(tarball, "README"))
    };

    if !is_metadata_only {
        state
            .storage
            .save(&meta.name, &meta.vers, tarball)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }

    if let Some(ref content) = readme {
        let _ = state.storage.save_readme(&meta.name, &meta.vers, content.as_bytes()).await;
    }

    state
        .db
        .publish_version(
            auth.user.id,
            &meta.name,
            channel,
            meta.description.as_deref(),
            meta.license.as_deref(),
            keywords.as_deref(),
            &meta.vers,
            &checksum,
            &dependencies,
            meta.upstream_url.as_deref(),
            meta.build_system.as_deref(),
            meta.supports.as_deref(),
        )
        .await?;

    state.metrics.publishes_total.inc();
    let ip = addr.ip().to_string();
    state.db.audit(Some(auth.user.id), "publish", Some(&meta.name), Some(&meta.vers), Some(&ip));
    tracing::info!(user = %auth.user.username, channel, "published {}@{}", meta.name, meta.vers);

    Ok(Json(json!({
        "ok": true,
        "warnings": { "invalid_categories": [], "invalid_badges": [], "other": [] }
    })))
}

/// Extract `[dependencies]` from `freight.toml` inside the tarball.
/// Returns a JSON object string, e.g. `{"zlib":"*","openssl":"*"}`.
/// Returns `"{}"` on any error (missing file, parse failure, etc.).
fn extract_dependencies(tarball: &[u8]) -> String {
    let deps = extract_dependencies_inner(tarball).unwrap_or_default();
    serde_json::to_string(&deps).unwrap_or_else(|_| "{}".to_string())
}

/// Extract a named file from a `.tar.gz` by filename (basename match only).
/// Returns `None` if the file is not present or cannot be read as UTF-8.
pub fn extract_file(tarball: &[u8], filename: &str) -> Option<String> {
    use flate2::read::GzDecoder;
    use tar::Archive;
    let gz = GzDecoder::new(tarball);
    let mut ar = Archive::new(gz);
    for entry in ar.entries().ok()? {
        // Skip unreadable entries rather than aborting the whole search.
        let mut entry = match entry { Ok(e) => e, Err(_) => continue };
        let path = match entry.path() { Ok(p) => p, Err(_) => continue };
        // Directories (e.g. the leading `.`) have no file_name(); skip them.
        let name = match path.file_name() {
            Some(n) => n.to_string_lossy().into_owned(),
            None => continue,
        };
        if name.eq_ignore_ascii_case(filename) {
            let mut content = String::new();
            return std::io::Read::read_to_string(&mut entry, &mut content)
                .ok()
                .map(|_| content);
        }
    }
    None
}

fn extract_dependencies_inner(tarball: &[u8]) -> Option<HashMap<String, String>> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let gz = GzDecoder::new(tarball);
    let mut ar = Archive::new(gz);

    for entry in ar.entries().ok()? {
        let mut entry = match entry { Ok(e) => e, Err(_) => continue };
        let path = match entry.path() { Ok(p) => p, Err(_) => continue };
        let file_name = match path.file_name() {
            Some(n) => n.to_string_lossy().into_owned(),
            None => continue,
        };
        if file_name != "freight.toml" { continue; }

        let mut content = String::new();
        if std::io::Read::read_to_string(&mut entry, &mut content).is_err() { continue; }

        #[derive(serde::Deserialize)]
        struct Manifest {
            #[serde(default)]
            dependencies: HashMap<String, toml::Value>,
        }
        let manifest: Manifest = toml::from_str(&content).ok()?;
        let deps = manifest.dependencies.into_iter()
            .filter_map(|(k, v)| {
                let ver = match v {
                    toml::Value::String(s) => s,
                    toml::Value::Table(t) => t.get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("*")
                        .to_string(),
                    _ => "*".to_string(),
                };
                Some((k, ver))
            })
            .collect();
        return Some(deps);
    }
    None
}

/// Extract `package.keywords` from `freight.toml` in the tarball.
/// Returns a comma-separated string or `None` if not present / empty.
fn extract_keywords(tarball: &[u8]) -> Option<String> {
    let content = extract_file(tarball, "freight.toml")?;

    #[derive(serde::Deserialize)]
    struct Manifest {
        package: Option<PackageMeta>,
    }
    #[derive(serde::Deserialize)]
    struct PackageMeta {
        #[serde(default)]
        keywords: Vec<String>,
    }

    let manifest: Manifest = toml::from_str(&content).ok()?;
    let kws = manifest.package?.keywords;
    let kws: Vec<String> = kws.iter()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect();
    if kws.is_empty() { None } else { Some(kws.join(",")) }
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
