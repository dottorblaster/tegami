// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use mail_core::store::{
    AccountRecord, AccountSource, AttachmentRecord, AuthKind, BodyState, FLAG_FLAGGED, FLAG_SEEN,
    FolderRecord, MessageRecord, OpKind, PendingOpRecord, Security, SpecialUse, Store,
    bits_to_flags, flags_to_bits, fts_query,
};
use tempfile::TempDir;

fn account() -> AccountRecord {
    AccountRecord {
        id: None,
        source: AccountSource::Goa,
        external_id: "account_1".to_string(),
        email: "user@example.org".to_string(),
        display_name: Some("Example User".to_string()),
        imap_host: Some("imap.example.org".to_string()),
        imap_port: Some(993),
        imap_security: Some(Security::Ssl),
        smtp_host: Some("smtp.example.org".to_string()),
        smtp_port: Some(465),
        smtp_security: Some(Security::Ssl),
        auth_kind: AuthKind::Password,
        username: Some("user@example.org".to_string()),
    }
}

fn folder(account_id: i64) -> FolderRecord {
    FolderRecord {
        id: None,
        account_id,
        name: "INBOX".to_string(),
        display_name: Some("Inbox".to_string()),
        special_use: Some(SpecialUse::Inbox),
        uidvalidity: Some(42),
        uidnext: Some(3),
        highestmodseq: None,
        unread_count: 1,
        total_count: 2,
        subscribed: true,
    }
}

fn message(folder_id: i64, uid: u32) -> MessageRecord {
    MessageRecord {
        id: None,
        folder_id,
        uid,
        modseq: None,
        message_id: Some(format!("<{uid}@example.org>")),
        thread_id: None,
        subject: format!("subject {uid}"),
        from_addr: Some("sender@example.org".to_string()),
        from_name: Some("Sender".to_string()),
        to_addrs: Some(r#"["user@example.org"]"#.to_string()),
        cc_addrs: None,
        date_sent: Some(1_700_000_000),
        date_recv: Some(1_700_000_001),
        in_reply_to: None,
        refs: None,
        flags: flags_to_bits(mail_core::envelope::MessageFlags {
            seen: true,
            ..Default::default()
        }),
        has_attach: false,
        size: Some(1234),
        structure: None,
        raw_path: None,
        body_state: BodyState::None,
    }
}

#[tokio::test]
async fn migrations_create_schema() {
    let store = Store::open(":memory:").unwrap();
    // The worker applies migrations at open; any query proves the schema exists.
    let accounts = store.accounts().await.unwrap();
    assert!(accounts.is_empty());
}

#[tokio::test]
async fn migrations_are_idempotent() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("tegami.db");

    let store = Store::open(&path).unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    assert!(account_id > 0);
    drop(store);

    let reopened = Store::open(&path).unwrap();
    let accounts = reopened.accounts().await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].external_id, "account_1");
    assert_eq!(accounts[0].email, "user@example.org");
}

