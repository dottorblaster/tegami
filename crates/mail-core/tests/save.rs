// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use mail_core::envelope::MessageFlags;
use mail_core::sync::{draft_flags, save_draft, save_sent, sent_flags};
use mail_core::{
    MailBackend,
    store::{AccountRecord, AccountSource, AuthKind, OutboxRecord, OutboxState, SpecialUse, Store},
};
use tempfile::TempDir;

use common::FakeBackend;

mod common;

fn account() -> AccountRecord {
    AccountRecord {
        id: None,
        source: AccountSource::Goa,
        external_id: "account_1".to_string(),
        email: "user@example.org".to_string(),
        display_name: None,
        imap_host: None,
        imap_port: None,
        imap_security: None,
        smtp_host: None,
        smtp_port: None,
        smtp_security: None,
        auth_kind: AuthKind::Password,
        username: None,
    }
}

fn sent_folder(account_id: i64, name: &str) -> mail_core::store::FolderRecord {
    mail_core::store::FolderRecord {
        id: None,
        account_id,
        name: name.to_string(),
        display_name: None,
        special_use: Some(SpecialUse::Sent),
        uidvalidity: None,
        uidnext: None,
        highestmodseq: None,
        unread_count: 0,
        total_count: 0,
        subscribed: true,
    }
}

async fn seeded() -> (Store, TempDir, i64) {
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    (store, TempDir::new().unwrap(), account_id)
}

fn written_raw(dir: &Path, bytes: &[u8]) -> String {
    let path = dir.join("raw.eml");
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

#[tokio::test]
async fn save_sent_appends_and_clears_the_outbox() {
    let (store, dir, account_id) = seeded().await;
    let folder_id = store
        .upsert_folder(sent_folder(account_id, "Sent"))
        .await
        .unwrap();
    let raw = b"From: me@example.org\r\nTo: you@example.org\r\nSubject: hi\r\n\r\nbody\r\n";
    let raw_path = written_raw(dir.path(), raw);
    let _id = store
        .enqueue_outbox(OutboxRecord {
            state: OutboxState::Sent,
            ..OutboxRecord::queued(account_id, raw_path.clone(), 1_700_000_000)
        })
        .await
        .unwrap();
    let mut backend = FakeBackend::with_folders(&[("Sent", 0)]);
    let _ = folder_id;

    let report = save_sent(&mut backend, &store, account_id).await.unwrap();
    assert_eq!(report.saved, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.skipped, 0);

    assert!(store.outbox(account_id).await.unwrap().is_empty());
    assert!(!Path::new(&raw_path).exists());
    assert_eq!(backend.uids("Sent").await.unwrap(), vec![1]);
    let ((folder, uid), body) = &backend.bodies[0];
    assert_eq!(folder, "Sent");
    assert_eq!(*uid, 1);
    assert_eq!(body, &raw);
}

#[tokio::test]
async fn save_sent_skips_entries_when_there_is_no_sent_folder() {
    let (store, dir, account_id) = seeded().await;
    let raw = b"Subject: hi\r\n\r\nbody\r\n";
    let raw_path = written_raw(dir.path(), raw);
    store
        .enqueue_outbox(OutboxRecord {
            state: OutboxState::Sent,
            ..OutboxRecord::queued(account_id, raw_path, 1_700_000_000)
        })
        .await
        .unwrap();
    let mut backend = FakeBackend::new();

    let report = save_sent(&mut backend, &store, account_id).await.unwrap();
    assert_eq!(report.saved, 0);
    assert_eq!(report.skipped, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(store.outbox(account_id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn save_sent_drops_entries_with_a_missing_raw_file() {
    let (store, _, account_id) = seeded().await;
    store
        .enqueue_outbox(OutboxRecord {
            state: OutboxState::Sent,
            ..OutboxRecord::queued(account_id, "gone.eml".to_string(), 1_700_000_000)
        })
        .await
        .unwrap();
    store
        .upsert_folder(sent_folder(account_id, "Sent"))
        .await
        .unwrap();
    let mut backend = FakeBackend::with_folders(&[("Sent", 0)]);

    let report = save_sent(&mut backend, &store, account_id).await.unwrap();
    assert_eq!(report.saved, 0);
    assert_eq!(report.failed, 1);
    assert!(store.outbox(account_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn save_sent_keeps_entries_when_the_server_rejects() {
    let (store, dir, account_id) = seeded().await;
    let raw_path = written_raw(dir.path(), b"Subject: hi\r\n\r\nbody\r\n");
    let id = store
        .enqueue_outbox(OutboxRecord {
            state: OutboxState::Sent,
            ..OutboxRecord::queued(account_id, raw_path, 1_700_000_000)
        })
        .await
        .unwrap();
    store
        .upsert_folder(sent_folder(account_id, "Sent"))
        .await
        .unwrap();
    let mut backend = FakeBackend::new(); // no "Sent" folder on the server

    let report = save_sent(&mut backend, &store, account_id).await.unwrap();
    assert_eq!(report.saved, 0);
    assert_eq!(report.failed, 1);
    let remaining = store.outbox(account_id).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, Some(id));
    assert_eq!(remaining[0].state, OutboxState::Sent);
}

#[tokio::test]
async fn delivery_flags_split_sent_and_drafts() {
    assert!(sent_flags().seen);
    assert!(!sent_flags().draft);
    assert!(draft_flags().seen);
    assert!(draft_flags().draft);
    assert_eq!(
        sent_flags(),
        MessageFlags {
            seen: true,
            ..MessageFlags::default()
        }
    );
}

#[tokio::test]
async fn save_draft_appends_and_replaces_the_previous_version() {
    let mut backend = FakeBackend::with_folders(&[("Drafts", 2)]);
    let raw = b"From: me@example.org\r\nTo: you@example.org\r\nSubject: drafty\r\n\r\nv2\r\n";

    let uid = save_draft(&mut backend, "Drafts", raw, Some(2))
        .await
        .unwrap();
    assert_eq!(uid, 3);

    let uids = backend.uids("Drafts").await.unwrap();
    assert_eq!(uids, vec![1, 3]);
    let draft = backend
        .fetch_envelopes("Drafts", &[3])
        .await
        .unwrap()
        .remove(0);
    assert!(draft.flags.draft);
    assert!(draft.flags.seen);
    assert!(
        backend
            .bodies
            .iter()
            .any(|((folder, uid), body)| folder == "Drafts" && *uid == 3 && body == raw)
    );
}

#[tokio::test]
async fn save_draft_without_replacement_keeps_previous_versions() {
    let mut backend = FakeBackend::with_folders(&[("Drafts", 1)]);
    let uid = save_draft(&mut backend, "Drafts", b"Subject: v1\r\n\r\nbody\r\n", None)
        .await
        .unwrap();
    assert_eq!(uid, 2);
    assert_eq!(backend.uids("Drafts").await.unwrap(), vec![1, 2]);
}
