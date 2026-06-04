//! Provider-agnostic OAuth 2.0 / OIDC support.
//!
//! Each provider is described by [`OAuthProviderConfig`], which can be loaded
//! from `[[serve.oauth]]` in the config file or constructed via the preset
//! helpers.  Call [`OAuthProviderConfig::resolve`] to perform OIDC discovery
//! (when `issuer` is set) and produce a ready-to-use [`OAuthProvider`].
//!
//! # Convenience env-var shortcuts
//!
//! | Variables | Preset |
//! |---|---|
//! | `GITHUB_CLIENT_ID` + `GITHUB_CLIENT_SECRET` | `github` |
//! | `GITLAB_CLIENT_ID` + `GITLAB_CLIENT_SECRET` | `gitlab` (OIDC via gitlab.com or `GITLAB_ISSUER`) |
//! | `GOOGLE_CLIENT_ID` + `GOOGLE_CLIENT_SECRET` | `google` (OIDC) |
//!
//! Company-specific OIDC providers (Okta, Azure AD, Keycloak, …) are
//! configured via `[[serve.oauth]]` in the config file with an `issuer` URL.

use anyhow::{Context, Result};
use serde::Deserialize;

// ── Config type (appears in `ServeConfig` / config file) ─────────────────────

/// One `[[serve.oauth]]` entry in the registry config file.
///
/// **OIDC auto-discovery** (recommended for GitLab, Google, Okta, Azure, …):
/// ```toml
/// [[serve.oauth]]
/// name          = "okta"
/// display_name  = "Okta SSO"
/// client_id     = "0oa…"
/// client_secret = "…"
/// issuer        = "https://company.okta.com"
/// ```
///
/// **Manual endpoints** (for non-OIDC providers, e.g. Gitea):
/// ```toml
/// [[serve.oauth]]
/// name                   = "gitea"
/// client_id              = "abc"
/// client_secret          = "def"
/// authorization_endpoint = "https://git.internal/login/oauth/authorize"
/// token_endpoint         = "https://git.internal/login/oauth/access_token"
/// userinfo_endpoint      = "https://git.internal/api/v1/user"
/// id_field               = "id"
/// username_field         = "login"
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthProviderConfig {
    /// Short URL slug used in routes: `/auth/{name}` and `/auth/{name}/callback`.
    pub name: String,
    /// Human-readable label for the login page (defaults to `name`).
    pub display_name: Option<String>,
    /// OAuth2 application client ID.
    pub client_id: String,
    /// OAuth2 application client secret.
    ///
    /// Never stored in the config file. At startup the secret is read from the
    /// env var `FREIGHT_OAUTH_<NAME_UPPER>_CLIENT_SECRET` (e.g.
    /// `FREIGHT_OAUTH_OKTA_CLIENT_SECRET`). Providers missing a secret are
    /// skipped with a warning.
    #[serde(skip)]
    pub client_secret: String,

    // ── OIDC auto-discovery ────────────────────────────────────────────────────
    /// OIDC issuer URL.  When set, freight fetches
    /// `{issuer}/.well-known/openid-configuration` to resolve all endpoints.
    /// Explicit endpoint fields below take precedence over discovered values.
    pub issuer: Option<String>,

    // ── Explicit endpoint overrides ────────────────────────────────────────────
    /// OAuth2 authorization endpoint (overrides OIDC discovery).
    pub authorization_endpoint: Option<String>,
    /// OAuth2 token exchange endpoint (overrides OIDC discovery).
    pub token_endpoint: Option<String>,
    /// UserInfo endpoint; omit for JWT-only (ID token) flows.
    pub userinfo_endpoint: Option<String>,
    /// Secondary endpoint called when the primary userinfo returns no email.
    /// Expected format: JSON array of `{ "email": "…", "primary": true, "verified": true }`.
    /// Pre-set to `https://api.github.com/user/emails` for the `github` preset.
    pub email_fallback_endpoint: Option<String>,

    // ── Scopes ─────────────────────────────────────────────────────────────────
    /// Scopes to request.  Defaults: `["openid","profile","email"]` for OIDC
    /// providers (those with `issuer`), `["read:user","user:email"]` otherwise.
    #[serde(default)]
    pub scopes: Vec<String>,

    // ── Userinfo field mapping ─────────────────────────────────────────────────
    /// JSON key in the userinfo response that holds the stable provider-side ID.
    /// Default: `"sub"` (OIDC), `"id"` (other).
    pub id_field: Option<String>,
    /// JSON key for the preferred login name.
    /// Default: `"preferred_username"` (OIDC), `"login"` (other).
    pub username_field: Option<String>,
    /// JSON key for the email address.  Default: `"email"`.
    pub email_field: Option<String>,
}

