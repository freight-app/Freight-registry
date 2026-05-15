use std::sync::Mutex;

use prometheus_client::{
    encoding::text::encode,
    metrics::{counter::Counter, gauge::Gauge},
    registry::Registry,
};

pub struct Metrics {
    registry:           Mutex<Registry>,
    // DB-queried snapshot gauges (updated on each /metrics scrape); i64 default
    pub packages_count:     Gauge,
    pub versions_count:     Gauge,
    pub users_count:        Gauge,
    pub tokens_active:      Gauge,
    pub downloads_db_total: Gauge,
    // In-process event counters
    pub publishes_total:    Counter,
    pub downloads_served:   Counter,
    pub logins_ok:          Counter,
    pub logins_fail:        Counter,
}

impl Metrics {
    pub fn new() -> Self {
        let mut registry = Registry::default();

        let packages_count:     Gauge   = Default::default();
        let versions_count:     Gauge   = Default::default();
        let users_count:        Gauge   = Default::default();
        let tokens_active:      Gauge   = Default::default();
        let downloads_db_total: Gauge   = Default::default();
        let publishes_total:    Counter = Default::default();
        let downloads_served:   Counter = Default::default();
        let logins_ok:          Counter = Default::default();
        let logins_fail:        Counter = Default::default();

        registry.register("freight_packages",        "Total packages",                          packages_count.clone());
        registry.register("freight_versions",        "Total versions published",                versions_count.clone());
        registry.register("freight_users",           "Total registered users",                  users_count.clone());
        registry.register("freight_tokens_active",   "Active (non-expired) tokens",             tokens_active.clone());
        registry.register("freight_downloads_total", "Sum of download counters across versions", downloads_db_total.clone());
        registry.register("freight_publishes",       "Packages published since server start",   publishes_total.clone());
        registry.register("freight_downloads",       "Downloads served since server start",     downloads_served.clone());
        registry.register("freight_logins_ok",       "Successful logins since server start",    logins_ok.clone());
        registry.register("freight_logins_fail",     "Failed logins since server start",        logins_fail.clone());

        Self {
            registry: Mutex::new(registry),
            packages_count,
            versions_count,
            users_count,
            tokens_active,
            downloads_db_total,
            publishes_total,
            downloads_served,
            logins_ok,
            logins_fail,
        }
    }

    pub fn encode(&self) -> String {
        let mut output = String::new();
        encode(&mut output, &self.registry.lock().unwrap()).unwrap();
        output
    }
}
