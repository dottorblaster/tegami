// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! lettre-backed SMTP sender.
//!
//! Maps the account's SMTP configuration onto a lettre transport: implicit TLS
//! for SMTPS, STARTTLS for the submission port and a plaintext connection for
//! trusted local relays. Authentication follows the credential kind, using
//! XOAUTH2 for OAuth2 accounts and PLAIN/LOGIN for password accounts.

use lettre::address::Envelope;
use lettre::transport::AsyncTransport;
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, Tokio1Executor};
use mail_core::account::SmtpConfig;
use mail_core::send::{MailEnvelope, MailSender};
use mail_core::{Credential, MailError};

/// Default port for implicit TLS (SMTPS).
pub const SUBMISSIONS_PORT: u16 = 465;
/// Default port for STARTTLS submission.
pub const SUBMISSION_PORT: u16 = 587;
/// Default port for a plaintext relay.
pub const RELAY_PORT: u16 = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    /// Plaintext connection, only for trusted local relays.
    None,
    /// Plaintext connection upgraded through STARTTLS.
    StartTls,
    /// Implicit TLS, as used by SMTPS.
    Implicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMechanism {
    Plain,
    Login,
    Xoauth2,
}

/// The transport settings derived from an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    pub accept_invalid_certs: bool,
    pub user_name: Option<String>,
    pub mechanisms: Vec<AuthMechanism>,
}

pub fn settings(config: &SmtpConfig, credential: &Credential) -> SmtpSettings {
    let tls = if config.use_ssl {
        TlsMode::Implicit
    } else if config.use_tls {
        TlsMode::StartTls
    } else {
        TlsMode::None
    };
    let port = config.port.unwrap_or(match tls {
        TlsMode::Implicit => SUBMISSIONS_PORT,
        TlsMode::StartTls => SUBMISSION_PORT,
        TlsMode::None => RELAY_PORT,
    });
    let (user_name, mechanisms) = if config.use_auth {
        (
            Some(config.user_name.clone()),
            mechanisms(config, credential),
        )
    } else {
        (None, Vec::new())
    };
    SmtpSettings {
        host: config.host.clone(),
        port,
        tls,
        accept_invalid_certs: config.accept_ssl_errors,
        user_name,
        mechanisms,
    }
}

fn mechanisms(config: &SmtpConfig, credential: &Credential) -> Vec<AuthMechanism> {
    if matches!(credential, Credential::OAuth2(_)) && config.auth_xoauth2 {
        return vec![AuthMechanism::Xoauth2];
    }
    let mut mechanisms = Vec::new();
    if config.auth_plain {
        mechanisms.push(AuthMechanism::Plain);
    }
    if config.auth_login {
        mechanisms.push(AuthMechanism::Login);
    }
    if mechanisms.is_empty() {
        mechanisms.push(AuthMechanism::Plain);
    }
    mechanisms
}

fn secret(credential: &Credential) -> &str {
    match credential {
        Credential::Password(secret) | Credential::OAuth2(secret) => secret,
    }
}

fn mechanism(mechanism: AuthMechanism) -> Mechanism {
    match mechanism {
        AuthMechanism::Plain => Mechanism::Plain,
        AuthMechanism::Login => Mechanism::Login,
        AuthMechanism::Xoauth2 => Mechanism::Xoauth2,
    }
}

fn address(email: &str) -> Result<lettre::Address, MailError> {
    email
        .parse()
        .map_err(|err| MailError::Protocol(format!("invalid address {email}: {err}")))
}

fn lettre_envelope(envelope: &MailEnvelope) -> Result<Envelope, MailError> {
    if envelope.recipients.is_empty() {
        return Err(MailError::Protocol("message has no recipients".to_string()));
    }
    let from = envelope
        .from
        .as_ref()
        .map(|address| self::address(&address.email))
        .transpose()?;
    let recipients = envelope
        .recipients
        .iter()
        .map(|address| self::address(&address.email))
        .collect::<Result<Vec<_>, _>>()?;
    Envelope::new(from, recipients).map_err(|err| MailError::Protocol(err.to_string()))
}

