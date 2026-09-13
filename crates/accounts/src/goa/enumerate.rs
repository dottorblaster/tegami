// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! GOA mail account enumeration.
//!
//! Walks the objects returned by [`ObjectManagerProxy::get_managed_objects`],
//! keeps the ones implementing `org.gnome.OnlineAccounts.Mail` and maps them
//! to [`mail_core::account::AccountConfig`].

use zbus::connection::Connection;

use mail_core::account::{AccountConfig, ImapConfig, SmtpConfig};

use crate::goa::{AccountProxy, MailProxy, ObjectManagerProxy};

const MAIL_INTERFACE: &str = "org.gnome.OnlineAccounts.Mail";
const ACCOUNT_INTERFACE: &str = "org.gnome.OnlineAccounts.Account";

pub async fn enumerate_mail_accounts(connection: &Connection) -> zbus::Result<Vec<AccountConfig>> {
    let manager = ObjectManagerProxy::builder(connection).build().await?;
    let objects = manager.get_managed_objects().await?;
    let mut accounts = Vec::new();
    for (path, interfaces) in objects {
        if interfaces.contains_key(MAIL_INTERFACE)
            && interfaces.contains_key(ACCOUNT_INTERFACE)
            && let Some(account) = mail_account(connection, path.as_str()).await
        {
            accounts.push(account);
        }
    }
    accounts.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(accounts)
}

async fn mail_account(connection: &Connection, path: &str) -> Option<AccountConfig> {
    let mail = MailProxy::builder(connection)
        .path(path)
        .ok()?
        .build()
        .await
        .ok()?;
    let account = AccountProxy::builder(connection)
        .path(path)
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

    Some(AccountConfig {
        id,
        name,
        email_address,
        provider_type: Some(provider_type),
        is_temporary,
        imap,
        smtp,
    })
}