#[tokio::test]
async fn account_upsert_and_query() {
    let store = Store::open(":memory:").unwrap();
    let id = store.upsert_account(account()).await.unwrap();

    let found = store.account("goa", "account_1").await.unwrap().unwrap();
    assert_eq!(found.id, Some(id));
    assert_eq!(found.source, AccountSource::Goa);
    assert_eq!(found.imap_security, Some(Security::Ssl));
    assert_eq!(found.smtp_port, Some(465));
    assert_eq!(found.auth_kind, AuthKind::Password);
    assert_eq!(found.username.as_deref(), Some("user@example.org"));

    let mut updated = account();
    updated.display_name = Some("Renamed".to_string());
    updated.auth_kind = AuthKind::OAuth2;
    let same_id = store.upsert_account(updated).await.unwrap();
    assert_eq!(same_id, id);

    let accounts = store.accounts().await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].display_name.as_deref(), Some("Renamed"));
    assert_eq!(accounts[0].auth_kind, AuthKind::OAuth2);

    let missing = store.account("eds", "nope").await.unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn folder_and_message_round_trip() {
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let folder_id = store.upsert_folder(folder(account_id)).await.unwrap();

    let folders = store.folders(account_id).await.unwrap();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].name, "INBOX");
    assert_eq!(folders[0].special_use, Some(SpecialUse::Inbox));
    assert_eq!(folders[0].uidvalidity, Some(42));
    assert_eq!(folders[0].unread_count, 1);
    assert!(folders[0].subscribed);

    store.upsert_message(message(folder_id, 1)).await.unwrap();
    store.upsert_message(message(folder_id, 2)).await.unwrap();

    let messages = store.messages(folder_id).await.unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].uid, 1);
    assert_eq!(messages[0].subject, "subject 1");
    assert_eq!(messages[0].from_addr.as_deref(), Some("sender@example.org"));
    assert_eq!(messages[0].flags, FLAG_SEEN);
    assert_eq!(messages[0].body_state, BodyState::None);

    let found = store.message(folder_id, 2).await.unwrap().unwrap();
    assert_eq!(found.subject, "subject 2");
    assert!(store.message(folder_id, 9).await.unwrap().is_none());

    let mut updated = message(folder_id, 1);
    updated.subject = "edited".to_string();
    updated.raw_path = Some("/tmp/1.eml".to_string());
    updated.body_state = BodyState::Full;
    let message_id = store.upsert_message(updated).await.unwrap();
    assert!(message_id > 0);
    let messages = store.messages(folder_id).await.unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].subject, "edited");
    assert_eq!(messages[0].raw_path.as_deref(), Some("/tmp/1.eml"));
    assert_eq!(messages[0].body_state, BodyState::Full);
}

