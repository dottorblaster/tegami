// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Persisting outgoing mail into its folders via APPEND.
//!
//! Delivered messages are appended to the Sent folder so the server holds a
//! copy even before the next sync; drafts get appended to the Drafts folder,
//! replacing the previous version when one exists. Appending uses UIDPLUS so
//! the assigned server UID is known, which keeps the local copy consistent.

use crate::MailBackend;
use crate::envelope::MessageFlags;
use crate::store::{OutboxState, SpecialUse, Store};

use super::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SaveReport {
    pub saved: usize,
    pub skipped: usize,
    pub failed: usize,
}

pub fn sent_flags() -> MessageFlags {
    MessageFlags {
        seen: true,
        ..MessageFlags::default()
    }
}

pub fn draft_flags() -> MessageFlags {
    MessageFlags {
        seen: true,
        draft: true,
        ..MessageFlags::default()
    }
}

/// Appends every delivered outbound message to the account's Sent folder and
/// clears the outbox entries (and their cached MIME files) once the server
/// accepted them.
pub async fn save_sent<B: MailBackend + ?Sized>(
    backend: &mut B,
    store: &Store,
    account_id: i64,
) -> Result<SaveReport> {
    let pending: Vec<_> = store
        .outbox(account_id)
        .await?
        .into_iter()
        .filter(|entry| entry.state == OutboxState::Sent)
        .collect();
    let mut report = SaveReport::default();
    if pending.is_empty() {
        return Ok(report);
    }
    let sent = store
        .folders(account_id)
        .await?
        .into_iter()
        .find(|folder| folder.special_use == Some(SpecialUse::Sent));
    let Some(sent) = sent else {
        report.skipped = pending.len();
        return Ok(report);
    };
    for entry in pending {
        let Some(id) = entry.id else {
            report.skipped += 1;
            continue;
        };
        let raw = match tokio::fs::read(&entry.raw_path).await {
            Ok(raw) => raw,
            Err(_) => {
                let _ = store.delete_outbox(id).await;
                let _ = tokio::fs::remove_file(&entry.raw_path).await;
                report.failed += 1;
                continue;
            }
        };
        match backend.append(&sent.name, sent_flags(), &raw).await {
            Ok(_) => {
                let _ = store.delete_outbox(id).await;
                let _ = tokio::fs::remove_file(&entry.raw_path).await;
                report.saved += 1;
            }
            Err(_) => report.failed += 1,
        }
    }
    Ok(report)
}

/// Appends a draft to a folder with the `\\Draft` flag, replacing the given
/// previous version when one exists, and returns the assigned UID.
pub async fn save_draft<B: MailBackend + ?Sized>(
    backend: &mut B,
    folder: &str,
    raw: &[u8],
    replace_uid: Option<u32>,
) -> Result<u32> {
    let uid = backend.append(folder, draft_flags(), raw).await?;
    if let Some(previous) = replace_uid {
        backend.delete_permanently(folder, &[previous]).await?;
    }
    Ok(uid)
}
