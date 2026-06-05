//! Provider-agnostic OAuth 2.0 / OIDC login handlers.
//!
//! Two endpoints per configured provider:
//!
//!   `GET /auth/:provider[?redirect_uri=<url>]`
//!     Generates a CSRF state token and redirects the browser to the provider's
//!     authorisation page.  Supply `redirect_uri` for the CLI local-server flow:
//!     the callback will redirect there with `?token=<access_token>` appended.
//!
//!   `GET /auth/:provider/callback?code=<code>&state=<state>`
//!     Verifies the CSRF state, exchanges the code for an access token,
//!     fetches the user's identity from the provider, creates or finds the
//!     freight account, issues an access + refresh token pair, and returns an
//!     HTML confirmation page (or the `redirect_uri` redirect).
//!
//! Providers are configured via `[[serve.oauth]]` in the config file or via
//! environment variables — see [`crate::oauth::OAuthProviderConfig`].

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use crate::{oauth::OAuthProvider, AppState, PendingOAuthState};
use super::ApiError;

/// CSRF state tokens older than this are rejected.
const STATE_TTL: Duration = Duration::from_secs(600);

// ── Query-param structs ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct StartParams {
    /// After login, redirect here with `?token=…` appended instead of showing
    /// the HTML success page.  Useful for CLI tools that spin up a local listener.
    #[serde(default)]
    pub redirect_uri: Option<String>,
}