pub struct SmtpSender {
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl SmtpSender {
    pub fn new(config: &SmtpConfig, credential: &Credential) -> Result<Self, MailError> {
        let settings = settings(config, credential);
        let mut builder =
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(settings.host.clone())
                .port(settings.port);

        if settings.tls != TlsMode::None {
            let parameters = TlsParameters::builder(settings.host.clone())
                .dangerous_accept_invalid_certs(settings.accept_invalid_certs)
                .build()
                .map_err(|err| MailError::Protocol(err.to_string()))?;
            let tls = if settings.tls == TlsMode::Implicit {
                Tls::Wrapper(parameters)
            } else {
                Tls::Required(parameters)
            };
            builder = builder.tls(tls);
        }

        if let Some(user_name) = settings.user_name {
            builder = builder
                .credentials(Credentials::new(user_name, secret(credential).to_string()))
                .authentication(settings.mechanisms.iter().copied().map(mechanism).collect());
        }

        Ok(Self {
            transport: builder.build(),
        })
    }
}

impl MailSender for SmtpSender {
    async fn send(&mut self, envelope: &MailEnvelope, raw: &[u8]) -> Result<(), MailError> {
        let envelope = lettre_envelope(envelope)?;
        self.transport
            .send_raw(&envelope, raw)
            .await
            .map_err(|err| MailError::Protocol(err.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smtp_config() -> SmtpConfig {
        SmtpConfig {
            accept_ssl_errors: false,
            host: "smtp.example.org".to_string(),
            port: None,
            use_auth: true,
            auth_login: false,
            auth_plain: true,
            auth_xoauth2: false,
            use_ssl: false,
            use_tls: false,
            user_name: "ada@lovelace.dev".to_string(),
        }
    }

    fn password() -> Credential {
        Credential::Password("secret".to_string())
    }

    #[test]
    fn derives_ports_and_tls_from_the_security_settings() {
        let derived = settings(&smtp_config(), &password());
        assert_eq!(derived.tls, TlsMode::None);
        assert_eq!(derived.port, RELAY_PORT);

        let mut config = smtp_config();
        config.use_tls = true;
        let derived = settings(&config, &password());
        assert_eq!(derived.tls, TlsMode::StartTls);
        assert_eq!(derived.port, SUBMISSION_PORT);

        let mut config = smtp_config();
        config.use_tls = false;
        config.use_ssl = true;
        let derived = settings(&config, &password());
        assert_eq!(derived.tls, TlsMode::Implicit);
        assert_eq!(derived.port, SUBMISSIONS_PORT);

        let mut config = smtp_config();
        config.use_ssl = true;
        config.port = Some(2525);
        assert_eq!(settings(&config, &password()).port, 2525);
    }

    #[test]
    fn picks_the_authentication_mechanisms() {
        let mut config = smtp_config();
        config.auth_plain = false;
        config.auth_login = true;
        assert_eq!(
            settings(&config, &password()).mechanisms,
            vec![AuthMechanism::Login]
        );

        let mut config = smtp_config();
        config.auth_plain = false;
        config.auth_login = false;
        assert_eq!(
            settings(&config, &password()).mechanisms,
            vec![AuthMechanism::Plain]
        );

        let mut config = smtp_config();
        config.auth_plain = false;
        config.auth_login = false;
        config.auth_xoauth2 = true;
        assert_eq!(
            settings(&config, &Credential::OAuth2("token".to_string())).mechanisms,
            vec![AuthMechanism::Xoauth2]
        );
        // A password account never asks for XOAUTH2.
        assert_eq!(
            settings(&config, &password()).mechanisms,
            vec![AuthMechanism::Plain]
        );
    }

    #[test]
    fn skips_credentials_when_the_account_does_not_authenticate() {
        let mut config = smtp_config();
        config.use_auth = false;
        let derived = settings(&config, &password());
        assert!(derived.user_name.is_none());
        assert!(derived.mechanisms.is_empty());
    }

    #[test]
    fn carries_the_invalid_certificate_escape_hatch() {
        let mut config = smtp_config();
        config.use_ssl = true;
        config.accept_ssl_errors = true;
        assert!(settings(&config, &password()).accept_invalid_certs);
    }

    #[test]
    fn rejects_envelopes_without_recipients() {
        assert!(lettre_envelope(&MailEnvelope::default()).is_err());
    }

    #[test]
    fn rejects_invalid_addresses() {
        let envelope = MailEnvelope {
            from: None,
            recipients: vec![mail_core::compose::Address::new(None, "not an address")],
        };
        assert!(lettre_envelope(&envelope).is_err());
    }

    #[test]
    fn builds_a_transport_for_every_security_mode() {
        for config in [
            smtp_config(),
            SmtpConfig {
                use_tls: true,
                ..smtp_config()
            },
            SmtpConfig {
                use_ssl: true,
                ..smtp_config()
            },
        ] {
            assert!(SmtpSender::new(&config, &password()).is_ok());
        }
    }
}
