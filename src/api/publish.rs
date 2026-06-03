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

    // Metadata-only stubs are published immediately (no source to verify).
    // Source packages start as `pending` when a verify_image is configured,
    // or are published immediately when no pipeline is set up.
    let needs_verification = !is_metadata_only && state.verify_image.is_some();

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
                   "received {}@{}", meta.name, meta.vers);

    // Launch the verification pipeline in a background task.
    // Reads the container's JSON output and either publishes or rejects the version.
    if needs_verification {
        let tarball_bytes  = tarball.to_vec();
        let pkg_name       = meta.name.clone();
        let pkg_vers       = meta.vers.clone();
        let pkg_channel    = channel.to_string();
        let db             = state.db.clone();
        let uid            = auth.user.id;
        let image          = state.verify_image.clone().unwrap();
        let scan_backend   = state.scan_backend.clone();
        tokio::spawn(async move {
            run_verification_pipeline(
                &tarball_bytes, &pkg_name, &pkg_vers, &pkg_channel,
                &image, &scan_backend, &db, uid,
            ).await;
        });
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

/// Run the full CI pipeline inside a container, then update the version status.
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
    image:        &str,
    scan_backend: &crate::ScanBackend,
    db:           &crate::db::Db,
    uid:          i64,
) {
    let label = format!("{name}@{version}");
    tracing::info!(pkg = %label, image, "starting verification pipeline");

    // Write tarball to a temp file so we can mount it into the container.
    let tmp = std::env::temp_dir().join(format!("freight-verify-{}.tar.gz", label_slug(&label)));
    if std::fs::write(&tmp, tarball).is_err() {
        let reason = "could not write temp file for verification container";
        tracing::error!(pkg = %label, reason, "pipeline setup failed");
        let _ = db.set_version_status(name, version, channel, "rejected", Some(reason)).await;
        db.audit(Some(uid), "verify_failed", Some(name), Some(version), None);
        return;
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
        tracing::warn!(pkg = %label, reason, "verification skipped — publishing immediately");
        // No container runtime: publish without verification.
        let _ = db.set_version_status(name, version, channel, "published", None).await;
        let _ = std::fs::remove_file(&tmp);
        return;
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
            tracing::error!(pkg = %label, %reason, "pipeline launch failed");
            let _ = db.set_version_status(name, version, channel, "rejected", Some(&reason)).await;
            db.audit(Some(uid), "verify_failed", Some(name), Some(version), None);
        }
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            match serde_json::from_str::<serde_json::Value>(stdout.trim()) {
                Ok(result) => {
                    let passed = result["passed"].as_bool().unwrap_or(false);
                    if passed {
                        tracing::info!(pkg = %label, "verification passed — publishing");
                        let _ = db.set_version_status(name, version, channel, "published", None).await;
                        db.audit(Some(uid), "verify_passed", Some(name), Some(version), None);
                    } else {
                        let reason = build_rejection_reason(&result);
                        tracing::warn!(pkg = %label, %reason, "verification failed — rejecting");
                        let _ = db.set_version_status(name, version, channel, "rejected", Some(&reason)).await;
                        db.audit(Some(uid), "verify_failed", Some(name), Some(version), Some(&reason));
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
                    tracing::error!(pkg = %label, code = ?out.status.code(), "container output was not JSON");
                    let _ = db.set_version_status(name, version, channel, "rejected", Some(&reason)).await;
                    db.audit(Some(uid), "verify_failed", Some(name), Some(version), None);
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

enum ScanOutcome {
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
fn scan_tarball(tarball: &[u8], label: &str, backend: &crate::ScanBackend) -> ScanOutcome {
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
fn scan_in_container(runtime: &str, tarball: &[u8], slug: &str) -> ScanOutcome {
    let tmp = std::env::temp_dir().join(format!("freight-scan-{slug}.tar.gz"));
    if std::fs::write(&tmp, tarball).is_err() {
        return ScanOutcome::Unavailable("could not write temp file for container scan".into());
    }

    let mount = format!("{}:/scan/pkg.tar.gz:ro", tmp.display());
    let result = std::process::Command::new(runtime)
        .args([
            "run", "--rm",
            "--network", "none",
            "--read-only",
            "--memory", "512m",
            "--cpus", "0.5",
            "--security-opt", "no-new-privileges",
            "-v", &mount,
            "clamav/clamav:latest",
            "clamscan",
            "--no-summary",
            "--infected",
            "--recursive",
            "/scan/pkg.tar.gz",
        ])
        .output();

    let _ = std::fs::remove_file(&tmp);

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
