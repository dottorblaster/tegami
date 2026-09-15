// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(dead_code)]

use std::time::{Duration, UNIX_EPOCH};

use mail_core::account::AccountConfig;
use mail_core::backend::Result;
use mail_core::envelope::{Address, Envelope, FlagChange, MessageFlags};
use mail_core::folder::{Folder, FolderDelta, FolderRole, FolderState};
use mail_core::{Credential, MailBackend, MailError};

pub struct FakeBackend {
    pub folders: Vec<(String, Vec<Envelope>)>,
    pub bodies: Vec<((String, u32), Vec<u8>)>,
    pub vanished: Vec<(String, u32)>,
    pub flag_calls: Vec<(String, Vec<u32>, FlagChange)>,
    pub move_calls: Vec<(String, String, Vec<u32>)>,
    pub copy_calls: Vec<(String, String, Vec<u32>)>,
    pub idle_calls: u32,
    pub modseq: u64,
    pub uid_validity: u32,
    pub condstore: bool,
    pub qresync: bool,
    pub idle_supported: bool,
    pub idle_delay: Duration,
    pub connected: bool,
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self {
            folders: Vec::new(),
            bodies: Vec::new(),
            vanished: Vec::new(),
            flag_calls: Vec::new(),
            move_calls: Vec::new(),
            copy_calls: Vec::new(),
            idle_calls: 0,
            modseq: 0,
            uid_validity: 1,
            condstore: true,
            qresync: true,
            idle_supported: false,
            idle_delay: Duration::from_millis(5),
            connected: false,
        }
    }
}

impl FakeBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_folders(folders: &[(&str, usize)]) -> Self {
        let mut backend = Self::new();
        for (name, count) in folders {
            backend.add_folder(name, *count);
        }
        backend
    }

    pub fn add_folder(&mut self, name: &str, count: usize) {
        let mut envelopes = Vec::new();
        for uid in 1..=count {
            self.modseq += 1;
            envelopes.push(envelope(uid as u32, &format!("{name} {uid}"), self.modseq));
        }
        self.folders.push((name.to_string(), envelopes));
    }

    pub fn add_message(&mut self, folder: &str, subject: &str, raw: &[u8]) -> u32 {
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
        let message = envelope(uid, subject, self.modseq);
        if let Some(envelopes) = self.folder_mut(folder) {
            envelopes.push(message);
        }
        self.bodies.push(((folder.to_string(), uid), raw.to_vec()));
        uid
    }

    pub fn set_body(&mut self, folder: &str, uid: u32, raw: &[u8]) {
        self.bodies.push(((folder.to_string(), uid), raw.to_vec()));
    }

    pub fn mark_seen(&mut self, folder: &str, uid: u32) {
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

    pub fn expunge(&mut self, folder: &str, uid: u32) {
        self.modseq += 1;
        if let Some(envelopes) = self.folder_mut(folder) {
            envelopes.retain(|envelope| envelope.uid != uid);
        }
        self.vanished.push((folder.to_string(), uid));
    }

    pub fn rebuild(&mut self, folder: &str, count: usize, uid_validity: u32) {
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
}

impl MailBackend for FakeBackend {
    async fn connect(&mut self, _config: &AccountConfig, _credential: &Credential) -> Result<()> {
        if self.connected {
            return Err(MailError::Protocol("already connected".to_string()));
        }
        self.connected = true;
        Ok(())
    }

    fn supports_idle(&self) -> bool {
        self.idle_supported
    }

    fn supports_condstore(&self) -> bool {
        self.condstore
    }

    fn supports_qresync(&self) -> bool {
        self.qresync
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }

    async fn folders(&mut self) -> Result<Vec<Folder>> {
        let mut folders: Vec<Folder> = self
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
            .collect();
        folders.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(folders)
    }

    async fn select(&mut self, folder: &str) -> Result<FolderState> {
        let envelopes = self.folder(folder)?;
        Ok(FolderState {
            uid_validity: self.uid_validity,
            uid_next: envelopes
                .iter()
                .map(|envelope| envelope.uid)
                .max()
                .unwrap_or(0)
                + 1,
            exists: envelopes.len() as u32,
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
        let base = self.modseq;
        let touched = match self.folder_mut(folder) {
            Some(envelopes) => {
                let mut touched = 0;
                for uid in uids {
                    if let Some(target) = envelopes.iter_mut().find(|envelope| envelope.uid == *uid)
                    {
                        touched += 1;
                        target.modseq = Some(base + touched);
                        apply_change(&mut target.flags, change);
                    }
                }
                touched
            }
            None => 0,
        };
        self.modseq = base + touched;
        Ok(())
    }

    async fn move_messages(&mut self, from: &str, to: &str, uids: &[u32]) -> Result<()> {
        self.move_calls
            .push((from.to_string(), to.to_string(), uids.to_vec()));
        let mut moved = Vec::new();
        if let Some(source) = self.folder_mut(from) {
            for uid in uids {
                if let Some(position) = source.iter().position(|envelope| envelope.uid == *uid) {
                    moved.push(source.remove(position));
                }
            }
        }
        if let Some(destination) = self.folder_mut(to) {
            for mut message in moved {
                let next = destination
                    .iter()
                    .map(|envelope| envelope.uid)
                    .max()
                    .unwrap_or(0)
                    + 1;
                message.uid = next;
                destination.push(message);
            }
        }
        Ok(())
    }

    async fn copy_messages(&mut self, from: &str, to: &str, uids: &[u32]) -> Result<()> {
        self.copy_calls
            .push((from.to_string(), to.to_string(), uids.to_vec()));
        let copies: Vec<Envelope> = self
            .folder(from)?
            .iter()
            .filter(|envelope| uids.contains(&envelope.uid))
            .cloned()
            .collect();
        if let Some(destination) = self.folder_mut(to) {
            for mut message in copies {
                let next = destination
                    .iter()
                    .map(|envelope| envelope.uid)
                    .max()
                    .unwrap_or(0)
                    + 1;
                message.uid = next;
                destination.push(message);
            }
        }
        Ok(())
    }

    async fn append(&mut self, folder: &str, flags: MessageFlags, raw: &[u8]) -> Result<u32> {
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
        let mut message = envelope(uid, "", self.modseq);
        message.flags = flags;
        if let Some(envelopes) = self.folder_mut(folder) {
            envelopes.push(message);
        }
        self.bodies.push(((folder.to_string(), uid), raw.to_vec()));
        Ok(uid)
    }

    async fn idle(&mut self, _folder: &str) -> Result<()> {
        if !self.idle_supported {
            return Err(MailError::Protocol("IDLE not supported".to_string()));
        }
        self.idle_calls += 1;
        if !self.idle_delay.is_zero() {
            tokio::time::sleep(self.idle_delay).await;
        }
        Ok(())
    }
}

fn apply_change(flags: &mut MessageFlags, change: FlagChange) {
    if let Some(value) = change.seen {
        flags.seen = value;
    }
    if let Some(value) = change.answered {
        flags.answered = value;
    }
    if let Some(value) = change.flagged {
        flags.flagged = value;
    }
    if let Some(value) = change.deleted {
        flags.deleted = value;
    }
    if let Some(value) = change.draft {
        flags.draft = value;
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
