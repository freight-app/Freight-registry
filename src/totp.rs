use anyhow::{anyhow, Result};
use totp_rs::{Algorithm, Secret, TOTP};

pub fn generate_secret_b32() -> String {
    match Secret::generate_secret().to_encoded() {
        Secret::Encoded(s) => s,
        _ => unreachable!("to_encoded always returns Encoded variant"),
    }
}

fn make_totp(secret_b32: &str, username: &str) -> Result<TOTP> {
    let secret = Secret::Encoded(secret_b32.to_string());
    let bytes = secret.to_bytes().map_err(|e| anyhow!("invalid TOTP secret: {e}"))?;
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        bytes,
        Some("freight-registry".to_string()),
        username.to_string(),
    )
    .map_err(|e| anyhow!("TOTP construction failed: {e}"))
}

/// Returns the `otpauth://` provisioning URI for use with authenticator apps.
pub fn provisioning_uri(secret_b32: &str, username: &str) -> Result<String> {
    Ok(make_totp(secret_b32, username)?.get_url())
}

/// Returns `true` if `code` is a valid current or adjacent TOTP code for `secret_b32`.
pub fn verify(secret_b32: &str, username: &str, code: &str) -> bool {
    match make_totp(secret_b32, username) {
        Ok(totp) => totp.check_current(code).unwrap_or(false),
        Err(_) => false,
    }
}
