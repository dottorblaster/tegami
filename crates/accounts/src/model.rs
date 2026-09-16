// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unified account model.
//!
//! A [`Account`] joins the transport configuration of an identity with
//! the discovery backends that back it. The same mailbox is typically
//! present both in GOA and in EDS, so the two are merged on the e-mail
//! address and both references are kept: the GOA object path drives
//! token and password retrieval through the daemon, the EDS source UID
//! drives Secret Service password lookups.

use mail_core::account::AccountConfig;
use mail_core::store::{AccountRecord, AccountSource, AuthKind, Security};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoaReference {
    pub account_path: String,
    pub provider_type: String,
    pub oauth2: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdsReference {
    pub uid: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub config: AccountConfig,
    pub goa: Option<GoaReference>,
    pub eds: Option<EdsReference>,
}

impl Account {
    pub fn id(&self) -> &str {
        &self.config.id
    }

    pub fn to_record(&self) -> AccountRecord {
        let (source, external_id) = match (&self.goa, &self.eds) {
            (Some(goa), _) => (AccountSource::Goa, goa.account_path.clone()),
            (None, Some(eds)) => (AccountSource::Eds, eds.uid.clone()),
            (None, None) => (AccountSource::Eds, self.config.id.clone()),
        };
        let auth_kind = match &self.goa {
            Some(goa) if goa.oauth2 => AuthKind::OAuth2,
            _ => AuthKind::Password,
        };
        let imap = self.config.imap.as_ref().map(|imap| {
            (
                imap.host.clone(),
                imap.port.map(i64::from),
                security_kind(imap.use_ssl, imap.use_tls),
                imap.user_name.clone(),
            )
        });
        let smtp = self.config.smtp.as_ref().map(|smtp| {
            (
                smtp.host.clone(),
                None,
                security_kind(smtp.use_ssl, smtp.use_tls),
                smtp.user_name.clone(),
            )
        });
        AccountRecord {
            id: None,
            source,
            external_id,
            email: self.config.email_address.clone(),
            display_name: Some(self.config.name.clone()).filter(|name| !name.is_empty()),
            imap_host: imap.as_ref().map(|(host, _, _, _)| host.clone()),
            imap_port: imap.as_ref().and_then(|(_, port, _, _)| *port),
            imap_security: imap.as_ref().and_then(|(_, _, security, _)| *security),
            smtp_host: smtp.as_ref().map(|(host, _, _, _)| host.clone()),
            smtp_port: smtp.as_ref().and_then(|(_, port, _, _)| *port),
            smtp_security: smtp.as_ref().and_then(|(_, _, security, _)| *security),
            auth_kind,
            username: imap
                .as_ref()
                .map(|(_, _, _, user)| user.clone())
                .or_else(|| smtp.as_ref().map(|(_, _, _, user)| user.clone())),
        }
    }
}

fn security_kind(use_ssl: bool, use_tls: bool) -> Option<Security> {
    if use_ssl {
        Some(Security::Ssl)
    } else if use_tls {
        Some(Security::StartTls)
    } else {
        Some(Security::None)
    }
}

#[cfg(test)]
mod tests {
    use mail_core::account::{ImapConfig, SmtpConfig};
    use mail_core::store::AccountSource;

    use super::*;

    fn account() -> Account {
        Account {
            config: AccountConfig {
                id: "goa://path/to/account".to_string(),
                name: "Ada".to_string(),
                email_address: "ada@lovelace.dev".to_string(),
                provider_type: Some("imap_smtp".to_string()),
                is_temporary: false,
                imap: Some(ImapConfig {
                    accept_ssl_errors: false,
                    host: "imap.lovelace.dev".to_string(),
                    use_ssl: true,
                    use_tls: false,
                    user_name: "ada".to_string(),
                    port: Some(993),
                }),
                smtp: Some(SmtpConfig {
                    accept_ssl_errors: false,
                    host: "smtp.lovelace.dev".to_string(),
                    port: Some(465),
                    use_auth: true,
                    auth_login: true,
                    auth_plain: false,
                    auth_xoauth2: false,
                    use_ssl: true,
                    use_tls: false,
                    user_name: "ada".to_string(),
                }),
            },
            goa: Some(GoaReference {
                account_path: "path/to/account".to_string(),
                provider_type: "imap_smtp".to_string(),
                oauth2: true,
            }),
            eds: None,
        }
    }

    #[test]
    fn to_record_goa_oauth2() {
        let record = account().to_record();
        assert_eq!(record.source, AccountSource::Goa);
        assert_eq!(record.external_id, "path/to/account");
        assert_eq!(record.email, "ada@lovelace.dev");
        assert_eq!(record.display_name.as_deref(), Some("Ada"));
        assert_eq!(record.auth_kind, AuthKind::OAuth2);
        assert_eq!(record.imap_host.as_deref(), Some("imap.lovelace.dev"));
        assert_eq!(record.imap_port, Some(993));
        assert_eq!(record.imap_security, Some(Security::Ssl));
        assert_eq!(record.username.as_deref(), Some("ada"));
    }

    #[test]
    fn to_record_eds_password_fallback() {
        let mut account = account();
        account.goa = None;
        account.eds = Some(EdsReference {
            uid: "source-1".to_string(),
        });
        account.config.imap.as_mut().unwrap().use_ssl = false;
        account.config.imap.as_mut().unwrap().use_tls = true;
        let record = account.to_record();
        assert_eq!(record.source, AccountSource::Eds);
        assert_eq!(record.external_id, "source-1");
        assert_eq!(record.auth_kind, AuthKind::Password);
        assert_eq!(record.imap_security, Some(Security::StartTls));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountChange {
    Added(Account),
    Removed(String),
    Modified(Account),
}
