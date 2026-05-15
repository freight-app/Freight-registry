use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};

// ── Per-IP buckets ────────────────────────────────────────────────────────────

/// Two per-IP buckets.
///
/// - `api`:   120 req/min — covers all read endpoints.
/// - `write`:  10 req/min — publish, login, register.
/// - `login`:  username-level lockout after repeated failures.
pub struct Limiters {
    pub api:   Arc<DefaultKeyedRateLimiter<IpAddr>>,
    pub write: Arc<DefaultKeyedRateLimiter<IpAddr>>,
    pub login: LoginLimiter,
}

impl Limiters {
    pub fn new(read_rpm: u32, write_rpm: u32) -> Self {
        Self {
            api:   Arc::new(RateLimiter::keyed(
                Quota::per_minute(NonZeroU32::new(read_rpm.max(1)).unwrap()),
            )),
            write: Arc::new(RateLimiter::keyed(
                Quota::per_minute(NonZeroU32::new(write_rpm.max(1)).unwrap()),
            )),
            login: LoginLimiter::new(),
        }
    }

    /// Extract the client IP from a request's `ConnectInfo` extension.
    pub fn ip_from_extensions(ext: &axum::http::Extensions) -> IpAddr {
        ext.get::<axum::extract::ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip())
            .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
    }
}

// ── Username-level login lockout ──────────────────────────────────────────────

const MAX_FAILURES:      u32      = 5;
const FAILURE_WINDOW:    Duration = Duration::from_secs(10 * 60); // reset counter after 10 min
const LOCKOUT_DURATION:  Duration = Duration::from_secs(15 * 60); // locked for 15 min

struct LoginState {
    failures:     u32,
    window_start: Instant,
    locked_until: Option<Instant>,
}

pub struct LoginLimiter {
    map: Mutex<HashMap<String, LoginState>>,
}

impl LoginLimiter {
    pub fn new() -> Self {
        Self { map: Mutex::new(HashMap::new()) }
    }

    /// Returns `true` if this username is currently locked out.
    pub fn is_locked(&self, username: &str) -> bool {
        let map = self.map.lock().unwrap();
        if let Some(s) = map.get(&username.to_lowercase()) {
            if let Some(until) = s.locked_until {
                return Instant::now() < until;
            }
        }
        false
    }

    /// Record a failed attempt. Returns `true` if the account just became locked.
    pub fn record_failure(&self, username: &str) -> bool {
        let mut map = self.map.lock().unwrap();
        let now = Instant::now();
        let key = username.to_lowercase();
        let state = map.entry(key).or_insert_with(|| LoginState {
            failures:     0,
            window_start: now,
            locked_until: None,
        });

        // Reset counter if the failure window has expired.
        if now.duration_since(state.window_start) > FAILURE_WINDOW {
            state.failures     = 0;
            state.window_start = now;
            state.locked_until = None;
        }

        state.failures += 1;
        if state.failures >= MAX_FAILURES {
            state.locked_until = Some(now + LOCKOUT_DURATION);
            true
        } else {
            false
        }
    }

    /// Clear the failure counter after a successful login.
    pub fn record_success(&self, username: &str) {
        self.map.lock().unwrap().remove(&username.to_lowercase());
    }
}
