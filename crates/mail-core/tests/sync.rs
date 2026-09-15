// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::FakeBackend;
use mail_core::envelope::FlagChange;
use mail_core::store::{AccountRecord, AccountSource, AuthKind, Store};
use mail_core::sync::{
    EnvelopeWindow, fetch_body, queue_delete, queue_move, queue_set_flags, replay_pending,
    sync_account, sync_folder,
};
use tempfile::TempDir;

fn account() -> AccountRecord {
    AccountRecord {
        id: None,
        source: AccountSource::Goa,
        external_id: "account_1".to_string(),
        email: "me@example.org".to_string(),
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

async fn open_store(backend: &mut FakeBackend) -> (Store, i64) {
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    sync_account(backend, &store, account_id, EnvelopeWindow::new(10))
        .await
        .unwrap();
    (store, account_id)
}

async fn folder_record(
    store: &Store,
    account_id: i64,
    name: &str,
) -> mail_core::store::FolderRecord {
    store
        .folders(account_id)
        .await
        .unwrap()
        .into_iter()
        .find(|folder| folder.name == name)
        .unwrap()
}

const MULTIPART: &str = concat!(
    "From: Sender <sender@example.org>\r\n",
    "To: me@example.org\r\n",
    "Subject: greeting\r\n",
    "MIME-Version: 1.0\r\n",
    "Content-Type: multipart/mixed; boundary=\"BOUND\"\r\n",
    "\r\n",
    "--BOUND\r\n",
    "Content-Type: text/plain; charset=\"utf-8\"\r\n",
    "\r\n",
    "Hello world\r\n",
    "--BOUND\r\n",
    "Content-Type: application/pdf; name=\"doc.pdf\"\r\n",
    "Content-Disposition: attachment; filename=\"doc.pdf\"\r\n",
    "Content-Transfer-Encoding: base64\r\n",
    "\r\n",
    "SGVsbG8=\r\n",
    "--BOUND--\r\n",
);

#[test]
fn envelope_window_selects_newest() {
    let window = EnvelopeWindow::new(3);
    assert_eq!(window.select(&[1, 2, 3, 4, 5]), &[3, 4, 5]);
    assert_eq!(window.select(&[1, 2]), &[1, 2]);
    assert!(window.select(&[]).is_empty());
    assert_eq!(EnvelopeWindow::default().max_messages, 500);
}

#[tokio::test]
async fn initial_sync_persists_window() {
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let mut backend = FakeBackend::with_folders(&[("INBOX", 5), ("Archive", 1)]);

    let reports = sync_account(&mut backend, &store, account_id, EnvelopeWindow::new(2))
        .await
        .unwrap();
    assert_eq!(reports.len(), 2);
    assert!(reports.iter().all(|report| !report.incremental));

    let folders = store.folders(account_id).await.unwrap();
    let inbox = folders
        .iter()
        .find(|folder| folder.name == "INBOX")
        .unwrap();
    let inbox_id = inbox.id.unwrap();
    let inbox_report = reports
        .iter()
        .find(|report| report.folder_id == inbox_id)
        .expect("inbox report");
    // The window limits the initial fetch to the two newest messages.
    assert_eq!(inbox_report.new_uids, vec![4, 5]);

    assert_eq!(inbox.special_use, Some(mail_core::store::SpecialUse::Inbox));
    assert!(inbox.highestmodseq.is_some());

    let messages = store.messages(inbox_id).await.unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].uid, 4);
    assert_eq!(messages[1].uid, 5);
    assert_eq!(messages[1].subject, "INBOX 5");
    assert_eq!(messages[1].from_addr.as_deref(), Some("sender@example.org"));
    assert_eq!(
        messages[1].to_addrs.as_deref(),
        Some(r#"["me@example.org"]"#)
    );
    assert_eq!(messages[1].size, Some(105));
    assert_eq!(messages[1].date_recv, Some(1_700_000_005));
    assert_eq!(messages[1].modseq, Some(5));
    assert_eq!(
        messages[0].flags & mail_core::store::FLAG_SEEN,
        mail_core::store::FLAG_SEEN
    );
    assert_eq!(messages[1].flags & mail_core::store::FLAG_SEEN, 0);

    let archive = folders
        .iter()
        .find(|folder| folder.name == "Archive")
        .unwrap();
    assert_eq!(store.messages(archive.id.unwrap()).await.unwrap().len(), 1);
}

#[tokio::test]
async fn incremental_sync_applies_delta_and_vanished() {
    let mut backend = FakeBackend::with_folders(&[("INBOX", 3)]);
    let (store, account_id) = open_store(&mut backend).await;
    let inbox_id = folder_record(&store, account_id, "INBOX").await.id.unwrap();
    assert_eq!(store.message_uids(inbox_id).await.unwrap(), vec![1, 2, 3]);

    let new_uid = backend.add_message("INBOX", "brand new", b"");
    backend.mark_seen("INBOX", 1);
    backend.expunge("INBOX", 2);

    let folder = folder_record(&store, account_id, "INBOX").await;
    let report = sync_folder(&mut backend, &store, &folder, EnvelopeWindow::new(10))
        .await
        .unwrap();
    assert!(report.incremental);
    assert_eq!(report.changed, 2);
    assert_eq!(report.vanished, 1);
    assert_eq!(report.new_uids, vec![new_uid]);

    assert_eq!(
        store.message_uids(inbox_id).await.unwrap(),
        vec![1, 3, new_uid]
    );
    assert!(
        store.message(inbox_id, 1).await.unwrap().unwrap().flags & mail_core::store::FLAG_SEEN != 0
    );
    assert!(store.message(inbox_id, 2).await.unwrap().is_none());
    assert_eq!(
        store
            .message(inbox_id, new_uid)
            .await
            .unwrap()
            .unwrap()
            .subject,
        "brand new"
    );
}

#[tokio::test]
async fn condstore_without_qresync_rescans_vanished() {
    let mut backend = FakeBackend::with_folders(&[("INBOX", 3)]);
    backend.qresync = false;
    let (store, account_id) = open_store(&mut backend).await;
    let inbox_id = folder_record(&store, account_id, "INBOX").await.id.unwrap();

    backend.mark_seen("INBOX", 3);
    backend.expunge("INBOX", 1);

    let folder = folder_record(&store, account_id, "INBOX").await;
    let report = sync_folder(&mut backend, &store, &folder, EnvelopeWindow::new(10))
        .await
        .unwrap();
    assert!(report.incremental);
    assert_eq!(report.changed, 1);
    assert_eq!(report.vanished, 1);
    assert_eq!(store.message_uids(inbox_id).await.unwrap(), vec![2, 3]);
}

#[tokio::test]
async fn without_condstore_uses_uid_rescan() {
    let mut backend = FakeBackend::with_folders(&[("INBOX", 3)]);
    backend.condstore = false;
    backend.qresync = false;
    let (store, account_id) = open_store(&mut backend).await;
    let inbox_id = folder_record(&store, account_id, "INBOX").await.id.unwrap();
    assert_eq!(
        folder_record(&store, account_id, "INBOX")
            .await
            .highestmodseq,
        None
    );

    let new_uid = backend.add_message("INBOX", "fresh", b"");
    backend.expunge("INBOX", 2);

    let folder = folder_record(&store, account_id, "INBOX").await;
    let report = sync_folder(&mut backend, &store, &folder, EnvelopeWindow::new(10))
        .await
        .unwrap();
    assert!(!report.incremental);
    assert_eq!(report.changed, 1);
    assert_eq!(report.vanished, 1);
    assert_eq!(
        store.message_uids(inbox_id).await.unwrap(),
        vec![1, 3, new_uid]
    );
}

#[tokio::test]
async fn uidvalidity_change_invalidates_and_resyncs() {
    let mut backend = FakeBackend::with_folders(&[("INBOX", 3)]);
    let (store, account_id) = open_store(&mut backend).await;
    let inbox_id = folder_record(&store, account_id, "INBOX").await.id.unwrap();
    assert_eq!(store.message_uids(inbox_id).await.unwrap(), vec![1, 2, 3]);

    backend.rebuild("INBOX", 2, 2);

    let folder = folder_record(&store, account_id, "INBOX").await;
    assert_eq!(folder.uidvalidity, Some(1));
    let report = sync_folder(&mut backend, &store, &folder, EnvelopeWindow::new(10))
        .await
        .unwrap();
    assert!(report.uidvalidity_changed);
    assert!(!report.incremental);
    assert_eq!(report.changed, 2);
    assert_eq!(report.vanished, 0);

    assert_eq!(store.message_uids(inbox_id).await.unwrap(), vec![1, 2]);
    assert_eq!(
        folder_record(&store, account_id, "INBOX").await.uidvalidity,
        Some(2)
    );
}

#[tokio::test]
async fn fetch_body_stores_raw_and_attachments() {
    let mut backend = FakeBackend::with_folders(&[("INBOX", 1)]);
    let (store, account_id) = open_store(&mut backend).await;
    let inbox_id = folder_record(&store, account_id, "INBOX").await.id.unwrap();
    backend.set_body("INBOX", 1, MULTIPART.as_bytes());

    let dir = TempDir::new().unwrap();
    let fetch = fetch_body(&mut backend, &store, "INBOX", inbox_id, 1, dir.path())
        .await
        .unwrap();
    assert_eq!(fetch.attachments, 1);

    let raw = std::fs::read(&fetch.raw_path).unwrap();
    assert!(String::from_utf8_lossy(&raw).contains("doc.pdf"));

    let message = store.message(inbox_id, 1).await.unwrap().unwrap();
    assert_eq!(message.body_state, mail_core::store::BodyState::Full);
    assert!(message.has_attach);
    assert_eq!(
        message.raw_path.as_deref(),
        Some(fetch.raw_path.to_str().unwrap())
    );

    let attachments = store.attachments(fetch.message_id).await.unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].filename.as_deref(), Some("doc.pdf"));
    assert_eq!(attachments[0].mime_type.as_deref(), Some("application/pdf"));

    let hits = store
        .search(&mail_core::store::fts_query("Hello"), 10)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].message.uid, 1);
}

