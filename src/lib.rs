pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod metrics;
pub mod rate_limit;
pub mod storage;
pub mod totp;
pub mod validate;

use db::Db;
use metrics::Metrics;
use rate_limit::Limiters;
use storage::Storage;

pub struct AppState {
    pub db:              Db,
    pub storage:         Storage,
    pub base_url:        String,
    pub limiters:        Limiters,
    pub metrics:         Metrics,
    /// Base URL of an upstream registry to proxy unknown packages from.
    pub mirror_upstream: Option<String>,
    /// Maximum number of packages a non-admin user may own simultaneously.
    /// `None` means no limit. Admins are always exempt.
    pub max_packages_per_user: Option<u32>,
}
