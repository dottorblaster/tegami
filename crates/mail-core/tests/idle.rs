// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::Duration;

use mail_core::account::AccountConfig;
use mail_core::backend::Result;
use mail_core::envelope::{Envelope, FlagChange, MessageFlags};
use mail_core::folder::{Folder, FolderDelta, FolderState};
use mail_core::sync::{IdleEvent, IdleWorker};
use mail_core::{Credential, MailBackend, MailError};
use tokio::sync::mpsc;

struct IdleBackend {
    supported: bool,
}

impl IdleBackend {
    fn new(supported: bool) -> Self {
        Self { supported }
    }
}

impl MailBackend for IdleBackend {
    async fn connect(&mut self, _config: &AccountConfig, _credential: &Credential) -> Result<()> {
        Ok(())
    }

    fn supports_idle(&self) -> bool {
        self.supported
    }

    fn supports_condstore(&self) -> bool {
        false
    }

    fn supports_qresync(&self) -> bool {
        false
    }

    async fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }

    async fn folders(&mut self) -> Result<Vec<Folder>> {
        Ok(Vec::new())
    }

    async fn select(&mut self, _folder: &str) -> Result<FolderState> {
        Err(unsupported())
    }

    async fn uids(&mut self, _folder: &str) -> Result<Vec<u32>> {
        Err(unsupported())
    }

    async fn fetch_envelopes(&mut self, _folder: &str, _uids: &[u32]) -> Result<Vec<Envelope>> {
        Err(unsupported())
    }

    async fn fetch_delta(&mut self, _folder: &str, _since_modseq: u64) -> Result<FolderDelta> {
        Err(unsupported())
    }

    async fn fetch_message(&mut self, _folder: &str, _uid: u32) -> Result<Vec<u8>> {
        Err(unsupported())
    }

    async fn set_flags(&mut self, _folder: &str, _uids: &[u32], _change: FlagChange) -> Result<()> {
        Err(unsupported())
    }

    async fn move_messages(&mut self, _from: &str, _to: &str, _uids: &[u32]) -> Result<()> {
        Err(unsupported())
    }

    async fn copy_messages(&mut self, _from: &str, _to: &str, _uids: &[u32]) -> Result<()> {
        Err(unsupported())
    }

    async fn append(&mut self, _folder: &str, _flags: MessageFlags, _raw: &[u8]) -> Result<u32> {
        Err(unsupported())
    }

    async fn idle(&mut self, _folder: &str) -> Result<()> {
        if !self.supported {
            return Err(unsupported());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
        Ok(())
    }
}

fn unsupported() -> MailError {
    MailError::Protocol("unsupported".to_string())
}

#[tokio::test]
async fn idle_worker_emits_changes() {
    let (sender, mut receiver) = mpsc::channel(4);
    let worker = IdleWorker::spawn(IdleBackend::new(true), "INBOX", sender);

    assert_eq!(
        receiver.recv().await.unwrap(),
        IdleEvent::Changed {
            folder: "INBOX".to_string()
        }
    );
    assert_eq!(
        receiver.recv().await.unwrap(),
        IdleEvent::Changed {
            folder: "INBOX".to_string()
        }
    );
    worker.abort();
}

#[tokio::test]
async fn idle_worker_reports_unsupported() {
    let (sender, mut receiver) = mpsc::channel(4);
    let worker = IdleWorker::spawn(IdleBackend::new(false), "INBOX", sender);

    assert_eq!(
        receiver.recv().await.unwrap(),
        IdleEvent::Unsupported {
            folder: "INBOX".to_string()
        }
    );
    assert!(receiver.try_recv().is_err());
    worker.abort();
}
