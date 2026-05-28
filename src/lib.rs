pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod mail;
pub mod metrics;
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
use rate_limit::Limiters;
use storage::Storage;

/// GitHub OAuth 2.0 application credentials.
/// Obtained from https://github.com/settings/applications/new.
pub struct GitHubOAuthConfig {
    pub client_id:     String,
    pub client_secret: String,
}

/// In-flight OAuth state entry (CSRF protection).
pub struct PendingOAuthState {
    pub created_at:   Instant,
    /// Client-supplied `redirect_uri` from the initial `/auth/github` request.
    pub redirect_uri: Option<String>,
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
    /// GitHub OAuth credentials. `None` when not configured.
    pub github_oauth: Option<GitHubOAuthConfig>,
    /// Pending CSRF state tokens for in-flight OAuth flows.
    /// Keyed by the hex state token; value tracks creation time + optional redirect_uri.
    pub oauth_states: Arc<Mutex<HashMap<String, PendingOAuthState>>>,
}
