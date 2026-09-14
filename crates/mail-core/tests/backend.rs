// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A minimal in-memory backend exercising the [`MailBackend`] contract.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use mail_core::account::AccountConfig;
use mail_core::backend::Result;
use mail_core::envelope::{Address, Envelope, FlagChange, MessageFlags};
use mail_core::folder::{Folder, FolderDelta, FolderRole, FolderState};
use mail_core::{Credential, MailBackend, MailError};

struct FakeMessage {
    envelope: Envelope,
    raw: Vec<u8>,
}

struct FakeFolder {
    messages: HashMap<u32, FakeMessage>,
    uid_next: u32,
    uid_validity: u32,
}

impl FakeFolder {
    fn new() -> Self {
        Self {
            messages: HashMap::new(),
            uid_next: 1,
            uid_validity: 42,
        }
    }
}

struct FakeBackend {
    connected: bool,
    folders: HashMap<String, FakeFolder>,
    idle_notifications: u32,
}

impl FakeBackend {
    fn new() -> Self {
        Self {
            connected: false,
            folders: HashMap::new(),
            idle_notifications: 0,
        }
    }

    fn add(&mut self, folder: &str, subject: &str, raw: &[u8]) -> Result<u32> {
        let folder_entry = self
            .folders
            .get_mut(folder)
            .ok_or_else(|| MailError::Protocol(format!("no such folder {folder}")))?;
        let uid = folder_entry.uid_next;
        folder_entry.uid_next += 1;
        let created = SystemTime::now();
        folder_entry.messages.insert(
            uid,
            FakeMessage {
                envelope: Envelope {
                    uid,
                    modseq: None,
                    flags: MessageFlags::default(),
                    size: raw.len() as u32,
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
                    date: Some(created),
                    message_id: Some(format!("<{uid}@example.org>")),
                    in_reply_to: None,
                    references: Vec::new(),
                },
                raw: raw.to_vec(),
            },
        );
        Ok(uid)
    }
}

impl MailBackend for FakeBackend {
    async fn connect(&mut self, _config: &AccountConfig, _credential: &Credential) -> Result<()> {
        if self.connected {
            return Err(MailError::Protocol("already connected".to_string()));
        }
        self.connected = true;
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }

    fn supports_condstore(&self) -> bool {
        false
    }

    fn supports_qresync(&self) -> bool {
        false
    }

    async fn folders(&mut self) -> Result<Vec<Folder>> {
        let mut folders: Vec<Folder> = self
            .folders
            .keys()
            .map(|id| {
                let role = if id == "INBOX" {
                    FolderRole::Inbox
                } else {
                    FolderRole::Other
                };
                Folder {
                    id: id.clone(),
                    name: id.clone(),
                    role,
                }
            })
            .collect();
        folders.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(folders)
    }

    async fn select(&mut self, folder: &str) -> Result<FolderState> {
        let folder = self
            .folders
            .get(folder)
            .ok_or_else(|| MailError::Protocol(format!("no such folder {folder}")))?;
        Ok(FolderState {
            uid_validity: folder.uid_validity,
            uid_next: folder.uid_next,
            exists: folder.messages.len() as u32,
            recent: 0,
            unseen: None,
            highest_modseq: None,
        })
    }

    async fn uids(&mut self, folder: &str) -> Result<Vec<u32>> {
        let folder = self
            .folders
            .get(folder)
            .ok_or_else(|| MailError::Protocol(format!("no such folder {folder}")))?;
        let mut uids: Vec<u32> = folder.messages.keys().copied().collect();
        uids.sort_unstable();
        Ok(uids)
    }

