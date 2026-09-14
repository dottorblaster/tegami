// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use imap::ImapBackend;
use mail_core::account::{AccountConfig, ImapConfig};
use mail_core::envelope::MessageFlags;
use mail_core::store::{AccountRecord, AccountSource, AuthKind, BodyState, Store, fts_query};
use mail_core::sync::{EnvelopeWindow, IdleEvent, IdleWorker, fetch_body, sync_account};
use mail_core::{Credential, MailBackend};
use tempfile::TempDir;
use tokio::sync::mpsc;

fn greenmail_host() -> Option<String> {
    std::env::var("TEGAMI_GREENMAIL_HOST").ok()
}

fn greenmail_config(host: &str) -> AccountConfig {
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
            port: Some(3143),
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
        .connect(
            &greenmail_config(host),
            &Credential::Password("pass".to_string()),
        )
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

async fn setup(host: &str) -> (ImapBackend, Store, i64) {
    let backend = connect(host).await;
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account_record()).await.unwrap();
    (backend, store, account_id)
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
async fn sync_uses_uid_rescan_fallback_without_condstore() {
    let Some(host) = greenmail_host() else {
        eprintln!("skipping: set TEGAMI_GREENMAIL_HOST to run GreenMail tests");
        return;
    };
    let (mut backend, store, account_id) = setup(&host).await;
    assert!(!backend.supports_condstore());
    assert!(!backend.supports_qresync());

    let subject = unique("tegami-sync");
    backend
        .append("INBOX", MessageFlags::default(), &raw_message(&subject))
        .await
        .unwrap();

    let reports = sync_account(&mut backend, &store, account_id, EnvelopeWindow::new(1000))
        .await
        .unwrap();
    assert!(reports.iter().all(|report| !report.incremental));
    let inbox_id = inbox(&store, account_id).await;
    let messages = store.messages(inbox_id).await.unwrap();
    assert!(messages.iter().any(|message| message.subject == subject));

    let folder = store
        .folders(account_id)
        .await
        .unwrap()
        .into_iter()
        .find(|folder| folder.name == "INBOX")
        .unwrap();
    let second =
        mail_core::sync::sync_folder(&mut backend, &store, &folder, EnvelopeWindow::new(1000))
            .await
            .unwrap();
    assert!(!second.incremental);
    assert_eq!(second.changed, 0);

    let second_subject = unique("tegami-sync");
    backend
        .append(
            "INBOX",
            MessageFlags::default(),
            &raw_message(&second_subject),
        )
        .await
        .unwrap();
    let third =
        mail_core::sync::sync_folder(&mut backend, &store, &folder, EnvelopeWindow::new(1000))
            .await
            .unwrap();
    assert!(!third.incremental);
    assert!(third.changed >= 1);
    let messages = store.messages(inbox_id).await.unwrap();
    assert!(
        messages
            .iter()
            .any(|message| message.subject == second_subject)
    );
}

#[tokio::test]
async fn fetch_body_caches_raw_and_indexes_text() {
    let Some(host) = greenmail_host() else {
        eprintln!("skipping: set TEGAMI_GREENMAIL_HOST to run GreenMail tests");
        return;
    };
    let (mut backend, store, account_id) = setup(&host).await;

    let subject = unique("tegami-body");
    backend
        .append("INBOX", MessageFlags::default(), &raw_message(&subject))
        .await
        .unwrap();
    sync_account(&mut backend, &store, account_id, EnvelopeWindow::new(1000))
        .await
        .unwrap();

    let inbox_id = inbox(&store, account_id).await;
    let target = store
        .messages(inbox_id)
        .await
        .unwrap()
        .into_iter()
        .find(|message| message.subject == subject)
        .unwrap();
    assert_eq!(target.body_state, BodyState::None);

    let dir = TempDir::new().unwrap();
    let fetched = fetch_body(
        &mut backend,
        &store,
        "INBOX",
        inbox_id,
        target.uid,
        dir.path(),
    )
    .await
    .unwrap();
    assert_eq!(fetched.attachments, 0);

    let stored = store.message(inbox_id, target.uid).await.unwrap().unwrap();
    assert_eq!(stored.body_state, BodyState::Full);
    assert!(stored.raw_path.as_deref().is_some());
    assert_eq!(
        std::fs::read(stored.raw_path.as_deref().unwrap()).unwrap(),
        raw_message(&subject)
    );

    let hits = store.search(&fts_query("Body"), 100).await.unwrap();
    assert!(
        hits.iter()
            .any(|hit| hit.message.uid == target.uid && hit.snippet.contains("Body"))
    );
}

#[tokio::test]
async fn idle_detects_new_mail() {
    let Some(host) = greenmail_host() else {
        eprintln!("skipping: set TEGAMI_GREENMAIL_HOST to run GreenMail tests");
        return;
    };
    let mut sender = connect(&host).await;
    let (event_tx, mut event_rx) = mpsc::channel(4);
    let worker = IdleWorker::spawn(connect(&host).await, "INBOX", event_tx);

    tokio::time::sleep(Duration::from_secs(2)).await;
    let subject = unique("tegami-idle");
    sender
        .append("INBOX", MessageFlags::default(), &raw_message(&subject))
        .await
        .unwrap();

    let event = tokio::time::timeout(Duration::from_secs(30), event_rx.recv())
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
}