// ── Resolved provider (ready to use in handlers) ─────────────────────────────

/// Fully-resolved OAuth/OIDC provider with concrete endpoint URLs.
/// Built from [`OAuthProviderConfig::resolve`].
#[derive(Debug, Clone)]
pub struct OAuthProvider {
    pub name:                    String,
    pub display_name:            String,
    pub client_id:               String,
    pub client_secret:           String,
    pub authorization_endpoint:  String,
    pub token_endpoint:          String,
    pub userinfo_endpoint:       Option<String>,
    pub email_fallback_endpoint: Option<String>,
    pub scopes:                  Vec<String>,
    pub id_field:                String,
    pub username_field:          String,
    pub email_field:             String,
}

// ── OIDC discovery document (internal) ───────────────────────────────────────

#[derive(Deserialize)]
struct OidcDiscovery {
    authorization_endpoint: String,
    token_endpoint:         String,
    #[serde(default)]
    userinfo_endpoint:      Option<String>,
}

// ── Preset constructors ───────────────────────────────────────────────────────

impl OAuthProviderConfig {
    /// Build the **GitHub** preset from `GITHUB_CLIENT_ID` / `GITHUB_CLIENT_SECRET`.
    /// Returns `None` when either variable is unset or empty.
    pub fn github_from_env() -> Option<Self> {
        let id     = std::env::var("GITHUB_CLIENT_ID").ok().filter(|s| !s.is_empty())?;
        let secret = std::env::var("GITHUB_CLIENT_SECRET").ok().filter(|s| !s.is_empty())?;
        Some(Self {
            name:                    "github".into(),
            display_name:            Some("GitHub".into()),
            client_id:               id,
            client_secret:           secret,
            issuer:                  None,
            authorization_endpoint:  Some("https://github.com/login/oauth/authorize".into()),
            token_endpoint:          Some("https://github.com/login/oauth/access_token".into()),
            userinfo_endpoint:       Some("https://api.github.com/user".into()),
            email_fallback_endpoint: Some("https://api.github.com/user/emails".into()),
            scopes:                  vec!["read:user".into(), "user:email".into()],
            id_field:                Some("id".into()),
            username_field:          Some("login".into()),
            email_field:             Some("email".into()),
        })
    }

