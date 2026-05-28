//! GitHub OAuth 2.0 login flow.
//!
//! Two public endpoints:
//!
//!   `GET /auth/github[?redirect_uri=<url>]`
//!     Generates a CSRF state token and redirects the browser to GitHub's
//!     authorisation page.  If `redirect_uri` is provided the callback will
//!     redirect there with `?token=<access_token>` appended once the flow
//!     completes (useful for CLI tools that spin up a local listener).
//!
//!   `GET /auth/github/callback?code=<code>&state=<state>`
//!     Verifies the CSRF state, exchanges the code for a GitHub access token,
//!     fetches the user's GitHub identity, creates or finds the matching
//!     freight account, issues an access + refresh token pair, and returns an
//!     HTML page with the token and copy/paste instructions.
//!
//! Configuration:
//!   `--github-client-id` / `GITHUB_CLIENT_ID`
//!   `--github-client-secret` / `GITHUB_CLIENT_SECRET`
//!
//! OAuth-only users are stored with `password_hash = "!oauth:github"`.  That
//! prefix is detected by the password-login handler and turns a generic 500
//! into a helpful "use GitHub login" 401.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use crate::AppState;
use super::ApiError;

const GITHUB_AUTHORIZE: &str = "https://github.com/login/oauth/authorize";
const GITHUB_TOKEN:     &str = "https://github.com/login/oauth/access_token";
const GITHUB_USER:      &str = "https://api.github.com/user";
const GITHUB_EMAILS:    &str = "https://api.github.com/user/emails";
/// States older than this are rejected (CSRF window).
const STATE_TTL: Duration = Duration::from_secs(600);

// ── Query-param structs ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct StartParams {
    /// After a successful login the callback will redirect here (with `?token=…`
    /// appended).  Omit to stay on the HTML confirmation page.
    #[serde(default)]
    pub redirect_uri: Option<String>,
}

#[derive(Deserialize)]
pub struct CallbackParams {
    pub code:  String,
    pub state: String,
    /// GitHub sets this when the user denies access.
    #[serde(default)]
    pub error: Option<String>,
}

// ── GitHub API response types ─────────────────────────────────────────────────

