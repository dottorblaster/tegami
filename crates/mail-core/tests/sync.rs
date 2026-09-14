// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::{Duration, UNIX_EPOCH};

use mail_core::account::AccountConfig;
use mail_core::backend::Result;
use mail_core::envelope::{Address, Envelope, FlagChange, MessageFlags};
use mail_core::folder::{Folder, FolderDelta, FolderRole, FolderState};
use mail_core::store::{AccountRecord, AccountSource, AuthKind, Store};
use mail_core::sync::{
    EnvelopeWindow, fetch_body, queue_delete, queue_move, queue_set_flags, replay_pending,
    sync_account, sync_folder,
};
use mail_core::{Credential, MailBackend, MailError};
use tempfile::TempDir;

struct FakeBackend {
    folders: Vec<(String, Vec<Envelope>)>,
    bodies: Vec<((String, u32), Vec<u8>)>,
    vanished: Vec<(String, u32)>,
    flag_calls: Vec<(String, Vec<u32>, FlagChange)>,
    move_calls: Vec<(String, String, Vec<u32>)>,
    modseq: u64,
    uid_validity: u32,
    condstore: bool,
    qresync: bool,
}

impl FakeBackend {
    fn new(folders: &[(&str, usize)]) -> Self {
        let mut backend = Self {
            folders: Vec::new(),
            bodies: Vec::new(),
            vanished: Vec::new(),
            flag_calls: Vec::new(),
            move_calls: Vec::new(),
            modseq: 0,
            uid_validity: 1,
            condstore: true,
            qresync: true,
        };
        for (name, count) in folders {
            let mut envelopes = Vec::new();
            for uid in 1..=*count {
                backend.modseq += 1;
                envelopes.push(envelope(
                    uid as u32,
                    &format!("{name} {uid}"),
                    backend.modseq,
                ));
            }
            backend.folders.push((name.to_string(), envelopes));
        }
        backend
    }

    fn folder(&self, name: &str) -> Result<&Vec<Envelope>> {
        self.folders
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, envelopes)| envelopes)
            .ok_or_else(|| MailError::Protocol(format!("no such folder {name}")))
    }

    fn folder_mut(&mut self, name: &str) -> Option<&mut Vec<Envelope>> {
        self.folders
            .iter_mut()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, envelopes)| envelopes)
    }

    fn add_message(&mut self, folder: &str, subject: &str) -> u32 {
        let uid = self
            .folder(folder)
            .map(|envelopes| {
                envelopes
                    .iter()
                    .map(|envelope| envelope.uid)
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0)
            + 1;
        self.modseq += 1;
        let envelope = envelope(uid, subject, self.modseq);
        self.folder_mut(folder).unwrap().push(envelope);
        uid
    }

    fn set_body(&mut self, folder: &str, uid: u32, raw: &[u8]) {
        self.bodies.push(((folder.to_string(), uid), raw.to_vec()));
    }

    fn mark_seen(&mut self, folder: &str, uid: u32) {
        self.modseq += 1;
        let modseq = self.modseq;
        if let Some(target) = self
            .folder_mut(folder)
            .and_then(|envelopes| envelopes.iter_mut().find(|envelope| envelope.uid == uid))
        {
            target.flags.seen = true;
            target.modseq = Some(modseq);
        }
    }

    fn expunge(&mut self, folder: &str, uid: u32) {
        self.modseq += 1;
        if let Some(envelopes) = self.folder_mut(folder) {
            envelopes.retain(|envelope| envelope.uid != uid);
        }
        self.vanished.push((folder.to_string(), uid));
    }

    fn rebuild(&mut self, folder: &str, count: usize, uid_validity: u32) {
        self.uid_validity = uid_validity;
        let mut envelopes = Vec::new();
        for uid in 1..=count {
            self.modseq += 1;
            envelopes.push(envelope(
                uid as u32,
                &format!("{folder} {uid}"),
                self.modseq,
            ));
        }
        if let Some(existing) = self.folder_mut(folder) {
            *existing = envelopes;
        }
    }
}

fn envelope(uid: u32, subject: &str, modseq: u64) -> Envelope {
    Envelope {
        uid,
        modseq: Some(modseq),
        flags: MessageFlags {
            seen: uid.is_multiple_of(2),
            ..MessageFlags::default()
        },
        size: 100 + uid,
        subject: subject.to_string(),
        from: vec![Address {
            name: Some("Sender".to_string()),
            address: Some("sender@example.org".to_string()),
        }],
        to: vec![Address {
            name: None,
            address: Some("me@example.org".to_string()),
        }],
        cc: Vec::new(),
        date: Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000 + u64::from(uid))),
        message_id: Some(format!("<{uid}@example.org>")),
        in_reply_to: None,
        references: Vec::new(),
    }
}

impl MailBackend for FakeBackend {
    async fn connect(&mut self, _config: &AccountConfig, _credential: &Credential) -> Result<()> {
        Ok(())
    }

    fn supports_idle(&self) -> bool {
        false
    }

    fn supports_condstore(&self) -> bool {
        self.condstore
    }

    fn supports_qresync(&self) -> bool {
        self.qresync
    }

