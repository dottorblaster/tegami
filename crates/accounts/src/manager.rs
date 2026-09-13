// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unified account manager.
//!
//! Merges the accounts discovered from GOA and from EDS into a single
//! [`Account`] list, de-duplicated on the e-mail address, and tracks
//! changes: every signal emitted by the two ObjectManager services
//! (interfaces added/removed, properties changed) triggers a fresh
//! enumeration and the diff is exposed as [`AccountChange`] events.

use std::collections::HashMap;
use std::pin::Pin;

use futures_core::Stream;
use tokio::sync::mpsc;
use zbus::MessageStream;
use zbus::connection::Connection;
use zbus::match_rule::MatchRule;
use zbus::message::Type;

use mail_core::account::AccountConfig;

use crate::eds::discover::{default_sources_dir, discover_mail_accounts};
use crate::goa::{MailAccount, enumerate_accounts};
use crate::model::{Account, AccountChange, EdsReference, GoaReference};

const GOA_SERVICE: &str = "org.gnome.OnlineAccounts";
const EDS_SERVICE: &str = "org.gnome.evolution.dataserver.Sources5";

pub struct AccountManager {
    connection: Connection,
}

impl AccountManager {
    pub fn new(connection: Connection) -> Self {
        Self { connection }
    }

    pub async fn accounts(&self) -> Vec<Account> {
        refresh(&self.connection).await
    }

    pub async fn changes(&self) -> zbus::Result<mpsc::Receiver<AccountChange>> {
        let (refresh_tx, mut refresh_rx) = mpsc::channel(16);
        let (events_tx, events_rx) = mpsc::channel(32);

        let goa_stream = signal_stream(&self.connection, GOA_SERVICE).await?;
        let eds_stream = signal_stream(&self.connection, EDS_SERVICE).await?;
        let connection = self.connection.clone();
        tokio::spawn(async move {
            let goa_tx = refresh_tx.clone();
            let eds_tx = refresh_tx;
            tokio::spawn(forward(goa_stream, goa_tx));
            tokio::spawn(forward(eds_stream, eds_tx));

            let mut snapshot = refresh(&connection).await;
            while refresh_rx.recv().await.is_some() {
                let current = refresh(&connection).await;
                for change in diff(&snapshot, &current) {
                    if events_tx.send(change).await.is_err() {
                        return;
                    }
                }
                snapshot = current;
            }
        });
        Ok(events_rx)
    }
}

pub fn merge(goa: &[MailAccount], eds: &[AccountConfig]) -> Vec<Account> {
    let mut eds_by_email: HashMap<String, &AccountConfig> = HashMap::new();
    for config in eds {
        eds_by_email
            .entry(config.email_address.to_lowercase())
            .or_insert(config);
    }
    let mut accounts: Vec<Account> = goa
        .iter()
        .map(|goa| {
            let key = goa.config.email_address.to_lowercase();
            let mut account = Account {
                config: goa.config.clone(),
                goa: Some(GoaReference {
                    account_path: goa.path.to_string(),
                    provider_type: goa.config.provider_type.clone().unwrap_or_default(),
                    oauth2: goa.oauth2,
                }),
                eds: None,
            };
            if let Some(eds) = eds_by_email.remove(&key) {
                account.config = merge_config(&goa.config, eds);
                account.eds = Some(EdsReference {
                    uid: eds.id.clone(),
                });
            }
            account
        })
        .collect();
    accounts.extend(eds_by_email.into_values().map(|config| Account {
        config: config.clone(),
        goa: None,
        eds: Some(EdsReference {
            uid: config.id.clone(),
        }),
    }));
    accounts.sort_by(|a, b| a.id().cmp(b.id()));
    accounts
}

fn merge_config(goa: &AccountConfig, eds: &AccountConfig) -> AccountConfig {
    AccountConfig {
        id: goa.id.clone(),
        name: if eds.name.is_empty() {
            goa.name.clone()
        } else {
            eds.name.clone()
        },
        email_address: eds.email_address.clone(),
        provider_type: goa
            .provider_type
            .clone()
            .or_else(|| eds.provider_type.clone()),
        is_temporary: goa.is_temporary || eds.is_temporary,
        imap: eds.imap.clone().or_else(|| goa.imap.clone()),
        smtp: eds.smtp.clone().or_else(|| goa.smtp.clone()),
    }
}

async fn refresh(connection: &Connection) -> Vec<Account> {
    let goa = enumerate_accounts(connection).await.unwrap_or_default();
    let eds = discover_mail_accounts(connection, default_sources_dir())
        .await
        .unwrap_or_default();
    merge(&goa, &eds)
}

fn diff(previous: &[Account], current: &[Account]) -> Vec<AccountChange> {
    let previous: HashMap<&str, &Account> = previous
        .iter()
        .map(|account| (account.id(), account))
        .collect();
    let current: HashMap<&str, &Account> = current
        .iter()
        .map(|account| (account.id(), account))
        .collect();
    let mut changes = Vec::new();
    for (id, account) in &current {
        if !previous.contains_key(id) {
            changes.push(AccountChange::Added((*account).clone()));
        }
    }
    for id in previous.keys() {
        if !current.contains_key(id) {
            changes.push(AccountChange::Removed(id.to_string()));
        }
    }
    for (id, account) in &current {
        if let Some(previous) = previous.get(id)
            && previous != account
        {
            changes.push(AccountChange::Modified((*account).clone()));
        }
    }
    changes
}

async fn signal_stream(connection: &Connection, sender: &str) -> zbus::Result<MessageStream> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(sender)?
        .build();
    MessageStream::for_match_rule(rule, connection, None).await
}

async fn forward(mut stream: MessageStream, tx: mpsc::Sender<()>) {
    loop {
        let message = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await;
        if message.is_none() || tx.send(()).await.is_err() {
            break;
        }
    }
}