#[derive(Deserialize)]
pub struct CallbackParams {
    pub code:  String,
    pub state: String,
    /// Set by the provider when the user denies access.
    #[serde(default)]
    pub error: Option<String>,
    /// Human-readable denial reason (provider-dependent).
    #[serde(default)]
    pub error_description: Option<String>,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// `GET /auth/:provider[?redirect_uri=<url>]`
///
/// Generates a CSRF state token and redirects to the provider's auth page.
pub async fn oauth_start(
    State(state): State<Arc<AppState>>,
    Path(provider_name): Path<String>,
    Query(params): Query<StartParams>,
) -> Response {
    let Some(provider) = find_provider(&state, &provider_name) else {
        return ApiError::not_found(format!(
            "OAuth provider '{provider_name}' is not configured on this server"
        ))
        .into_response();
    };

    // 32 random bytes → 64-char hex CSRF token.
    let raw: [u8; 32] = rand::random();
    let csrf = hex::encode(raw);

    {
        let mut map = state.oauth_states.lock().unwrap();
        // Prune expired entries before inserting.
        let cutoff = Instant::now().checked_sub(STATE_TTL).unwrap_or(Instant::now());
        map.retain(|_, s| s.created_at > cutoff);
        map.insert(csrf.clone(), PendingOAuthState {
            created_at:    Instant::now(),
            provider_name: provider_name.clone(),
            redirect_uri:  params.redirect_uri,
        });
    }

    let callback_url = format!("{}/auth/{provider_name}/callback", state.base_url);
    let scope_str = provider.scopes.join(" ");

    let mut auth_url = url::Url::parse(&provider.authorization_endpoint)
        .unwrap_or_else(|_| panic!("invalid authorization_endpoint for provider '{provider_name}'"));
    auth_url
        .query_pairs_mut()
        .append_pair("client_id",     &provider.client_id)
        .append_pair("redirect_uri",  &callback_url)
        .append_pair("scope",         &scope_str)
        .append_pair("state",         &csrf)
        .append_pair("response_type", "code");

    Redirect::temporary(auth_url.as_str()).into_response()
}

/// `GET /auth/:provider/callback?code=…&state=…`
///
/// Completes the OAuth flow: verifies CSRF state, exchanges the code, fetches
/// the user identity, creates or finds the freight account, issues tokens.
pub async fn oauth_callback(
    State(state): State<Arc<AppState>>,
    Path(provider_name): Path<String>,
    Query(params): Query<CallbackParams>,
) -> Response {
    // Did the user deny access?
    if let Some(ref err) = params.error {
        let detail = params.error_description.as_deref().unwrap_or("");
        let msg = if detail.is_empty() {
            format!("{provider_name} denied access: {err}")
        } else {
            format!("{provider_name} denied access: {err} — {detail}")
        };
        return Html(error_page(&msg)).into_response();
    }

    let Some(provider) = find_provider(&state, &provider_name) else {
        return Html(error_page(&format!(
            "OAuth provider '{provider_name}' is not configured on this server"
        )))
        .into_response();
    };
    // Clone so we can use it after find_provider's lifetime.
    let provider = provider.clone();

    // Pop and verify CSRF state.
    let pending = {
        let mut map = state.oauth_states.lock().unwrap();
        match map.remove(&params.state) {
            None => {
                return Html(error_page("Invalid OAuth state — possible CSRF attempt")).into_response();
            }
            Some(s) if s.provider_name != provider_name => {
                return Html(error_page("OAuth state mismatch — possible CSRF attempt")).into_response();
            }
            Some(s) if s.created_at.elapsed() >= STATE_TTL => {
                return Html(error_page("OAuth state expired — please try again")).into_response();
            }
            Some(s) => s,
        }
    };

    // Exchange authorisation code for an access token.
    let access_token = match exchange_code(&provider, &params.code, &state.base_url, &provider_name).await {
        Ok(t)  => t,
        Err(e) => {
            tracing::error!(provider = %provider_name, "token exchange failed: {e:#}");
            return Html(error_page(&format!(
                "Failed to exchange code with {} — please try again",
                provider.display_name
            )))
            .into_response();
        }
    };

    // Fetch user identity (provider_id, login, optional email).
    let (provider_id, login, email) = match fetch_userinfo(&provider, &access_token).await {
        Ok(info) => info,
        Err(e)   => {
            tracing::error!(provider = %provider_name, "userinfo fetch failed: {e:#}");
            return Html(error_page(&format!(
                "Failed to fetch user info from {} — please try again",
                provider.display_name
            )))
            .into_response();
        }
    };

    // Find or create the freight user account.
    let user = match state
        .db
        .find_or_create_oauth_user(&provider_name, &provider_id, &login, email.as_deref())
        .await
    {
        Ok(u)  => u,
        Err(e) => {
            tracing::error!(provider = %provider_name, "user lookup/create failed: {e:#}");
            return Html(error_page(
                "Failed to create or find your freight account — please try again",
            ))
            .into_response();
        }
    };

    // Issue a 90-day access token + 30-day refresh token.
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let access_name  = format!("{provider_name}-{ts}");
    let refresh_name = format!("{provider_name}-refresh-{ts}");

    let (token, refresh_token) = match tokio::try_join!(
        state.db.create_token(user.id, &access_name,  Some(90), "access",  "publish", None),
        state.db.create_token(user.id, &refresh_name, Some(30), "refresh", "publish", None),
    ) {
        Ok(pair) => pair,
        Err(e)   => {
            tracing::error!("token creation failed: {e:#}");
            return Html(error_page("Failed to issue an API token — please try again")).into_response();
        }
    };

    state.db.audit(Some(user.id), "oauth_login", None, None, None);
    tracing::info!(user = %user.username, provider = %provider_name, "OAuth login");

    // Redirect to client if requested (CLI local-server flow).
    if let Some(mut redir) = pending.redirect_uri {
        let sep = if redir.contains('?') { '&' } else { '?' };
        redir.push(sep);
        redir.push_str("token=");
        redir.push_str(&token);
        return Redirect::temporary(&redir).into_response();
    }

    Html(success_page(&user.username, &provider.display_name, &token, &refresh_token)).into_response()
}

// ── HTTP helpers ──────────────────────────────────────────────────────────────

fn find_provider<'a>(state: &'a AppState, name: &str) -> Option<&'a OAuthProvider> {
    state.oauth_providers.iter().find(|p| p.name == name)
}