    /// Build the **GitLab** preset from `GITLAB_CLIENT_ID` / `GITLAB_CLIENT_SECRET`.
    ///
    /// Uses OIDC discovery against `https://gitlab.com` by default.
    /// Set `GITLAB_ISSUER` to point at a self-hosted instance.
    pub fn gitlab_from_env() -> Option<Self> {
        let id     = std::env::var("GITLAB_CLIENT_ID").ok().filter(|s| !s.is_empty())?;
        let secret = std::env::var("GITLAB_CLIENT_SECRET").ok().filter(|s| !s.is_empty())?;
        let issuer = std::env::var("GITLAB_ISSUER")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "https://gitlab.com".into());
        Some(Self {
            name:                    "gitlab".into(),
            display_name:            Some("GitLab".into()),
            client_id:               id,
            client_secret:           secret,
            issuer:                  Some(issuer),
            authorization_endpoint:  None,
            token_endpoint:          None,
            userinfo_endpoint:       None,
            email_fallback_endpoint: None,
            scopes:                  vec!["openid".into(), "profile".into(), "email".into()],
            id_field:                Some("sub".into()),
            username_field:          Some("preferred_username".into()),
            email_field:             Some("email".into()),
        })
    }

    /// Build the **Google** preset from `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET`.
    pub fn google_from_env() -> Option<Self> {
        let id     = std::env::var("GOOGLE_CLIENT_ID").ok().filter(|s| !s.is_empty())?;
        let secret = std::env::var("GOOGLE_CLIENT_SECRET").ok().filter(|s| !s.is_empty())?;
        Some(Self {
            name:                    "google".into(),
            display_name:            Some("Google".into()),
            client_id:               id,
            client_secret:           secret,
            issuer:                  Some("https://accounts.google.com".into()),
            authorization_endpoint:  None,
            token_endpoint:          None,
            userinfo_endpoint:       None,
            email_fallback_endpoint: None,
            scopes:                  vec!["openid".into(), "profile".into(), "email".into()],
            id_field:                Some("sub".into()),
            username_field:          Some("email".into()),
            email_field:             Some("email".into()),
        })
    }

    /// Resolve the config: load the client secret from the environment, run
    /// OIDC discovery (if `issuer` is set), fill in field-mapping defaults,
    /// and return a ready-to-use [`OAuthProvider`].
    ///
    /// The secret is read from `FREIGHT_OAUTH_<NAME_UPPER>_CLIENT_SECRET`
    /// (e.g. `FREIGHT_OAUTH_OKTA_CLIENT_SECRET`). Returns an error when the
    /// env var is absent or empty so the caller can skip the provider.
    ///
    /// Call this once at startup; the resulting `OAuthProvider` is stored in
    /// `AppState` and reused for every request.
    pub async fn resolve(mut self) -> Result<OAuthProvider> {
        // Load secret from env if not already set (preset constructors set it directly).
        if self.client_secret.is_empty() {
            let env_key = format!(
                "FREIGHT_OAUTH_{}_CLIENT_SECRET",
                self.name.to_ascii_uppercase().replace('-', "_")
            );
            self.client_secret = std::env::var(&env_key).unwrap_or_default();
            if self.client_secret.is_empty() {
                anyhow::bail!(
                    "OAuth provider '{}': set {} env var to supply the client secret",
                    self.name, env_key
                );
            }
        }
        let is_oidc = self.issuer.is_some();

        let (auth_ep, token_ep, userinfo_ep) = if let Some(ref issuer) = self.issuer {
            let discovery_url = format!(
                "{}/.well-known/openid-configuration",
                issuer.trim_end_matches('/')
            );
            let disc: OidcDiscovery = reqwest::Client::new()
                .get(&discovery_url)
                .header("User-Agent", "freight-registry")
                .send()
                .await
                .with_context(|| format!("OIDC discovery: GET {discovery_url}"))?
                .error_for_status()
                .with_context(|| format!("OIDC discovery: bad status for {discovery_url}"))?
                .json()
                .await
                .with_context(|| format!("OIDC discovery: parse error for {discovery_url}"))?;

            (
                self.authorization_endpoint.unwrap_or(disc.authorization_endpoint),
                self.token_endpoint.unwrap_or(disc.token_endpoint),
                self.userinfo_endpoint.or(disc.userinfo_endpoint),
            )
        } else {
            let auth = self.authorization_endpoint.ok_or_else(|| {
                anyhow::anyhow!(
                    "OAuth provider '{}': set `issuer` (OIDC) or `authorization_endpoint`",
                    self.name
                )
            })?;
            let token = self.token_endpoint.ok_or_else(|| {
                anyhow::anyhow!(
                    "OAuth provider '{}': set `issuer` (OIDC) or `token_endpoint`",
                    self.name
                )
            })?;
            (auth, token, self.userinfo_endpoint)
        };

        let scopes = if self.scopes.is_empty() {
            if is_oidc {
                vec!["openid".into(), "profile".into(), "email".into()]
            } else {
                vec!["read:user".into()]
            }
        } else {
            self.scopes
        };

        let id_field       = self.id_field      .unwrap_or_else(|| if is_oidc { "sub".into()                } else { "id".into()    });
        let username_field = self.username_field .unwrap_or_else(|| if is_oidc { "preferred_username".into() } else { "login".into() });
        let email_field    = self.email_field    .unwrap_or_else(|| "email".into());
        let display_name   = self.display_name   .unwrap_or_else(|| title_case(&self.name));

        Ok(OAuthProvider {
            name:                    self.name,
            display_name,
            client_id:               self.client_id,
            client_secret:           self.client_secret,
            authorization_endpoint:  auth_ep,
            token_endpoint:          token_ep,
            userinfo_endpoint:       userinfo_ep,
            email_fallback_endpoint: self.email_fallback_endpoint,
            scopes,
            id_field,
            username_field,
            email_field,
        })
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None    => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}