#[derive(Deserialize)]
struct GhTokenResponse {
    access_token:      Option<String>,
    error:             Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct GhUser {
    id:    i64,
    login: String,
    email: Option<String>,
}

#[derive(Deserialize)]
struct GhEmail {
    email:    String,
    primary:  bool,
    verified: bool,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// `GET /auth/github[?redirect_uri=<url>]`
///
/// Redirects the browser to GitHub's OAuth authorisation page.
pub async fn github_start(
    State(state): State<Arc<AppState>>,
    Query(params): Query<StartParams>,
) -> Response {
    let Some(cfg) = &state.github_oauth else {
        return ApiError::bad_request(
            "GitHub OAuth is not configured on this server — \
             set GITHUB_CLIENT_ID and GITHUB_CLIENT_SECRET",
        )
        .into_response();
    };

    // 32 random bytes → 64-char hex string used as CSRF state.
    let raw: [u8; 32] = rand::random();
    let csrf = hex::encode(raw);

    {
        let mut map = state.oauth_states.lock().unwrap();
        // Prune expired states before inserting.
        let cutoff = Instant::now().checked_sub(STATE_TTL).unwrap_or(Instant::now());
        map.retain(|_, s| s.created_at > cutoff);
        map.insert(csrf.clone(), crate::PendingOAuthState {
            created_at:   Instant::now(),
            redirect_uri: params.redirect_uri,
        });
    }

    let callback_url = format!("{}/auth/github/callback", state.base_url);

    let mut auth_url = url::Url::parse(GITHUB_AUTHORIZE)
        .expect("static GitHub URL is valid");
    auth_url
        .query_pairs_mut()
        .append_pair("client_id",    &cfg.client_id)
        .append_pair("redirect_uri", &callback_url)
        .append_pair("scope",        "read:user user:email")
        .append_pair("state",        &csrf);

    Redirect::temporary(auth_url.as_str()).into_response()
}

/// `GET /auth/github/callback?code=…&state=…`
///
/// Completes the OAuth flow: verifies CSRF state, exchanges the code, fetches
/// the GitHub user, creates or finds the freight account, issues tokens.
pub async fn github_callback(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CallbackParams>,
) -> Response {
    // Did the user deny the OAuth request?
    if let Some(err) = params.error {
        return Html(error_page(&format!("GitHub denied access: {err}"))).into_response();
    }

    let Some(cfg) = &state.github_oauth else {
        return Html(error_page("GitHub OAuth is not configured on this server")).into_response();
    };

    // Pop and verify CSRF state.
    let pending = {
        let mut map = state.oauth_states.lock().unwrap();
        match map.remove(&params.state) {
            Some(s) if s.created_at.elapsed() < STATE_TTL => s,
            Some(_) => return Html(error_page("OAuth state expired — please try again")).into_response(),
            None    => return Html(error_page("Invalid OAuth state — possible CSRF attempt")).into_response(),
        }
    };

    // Exchange the authorisation code for a GitHub access token.
    let gh_token = match exchange_code(cfg, &params.code, &state.base_url).await {
        Ok(t)  => t,
        Err(e) => {
            tracing::error!("GitHub token exchange failed: {e:#}");
            return Html(error_page("Failed to exchange code with GitHub — please try again")).into_response();
        }
    };

    // Fetch the authenticated user's GitHub profile.
    let gh_user = match fetch_github_user(&gh_token).await {
        Ok(u)  => u,
        Err(e) => {
            tracing::error!("GitHub user fetch failed: {e:#}");
            return Html(error_page("Failed to fetch GitHub user info — please try again")).into_response();
        }
    };

    // Use the email from the profile; fall back to the primary verified email
    // from the /emails endpoint (needed when the user has a private email).
    let email = match gh_user.email {
        Some(ref e) if !e.is_empty() => Some(e.clone()),
        _ => fetch_github_primary_email(&gh_token).await.unwrap_or(None),
    };

    // Find or create the freight user account.
    let user = match state
        .db
        .find_or_create_oauth_user("github", &gh_user.id.to_string(), &gh_user.login, email.as_deref())
        .await
    {
        Ok(u)  => u,
        Err(e) => {
            tracing::error!("OAuth user lookup/create failed: {e:#}");
            return Html(error_page("Failed to create or find your freight account — please try again")).into_response();
        }
    };

    // Issue a 90-day access token + 30-day refresh token, same as password login.
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let access_name  = format!("github-{ts}");
    let refresh_name = format!("github-refresh-{ts}");

    let (token, refresh_token) = match tokio::try_join!(
        state.db.create_token(user.id, &access_name,  Some(90), "access",  "publish"),
        state.db.create_token(user.id, &refresh_name, Some(30), "refresh", "publish"),
    ) {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!("token creation failed: {e:#}");
            return Html(error_page("Failed to issue an API token — please try again")).into_response();
        }
    };

    state.db.audit(Some(user.id), "oauth_login", None, None, None);
    tracing::info!(user = %user.username, provider = "github", "OAuth login");

    // If the client asked for a redirect (e.g. CLI local-server flow), send them there.
    if let Some(mut redir) = pending.redirect_uri {
        let sep = if redir.contains('?') { '&' } else { '?' };
        redir.push(sep);
        redir.push_str("token=");
        redir.push_str(&token);
        return Redirect::temporary(&redir).into_response();
    }

