pub mod api;
pub mod auth;
pub mod db;
pub mod rate_limit;
pub mod storage;
pub mod totp;
pub mod validate;

use db::Db;
use rate_limit::Limiters;
use storage::Storage;

pub struct AppState {
    pub db:       Db,
    pub storage:  Storage,
    pub base_url: String,
    pub limiters: Limiters,
}
