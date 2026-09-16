// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Offline outbox and retry policy.
//!
//! A composed message is persisted on disk and recorded in the store before
//! the transport sees it, so a failure -- unreachable server, expired token,
//! no connectivity at all -- never loses the mail. [`drain`] submits every due
//! entry and reschedules the ones that fail with an exponential backoff.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::backend::MailError;
use crate::send::{MailSender, envelope};
use crate::store::{OutboxRecord, OutboxState, Store, StoreError};

/// Delay applied after the first failed attempt, in seconds.
pub const BASE_BACKOFF_SECS: i64 = 30;
/// Upper bound for the exponential backoff, in seconds.
pub const MAX_BACKOFF_SECS: i64 = 60 * 60;
/// Attempts after which a message stops being retried and is marked failed.
pub const MAX_ATTEMPTS: i64 = 10;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub enum OutboxError {
    Store(StoreError),
    Io(std::io::Error),
    Mail(MailError),
}

impl std::fmt::Display for OutboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(err) => write!(f, "outbox store error: {err}"),
            Self::Io(err) => write!(f, "outbox i/o error: {err}"),
            Self::Mail(err) => write!(f, "outbox transport error: {err}"),
        }
    }
}

impl std::error::Error for OutboxError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(err) => Some(err),
            Self::Io(err) => Some(err),
            Self::Mail(err) => Some(err),
        }
    }
}

impl From<StoreError> for OutboxError {
    fn from(err: StoreError) -> Self {
        Self::Store(err)
    }
}

impl From<std::io::Error> for OutboxError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<MailError> for OutboxError {
    fn from(err: MailError) -> Self {
        Self::Mail(err)
    }
}

pub type Result<T> = std::result::Result<T, OutboxError>;

/// How a drain pass selects the entries it submits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainMode {
    /// Only entries whose backoff has elapsed.
    Due,
    /// Every pending entry, used on reconnect and on startup.
    Force,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DrainReport {
    pub sent: usize,
    pub retried: usize,
    pub failed: usize,
}

impl DrainReport {
    pub fn is_empty(&self) -> bool {
        self.sent == 0 && self.retried == 0 && self.failed == 0
    }
}

/// The backoff applied after `attempts` failed attempts.
pub fn backoff(attempts: i64) -> i64 {
    let exponent = attempts.clamp(1, 20) - 1;
    BASE_BACKOFF_SECS
        .saturating_mul(1i64 << exponent)
        .min(MAX_BACKOFF_SECS)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn raw_name(timestamp: i64) -> String {
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(0);
    format!("{timestamp}-{nanos:09}-{sequence}.eml")
}

/// Persists a built message and returns its outbox id.
pub async fn enqueue(store: &Store, account_id: i64, raw: &[u8], directory: &Path) -> Result<i64> {
    tokio::fs::create_dir_all(directory).await?;
    let timestamp = now();
    let path: PathBuf = directory.join(raw_name(timestamp));
    tokio::fs::write(&path, raw).await?;
    let entry = OutboxRecord::queued(account_id, path.to_string_lossy().into_owned(), timestamp);
    match store.enqueue_outbox(entry).await {
        Ok(id) => Ok(id),
        Err(err) => {
            let _ = tokio::fs::remove_file(&path).await;
            Err(err.into())
        }
    }
}

/// Submits every selected entry, rescheduling or failing the ones that bounce.
pub async fn drain<M: MailSender + ?Sized>(
    sender: &mut M,
    store: &Store,
    account_id: i64,
    now: i64,
    mode: DrainMode,
) -> Result<DrainReport> {
    let entries = match mode {
        DrainMode::Due => store.due_outbox(account_id, now).await?,
        DrainMode::Force => store
            .outbox(account_id)
            .await?
            .into_iter()
            .filter(OutboxRecord::is_pending)
            .collect(),
    };
    let mut report = DrainReport::default();
    for entry in entries {
        if entry.id.is_none() {
            continue;
        }
        match submit(sender, store, &entry, now).await? {
            Outcome::Sent => report.sent += 1,
            Outcome::Retried => report.retried += 1,
            Outcome::Failed => report.failed += 1,
            Outcome::Skipped => {}
        }
    }
    Ok(report)
}

enum Outcome {
    Sent,
    Retried,
    Failed,
    Skipped,
}

async fn submit<M: MailSender + ?Sized>(
    sender: &mut M,
    store: &Store,
    entry: &OutboxRecord,
    now: i64,
) -> Result<Outcome> {
    let Some(id) = entry.id else {
        return Ok(Outcome::Skipped);
    };
    let raw = match tokio::fs::read(&entry.raw_path).await {
        Ok(raw) => raw,
        Err(err) => {
            let detail = format!("cannot read {}: {err}", entry.raw_path);
            store
                .update_outbox(id, OutboxState::Failed, entry.attempts, Some(detail), None)
                .await?;
            return Ok(Outcome::Failed);
        }
    };
    match envelope(&raw) {
        Some(envelope) if !envelope.recipients.is_empty() => {
            store
                .update_outbox(id, OutboxState::Sending, entry.attempts, None, None)
                .await?;
            match sender.send(&envelope, &raw).await {
                Ok(()) => {
                    store
                        .update_outbox(id, OutboxState::Sent, entry.attempts, None, None)
                        .await?;
                    Ok(Outcome::Sent)
                }
                Err(err) => reschedule(store, entry, &err.to_string(), now).await,
            }
        }
        _ => {
            let detail = "message has no usable recipients".to_string();
            store
                .update_outbox(id, OutboxState::Failed, entry.attempts, Some(detail), None)
                .await?;
            Ok(Outcome::Failed)
        }
    }
}

async fn reschedule(
    store: &Store,
    entry: &OutboxRecord,
    detail: &str,
    now: i64,
) -> Result<Outcome> {
    let Some(id) = entry.id else {
        return Ok(Outcome::Skipped);
    };
    let attempts = entry.attempts + 1;
    if attempts >= MAX_ATTEMPTS {
        store
            .update_outbox(
                id,
                OutboxState::Failed,
                attempts,
                Some(detail.to_string()),
                None,
            )
            .await?;
        Ok(Outcome::Failed)
    } else {
        store
            .update_outbox(
                id,
                OutboxState::Queued,
                attempts,
                Some(detail.to_string()),
                Some(now + backoff(attempts)),
            )
            .await?;
        Ok(Outcome::Retried)
    }
}

/// The instant the next pending entry becomes due, if any.
pub async fn next_retry(store: &Store, account_id: i64) -> Result<Option<i64>> {
    let mut earliest: Option<i64> = None;
    for entry in store
        .outbox(account_id)
        .await?
        .iter()
        .filter(|e| e.is_pending())
    {
        let at = entry.send_after.unwrap_or(0);
        earliest = Some(earliest.map_or(at, |current: i64| current.min(at)));
    }
    Ok(earliest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_exponentially_and_caps() {
        assert_eq!(backoff(1), BASE_BACKOFF_SECS);
        assert_eq!(backoff(2), BASE_BACKOFF_SECS * 2);
        assert_eq!(backoff(3), BASE_BACKOFF_SECS * 4);
        assert_eq!(backoff(20), MAX_BACKOFF_SECS);
        assert_eq!(backoff(100), MAX_BACKOFF_SECS);
        assert_eq!(backoff(0), BASE_BACKOFF_SECS);
    }

    #[test]
    fn raw_names_are_unique() {
        assert_ne!(raw_name(1), raw_name(1));
    }
}