    // Otherwise return a browser-friendly HTML page with the token.
    Html(success_page(&user.username, &token, &refresh_token)).into_response()
}

// ── GitHub HTTP helpers ───────────────────────────────────────────────────────

async fn exchange_code(
    cfg: &crate::GitHubOAuthConfig,
    code: &str,
    base_url: &str,
) -> anyhow::Result<String> {
    let callback_url = format!("{base_url}/auth/github/callback");
    let resp: GhTokenResponse = reqwest::Client::new()
        .post(GITHUB_TOKEN)
        .header("Accept", "application/json")
        .header("User-Agent", "freight-registry")
        .form(&[
            ("client_id",     cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
            ("code",          code),
            ("redirect_uri",  &callback_url),
        ])
        .send()
        .await?
        .json()
        .await?;

    if let Some(err) = resp.error {
        let desc = resp.error_description.unwrap_or_default();
        anyhow::bail!("GitHub returned error '{err}': {desc}");
    }
    resp.access_token
        .filter(|t| !t.is_empty())
        .ok_or_else(|| anyhow::anyhow!("GitHub response contained no access_token"))
}

async fn fetch_github_user(token: &str) -> anyhow::Result<GhUser> {
    let user = reqwest::Client::new()
        .get(GITHUB_USER)
        .bearer_auth(token)
        .header("User-Agent", "freight-registry")
        .send()
        .await?
        .json::<GhUser>()
        .await?;
    Ok(user)
}

async fn fetch_github_primary_email(token: &str) -> anyhow::Result<Option<String>> {
    let emails: Vec<GhEmail> = reqwest::Client::new()
        .get(GITHUB_EMAILS)
        .bearer_auth(token)
        .header("User-Agent", "freight-registry")
        .send()
        .await?
        .json()
        .await?;
    Ok(emails
        .into_iter()
        .find(|e| e.primary && e.verified)
        .map(|e| e.email))
}

// ── HTML pages ────────────────────────────────────────────────────────────────

fn success_page(username: &str, token: &str, _refresh_token: &str) -> String {
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
    background: #f8f9fa;
    color: #212529;
    margin: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 100vh;
    padding: 20px;
  }}
  .card {{
    background: #fff;
    border-radius: 10px;
    box-shadow: 0 2px 16px rgba(0,0,0,.08);
    padding: 40px;
    max-width: 600px;
    width: 100%;
  }}
  h1 {{ color: #2d8a4e; margin: 0 0 8px; font-size: 1.5rem; }}
  .subtitle {{ color: #6c757d; margin: 0 0 24px; }}
  label {{ font-size: .85rem; font-weight: 600; color: #495057; display: block; margin-bottom: 6px; }}
  .token-box {{
    background: #f4f4f4;
    border: 1px solid #dee2e6;
    border-radius: 6px;
    padding: 14px 16px;
    font-family: "SFMono-Regular", Consolas, monospace;
    font-size: .88rem;
    word-break: break-all;
    color: #212529;
    margin-bottom: 12px;
  }}
  .copy-btn {{
    background: #2d8a4e;
    color: #fff;
    border: none;
    border-radius: 6px;
    padding: 9px 18px;
    font-size: .88rem;
    cursor: pointer;
    transition: background .15s;
  }}
  .copy-btn:hover {{ background: #24703f; }}
  hr {{ border: none; border-top: 1px solid #dee2e6; margin: 24px 0; }}
  pre {{
    background: #f4f4f4;
    border-radius: 6px;
    padding: 12px 16px;
    font-size: .85rem;
    overflow-x: auto;
    white-space: pre-wrap;
    word-break: break-all;
  }}
  .note {{ font-size: .85rem; color: #6c757d; margin-top: 8px; }}
</style>
</head>
<body>
<div class="card">
  <h1>✓ Logged in as {username}</h1>
  <p class="subtitle">Your freight API token is ready. Save it — it won't be shown again.</p>

  <label for="tok">API token</label>
  <div class="token-box" id="tok">{token}</div>
  <button class="copy-btn" onclick="copyToken()">Copy token</button>

  <hr>

  <label>Save with the CLI</label>
  <pre>freight login --token {token}</pre>
  <p class="note">
    Or run <code>freight login</code> and paste the token when prompted.
  </p>
</div>

<script>
function copyToken() {{
  const text = document.getElementById('tok').textContent.trim();
  navigator.clipboard.writeText(text).then(function() {{
    const btn = document.querySelector('.copy-btn');
    btn.textContent = '✓ Copied!';
    setTimeout(function() {{ btn.textContent = 'Copy token'; }}, 2000);
  }}).catch(function() {{
    const btn = document.querySelector('.copy-btn');
    btn.textContent = 'Select and copy manually';
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
    background: #f8f9fa;
    color: #212529;
    margin: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 100vh;
    padding: 20px;
  }}
  .card {{
    background: #fff;
    border-radius: 10px;
    box-shadow: 0 2px 16px rgba(0,0,0,.08);
    padding: 40px;
    max-width: 520px;
    width: 100%;
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
