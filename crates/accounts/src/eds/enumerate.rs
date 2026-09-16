// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! EDS mail account enumeration.
//!
//! Walks the sources exposed by the Sources5 ObjectManager, keeps the
//! mail account sources (those carrying the `Mail Account` extension with
//! an imap backend, enabled) and maps them to
//! [`mail_core::account::AccountConfig`], resolving the display identity
//! and the SMTP transport through the source graph.

use zbus::connection::Connection;
use zbus::zvariant::OwnedObjectPath;

use mail_core::account::{AccountConfig, ImapConfig, SmtpConfig};

use crate::eds::parse::SourceData;
use crate::eds::source::{ObjectManagerProxy, SourceProxy};

const SOURCE_INTERFACE: &str = "org.gnome.evolution.dataserver.Source";

#[derive(Clone, Debug)]
pub struct Source {
    pub uid: String,
    pub data: SourceData,
}

pub async fn enumerate_mail_accounts(connection: &Connection) -> zbus::Result<Vec<AccountConfig>> {
    let manager = ObjectManagerProxy::builder(connection).build().await?;
    let objects = manager.get_managed_objects().await?;
    let mut sources = Vec::new();
    for (path, interfaces) in objects {
        if interfaces.contains_key(SOURCE_INTERFACE)
            && let Some(source) = source_raw(connection, path).await
        {
            sources.push(source);
        }
    }
    Ok(parse_sources(&sources))
}

pub fn mail_accounts(sources: &[Source]) -> Vec<AccountConfig> {
    let mut accounts: Vec<_> = sources
        .iter()
        .filter(|source| source.data.boolean("Data Source", "Enabled") != Some(false))
        .filter(|source| is_imap_account(source))
        .filter_map(|source| account_config(sources, source))
        .collect();
    accounts.sort_by(|a, b| a.id.cmp(&b.id));
    accounts
}

fn parse_sources(sources: &[(String, String)]) -> Vec<AccountConfig> {
    let sources: Vec<Source> = sources
        .iter()
        .filter_map(|(uid, data)| {
            Some(Source {
                uid: uid.clone(),
                data: SourceData::parse(data)?,
            })
        })
        .collect();
    mail_accounts(&sources)
}

async fn source_raw(connection: &Connection, path: OwnedObjectPath) -> Option<(String, String)> {
    let proxy = SourceProxy::builder(connection)
        .path(path.as_str())
        .ok()?
        .build()
        .await
        .ok()?;
    let uid = proxy.uid().await.ok()?;
    let data = proxy.data().await.ok()?;
    Some((uid, data))
}

fn is_imap_account(source: &Source) -> bool {
    matches!(
        source.data.string("Mail Account", "BackendName").as_deref(),
        Some("imapx" | "imap")
    )
}

fn account_config(sources: &[Source], source: &Source) -> Option<AccountConfig> {
    let identity = source
        .data
        .string("Mail Account", "IdentityUid")
        .and_then(|uid| sources.iter().find(|source| source.uid == uid));
    let name = identity
        .and_then(|identity| identity.data.string("Mail Identity", "Name"))
        .or_else(|| source.data.string("Data Source", "DisplayName"));
    let email_address = identity
        .and_then(|identity| identity.data.string("Mail Identity", "Address"))
        .or_else(|| source.data.string("Authentication", "User"));
    Some(AccountConfig {
        id: source.uid.clone(),
        name: name.unwrap_or_default(),
        email_address: email_address.unwrap_or_default(),
        provider_type: None,
        is_temporary: false,
        imap: imap_config(source),
        smtp: identity.and_then(|identity| smtp_config(sources, identity)),
    })
}

fn imap_config(source: &Source) -> Option<ImapConfig> {
    Some(ImapConfig {
        accept_ssl_errors: false,
        host: source.data.string("Authentication", "Host")?,
        use_ssl: transport_security(source, "ssl-on-alternate-port"),
        use_tls: transport_security(source, "starttls-on-standard-port"),
        user_name: source
            .data
            .string("Authentication", "User")
            .unwrap_or_default(),
        port: source
            .data
            .string("Authentication", "Port")
            .and_then(|port| port.parse().ok())
            .filter(|port| *port != 0),
    })
}

fn smtp_config(sources: &[Source], identity: &Source) -> Option<SmtpConfig> {
    let transport_uid = identity.data.string("Mail Submission", "TransportUid")?;
    let transport = sources.iter().find(|source| source.uid == transport_uid)?;
    if transport
        .data
        .string("Mail Transport", "BackendName")
        .as_deref()
        != Some("smtp")
    {
        return None;
    }
    let authentication = transport
        .data
        .string("Authentication", "Method")
        .unwrap_or_default();
    Some(SmtpConfig {
        accept_ssl_errors: false,
        host: transport.data.string("Authentication", "Host")?,
        port: transport
            .data
            .string("Authentication", "Port")
            .and_then(|port| port.parse().ok())
            .filter(|port| *port != 0),
        use_auth: authentication != "none",
        auth_login: authentication == "LOGIN",
        auth_plain: authentication == "PLAIN",
        auth_xoauth2: authentication == "XOAUTH2",
        use_ssl: transport_security(transport, "ssl-on-alternate-port"),
        use_tls: transport_security(transport, "starttls-on-standard-port"),
        user_name: transport
            .data
            .string("Authentication", "User")
            .unwrap_or_default(),
    })
}

fn transport_security(source: &Source, method: &str) -> bool {
    source.data.string("Security", "Method").as_deref() == Some(method)
}