/// Exchange an authorisation code for a bearer access token.
async fn exchange_code(
    provider:      &OAuthProvider,
    code:          &str,
    base_url:      &str,
    provider_name: &str,
) -> anyhow::Result<String> {
    let callback_url = format!("{base_url}/auth/{provider_name}/callback");

    #[derive(Deserialize)]
    struct TokenResp {
        access_token:      Option<String>,
        error:             Option<String>,
        error_description: Option<String>,
    }

    let resp: TokenResp = reqwest::Client::new()
        .post(&provider.token_endpoint)
        .header("Accept",     "application/json")
        .header("User-Agent", "freight-registry")
        .form(&[
            ("grant_type",    "authorization_code"),
            ("client_id",     provider.client_id.as_str()),
            ("client_secret", provider.client_secret.as_str()),
            ("code",          code),
            ("redirect_uri",  callback_url.as_str()),
        ])
        .send()
        .await?
        .json()
        .await?;

    if let Some(err) = resp.error {
        let desc = resp.error_description.unwrap_or_default();
        anyhow::bail!("provider returned error '{err}': {desc}");
    }
    resp.access_token
        .filter(|t| !t.is_empty())
        .ok_or_else(|| anyhow::anyhow!("provider response contained no access_token"))
}

/// Fetch the user's identity from the userinfo endpoint.
///
/// Returns `(provider_id, login, optional_email)`.  `provider_id` is always
/// a non-empty string (numeric IDs like GitHub's are stringified).
async fn fetch_userinfo(
    provider: &OAuthProvider,
    token:    &str,
) -> anyhow::Result<(String, String, Option<String>)> {
    let endpoint = provider
        .userinfo_endpoint
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!(
            "no userinfo_endpoint configured for provider '{}'",
            provider.name
        ))?;

    let data: serde_json::Value = reqwest::Client::new()
        .get(endpoint)
        .bearer_auth(token)
        .header("User-Agent", "freight-registry")
        .header("Accept",     "application/json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let provider_id = json_str(&data, &provider.id_field)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!(
            "userinfo missing '{}' field (provider '{}')",
            provider.id_field, provider.name
        ))?;

    let login = json_str(&data, &provider.username_field).unwrap_or_default();
    let mut email = json_str(&data, &provider.email_field);

    // Fall back to the secondary email endpoint if the primary had no email.
    if email.as_deref().unwrap_or("").is_empty() {
        if let Some(ref ep) = provider.email_fallback_endpoint {
            email = fetch_fallback_email(ep, token).await.unwrap_or(None);
        }
    }

    Ok((provider_id, login, email))
}

/// Call a secondary email endpoint and return the primary verified email.
/// Expected response: `[{ "email": "…", "primary": true, "verified": true }]`.
async fn fetch_fallback_email(endpoint: &str, token: &str) -> anyhow::Result<Option<String>> {
    #[derive(Deserialize)]
    struct EmailEntry {
        email:    String,
        primary:  bool,
        verified: bool,
    }

    let entries: Vec<EmailEntry> = reqwest::Client::new()
        .get(endpoint)
        .bearer_auth(token)
        .header("User-Agent", "freight-registry")
        .header("Accept",     "application/json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(entries
        .into_iter()
        .find(|e| e.primary && e.verified)
        .map(|e| e.email))
}

/// Extract a JSON field as `String`, coercing numbers/booleans to string.
/// GitHub returns `"id": 12345` (number); OIDC providers return `"sub": "…"` (string).
fn json_str(val: &serde_json::Value, field: &str) -> Option<String> {
    match val.get(field) {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(serde_json::Value::Number(n)) => Some(n.to_string()),
        Some(serde_json::Value::Bool(b))   => Some(b.to_string()),
        _                                  => None,
    }
}

// ── HTML pages ────────────────────────────────────────────────────────────────