    async fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }

    async fn folders(&mut self) -> Result<Vec<Folder>> {
        Ok(self
            .folders
            .iter()
            .map(|(name, _)| Folder {
                id: name.clone(),
                name: name.clone(),
                role: if name == "INBOX" {
                    FolderRole::Inbox
                } else {
                    FolderRole::Other
                },
            })
            .collect())
    }

    async fn select(&mut self, folder: &str) -> Result<FolderState> {
        let count = self.folder(folder)?.len() as u32;
        Ok(FolderState {
            uid_validity: self.uid_validity,
            uid_next: count + 1,
            exists: count,
            recent: 0,
            unseen: None,
            highest_modseq: self.condstore.then_some(self.modseq),
        })
    }

    async fn uids(&mut self, folder: &str) -> Result<Vec<u32>> {
        let mut uids: Vec<u32> = self
            .folder(folder)?
            .iter()
            .map(|envelope| envelope.uid)
            .collect();
        uids.sort_unstable();
        Ok(uids)
    }

    async fn fetch_envelopes(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<Envelope>> {
        let folder = self.folder(folder)?;
        let mut envelopes: Vec<Envelope> = uids
            .iter()
            .filter_map(|uid| folder.iter().find(|envelope| envelope.uid == *uid))
            .cloned()
            .collect();
        envelopes.sort_by_key(|envelope| envelope.uid);
        Ok(envelopes)
    }

    async fn fetch_delta(&mut self, folder: &str, since_modseq: u64) -> Result<FolderDelta> {
        let changed = self
            .folder(folder)?
            .iter()
            .filter(|envelope| {
                envelope
                    .modseq
                    .map(|modseq| modseq > since_modseq)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        let vanished = self
            .vanished
            .iter()
            .filter(|(name, _)| name == folder)
            .map(|(_, uid)| *uid)
            .collect();
        Ok(FolderDelta { changed, vanished })
    }

    async fn fetch_message(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        self.bodies
            .iter()
            .find(|((name, candidate), _)| name == folder && *candidate == uid)
            .map(|(_, raw)| raw.clone())
            .ok_or_else(|| MailError::Protocol(format!("no body {uid} in {folder}")))
    }

    async fn set_flags(&mut self, folder: &str, uids: &[u32], change: FlagChange) -> Result<()> {
        self.flag_calls
            .push((folder.to_string(), uids.to_vec(), change));
        Ok(())
    }

    async fn move_messages(&mut self, from: &str, to: &str, uids: &[u32]) -> Result<()> {
        self.move_calls
            .push((from.to_string(), to.to_string(), uids.to_vec()));
        Ok(())
    }

    async fn copy_messages(&mut self, _from: &str, _to: &str, _uids: &[u32]) -> Result<()> {
        Ok(())
    }

    async fn append(&mut self, _folder: &str, _flags: MessageFlags, _raw: &[u8]) -> Result<u32> {
        Ok(0)
    }

    async fn idle(&mut self, _folder: &str) -> Result<()> {
        Ok(())
    }
}

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
    let mut backend = FakeBackend::new(&[("INBOX", 5), ("Archive", 1)]);

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
    assert_eq!(inbox.special_use, Some(mail_core::store::SpecialUse::Inbox));
    assert!(inbox.highestmodseq.is_some());
    let inbox_id = inbox.id.unwrap();

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
    let mut backend = FakeBackend::new(&[("INBOX", 3)]);
    let (store, account_id) = open_store(&mut backend).await;
    let inbox_id = folder_record(&store, account_id, "INBOX").await.id.unwrap();
    assert_eq!(store.message_uids(inbox_id).await.unwrap(), vec![1, 2, 3]);

    let new_uid = backend.add_message("INBOX", "brand new");
    backend.mark_seen("INBOX", 1);
    backend.expunge("INBOX", 2);

    let folder = folder_record(&store, account_id, "INBOX").await;
    let report = sync_folder(&mut backend, &store, &folder, EnvelopeWindow::new(10))
        .await
        .unwrap();
    assert!(report.incremental);
    assert_eq!(report.changed, 2);
    assert_eq!(report.vanished, 1);

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
    let mut backend = FakeBackend::new(&[("INBOX", 3)]);
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
    let mut backend = FakeBackend::new(&[("INBOX", 3)]);
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

    let new_uid = backend.add_message("INBOX", "fresh");
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
    let mut backend = FakeBackend::new(&[("INBOX", 3)]);
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
    let mut backend = FakeBackend::new(&[("INBOX", 1)]);
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
    let mut backend = FakeBackend::new(&[("INBOX", 2), ("Archive", 1)]);
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
    let mut backend = FakeBackend::new(&[("INBOX", 1)]);
    let (store, account_id) = open_store(&mut backend).await;

    queue_delete(&store, account_id, 9999, 1).await.unwrap();
    let report = replay_pending(&mut backend, &store, account_id)
        .await
        .unwrap();
    assert_eq!(report.replayed, 0);
    assert_eq!(report.dropped, 1);
    assert!(store.pending_ops(account_id).await.unwrap().is_empty());
}
