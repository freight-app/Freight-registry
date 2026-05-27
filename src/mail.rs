//! Email delivery abstraction.
//!
//! [`Mailer`] is the single trait used everywhere in the server for outbound
//! email.  Two implementations ship out-of-the-box:
//!
//! * [`StdoutMailer`] — logs links to the server console via `tracing::warn!`.
//!   This is the default when no `[serve.smtp]` / `--smtp-*` configuration is
//!   present.  It lets operators copy-paste links during development or when
//!   running without an SMTP server.
//!
//! * [`SmtpMailer`] — delivers real email via STARTTLS or TLS using
//!   [`lettre`].  Enabled by providing `--smtp-host` (or the config-file
//!   equivalent).
//!
//! Both implementations are fire-and-forget: a delivery failure is logged but
//! never propagates to the caller.  The token is already written to the DB
//! before `send_*` is called, so a failed email is recoverable (the admin can
//! re-request a reset; a user can contact the admin for manual verification).

use async_trait::async_trait;

// ── Trait ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait Mailer: Send + Sync {
    /// Send an email-verification link to a newly registered user.
    async fn send_verification(&self, to_email: &str, username: &str, link: &str);

    /// Send a password-reset link to a user who requested it.
    async fn send_password_reset(&self, to_email: &str, username: &str, link: &str);
}

// ── StdoutMailer (default / fallback) ────────────────────────────────────────

/// Logs links to the console instead of sending real email.
///
/// Useful during development and in environments without an SMTP server.
/// Output goes through `tracing::warn!` so it appears prominently in logs.
pub struct StdoutMailer;

#[async_trait]
impl Mailer for StdoutMailer {
    async fn send_verification(&self, to_email: &str, username: &str, link: &str) {
        tracing::warn!(
            user  = %username,
            email = %to_email,
            "EMAIL VERIFICATION LINK (expires 24 h) — configure [smtp] to send real email: {link}",
        );
    }

    async fn send_password_reset(&self, to_email: &str, username: &str, link: &str) {
        tracing::warn!(
            user  = %username,
            email = %to_email,
            "PASSWORD RESET LINK (expires 1 h) — configure [smtp] to send real email: {link}",
        );
    }
}

// ── SmtpMailer ────────────────────────────────────────────────────────────────

/// SMTP email delivery via lettre.
///
/// Constructed from [`SmtpConfig`].  Connection errors and delivery failures
/// are logged at `error` level and swallowed — the caller always succeeds.
pub struct SmtpMailer {
    transport: lettre::AsyncSmtpTransport<lettre::Tokio1Executor>,
    from:      lettre::message::Mailbox,
}

impl SmtpMailer {
    /// Build an `SmtpMailer` from a config block.
    ///
    /// Returns `Err` if the `from` address or TLS setup is invalid.
    pub fn new(cfg: &SmtpConfig) -> anyhow::Result<Self> {
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::AsyncSmtpTransport;

        let from: lettre::message::Mailbox = cfg
            .from
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid smtp.from address '{}': {e}", cfg.from))?;

        let builder = match cfg.tls.as_deref().unwrap_or("starttls") {
            "tls" => AsyncSmtpTransport::<lettre::Tokio1Executor>::relay(&cfg.host)?
                .port(cfg.port.unwrap_or(465)),
            "none" => AsyncSmtpTransport::<lettre::Tokio1Executor>::builder_dangerous(&cfg.host)
                .port(cfg.port.unwrap_or(25)),
            _ /* starttls / default */ =>
                AsyncSmtpTransport::<lettre::Tokio1Executor>::starttls_relay(&cfg.host)?
                    .port(cfg.port.unwrap_or(587)),
        };

        let transport = if let (Some(user), Some(pass)) =
            (cfg.username.as_deref(), cfg.password.as_deref())
        {
            builder
                .credentials(Credentials::new(user.to_owned(), pass.to_owned()))
                .build()
        } else {
            builder.build()
        };

        Ok(Self { transport, from })
    }

    async fn send_message(&self, to: &str, subject: &str, body: String) {
        use lettre::{AsyncTransport, Message};

        let to_mailbox: lettre::message::Mailbox = match to.parse() {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("smtp: invalid recipient address '{to}': {e}");
                return;
            }
        };

        let email = match Message::builder()
            .from(self.from.clone())
            .to(to_mailbox)
            .subject(subject)
            .body(body)
        {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("smtp: failed to build email: {e}");
                return;
            }
        };

        if let Err(e) = self.transport.send(email).await {
            tracing::error!("smtp: delivery failed to '{to}': {e}");
        } else {
            tracing::info!("smtp: delivered '{subject}' to '{to}'");
        }
    }
}

#[async_trait]
impl Mailer for SmtpMailer {
    async fn send_verification(&self, to_email: &str, username: &str, link: &str) {
        let body = format!(
            "Hello {username},\n\n\
             Please verify your email address by visiting the link below.\n\
             This link expires in 24 hours.\n\n\
             {link}\n\n\
             If you did not register, you can safely ignore this email.\n"
        );
        self.send_message(to_email, "Verify your freight registry email", body)
            .await;
    }

    async fn send_password_reset(&self, to_email: &str, username: &str, link: &str) {
        let body = format!(
            "Hello {username},\n\n\
             A password reset was requested for your account.\n\
             Use the link below to set a new password. It expires in 1 hour.\n\n\
             {link}\n\n\
             If you did not request a reset, you can safely ignore this email.\n"
        );
        self.send_message(to_email, "Reset your freight registry password", body)
            .await;
    }
}

// ── Config ────────────────────────────────────────────────────────────────────

/// SMTP settings — mirrors the `[serve.smtp]` config-file block and the
/// `--smtp-*` CLI flags.
#[derive(Clone, Debug, Default)]
pub struct SmtpConfig {
    /// SMTP server hostname (required to enable real email delivery)
    pub host:     String,
    /// SMTP port (default: 587 for STARTTLS, 465 for TLS, 25 for none)
    pub port:     Option<u16>,
    /// SMTP login username
    pub username: Option<String>,
    /// SMTP login password
    pub password: Option<String>,
    /// Sender address shown to recipients, e.g. `"Freight <noreply@example.com>"`
    pub from:     String,
    /// TLS mode: `"starttls"` (default), `"tls"`, or `"none"`
    pub tls:      Option<String>,
}
