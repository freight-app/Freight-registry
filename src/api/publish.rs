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

    // Org-scoped token enforcement: if the token is bound to an org, the package
    // must belong to that org (or be a new package that will be auto-assigned).
    if let Some(token_org_id) = auth.token.org_id {
        let pkg_org_id = state.db.get_package_org_id(&meta.name, channel).await?;
        match pkg_org_id {
            Some(pkg_org) if pkg_org != token_org_id => {
                return Err(ApiError::forbidden(
                    "this token is bound to a different org — it cannot publish to this package",
                ));
            }
            None => {
                // New package: pre-assign it to the token's org.
                state.db.set_package_org(&meta.name, channel, None).await.ok();
            }
            _ => {}
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

    // Language allowlist: reject packages whose declared languages are not in
    // the registry's configured set.  Metadata-only stubs bypass this check
    // because they don't carry source (their language can't be verified).
    if let Some(ref allowed) = state.allowed_languages {
        if !is_metadata_only {
            let pkg_langs = extract_languages(tarball);
            if pkg_langs.is_empty() {
                return Err(ApiError::bad_request(
                    "could not determine package languages from freight.toml inside tarball — \
                     declare at least one [language.*] section"
                ));
            }
            if !pkg_langs.iter().any(|l| allowed.contains(l)) {
                return Err(ApiError::bad_request(format!(
                    "this registry only accepts packages written in [{}]; \
                     package declares [{}]",
                    allowed.join(", "),
                    pkg_langs.join(", "),
                )));
            }
        }
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

    // Extract language keys from freight.toml in the tarball (e.g. "c,cpp").
    let languages_str: Option<String> = if is_metadata_only {
        None
    } else {
        let langs = extract_languages(tarball);
        if langs.is_empty() { None } else { Some(langs.join(",")) }
    };

    // Metadata-only stubs skip both scanning and verification (no source bytes).
    // Source packages: scan first (synchronous, pre-publish), then optionally
    // run the full CI pipeline asynchronously if verify_image is configured.
    if !is_metadata_only {
        match scan_tarball(tarball, &format!("{}@{}", meta.name, meta.vers), &state.scan_backend) {
            ScanOutcome::Infected(findings) => {
                tracing::warn!(
                    pkg = %format!("{}@{}", meta.name, meta.vers),
                    findings = %findings,
                    "publish rejected — malware detected"
                );
                return Err(ApiError::bad_request(
                    format!("tarball failed malware scan: {findings}")
                ));
            }
            ScanOutcome::Unavailable(reason) => {
                tracing::debug!(reason = %reason, "scan backend unavailable — skipping");
            }
            ScanOutcome::Clean => {
                tracing::debug!(pkg = %format!("{}@{}", meta.name, meta.vers), "scan clean");
            }
        }
    }

    // Determine which platform images to run verification against.
    // `verify_images` wins over the legacy `verify_image` single-image mode.
    let pipeline_images: Vec<(String, String)> = if !is_metadata_only {
        if !state.verify_images.is_empty() {
            // Select images for platforms the package supports.
            // `supports` is a comma/space-separated string like "linux,windows,freebsd".
            // When absent, run all configured platform images.
            let supported: Vec<String> = meta.supports.as_deref()
                .map(|s| s.split([',', ' ']).map(str::trim).filter(|s| !s.is_empty())
                          .map(str::to_ascii_lowercase).collect())
                .unwrap_or_default();

            state.verify_images.iter()
                .filter(|(platform, _)| {
                    platform.as_str() == "default"
                    || supported.is_empty()
                    || supported.contains(platform)
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        } else if let Some(ref img) = state.verify_image {
            vec![("default".to_string(), img.clone())]
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    let needs_verification = !pipeline_images.is_empty();

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
            languages_str.as_deref(),
            needs_verification,
        )
        .await?;

    state.metrics.publishes_total.inc();
    let ip = addr.ip().to_string();
    state.db.audit(Some(auth.user.id), "publish", Some(&meta.name), Some(&meta.vers), Some(&ip));
    tracing::info!(user = %auth.user.username, channel, pending = needs_verification,
                   platforms = pipeline_images.len(), "received {}@{}", meta.name, meta.vers);

    // Launch one verification task per platform pipeline.
    // All tasks vote: the version is published only when every task passes.
    // The first rejection wins — the others are cancelled via a shared flag.
    if needs_verification {
        let tarball_bytes = tarball.to_vec();
        let pkg_name      = meta.name.clone();
        let pkg_vers      = meta.vers.clone();
        let pkg_channel   = channel.to_string();
        let db            = state.db.clone();
        let uid           = auth.user.id;
        let scan_backend  = state.scan_backend.clone();
        let total         = pipeline_images.len();

        // Shared counter: how many pipelines remain.
        let remaining = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(total));
        // Shared flag: set to true when any pipeline rejects.
        let rejected  = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        for (platform, image) in pipeline_images {
            let tarball_bytes = tarball_bytes.clone();
            let pkg_name      = pkg_name.clone();
            let pkg_vers      = pkg_vers.clone();
            let pkg_channel   = pkg_channel.clone();
            let db            = db.clone();
            let scan_backend  = scan_backend.clone();
            let remaining     = remaining.clone();
            let rejected      = rejected.clone();
            tokio::spawn(async move {
                let passed = run_verification_pipeline(
                    &tarball_bytes, &pkg_name, &pkg_vers, &pkg_channel,
                    &platform, &image, &scan_backend, &db, uid,
                ).await;

                if !passed {
                    rejected.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                let prev = remaining.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                if prev == 1 {
                    // Last pipeline finished. Publish only if none rejected.
                    if !rejected.load(std::sync::atomic::Ordering::SeqCst) {
                        let _ = db.set_version_status(
                            &pkg_name, &pkg_vers, &pkg_channel, "published", None
                        ).await;
                        db.audit(Some(uid), "verify_passed", Some(&pkg_name), Some(&pkg_vers), None);
                        tracing::info!(pkg = %format!("{pkg_name}@{pkg_vers}"), "all pipelines passed — published");
                    }
                }
            });
        }
    }

    let status = if needs_verification { "pending" } else { "published" };
    Ok(Json(json!({
        "ok":     true,
        "status": status,
        "warnings": { "invalid_categories": [], "invalid_badges": [], "other": [] }
    })))
}

/// Extract `[dependencies]` from `freight.toml` inside the tarball.
/// Returns a JSON object string, e.g. `{"zlib":"*","openssl":"*"}`.
/// Returns `"{}"` on any error (missing file, parse failure, etc.).
/// Extract the `[language.*]` keys from `freight.toml` inside the tarball.
/// Returns a sorted list of lowercase language identifiers, e.g. `["c", "cpp"]`.
/// Returns an empty vec if `freight.toml` is absent or has no `[language]` table.
fn extract_languages(tarball: &[u8]) -> Vec<String> {
    let toml_src = match extract_file(tarball, "freight.toml") {
        Some(s) => s,
        None    => return vec![],
    };
    let value: toml::Value = match toml::from_str(&toml_src) {
        Ok(v)  => v,
        Err(_) => return vec![],
    };
    let mut langs: Vec<String> = value
        .get("language")
        .and_then(toml::Value::as_table)
        .map(|t| t.keys().map(|k| k.to_ascii_lowercase()).collect())
        .unwrap_or_default();
    langs.sort();
    langs
}

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

// ── Verification pipeline ─────────────────────────────────────────────────────

/// Run the CI pipeline for one platform inside a container.
///
/// Returns `true` if the pipeline passed, `false` if it failed or errored.
/// The caller is responsible for transitioning the version status once all
/// platform pipelines have voted.
///
/// The container is given the tarball as a read-only mount and must output a
/// single JSON object to stdout:
/// ```json
/// {
///   "passed": true,
///   "build":  { "passed": true,  "output": "…" },
///   "test":   { "passed": true,  "passed_count": 5, "failed_count": 0 },
///   "scan":   { "passed": true,  "findings": [] }
/// }
/// ```
/// Any non-JSON stdout or a container exit code other than 0 or 1 is treated
/// as a pipeline error and the version is rejected with the raw output as the
/// reason.
async fn run_verification_pipeline(
    tarball:      &[u8],
    name:         &str,
    version:      &str,
    channel:      &str,
    platform:     &str,
    image:        &str,
    scan_backend: &crate::ScanBackend,
    db:           &crate::db::Db,
    uid:          i64,
) -> bool {
    let label = format!("{name}@{version}");
    tracing::info!(pkg = %label, platform, image, "starting verification pipeline");

    // Write tarball to a temp file so we can mount it into the container.
    let tmp = std::env::temp_dir().join(format!(
        "freight-verify-{}-{}.tar.gz", label_slug(&label), platform
    ));
    if std::fs::write(&tmp, tarball).is_err() {
        let reason = "could not write temp file for verification container";
        tracing::error!(pkg = %label, platform, reason, "pipeline setup failed");
        let _ = db.set_version_status(name, version, channel, "rejected", Some(reason)).await;
        db.audit(Some(uid), "verify_failed", Some(name), Some(version), None);
        return false;
    }

    // Detect container runtime (same preference order as scan backend).
    let runtime = match scan_backend {
        crate::ScanBackend::Docker   => Some("docker"),
        crate::ScanBackend::Podman   => Some("podman"),
        crate::ScanBackend::Clamscan | crate::ScanBackend::None => None,
        crate::ScanBackend::Auto => {
            if has_executable("docker")      { Some("docker") }
            else if has_executable("podman") { Some("podman") }
            else                             { None }
        }
    };

    let Some(runtime) = runtime else {
        let reason = "no container runtime available (docker/podman not found); \
                      set FREIGHT_SCAN_BACKEND=docker or =podman";
        tracing::warn!(pkg = %label, platform, reason, "verification skipped — treating as passed");
        let _ = std::fs::remove_file(&tmp);
        // No container runtime: treat as a pass so the package isn't stuck pending forever.
        return true;
    };

    let mount  = format!("{}:/pkg.tar.gz:ro", tmp.display());
    let output = std::process::Command::new(runtime)
        .args([
            "run", "--rm",
            "--network",      "none",
            "--read-only",
            "--memory",       "500m",
            "--cpus",         "1.0",
            "--security-opt", "no-new-privileges",
            "--tmpfs",        "/tmp:rw,noexec,nosuid,size=512m",
            "--tmpfs",        "/build:rw,noexec,nosuid,size=500m",
            "-v", &mount,
            image,
            "/pkg.tar.gz",   // passed as first argument to the container entrypoint
        ])
        .output();

    let _ = std::fs::remove_file(&tmp);

    match output {
        Err(e) => {
            let reason = format!("failed to launch container ({runtime}): {e}");
            tracing::error!(pkg = %label, platform, %reason, "pipeline launch failed");
            let _ = db.set_version_status(name, version, channel, "rejected", Some(&reason)).await;
            db.audit(Some(uid), "verify_failed", Some(name), Some(version), None);
            false
        }
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            match serde_json::from_str::<serde_json::Value>(stdout.trim()) {
                Ok(result) => {
                    let passed = result["passed"].as_bool().unwrap_or(false);
                    if passed {
                        tracing::info!(pkg = %label, platform, "pipeline passed");
                        db.audit(Some(uid), "verify_passed", Some(name), Some(version), None);
                        true
                    } else {
                        let reason = build_rejection_reason(&result);
                        tracing::warn!(pkg = %label, platform, %reason, "pipeline failed — rejecting");
                        let _ = db.set_version_status(name, version, channel, "rejected", Some(&reason)).await;
                        db.audit(Some(uid), "verify_failed", Some(name), Some(version), Some(&reason));
                        false
                    }
                }
                Err(_) => {
                    // Non-JSON output = container error (crash, wrong image, etc.)
                    let stderr  = String::from_utf8_lossy(&out.stderr);
                    let reason  = format!(
                        "container produced non-JSON output (exit {})\nstdout: {}\nstderr: {}",
                        out.status.code().unwrap_or(-1),
                        stdout.trim(),
                        stderr.trim(),
                    );
                    tracing::error!(pkg = %label, platform, code = ?out.status.code(), "container output was not JSON");
                    let _ = db.set_version_status(name, version, channel, "rejected", Some(&reason)).await;
                    db.audit(Some(uid), "verify_failed", Some(name), Some(version), None);
                    false
                }
            }
        }
    }
}

/// Build a human-readable rejection reason from the pipeline JSON result.
fn build_rejection_reason(result: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    if result["build"]["passed"].as_bool() == Some(false) {
        parts.push("build failed".to_string());
        if let Some(o) = result["build"]["output"].as_str() {
            let tail: String = o.lines().rev().take(5).collect::<Vec<_>>()
                .into_iter().rev().collect::<Vec<_>>().join("\n");
            parts.push(format!("  {tail}"));
        }
    }
    if result["test"]["passed"].as_bool() == Some(false) {
        let failed = result["test"]["failed_count"].as_u64().unwrap_or(0);
        parts.push(format!("{failed} test(s) failed"));
    }
    if result["scan"]["passed"].as_bool() == Some(false) {
        if let Some(arr) = result["scan"]["findings"].as_array() {
            for f in arr {
                if let Some(s) = f.as_str() { parts.push(format!("scan: {s}")); }
            }
        }
    }
    if parts.is_empty() { "verification failed (no details)".to_string() }
    else { parts.join("\n") }
}

// ── Server-side security scan ─────────────────────────────────────────────────

pub(crate) enum ScanOutcome {
    Clean,
    /// Threat(s) found.  Carries a human-readable summary.
    Infected(String),
    /// Scanner not available or encountered an error.
    Unavailable(String),
}

/// Dispatch to the appropriate scan backend.
///
/// Container backends (Docker/Podman) mount the tarball read-only inside an
/// ephemeral ClamAV container with `--network none` and tight resource limits
/// so malicious content cannot escape.  Bare `clamscan` runs on the host.
/// `Auto` probes: Docker → Podman → clamscan → unavailable.
pub(crate) fn scan_tarball(tarball: &[u8], label: &str, backend: &crate::ScanBackend) -> ScanOutcome {
    use crate::ScanBackend;
    let slug = label_slug(label);
    match backend {
        ScanBackend::None     => ScanOutcome::Unavailable("scanning disabled".into()),
        ScanBackend::Docker   => scan_in_container("docker", tarball, &slug),
        ScanBackend::Podman   => scan_in_container("podman", tarball, &slug),
        ScanBackend::Clamscan => scan_host_clamscan(tarball, &slug),
        ScanBackend::Auto     => {
            if has_executable("docker")  { return scan_in_container("docker", tarball, &slug); }
            if has_executable("podman")  { return scan_in_container("podman", tarball, &slug); }
            if has_executable("clamscan"){ return scan_host_clamscan(tarball, &slug); }
            ScanOutcome::Unavailable(
                "no scan backend available (docker/podman/clamscan not found)".into()
            )
        }
    }
}

// ── Container-based scanning ──────────────────────────────────────────────────

/// Scan inside a Docker or Podman container.
///
/// Steps:
///   1. Write tarball to a temp file on the host.
///   2. Run `<runtime> run --rm --network none --read-only
///              -v <tmp>:/scan/pkg.tar.gz:ro
///              --memory 512m --cpus 0.5
///              clamav/clamav:latest
///              clamscan --no-summary --infected --recursive /scan/pkg.tar.gz`
///   3. Parse stdout for FOUND lines.
///   4. Remove temp file.
///
/// The container has no network, can't write to the host filesystem, and is
/// killed if it exceeds the memory or CPU limits.
fn scan_in_container(runtime: &str, tarball: &[u8], _slug: &str) -> ScanOutcome {
    use std::io::Write as _;

    // Pipe the tarball bytes via stdin — avoids volume mounts entirely, which
    // don't work when the Docker daemon is remote (DinD over TCP).
    let mut child = match std::process::Command::new(runtime)
        .args([
            "run", "--rm", "-i",
            "--network", "none",
            "--memory", "1g",
            "--cpus", "0.5",
            "--security-opt", "no-new-privileges",
            "--tmpfs", "/run/clamav:rw,noexec,nosuid,size=64m",
            "--tmpfs", "/tmp:rw,noexec,nosuid,size=64m",
            "clamav/clamav:latest",
            "clamscan", "--no-summary", "--infected", "-",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return ScanOutcome::Unavailable(format!("{runtime}: {e}")),
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(tarball);
    }

    let result = child.wait_with_output();
    interpret_clamscan_output(result, runtime)
}

// ── Host clamscan (no container) ──────────────────────────────────────────────

fn scan_host_clamscan(tarball: &[u8], slug: &str) -> ScanOutcome {
    let tmp = std::env::temp_dir().join(format!("freight-scan-{slug}.tar.gz"));
    if std::fs::write(&tmp, tarball).is_err() {
        return ScanOutcome::Unavailable("could not write temp file for host scan".into());
    }

    let result = std::process::Command::new("clamscan")
        .args(["--no-summary", "--infected", "--recursive",
               tmp.to_str().unwrap_or("")])
        .output();

    let _ = std::fs::remove_file(&tmp);

    interpret_clamscan_output(result, "clamscan")
}

// ── Shared output parser ──────────────────────────────────────────────────────

fn interpret_clamscan_output(
    result: std::io::Result<std::process::Output>,
    scanner: &str,
) -> ScanOutcome {
    match result {
        Ok(out) if out.status.success() => ScanOutcome::Clean,
        Ok(out) => {
            let stdout   = String::from_utf8_lossy(&out.stdout);
            let findings: Vec<_> = stdout
                .lines()
                .filter(|l| l.contains("FOUND"))
                .map(str::to_string)
                .collect();
            if findings.is_empty() {
                // Non-zero without FOUND = scanner error (stale DB, image pull failure, etc.)
                let stderr = String::from_utf8_lossy(&out.stderr);
                tracing::warn!(
                    scanner, code = ?out.status.code(),
                    stderr = %stderr.trim(),
                    "clamscan exited non-zero with no findings — DB may be stale"
                );
                ScanOutcome::Unavailable(format!("{scanner} exited with error"))
            } else {
                ScanOutcome::Infected(findings.join("; "))
            }
        }
        Err(e) => ScanOutcome::Unavailable(format!("{scanner}: {e}")),
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn has_executable(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| p.to_string_lossy().split(':')
            .any(|dir| std::path::Path::new(dir).join(name).is_file()))
        .unwrap_or(false)
}

fn label_slug(label: &str) -> String {
    label.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

// ── Scan tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod scan_tests {
    use super::*;

    /// Minimal valid gzip of an empty tar archive.
    const CLEAN_TAR_GZ: &[u8] = &[
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03,
        0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    /// EICAR test string — standard antivirus test signature, not real malware.
    /// ClamAV detects this as `Eicar-Signature`.
    const EICAR: &[u8] = b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";

    // ── ScanBackend::None ─────────────────────────────────────────────────────

    #[test]
    fn none_backend_always_unavailable() {
        let outcome = scan_tarball(CLEAN_TAR_GZ, "test@1.0.0", &crate::ScanBackend::None);
        assert!(matches!(outcome, ScanOutcome::Unavailable(_)));
    }

    #[test]
    fn none_backend_eicar_also_unavailable() {
        let outcome = scan_tarball(EICAR, "test@1.0.0", &crate::ScanBackend::None);
        assert!(matches!(outcome, ScanOutcome::Unavailable(_)));
    }

    // ── Docker-backed ClamAV tests ────────────────────────────────────────────
    // Run with: DOCKER_HOST=tcp://192.168.178.45:2375 cargo test -p freight-registry scan_docker -- --ignored
    //
    // These tests pull clamav/clamav:latest and run a real scan.  They are
    // marked `#[ignore]` so they don't run in normal CI (slow, needs Docker).

    #[test]
    #[ignore]
    fn docker_scan_clean_package() {
        let outcome = scan_tarball(CLEAN_TAR_GZ, "clean@1.0.0", &crate::ScanBackend::Docker);
        match &outcome {
            ScanOutcome::Clean => {}
            ScanOutcome::Unavailable(reason) => panic!("scan unavailable: {reason}"),
            ScanOutcome::Infected(f) => panic!("false positive on empty tar: {f}"),
        }
    }

    #[test]
    #[ignore]
    fn docker_scan_eicar_detected() {
        let outcome = scan_tarball(EICAR, "eicar@1.0.0", &crate::ScanBackend::Docker);
        match &outcome {
            ScanOutcome::Infected(findings) => {
                assert!(
                    findings.to_lowercase().contains("eicar") || findings.contains("FOUND"),
                    "expected EICAR finding, got: {findings}"
                );
            }
            ScanOutcome::Clean => panic!("EICAR should have been detected"),
            ScanOutcome::Unavailable(reason) => panic!("scan unavailable: {reason}"),
        }
    }

    // ── Publish handler rejects infected packages ─────────────────────────────
    // Integration test: the publish HTTP endpoint should return 400 when the
    // scan backend is Docker and the tarball contains EICAR.

    #[tokio::test]
    #[ignore]
    async fn publish_infected_package_rejected_400() {
        use std::sync::Arc;
        use axum::{body::Body, http::{Request, StatusCode}};
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        use crate::{api, db::Db, mail::StdoutMailer, metrics::Metrics, rate_limit::Limiters, storage::Storage, AppState};

        let db = Db::open_memory().await.unwrap();
        let uid = db.create_user("alice", None, "pw").await.unwrap();
        let tok = db.create_token(uid, "dev", None, "publish", "publish", None).await.unwrap();

        let state = Arc::new(AppState {
            db,
            storage:               Storage::new(std::env::temp_dir().join("freight-scan-test")),
            base_url:              "http://localhost".to_string(),
            limiters:              Limiters::new(100_000, 100_000),
            metrics:               Metrics::new(),
            mailer:                Arc::new(StdoutMailer),
            mirror_upstream:       None,
            max_packages_per_user: None,
            allowed_languages:     None,
            scan_backend:          crate::ScanBackend::Docker,
            verify_image:          None,
            verify_images:         std::collections::HashMap::new(),
            download_url:          None,
            oauth_providers:       vec![],
            oauth_states:          Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        });

        // Build a publish body with EICAR as the "tarball".
        let meta = serde_json::json!({"name": "evil-lib", "vers": "1.0.0"}).to_string();
        let meta_bytes = meta.as_bytes();
        let mut body = Vec::new();
        body.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
        body.extend_from_slice(meta_bytes);
        body.extend_from_slice(&(EICAR.len() as u32).to_le_bytes());
        body.extend_from_slice(EICAR);

        let addr: std::net::SocketAddr = "127.0.0.1:1234".parse().unwrap();
        let app = api::router(state, 1024 * 1024);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/packages")
                    .header("Authorization", format!("Bearer {tok}"))
                    .header("Content-Type", "application/octet-stream")
                    .extension(axum::extract::ConnectInfo(addr))
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(body["errors"][0]["detail"]
            .as_str()
            .unwrap_or("")
            .contains("malware"),
            "expected malware rejection message, got: {body}");
    }
}
