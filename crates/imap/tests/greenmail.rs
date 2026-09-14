// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use imap::ImapBackend;
use mail_core::account::{AccountConfig, ImapConfig};
use mail_core::envelope::MessageFlags;
use mail_core::store::{AccountRecord, AccountSource, AuthKind, Store};
use mail_core::sync::{EnvelopeWindow, IdleEvent, IdleWorker, sync_account, sync_folder};
use mail_core::{Credential, MailBackend};
use tokio::sync::mpsc;

fn greenmail_host() -> Option<String> {
    std::env::var("TEGAMI_GREENMAIL_HOST").ok()
}

fn config(host: &str) -> AccountConfig {
    AccountConfig {
        id: "greenmail".to_string(),
        name: "GreenMail".to_string(),
        email_address: "user@localhost".to_string(),
        provider_type: None,
        is_temporary: false,
        imap: Some(ImapConfig {
            accept_ssl_errors: false,
            host: host.to_string(),
            use_ssl: false,
            use_tls: false,
            user_name: "user".to_string(),
        }),
        smtp: None,
    }
}

fn account_record() -> AccountRecord {
    AccountRecord {
        id: None,
        source: AccountSource::Goa,
        external_id: "greenmail".to_string(),
        email: "user@localhost".to_string(),
        display_name: None,
        imap_host: None,
        imap_port: None,
        imap_security: None,
        smtp_host: None,
        smtp_port: None,
        smtp_security: None,
        auth_kind: AuthKind::Password,
        username: Some("user".to_string()),
    }
}

async fn connect(host: &str) -> ImapBackend {
    let mut backend = ImapBackend::new();
    backend
        .connect(&config(host), &Credential::Password("pass".to_string()))
        .await
        .unwrap();
    backend
}

fn raw_message(subject: &str) -> Vec<u8> {
    format!(
        "From: Sender <sender@example.org>\r\nTo: user@localhost\r\nSubject: {subject}\r\nMessage-ID: <{subject}@example.org>\r\n\r\nBody of {subject}\r\n"
    )
    .into_bytes()
}

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}-{nanos}")
}

async fn inbox(store: &Store, account_id: i64) -> i64 {
    store
        .folders(account_id)
        .await
        .unwrap()
        .into_iter()
        .find(|folder| folder.name == "INBOX")
        .unwrap()
        .id
        .unwrap()
}

#[tokio::test]
async fn sync_qresync_fallback_and_idle_against_greenmail() {
    let Some(host) = greenmail_host() else {
        eprintln!("skipping: set TEGAMI_GREENMAIL_HOST to run GreenMail tests");
        return;
    };

    let mut backend = connect(&host).await;
    assert!(!backend.supports_condstore());
    assert!(!backend.supports_qresync());

    let subject = unique("tegami-sync");
    backend
        .append("INBOX", MessageFlags::default(), &raw_message(&subject))
        .await
        .unwrap();

    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account_record()).await.unwrap();
    sync_account(&mut backend, &store, account_id, EnvelopeWindow::new(50))
        .await
        .unwrap();

    let inbox_id = inbox(&store, account_id).await;
    let messages = store.messages(inbox_id).await.unwrap();
    assert!(messages.iter().any(|message| message.subject == subject));

    let idle_backend = connect(&host).await;
    let (sender, mut receiver) = mpsc::channel(4);
    let worker = IdleWorker::spawn(idle_backend, "INBOX", sender);

    tokio::time::sleep(Duration::from_secs(2)).await;
    let idle_subject = unique("tegami-idle");
    backend
        .append(
            "INBOX",
            MessageFlags::default(),
            &raw_message(&idle_subject),
        )
        .await
        .unwrap();

    let event = tokio::time::timeout(Duration::from_secs(30), receiver.recv())
        .await
        .expect("idle event timed out")
        .expect("idle channel closed");
    assert_eq!(
        event,
        IdleEvent::Changed {
            folder: "INBOX".to_string()
        }
    );
    worker.abort();

    let folder = store
        .folders(account_id)
        .await
        .unwrap()
        .into_iter()
        .find(|folder| folder.name == "INBOX")
        .unwrap();
    let report = sync_folder(&mut backend, &store, &folder, EnvelopeWindow::new(50))
        .await
        .unwrap();
    assert!(!report.incremental);
    assert!(report.changed >= 1);

    let messages = store.messages(inbox_id).await.unwrap();
    assert!(
        messages
            .iter()
            .any(|message| message.subject == idle_subject)
    );
}
