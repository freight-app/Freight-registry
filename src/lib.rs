pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod mail;
pub mod metrics;
pub mod oauth;
pub mod rate_limit;
pub mod storage;
pub mod totp;
pub mod validate;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use db::Db;
use mail::Mailer;
use metrics::Metrics;
use oauth::OAuthProvider;
use rate_limit::Limiters;
use storage::Storage;

/// How uploaded tarballs are scanned for malware after a successful publish.
///
/// The registry operator sets this via `--scan-backend` or
/// `FREIGHT_SCAN_BACKEND`.  When `Auto`, the server probes for Docker, then
/// Podman, then bare `clamscan` at startup and uses the first it finds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ScanBackend {
    /// Probe at startup: Docker → Podman → clamscan → heuristics.
    #[default]
    Auto,
    /// Run `clamscan` inside a Docker container (fully isolated).
    Docker,
    /// Run `clamscan` inside a Podman container (rootless, fully isolated).
    Podman,
    /// Run `clamscan` directly on the host (no container isolation).
    Clamscan,
    /// No scanning.
    None,
}

impl std::str::FromStr for ScanBackend {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "auto"     => Ok(Self::Auto),
            "docker"   => Ok(Self::Docker),
            "podman"   => Ok(Self::Podman),
            "clamscan" => Ok(Self::Clamscan),
            "none"     => Ok(Self::None),
            other      => Err(format!("unknown scan backend `{other}`; use auto|docker|podman|clamscan|none")),
        }
    }
}

/// In-flight OAuth state entry (CSRF protection).
pub struct PendingOAuthState {
    pub created_at:    Instant,
    /// Name of the provider that initiated this flow (e.g. `"github"`).
    /// Verified in the callback to prevent cross-provider state reuse.
    pub provider_name: String,
    /// Client-supplied `redirect_uri` from the initial `/auth/:provider` request.
    pub redirect_uri:  Option<String>,
}

pub struct AppState {
    pub db:              Db,
    pub storage:         Storage,
    pub base_url:        String,
    pub limiters:        Limiters,
    pub metrics:         Metrics,
    pub mailer:          Arc<dyn Mailer>,
    /// Base URL of an upstream registry to proxy unknown packages from.
    pub mirror_upstream: Option<String>,
    /// Maximum number of packages a non-admin user may own simultaneously.
    /// `None` means no limit. Admins are always exempt.
    pub max_packages_per_user: Option<u32>,
    /// If `Some`, only packages that declare at least one of these languages
    /// (via `[language.<key>]` in `freight.toml`) are accepted.  `None` means
    /// all languages are allowed.  Example: `["c", "cpp", "fortran"]`.
    pub allowed_languages: Option<Vec<String>>,
    /// How uploaded tarballs are scanned for malware after publish.
    pub scan_backend: ScanBackend,
    /// Container image for the CI verification pipeline (build + test + scan).
    /// When `Some`, each source publish starts as `pending` and is only made
    /// public after the container job passes.  `None` publishes immediately.
    pub verify_image: Option<String>,
    /// Base URL of a separate download server.  When set, `/download` endpoints
    /// redirect there instead of streaming bytes through this server.
    /// See `config.rs` for the full priority chain.
    pub download_url: Option<String>,
    /// Resolved OAuth/OIDC providers.  Empty when OAuth is not configured.
    /// Keyed by iteration; look up by `provider.name`.
    pub oauth_providers: Vec<OAuthProvider>,
    /// Pending CSRF state tokens for in-flight OAuth flows.
    /// Keyed by the hex state token; value tracks creation time + provider + optional redirect_uri.
    pub oauth_states: Arc<Mutex<HashMap<String, PendingOAuthState>>>,
}