fn success_page(username: &str, provider: &str, token: &str, _refresh_token: &str) -> String {
    format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Freight — logged in</title>
<style>
  *, *::before, *::after {{ box-sizing: border-box; }}
  body {{
    font-family: system-ui, -apple-system, sans-serif;
    background: #f8f9fa; color: #212529; margin: 0;
    display: flex; align-items: center; justify-content: center;
    min-height: 100vh; padding: 20px;
  }}
  .card {{
    background: #fff; border-radius: 10px;
    box-shadow: 0 2px 16px rgba(0,0,0,.08);
    padding: 40px; max-width: 600px; width: 100%;
  }}
  h1 {{ color: #2d8a4e; margin: 0 0 8px; font-size: 1.5rem; }}
  .subtitle {{ color: #6c757d; margin: 0 0 24px; }}
  label {{ font-size: .85rem; font-weight: 600; color: #495057; display: block; margin-bottom: 6px; }}
  .token-box {{
    background: #f4f4f4; border: 1px solid #dee2e6; border-radius: 6px;
    padding: 14px 16px; font-family: "SFMono-Regular", Consolas, monospace;
    font-size: .88rem; word-break: break-all; color: #212529; margin-bottom: 12px;
  }}
  .copy-btn {{
    background: #2d8a4e; color: #fff; border: none; border-radius: 6px;
    padding: 9px 18px; font-size: .88rem; cursor: pointer; transition: background .15s;
  }}
  .copy-btn:hover {{ background: #24703f; }}
  hr {{ border: none; border-top: 1px solid #dee2e6; margin: 24px 0; }}
  pre {{
    background: #f4f4f4; border-radius: 6px; padding: 12px 16px;
    font-size: .85rem; overflow-x: auto; white-space: pre-wrap; word-break: break-all;
  }}
  .note {{ font-size: .85rem; color: #6c757d; margin-top: 8px; }}
</style>
</head>
<body>
<div class="card">
  <h1>✓ Logged in as {username}</h1>
  <p class="subtitle">Authenticated via {provider}. Your freight API token is ready — save it, it won't be shown again.</p>

  <label for="tok">API token</label>
  <div class="token-box" id="tok">{token}</div>
  <button class="copy-btn" onclick="copyToken()">Copy token</button>

  <hr>

  <label>Save with the CLI</label>
  <pre>freight login --token {token}</pre>
  <p class="note">Or run <code>freight login</code> and paste the token when prompted.</p>
</div>
<script>
function copyToken() {{
  navigator.clipboard.writeText(document.getElementById('tok').textContent.trim())
    .then(function() {{
      var b = document.querySelector('.copy-btn');
      b.textContent = '✓ Copied!';
      setTimeout(function() {{ b.textContent = 'Copy token'; }}, 2000);
    }})
    .catch(function() {{
      document.querySelector('.copy-btn').textContent = 'Select and copy manually';
    }});
}}
</script>
</body>
</html>
"#)
}

fn error_page(message: &str) -> String {
    format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Freight — login failed</title>
<style>
  *, *::before, *::after {{ box-sizing: border-box; }}
  body {{
    font-family: system-ui, -apple-system, sans-serif;
    background: #f8f9fa; color: #212529; margin: 0;
    display: flex; align-items: center; justify-content: center;
    min-height: 100vh; padding: 20px;
  }}
  .card {{
    background: #fff; border-radius: 10px;
    box-shadow: 0 2px 16px rgba(0,0,0,.08);
    padding: 40px; max-width: 520px; width: 100%;
  }}
  h1 {{ color: #c0392b; margin: 0 0 12px; font-size: 1.4rem; }}
  p {{ color: #495057; }}
  a {{ color: #2d8a4e; }}
</style>
</head>
<body>
<div class="card">
  <h1>✗ Login failed</h1>
  <p>{message}</p>
  <p><a href="javascript:history.back()">← Go back</a></p>
</div>
</body>
</html>
"#)
}
