// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! GOA mail account enumeration.
//!
//! Walks the objects returned by [`ObjectManagerProxy::get_managed_objects`],
//! keeps the ones implementing `org.gnome.OnlineAccounts.Mail` and maps them
//! to [`mail_core::account::AccountConfig`].

use std::collections::HashMap;

use zbus::connection::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use mail_core::account::{AccountConfig, ImapConfig, SmtpConfig};

use crate::goa::{AccountProxy, MailProxy, ObjectManagerProxy};

const MAIL_INTERFACE: &str = "org.gnome.OnlineAccounts.Mail";
const ACCOUNT_INTERFACE: &str = "org.gnome.OnlineAccounts.Account";
const OAUTH2_INTERFACE: &str = "org.gnome.OnlineAccounts.OAuth2Based";

#[derive(Clone, Debug)]
pub struct MailAccount {
    pub config: AccountConfig,
    pub path: OwnedObjectPath,
    pub oauth2: bool,
}

pub async fn enumerate_mail_accounts(connection: &Connection) -> zbus::Result<Vec<AccountConfig>> {
    Ok(enumerate_accounts(connection)
        .await?
        .into_iter()
        .map(|account| account.config)
        .collect())
}

pub async fn enumerate_accounts(connection: &Connection) -> zbus::Result<Vec<MailAccount>> {
    let manager = ObjectManagerProxy::builder(connection).build().await?;
    let objects = manager.get_managed_objects().await?;
    let mut accounts = Vec::new();
    for (path, interfaces) in objects {
        let oauth2 = interfaces.contains_key(OAUTH2_INTERFACE);
        if is_mail_object(&interfaces)
            && let Some(account) = mail_account(connection, path, oauth2).await
        {
            accounts.push(account);
        }
    }
    accounts.sort_by(|a, b| a.config.id.cmp(&b.config.id));
    Ok(accounts)
}

pub fn is_mail_object(interfaces: &HashMap<String, HashMap<String, OwnedValue>>) -> bool {
    interfaces.contains_key(MAIL_INTERFACE) && interfaces.contains_key(ACCOUNT_INTERFACE)
}

async fn mail_account(
    connection: &Connection,
    path: OwnedObjectPath,
    oauth2: bool,
) -> Option<MailAccount> {
    let mail = MailProxy::builder(connection)
        .path(path.as_str())
        .ok()?
        .build()
        .await
        .ok()?;
    let account = AccountProxy::builder(connection)
        .path(path.as_str())
        .ok()?
        .build()
        .await
        .ok()?;

    let provider_type = account.provider_type().await.ok()?;
    let id = account.id().await.ok()?;
    let is_temporary = account.is_temporary().await.ok()?;
    let name = mail.name().await.ok()?;
    let email_address = mail.email_address().await.ok()?;
    let imap = if mail.imap_supported().await.ok()? {
        Some(ImapConfig {
            accept_ssl_errors: mail.imap_accept_ssl_errors().await.ok()?,
            host: mail.imap_host().await.ok()?,
            use_ssl: mail.imap_use_ssl().await.ok()?,
            use_tls: mail.imap_use_tls().await.ok()?,
            user_name: mail.imap_user_name().await.ok()?,
        })
    } else {
        None
    };
    let smtp = if mail.smtp_supported().await.ok()? {
        Some(SmtpConfig {
            accept_ssl_errors: mail.smtp_accept_ssl_errors().await.ok()?,
            host: mail.smtp_host().await.ok()?,
            use_auth: mail.smtp_use_auth().await.ok()?,
            auth_login: mail.smtp_auth_login().await.ok()?,
            auth_plain: mail.smtp_auth_plain().await.ok()?,
            auth_xoauth2: mail.smtp_auth_xoauth2().await.ok()?,
            use_ssl: mail.smtp_use_ssl().await.ok()?,
            use_tls: mail.smtp_use_tls().await.ok()?,
            user_name: mail.smtp_user_name().await.ok()?,
        })
    } else {
        None
    };

    Some(MailAccount {
        config: AccountConfig {
            id,
            name,
            email_address,
            provider_type: Some(provider_type),
            is_temporary,
            imap,
            smtp,
        },
        path,
        oauth2,
    })
}
