// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::{Duration, UNIX_EPOCH};

use mail_core::account::AccountConfig;
use mail_core::backend::Result;
use mail_core::envelope::{Address, Envelope, FlagChange, MessageFlags};
use mail_core::folder::{Folder, FolderRole, FolderState};
use mail_core::{Credential, MailBackend, MailError};
use store::{AccountRecord, AccountSource, AuthKind, Store};
use sync::{EnvelopeWindow, sync_account};

struct FakeBackend {
    folders: Vec<(String, Vec<Envelope>)>,
}

impl FakeBackend {
    fn new(folders: &[(&str, usize)]) -> Self {
        Self {
            folders: folders
                .iter()
                .map(|(name, count)| {
                    let envelopes = (1..=*count).map(|uid| envelope(uid as u32, name)).collect();
                    (name.to_string(), envelopes)
                })
                .collect(),
        }
    }

    fn folder(&self, name: &str) -> Result<&Vec<Envelope>> {
        self.folders
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, envelopes)| envelopes)
            .ok_or_else(|| MailError::Protocol(format!("no such folder {name}")))
    }
}

fn envelope(uid: u32, folder: &str) -> Envelope {
    Envelope {
        uid,
        flags: MessageFlags {
            seen: uid.is_multiple_of(2),
            ..MessageFlags::default()
        },
        size: 100 + uid,
        subject: format!("{folder} {uid}"),
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
            uid_validity: 1,
            uid_next: count + 1,
            exists: count,
            recent: 0,
            unseen: None,
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

    async fn fetch_message(&mut self, _folder: &str, _uid: u32) -> Result<Vec<u8>> {
        Err(MailError::Protocol("unsupported".to_string()))
    }

    async fn set_flags(&mut self, _folder: &str, _uids: &[u32], _change: FlagChange) -> Result<()> {
        Ok(())
    }

    async fn move_messages(&mut self, _from: &str, _to: &str, _uids: &[u32]) -> Result<()> {
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

    let folders = store.folders(account_id).await.unwrap();
    let inbox = folders
        .iter()
        .find(|folder| folder.name == "INBOX")
        .unwrap();
    assert_eq!(inbox.special_use, Some(store::SpecialUse::Inbox));
    let inbox_id = inbox.id.unwrap();

    let inbox_report = reports
        .iter()
        .find(|report| report.folder_id == inbox_id)
        .unwrap();
    assert_eq!(inbox_report.total, 5);
    assert_eq!(inbox_report.fetched, 2);

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
    assert_eq!(messages[0].flags & store::FLAG_SEEN, store::FLAG_SEEN);
    assert_eq!(messages[1].flags & store::FLAG_SEEN, 0);

    let archive = folders
        .iter()
        .find(|folder| folder.name == "Archive")
        .unwrap();
    let archive_id = archive.id.unwrap();
    let archive_report = reports
        .iter()
        .find(|report| report.folder_id == archive_id)
        .unwrap();
    assert_eq!(archive_report.total, 1);
    assert_eq!(archive_report.fetched, 1);
    assert_eq!(store.messages(archive_id).await.unwrap().len(), 1);
}
