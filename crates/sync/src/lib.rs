// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

mod message;

use mail_core::MailBackend;
use mail_core::backend::MailError;
use store::{Store, StoreError};

pub use message::message_record;

#[derive(Debug)]
pub enum SyncError {
    Backend(MailError),
    Store(StoreError),
    MissingFolderId(String),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Backend(err) => write!(f, "sync backend error: {err}"),
            Self::Store(err) => write!(f, "sync store error: {err}"),
            Self::MissingFolderId(name) => write!(f, "folder {name} has no store id"),
        }
    }
}

impl std::error::Error for SyncError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Backend(err) => Some(err),
            Self::Store(err) => Some(err),
            Self::MissingFolderId(_) => None,
        }
    }
}

impl From<MailError> for SyncError {
    fn from(err: MailError) -> Self {
        Self::Backend(err)
    }
}

impl From<StoreError> for SyncError {
    fn from(err: StoreError) -> Self {
        Self::Store(err)
    }
}

pub type Result<T> = std::result::Result<T, SyncError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvelopeWindow {
    pub max_messages: usize,
}

impl EnvelopeWindow {
    pub const fn new(max_messages: usize) -> Self {
        Self { max_messages }
    }

    pub fn select<'a>(&self, uids: &'a [u32]) -> &'a [u32] {
        let start = uids.len().saturating_sub(self.max_messages);
        &uids[start..]
    }
}

impl Default for EnvelopeWindow {
    fn default() -> Self {
        Self::new(500)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSync {
    pub folder_id: i64,
    pub total: usize,
    pub fetched: usize,
}

pub async fn sync_folder<B: MailBackend + ?Sized>(
    backend: &mut B,
    store: &Store,
    folder_id: i64,
    folder: &str,
    window: EnvelopeWindow,
) -> Result<FolderSync> {
    let uids = backend.uids(folder).await?;
    let selected = window.select(&uids);
    let envelopes = backend.fetch_envelopes(folder, selected).await?;
    let records = envelopes
        .iter()
        .map(|envelope| message_record(folder_id, envelope))
        .collect();
    store.upsert_messages(records).await?;
    Ok(FolderSync {
        folder_id,
        total: uids.len(),
        fetched: selected.len(),
    })
}

pub async fn sync_account<B: MailBackend + ?Sized>(
    backend: &mut B,
    store: &Store,
    account_id: i64,
    window: EnvelopeWindow,
) -> Result<Vec<FolderSync>> {
    let folders = backend.folders().await?;
    let stored = store.sync_folders(account_id, &folders).await?;
    let mut reports = Vec::with_capacity(stored.len());
    for folder in &stored {
        let folder_id = folder
            .id
            .ok_or_else(|| SyncError::MissingFolderId(folder.name.clone()))?;
        reports.push(sync_folder(backend, store, folder_id, &folder.name, window).await?);
    }
    Ok(reports)
}