#[tokio::test]
async fn sync_folders_persists_and_prunes() {
    use mail_core::folder::{Folder, FolderRole};

    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();

    let inbox = Folder {
        id: "INBOX".to_string(),
        name: "Inbox".to_string(),
        role: FolderRole::Inbox,
    };
    let important = Folder {
        id: "[Gmail]/Important".to_string(),
        name: "Important".to_string(),
        role: FolderRole::Important,
    };
    let work = Folder {
        id: "Work".to_string(),
        name: "Work".to_string(),
        role: FolderRole::Other,
    };

    let mut seeded = FolderRecord::from_folder(account_id, &inbox);
    seeded.uidvalidity = Some(99);
    seeded.uidnext = Some(12);
    store.upsert_folder(seeded).await.unwrap();

    let discovered = vec![inbox.clone(), important.clone(), work.clone()];
    let stored = store.sync_folders(account_id, &discovered).await.unwrap();
    assert_eq!(stored.len(), 3);

    let inbox_record = stored.iter().find(|folder| folder.name == "INBOX").unwrap();
    assert_eq!(inbox_record.special_use, Some(SpecialUse::Inbox));
    assert_eq!(inbox_record.display_name.as_deref(), Some("Inbox"));
    assert_eq!(inbox_record.uidvalidity, Some(99));
    assert_eq!(inbox_record.uidnext, Some(12));

    let important_record = stored
        .iter()
        .find(|folder| folder.name == "[Gmail]/Important")
        .unwrap();
    assert_eq!(important_record.special_use, Some(SpecialUse::Important));

    let work_record = stored.iter().find(|folder| folder.name == "Work").unwrap();
    assert_eq!(work_record.special_use, None);
    let work_id = work_record.id.unwrap();
    store.upsert_message(message(work_id, 1)).await.unwrap();

    let pruned = store
        .sync_folders(account_id, &[inbox.clone(), important.clone()])
        .await
        .unwrap();
    assert_eq!(pruned.len(), 2);
    assert!(pruned.iter().all(|folder| folder.name != "Work"));
    assert!(store.messages(work_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn set_message_flags_updates_subset() {
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let folder_id = store.upsert_folder(folder(account_id)).await.unwrap();
    for uid in 1..=3 {
        store.upsert_message(message(folder_id, uid)).await.unwrap();
    }

    let flags = FLAG_SEEN | FLAG_FLAGGED;
    store
        .set_message_flags(folder_id, &[1, 3], flags)
        .await
        .unwrap();

    let messages = store.messages(folder_id).await.unwrap();
    assert_eq!(messages[0].flags, flags);
    assert_eq!(messages[1].flags, FLAG_SEEN);
    assert_eq!(messages[2].flags, flags);
    assert!(bits_to_flags(messages[1].flags).seen);
    assert!(!bits_to_flags(messages[1].flags).flagged);

    store.set_message_flags(folder_id, &[], 0).await.unwrap();
}

#[tokio::test]
async fn folder_state_and_message_deletion() {
    use mail_core::folder::FolderState;

    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let folder_id = store.upsert_folder(folder(account_id)).await.unwrap();
    for uid in 1..=3 {
        store.upsert_message(message(folder_id, uid)).await.unwrap();
    }

    store
        .set_folder_state(
            folder_id,
            FolderState {
                uid_validity: 7,
                uid_next: 10,
                exists: 3,
                recent: 0,
                unseen: Some(2),
                highest_modseq: Some(1234),
            },
        )
        .await
        .unwrap();

    let folders = store.folders(account_id).await.unwrap();
    assert_eq!(folders[0].uidvalidity, Some(7));
    assert_eq!(folders[0].uidnext, Some(10));
    assert_eq!(folders[0].highestmodseq, Some(1234));
    assert_eq!(folders[0].unread_count, 2);
    assert_eq!(folders[0].total_count, 3);

    assert_eq!(store.message_uids(folder_id).await.unwrap(), vec![1, 2, 3]);
    store.delete_messages(folder_id, &[1, 3]).await.unwrap();
    assert_eq!(store.message_uids(folder_id).await.unwrap(), vec![2]);
    store.delete_messages(folder_id, &[]).await.unwrap();

    store.clear_messages(folder_id).await.unwrap();
    assert!(store.message_uids(folder_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn stores_message_body_and_attachments() {
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let folder_id = store.upsert_folder(folder(account_id)).await.unwrap();
    let message_id = store.upsert_message(message(folder_id, 1)).await.unwrap();

    store
        .set_message_body(
            folder_id,
            1,
            "/tmp/1.eml".to_string(),
            BodyState::Full,
            true,
        )
        .await
        .unwrap();

    let stored = store.message(folder_id, 1).await.unwrap().unwrap();
    assert_eq!(stored.raw_path.as_deref(), Some("/tmp/1.eml"));
    assert_eq!(stored.body_state, BodyState::Full);
    assert!(stored.has_attach);

    store
        .replace_attachments(
            message_id,
            vec![AttachmentRecord {
                id: None,
                message_id,
                part_id: "0".to_string(),
                filename: Some("doc.pdf".to_string()),
                mime_type: Some("application/pdf".to_string()),
                size: Some(5),
                content_id: None,
                disk_path: None,
            }],
        )
        .await
        .unwrap();

    let attachments = store.attachments(message_id).await.unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].filename.as_deref(), Some("doc.pdf"));
    assert_eq!(attachments[0].mime_type.as_deref(), Some("application/pdf"));
    assert_eq!(attachments[0].size, Some(5));

    store
        .replace_attachments(message_id, Vec::new())
        .await
        .unwrap();
    assert!(store.attachments(message_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn pending_ops_round_trip() {
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let folder_id = store.upsert_folder(folder(account_id)).await.unwrap();

    let id = store
        .enqueue_op(PendingOpRecord {
            id: None,
            account_id,
            kind: OpKind::SetFlags,
            folder_id: Some(folder_id),
            target_folder_id: None,
            uid: Some(7),
            payload: Some("+.-.".to_string()),
            created_at: None,
        })
        .await
        .unwrap();
    assert!(id > 0);

    let pending = store.pending_ops(account_id).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kind, OpKind::SetFlags);
    assert_eq!(pending[0].folder_id, Some(folder_id));
    assert_eq!(pending[0].uid, Some(7));
    assert_eq!(pending[0].payload.as_deref(), Some("+.-."));
    assert!(pending[0].created_at.is_some());

    store.delete_op(id).await.unwrap();
    assert!(store.pending_ops(account_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn search_indexes_headers_and_body() {
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let folder_id = store.upsert_folder(folder(account_id)).await.unwrap();
    let message_id = store.upsert_message(message(folder_id, 1)).await.unwrap();

    let hits = store.search(&fts_query("subject"), 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].message.uid, 1);
    assert_eq!(
        store.search(&fts_query("Sender"), 10).await.unwrap().len(),
        1
    );

    store
        .index_body(message_id, "pineapple pizza".to_string())
        .await
        .unwrap();
    let hits = store.search(&fts_query("pineapple"), 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].snippet.contains("pineapple"));

    let mut updated = message(folder_id, 1);
    updated.subject = "renamed".to_string();
    store.upsert_message(updated).await.unwrap();
    assert_eq!(
        store.search(&fts_query("renamed"), 10).await.unwrap().len(),
        1
    );
    assert_eq!(
        store
            .search(&fts_query("pineapple"), 10)
            .await
            .unwrap()
            .len(),
        1
    );

    store.delete_messages(folder_id, &[1]).await.unwrap();
    assert!(
        store
            .search(&fts_query("pineapple"), 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .search(&fts_query("renamed"), 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn search_empty_query_returns_no_hits() {
    let store = Store::open(":memory:").unwrap();
    assert!(store.search("", 10).await.unwrap().is_empty());
    assert!(store.search("   ", 10).await.unwrap().is_empty());
    assert!(store.search(&fts_query(""), 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn migration_backfills_fts_headers() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("tegami.db");

    let store = Store::open(&path).unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let folder_id = store.upsert_folder(folder(account_id)).await.unwrap();
    store.upsert_message(message(folder_id, 1)).await.unwrap();
    drop(store);

    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    drop(connection);

    let reopened = Store::open(&path).unwrap();
    let hits = reopened.search(&fts_query("subject"), 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].message.uid, 1);
    assert_eq!(
        reopened
            .search(&fts_query("Sender"), 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

async fn seeded_store() -> (Store, i64) {
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let folder_id = store.upsert_folder(folder(account_id)).await.unwrap();
    (store, folder_id)
}

async fn threaded_message(
    store: &Store,
    folder_id: i64,
    uid: u32,
    message_id: &str,
    in_reply_to: Option<&str>,
    references: Option<&str>,
    subject: &str,
) {
    let mut record = message(folder_id, uid);
    record.message_id = Some(message_id.to_string());
    record.in_reply_to = in_reply_to.map(str::to_string);
    record.refs = references.map(str::to_string);
    record.subject = subject.to_string();
    store.upsert_message(record).await.unwrap();
}

#[tokio::test]
async fn replies_share_the_root_thread_id() {
    let (store, folder_id) = seeded_store().await;
    threaded_message(&store, folder_id, 1, "<root@x>", None, None, "Hello").await;
    threaded_message(
        &store,
        folder_id,
        2,
        "<reply@x>",
        Some("<root@x>"),
        Some(r#"["<root@x>"]"#),
        "Re: Hello",
    )
    .await;

    let messages = store.messages(folder_id).await.unwrap();
    let root = messages.iter().find(|message| message.uid == 1).unwrap();
    let reply = messages.iter().find(|message| message.uid == 2).unwrap();
    assert_eq!(root.thread_id, root.id);
    assert_eq!(reply.thread_id, root.id);
}

#[tokio::test]
async fn threading_links_a_child_that_arrived_before_its_parent() {
    let (store, folder_id) = seeded_store().await;
    threaded_message(
        &store,
        folder_id,
        2,
        "<reply@x>",
        Some("<root@x>"),
        Some(r#"["<root@x>"]"#),
        "Re: Hello",
    )
    .await;
    threaded_message(&store, folder_id, 1, "<root@x>", None, None, "Hello").await;

    let messages = store.messages(folder_id).await.unwrap();
    let root = messages.iter().find(|message| message.uid == 1).unwrap();
    let reply = messages.iter().find(|message| message.uid == 2).unwrap();
    assert_eq!(root.thread_id, root.id);
    assert_eq!(reply.thread_id, root.id);
}

#[tokio::test]
async fn messages_without_references_fall_back_to_the_subject() {
    let (store, folder_id) = seeded_store().await;
    threaded_message(&store, folder_id, 1, "<a@x>", None, None, "Lunch?").await;
    threaded_message(&store, folder_id, 2, "<b@x>", None, None, "Re: Lunch?").await;

    let messages = store.messages(folder_id).await.unwrap();
    let thread = messages[0].thread_id;
    assert!(thread.is_some());
    assert!(messages.iter().all(|message| message.thread_id == thread));
}

#[tokio::test]
async fn thread_messages_returns_the_whole_conversation() {
    let (store, folder_id) = seeded_store().await;
    threaded_message(&store, folder_id, 1, "<root@x>", None, None, "Hello").await;
    threaded_message(
        &store,
        folder_id,
        2,
        "<reply@x>",
        Some("<root@x>"),
        Some(r#"["<root@x>"]"#),
        "Re: Hello",
    )
    .await;
    threaded_message(&store, folder_id, 3, "<other@x>", None, None, "Unrelated").await;

    let root = store.message(folder_id, 1).await.unwrap().unwrap();
    let conversation = store
        .thread_messages(root.thread_id.unwrap())
        .await
        .unwrap();
    assert_eq!(conversation.len(), 2);
    assert_eq!(conversation[0].uid, 1);
    assert_eq!(conversation[1].uid, 2);
}

#[tokio::test]
async fn folder_lookup_by_id() {
    let (store, folder_id) = seeded_store().await;

    let found = store.folder(folder_id).await.unwrap().unwrap();
    assert_eq!(found.id, Some(folder_id));
    assert_eq!(found.name, "INBOX");

    assert!(store.folder(folder_id + 1).await.unwrap().is_none());
}

#[tokio::test]
async fn upsert_account_returns_existing_id_after_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("tegami.db");

    let store = Store::open(&path).unwrap();
    let id = store.upsert_account(account()).await.unwrap();
    drop(store);

    let reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.upsert_account(account()).await.unwrap(), id);
}

#[tokio::test]
async fn upsert_folder_returns_existing_id_after_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("tegami.db");

    let store = Store::open(&path).unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let folder_id = store.upsert_folder(folder(account_id)).await.unwrap();
    drop(store);

    let reopened = Store::open(&path).unwrap();
    assert_eq!(
        reopened.upsert_folder(folder(account_id)).await.unwrap(),
        folder_id
    );
}

#[tokio::test]
async fn prune_accounts_removes_unlisted_accounts_and_cascades() {
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let folder_id = store.upsert_folder(folder(account_id)).await.unwrap();
    store.upsert_message(message(folder_id, 1)).await.unwrap();

    store
        .prune_accounts(&[("goa".to_string(), "account_1".to_string())])
        .await
        .unwrap();
    assert_eq!(store.accounts().await.unwrap().len(), 1);

    store
        .prune_accounts(&[("goa".to_string(), "account_2".to_string())])
        .await
        .unwrap();
    assert!(store.accounts().await.unwrap().is_empty());
    assert!(store.folders(account_id).await.unwrap().is_empty());
    assert!(
        store
            .search(&fts_query("subject"), 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn remote_content_allowlist_round_trip() {
    let store = Store::open(":memory:").unwrap();
    assert!(store.remote_content_senders().await.unwrap().is_empty());

    store
        .allow_remote_content("  Ada@Lovelace.dev ")
        .await
        .unwrap();
    store
        .allow_remote_content("ada@lovelace.dev")
        .await
        .unwrap();
    store.allow_remote_content("grace@navy.dev").await.unwrap();

    assert_eq!(
        store.remote_content_senders().await.unwrap(),
        vec!["ada@lovelace.dev".to_string(), "grace@navy.dev".to_string()]
    );
}
