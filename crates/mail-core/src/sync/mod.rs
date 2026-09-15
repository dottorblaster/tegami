// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

mod body;
mod idle;
mod message;
mod ops;

use std::collections::HashSet;

use crate::MailBackend;
use crate::backend::MailError;
use crate::store::{FolderRecord, MessageRecord, Store, StoreError};

pub use body::{BodyFetch, fetch_body};
pub use idle::{IdleEvent, IdleWorker};
pub use message::message_record;
pub use ops::{
    ReplayReport, decode_flag_change, encode_flag_change, queue_delete, queue_move,
    queue_set_flags, replay_pending,
};

#[derive(Debug)]
pub enum SyncError {
    Backend(MailError),
    Store(StoreError),
    Io(std::io::Error),
    Protocol(String),
    MissingFolderId(String),
    MissingMessage(i64, u32),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Backend(err) => write!(f, "sync backend error: {err}"),
            Self::Store(err) => write!(f, "sync store error: {err}"),
            Self::Io(err) => write!(f, "sync i/o error: {err}"),
            Self::Protocol(detail) => write!(f, "sync protocol error: {detail}"),
            Self::MissingFolderId(name) => write!(f, "folder {name} has no store id"),
            Self::MissingMessage(folder_id, uid) => {
                write!(f, "no message {uid} in folder {folder_id}")
            }
        }
    }
}

impl std::error::Error for SyncError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Backend(err) => Some(err),
            Self::Store(err) => Some(err),
            Self::Io(err) => Some(err),
            Self::MissingFolderId(_) | Self::MissingMessage(_, _) | Self::Protocol(_) => None,
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

impl From<std::io::Error> for SyncError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
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
    pub uidvalidity_changed: bool,
    pub incremental: bool,
    pub changed: usize,
    pub vanished: usize,
    pub new_uids: Vec<u32>,
}

pub async fn sync_folder<B: MailBackend + ?Sized>(
    backend: &mut B,
    store: &Store,
    folder: &FolderRecord,
    window: EnvelopeWindow,
) -> Result<FolderSync> {
    let folder_id = folder
        .id
        .ok_or_else(|| SyncError::MissingFolderId(folder.name.clone()))?;
    let state = backend.select(&folder.name).await?;
    let previous_validity = folder
        .uidvalidity
        .and_then(|value| u32::try_from(value).ok());
    let uidvalidity_changed = previous_validity
        .is_some_and(|stored| state.uid_validity != 0 && stored != state.uid_validity);
    if uidvalidity_changed {
        store.clear_messages(folder_id).await?;
    }
    store.set_folder_state(folder_id, state).await?;

    let previous_modseq = if uidvalidity_changed {
        None
    } else {
        folder
            .highestmodseq
            .filter(|_| backend.supports_condstore())
            .map(|modseq| modseq as u64)
    };

    let known: HashSet<u32> = store.message_uids(folder_id).await?.into_iter().collect();

    let (changed, vanished, incremental) = match previous_modseq {
        Some(modseq) => {
            let delta = backend.fetch_delta(&folder.name, modseq).await?;
            let vanished = if backend.supports_qresync() {
                delta.vanished
            } else {
                missing_uids(backend, store, folder_id, &folder.name).await?
            };
            (delta.changed, vanished, true)
        }
        None => {
            let uids = backend.uids(&folder.name).await?;
            let current: HashSet<u32> = uids.iter().copied().collect();
            let vanished: Vec<u32> = known
                .iter()
                .copied()
                .filter(|uid| !current.contains(uid))
                .collect();
            let new: Vec<u32> = window
                .select(&uids)
                .iter()
                .copied()
                .filter(|uid| !known.contains(uid))
                .collect();
            let envelopes = backend.fetch_envelopes(&folder.name, &new).await?;
            (envelopes, vanished, false)
        }
    };

    let new_uids: Vec<u32> = changed
        .iter()
        .map(|envelope| envelope.uid)
        .filter(|uid| !known.contains(uid))
        .collect();
    let records: Vec<MessageRecord> = changed
        .iter()
        .map(|envelope| message_record(folder_id, envelope))
        .collect();
    let changed_count = records.len();
    store.upsert_messages(records).await?;
    store.delete_messages(folder_id, &vanished).await?;
    Ok(FolderSync {
        folder_id,
        uidvalidity_changed,
        incremental,
        changed: changed_count,
        vanished: vanished.len(),
        new_uids,
    })
}

async fn missing_uids<B: MailBackend + ?Sized>(
    backend: &mut B,
    store: &Store,
    folder_id: i64,
    folder: &str,
) -> Result<Vec<u32>> {
    let stored = store.message_uids(folder_id).await?;
    let current: HashSet<u32> = backend.uids(folder).await?.into_iter().collect();
    Ok(stored
        .into_iter()
        .filter(|uid| !current.contains(uid))
        .collect())
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
        reports.push(sync_folder(backend, store, folder, window).await?);
    }
    Ok(reports)
}
