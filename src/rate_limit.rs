use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::Arc;

use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};

/// Two per-IP buckets.
///
/// - `api`:   120 req/min — covers all read endpoints.
/// - `write`:  10 req/min — publish and login (extra protection against brute-force / flooding).
pub struct Limiters {
    pub api:   Arc<DefaultKeyedRateLimiter<IpAddr>>,
    pub write: Arc<DefaultKeyedRateLimiter<IpAddr>>,
}

impl Limiters {
    pub fn new() -> Self {
        Self {
            api:   Arc::new(RateLimiter::keyed(
                Quota::per_minute(NonZeroU32::new(120).unwrap()),
            )),
            write: Arc::new(RateLimiter::keyed(
                Quota::per_minute(NonZeroU32::new(10).unwrap()),
            )),
        }
    }

    /// Extract the client IP from a request's `ConnectInfo` extension.
    pub fn ip_from_extensions(ext: &axum::http::Extensions) -> IpAddr {
        ext.get::<axum::extract::ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip())
            .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
    }
}