#[tokio::test]
async fn pending_ops_replay_and_drain() {
    let mut backend = FakeBackend::with_folders(&[("INBOX", 2), ("Archive", 1)]);
    let (store, account_id) = open_store(&mut backend).await;
    let folders = store.folders(account_id).await.unwrap();
    let inbox_id = folders
        .iter()
        .find(|folder| folder.name == "INBOX")
        .unwrap()
        .id
        .unwrap();
    let archive_id = folders
        .iter()
        .find(|folder| folder.name == "Archive")
        .unwrap()
        .id
        .unwrap();

    queue_set_flags(
        &store,
        account_id,
        inbox_id,
        1,
        FlagChange {
            seen: Some(true),
            ..FlagChange::default()
        },
    )
    .await
    .unwrap();
    queue_move(&store, account_id, inbox_id, 2, archive_id)
        .await
        .unwrap();
    queue_delete(&store, account_id, inbox_id, 1).await.unwrap();
    assert_eq!(store.pending_ops(account_id).await.unwrap().len(), 3);

    let report = replay_pending(&mut backend, &store, account_id)
        .await
        .unwrap();
    assert_eq!(report.replayed, 3);
    assert_eq!(report.dropped, 0);
    assert!(store.pending_ops(account_id).await.unwrap().is_empty());

    assert_eq!(backend.flag_calls.len(), 2);
    assert_eq!(backend.flag_calls[0].0, "INBOX");
    assert_eq!(backend.flag_calls[0].1, vec![1]);
    assert_eq!(backend.flag_calls[0].2.seen, Some(true));
    assert_eq!(backend.flag_calls[1].2.deleted, Some(true));
    assert_eq!(backend.move_calls.len(), 1);
    assert_eq!(backend.move_calls[0].1, "Archive");
    assert_eq!(backend.move_calls[0].2, vec![2]);
}

#[tokio::test]
async fn pending_ops_drop_unresolvable_folder() {
    let mut backend = FakeBackend::with_folders(&[("INBOX", 1)]);
    let (store, account_id) = open_store(&mut backend).await;

    queue_delete(&store, account_id, 9999, 1).await.unwrap();
    let report = replay_pending(&mut backend, &store, account_id)
        .await
        .unwrap();
    assert_eq!(report.replayed, 0);
    assert_eq!(report.dropped, 1);
    assert!(store.pending_ops(account_id).await.unwrap().is_empty());
}
