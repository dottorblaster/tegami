// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};

use crate::MailBackend;
use crate::mime::Attachment;
use crate::store::{AttachmentRecord, BodyState, Store};

use super::{Result, SyncError};

pub struct BodyFetch {
    pub message_id: i64,
    pub raw_path: PathBuf,
    pub attachments: usize,
}

pub async fn fetch_body<B: MailBackend + ?Sized>(
    backend: &mut B,
    store: &Store,
    folder: &str,
    folder_id: i64,
    uid: u32,
    body_dir: &Path,
) -> Result<BodyFetch> {
    let raw = backend.fetch_message(folder, uid).await?;
    let message = store
        .message(folder_id, uid)
        .await?
        .ok_or(SyncError::MissingMessage(folder_id, uid))?;
    let message_id = message
        .id
        .ok_or(SyncError::MissingMessage(folder_id, uid))?;

    tokio::fs::create_dir_all(body_dir).await?;
    let raw_path = body_dir.join(format!("{folder_id}-{uid}.eml"));
    tokio::fs::write(&raw_path, &raw).await?;

    let attachments: Vec<AttachmentRecord> = crate::mime::parse(&raw)
        .map(|parsed| {
            parsed
                .attachments
                .iter()
                .map(|attachment| attachment_record(message_id, attachment))
                .collect()
        })
        .unwrap_or_default();
    let attachment_count = attachments.len();

    store
        .set_message_body(
            folder_id,
            uid,
            raw_path.to_string_lossy().into_owned(),
            BodyState::Full,
            attachment_count > 0,
        )
        .await?;
    store.replace_attachments(message_id, attachments).await?;

    Ok(BodyFetch {
        message_id,
        raw_path,
        attachments: attachment_count,
    })
}

fn attachment_record(message_id: i64, attachment: &Attachment) -> AttachmentRecord {
    AttachmentRecord {
        id: None,
        message_id,
        part_id: attachment.part_id.clone(),
        filename: attachment.filename.clone(),
        mime_type: Some(attachment.mime_type.clone()),
        size: i64::try_from(attachment.size).ok(),
        content_id: attachment.content_id.clone(),
        disk_path: None,
    }
}
