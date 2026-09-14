// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use crate::MailBackend;
use crate::envelope::FlagChange;
use crate::store::{OpKind, PendingOpRecord, Store};

use super::{Result, SyncError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayReport {
    pub replayed: usize,
    pub dropped: usize,
    pub remaining: usize,
}

pub async fn queue_set_flags(
    store: &Store,
    account_id: i64,
    folder_id: i64,
    uid: u32,
    change: FlagChange,
) -> Result<i64> {
    let op = PendingOpRecord {
        id: None,
        account_id,
        kind: OpKind::SetFlags,
        folder_id: Some(folder_id),
        target_folder_id: None,
        uid: Some(uid),
        payload: Some(encode_flag_change(&change)),
        created_at: None,
    };
    Ok(store.enqueue_op(op).await?)
}

pub async fn queue_move(
    store: &Store,
    account_id: i64,
    folder_id: i64,
    uid: u32,
    target_folder_id: i64,
) -> Result<i64> {
    let op = PendingOpRecord {
        id: None,
        account_id,
        kind: OpKind::Move,
        folder_id: Some(folder_id),
        target_folder_id: Some(target_folder_id),
        uid: Some(uid),
        payload: None,
        created_at: None,
    };
    Ok(store.enqueue_op(op).await?)
}

pub async fn queue_delete(store: &Store, account_id: i64, folder_id: i64, uid: u32) -> Result<i64> {
    let op = PendingOpRecord {
        id: None,
        account_id,
        kind: OpKind::Delete,
        folder_id: Some(folder_id),
        target_folder_id: None,
        uid: Some(uid),
        payload: None,
        created_at: None,
    };
    Ok(store.enqueue_op(op).await?)
}

pub async fn replay_pending<B: MailBackend + ?Sized>(
    backend: &mut B,
    store: &Store,
    account_id: i64,
) -> Result<ReplayReport> {
    let folders: HashMap<i64, String> = store
        .folders(account_id)
        .await?
        .into_iter()
        .filter_map(|folder| Some((folder.id?, folder.name)))
        .collect();
    let ops = store.pending_ops(account_id).await?;
    let total = ops.len();
    let mut replayed = 0;
    let mut dropped = 0;
    for op in ops {
        let id = op
            .id
            .ok_or_else(|| SyncError::Protocol("pending op without id".to_string()))?;
        match apply(backend, &folders, &op).await? {
            Outcome::Applied => replayed += 1,
            Outcome::Dropped => dropped += 1,
        }
        store.delete_op(id).await?;
    }
    Ok(ReplayReport {
        replayed,
        dropped,
        remaining: total - replayed - dropped,
    })
}

enum Outcome {
    Applied,
    Dropped,
}

async fn apply<B: MailBackend + ?Sized>(
    backend: &mut B,
    folders: &HashMap<i64, String>,
    op: &PendingOpRecord,
) -> Result<Outcome> {
    let Some(folder) = op.folder_id.and_then(|id| folders.get(&id)) else {
        return Ok(Outcome::Dropped);
    };
    let Some(uid) = op.uid else {
        return Ok(Outcome::Dropped);
    };
    match op.kind {
        OpKind::SetFlags => {
            let change = op
                .payload
                .as_deref()
                .and_then(decode_flag_change)
                .ok_or_else(|| SyncError::Protocol("invalid flag payload".to_string()))?;
            backend.set_flags(folder, &[uid], change).await?;
        }
        OpKind::Move => {
            let Some(target) = op.target_folder_id.and_then(|id| folders.get(&id)) else {
                return Ok(Outcome::Dropped);
            };
            backend.move_messages(folder, target, &[uid]).await?;
        }
        OpKind::Delete => {
            let change = FlagChange {
                deleted: Some(true),
                ..FlagChange::default()
            };
            backend.set_flags(folder, &[uid], change).await?;
        }
    }
    Ok(Outcome::Applied)
}

pub fn encode_flag_change(change: &FlagChange) -> String {
    [
        change.seen,
        change.answered,
        change.flagged,
        change.deleted,
        change.draft,
    ]
    .iter()
    .map(|flag| match flag {
        Some(true) => '+',
        Some(false) => '-',
        None => '.',
    })
    .collect()
}

pub fn decode_flag_change(payload: &str) -> Option<FlagChange> {
    let chars: Vec<char> = payload.chars().collect();
    if chars.len() != 5 {
        return None;
    }
    let parse = |character: char| match character {
        '+' => Some(Some(true)),
        '-' => Some(Some(false)),
        '.' => Some(None),
        _ => None,
    };
    Some(FlagChange {
        seen: parse(chars[0])?,
        answered: parse(chars[1])?,
        flagged: parse(chars[2])?,
        deleted: parse(chars[3])?,
        draft: parse(chars[4])?,
    })
}

#[cfg(test)]
mod tests {
    use crate::envelope::FlagChange;

    use super::{decode_flag_change, encode_flag_change};

    #[test]
    fn flag_change_round_trip() {
        let change = FlagChange {
            seen: Some(true),
            answered: Some(false),
            flagged: None,
            deleted: Some(true),
            draft: None,
        };
        let encoded = encode_flag_change(&change);
        assert_eq!(encoded, "+-.+.");
        assert_eq!(decode_flag_change(&encoded), Some(change));
    }

    #[test]
    fn rejects_bad_payloads() {
        assert_eq!(decode_flag_change("+-"), None);
        assert_eq!(decode_flag_change("+x.-."), None);
    }
}