    async fn fetch_envelopes(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<Envelope>> {
        let folder = self
            .folders
            .get(folder)
            .ok_or_else(|| MailError::Protocol(format!("no such folder {folder}")))?;
        let mut envelopes: Vec<Envelope> = uids
            .iter()
            .filter_map(|uid| folder.messages.get(uid))
            .map(|message| message.envelope.clone())
            .collect();
        envelopes.sort_by_key(|envelope| envelope.uid);
        Ok(envelopes)
    }

    async fn fetch_delta(&mut self, _folder: &str, _since_modseq: u64) -> Result<FolderDelta> {
        Err(MailError::Protocol("CONDSTORE not supported".to_string()))
    }

    async fn fetch_message(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        self.folders
            .get(folder)
            .and_then(|folder| folder.messages.get(&uid))
            .map(|message| message.raw.clone())
            .ok_or_else(|| MailError::Protocol(format!("no message {uid} in {folder}")))
    }

    async fn set_flags(&mut self, folder: &str, uids: &[u32], change: FlagChange) -> Result<()> {
        let folder = self
            .folders
            .get_mut(folder)
            .ok_or_else(|| MailError::Protocol(format!("no such folder {folder}")))?;
        for uid in uids {
            if let Some(message) = folder.messages.get_mut(uid) {
                let flags = &mut message.envelope.flags;
                if let Some(seen) = change.seen {
                    flags.seen = seen;
                }
                if let Some(flagged) = change.flagged {
                    flags.flagged = flagged;
                }
            }
        }
        Ok(())
    }

    async fn move_messages(&mut self, from: &str, to: &str, uids: &[u32]) -> Result<()> {
        for uid in uids {
            if let Some(message) = self
                .folders
                .get_mut(from)
                .and_then(|folder| folder.messages.remove(uid))
            {
                let destination = self.folders.get_mut(to).unwrap();
                destination.uid_next += 1;
                destination.messages.insert(destination.uid_next, message);
            }
        }
        Ok(())
    }

    async fn copy_messages(&mut self, from: &str, to: &str, uids: &[u32]) -> Result<()> {
        let copies: Vec<FakeMessage> = uids
            .iter()
            .filter_map(|uid| {
                self.folders
                    .get(from)
                    .and_then(|folder| folder.messages.get(uid))
            })
            .map(|message| FakeMessage {
                envelope: message.envelope.clone(),
                raw: message.raw.clone(),
            })
            .collect();
        let destination = self
            .folders
            .get_mut(to)
            .ok_or_else(|| MailError::Protocol(format!("no such folder {to}")))?;
        for message in copies {
            destination.uid_next += 1;
            let uid = destination.uid_next;
            destination.messages.insert(uid, message);
        }
        Ok(())
    }

    async fn append(&mut self, folder: &str, flags: MessageFlags, raw: &[u8]) -> Result<u32> {
        let folder_entry = self
            .folders
            .get_mut(folder)
            .ok_or_else(|| MailError::Protocol(format!("no such folder {folder}")))?;
        let uid = folder_entry.uid_next;
        folder_entry.uid_next += 1;
        folder_entry.messages.insert(
            uid,
            FakeMessage {
                envelope: Envelope {
                    uid,
                    modseq: None,
                    flags,
                    size: raw.len() as u32,
                    subject: String::new(),
                    from: Vec::new(),
                    to: Vec::new(),
                    cc: Vec::new(),
                    date: None,
                    message_id: None,
                    in_reply_to: None,
                    references: Vec::new(),
                },
                raw: raw.to_vec(),
            },
        );
        Ok(uid)
    }

    async fn idle(&mut self, _folder: &str) -> Result<()> {
        tokio::time::sleep(Duration::from_millis(10)).await;
        self.idle_notifications += 1;
        Ok(())
    }
}

#[tokio::test]
async fn mail_backend_round_trip() {
    let mut backend = FakeBackend::new();
    backend
        .folders
        .insert("INBOX".to_string(), FakeFolder::new());
    backend
        .folders
        .insert("Archive".to_string(), FakeFolder::new());
    let draft_uid = backend
        .add("INBOX", "hello", b"Subject: hello\r\n\r\nbody")
        .unwrap();
    backend
        .add("INBOX", "unread", b"Subject: unread\r\n\r\nbody")
        .unwrap();

    let config = AccountConfig {
        id: "test".to_string(),
        name: "Test".to_string(),
        email_address: "me@example.org".to_string(),
        provider_type: None,
        is_temporary: false,
        imap: None,
        smtp: None,
    };

    backend
        .connect(&config, &Credential::Password("hunter2".to_string()))
        .await
        .unwrap();
    let folders = backend.folders().await.unwrap();
    assert_eq!(folders.len(), 2);
    assert_eq!(folders[0].role, FolderRole::Other);
    assert_eq!(folders[1].id, "INBOX");
    assert_eq!(folders[1].role, FolderRole::Inbox);

    let state = backend.select("INBOX").await.unwrap();
    assert_eq!(state.uid_validity, 42);
    assert_eq!(state.exists, 2);

    let backend_uids = backend.uids("INBOX").await.unwrap();
    assert_eq!(backend_uids, vec![draft_uid, draft_uid + 1]);

    let envelopes = backend
        .fetch_envelopes("INBOX", &backend_uids)
        .await
        .unwrap();
    assert_eq!(envelopes.len(), 2);
    assert_eq!(envelopes[0].subject, "hello");
    assert_eq!(
        envelopes[0].from[0].address.as_deref(),
        Some("sender@example.org")
    );
    assert!(envelopes[0].date.is_some());

    let body = backend.fetch_message("INBOX", draft_uid).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("Subject: hello"));

    let change = FlagChange {
        seen: Some(true),
        flagged: Some(true),
        ..FlagChange::default()
    };
    backend
        .set_flags("INBOX", &[draft_uid], change)
        .await
        .unwrap();
    let envelopes = backend
        .fetch_envelopes("INBOX", &backend_uids)
        .await
        .unwrap();
    let draft = envelopes.iter().find(|e| e.uid == draft_uid).unwrap();
    assert!(draft.flags.seen);
    assert!(draft.flags.flagged);

    backend
        .copy_messages("INBOX", "Archive", &[draft_uid])
        .await
        .unwrap();

    backend
        .move_messages("INBOX", "Archive", &[draft_uid])
        .await
        .unwrap();

    let appended = backend
        .append(
            "Archive",
            MessageFlags {
                draft: true,
                ..MessageFlags::default()
            },
            b"Subject: draft\r\n\r\nwork in progress",
        )
        .await
        .unwrap();
    assert!(appended > 0);

    backend.idle("INBOX").await.unwrap();
    assert_eq!(backend.idle_notifications, 1);
    backend.disconnect().await.unwrap();
}
