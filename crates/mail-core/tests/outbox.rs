// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::VecDeque;
use std::path::Path;

use mail_core::MailError;
use mail_core::compose::build;
use mail_core::outbox::{
    BASE_BACKOFF_SECS, DrainMode, MAX_ATTEMPTS, backoff, drain, enqueue, next_retry,
};
use mail_core::send::{MailEnvelope, MailSender};
use mail_core::send_worker::{SendCommand, SendEvent, SendWorker, SendWorkerConfig};
use mail_core::store::{AccountRecord, AccountSource, AuthKind, OutboxRecord, OutboxState, Store};
use tempfile::TempDir;

#[derive(Default)]
struct FakeSender {
    results: VecDeque<Result<(), MailError>>,
    sent: Vec<Vec<u8>>,
}

impl FakeSender {
    fn failing() -> Self {
        Self {
            results: VecDeque::from([Err(MailError::Disconnected)]),
            sent: Vec::new(),
        }
    }
}

impl MailSender for FakeSender {
    async fn send(&mut self, _envelope: &MailEnvelope, raw: &[u8]) -> Result<(), MailError> {
        self.sent.push(raw.to_vec());
        match self.results.pop_front() {
            Some(result) => result,
            None => Ok(()),
        }
    }
}

fn account() -> AccountRecord {
    AccountRecord {
        id: None,
        source: AccountSource::Goa,
        external_id: "account_1".to_string(),
        email: "user@example.org".to_string(),
        display_name: None,
        imap_host: None,
        imap_port: None,
        imap_security: None,
        smtp_host: Some("smtp.example.org".to_string()),
        smtp_port: Some(465),
        smtp_security: Some(mail_core::store::Security::Ssl),
        auth_kind: AuthKind::Password,
        username: None,
    }
}

fn message(to: &[&str]) -> Vec<u8> {
    build(&mail_core::compose::OutgoingMessage {
        from: Some(mail_core::compose::Address::new(
            Some("Ada".to_string()),
            "ada@lovelace.dev",
        )),
        to: to
            .iter()
            .map(|email| mail_core::compose::Address::new(None, *email))
            .collect(),
        subject: "Greetings".to_string(),
        text: Some("Hello there".to_string()),
        ..mail_core::compose::OutgoingMessage::default()
    })
    .unwrap()
}

async fn seeded() -> (Store, TempDir, i64) {
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    (store, TempDir::new().unwrap(), account_id)
}

fn entry(account_id: i64, raw_path: String) -> OutboxRecord {
    OutboxRecord::queued(account_id, raw_path, 1_700_000_000)
}

fn written_raw(dir: &Path, raw: &[u8]) -> String {
    let path = dir.join("raw.eml");
    std::fs::write(&path, raw).unwrap();
    path.to_string_lossy().into_owned()
}

#[tokio::test]
async fn enqueue_writes_the_raw_message_and_records_it() {
    let (store, dir, account_id) = seeded().await;
    let raw = message(&["grace@navy.dev"]);
    let id = enqueue(&store, account_id, &raw, dir.path()).await.unwrap();

    let records = store.outbox(account_id).await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, Some(id));
    assert_eq!(records[0].state, OutboxState::Queued);
    assert_eq!(records[0].attempts, 0);
    assert_eq!(records[0].send_after, None);
    assert!(Path::new(&records[0].raw_path).metadata().is_ok());
}

#[tokio::test]
async fn drain_submits_and_marks_sent() {
    let (store, dir, account_id) = seeded().await;
    let raw = message(&["grace@navy.dev"]);
    let id = enqueue(&store, account_id, &raw, dir.path()).await.unwrap();
    let mut sender = FakeSender::default();

    let report = drain(
        &mut sender,
        &store,
        account_id,
        1_700_000_100,
        DrainMode::Due,
    )
    .await
    .unwrap();
    assert_eq!(report.sent, 1);
    assert_eq!(report.retried, 0);
    assert_eq!(report.failed, 0);
    assert_eq!(sender.sent.len(), 1);
    assert_eq!(sender.sent[0], raw);

    let records = store.outbox(account_id).await.unwrap();
    assert_eq!(records[0].id, Some(id));
    assert_eq!(records[0].state, OutboxState::Sent);
    assert_eq!(records[0].send_after, None);
    assert!(next_retry(&store, account_id).await.unwrap().is_none());
}

#[tokio::test]
async fn failures_are_retried_with_backoff() {
    let (store, dir, account_id) = seeded().await;
    let raw = message(&["grace@navy.dev"]);
    let id = enqueue(&store, account_id, &raw, dir.path()).await.unwrap();
    let mut sender = FakeSender::failing();

    let now = 1_700_000_100;
    let report = drain(&mut sender, &store, account_id, now, DrainMode::Due)
        .await
        .unwrap();
    assert_eq!(report.sent, 0);
    assert_eq!(report.retried, 1);

    let record = store.outbox(account_id).await.unwrap().remove(0);
    assert_eq!(record.id, Some(id));
    assert_eq!(record.state, OutboxState::Queued);
    assert_eq!(record.attempts, 1);
    assert_eq!(record.send_after, Some(now + backoff(1)));
    assert_eq!(
        record.last_error.as_deref(),
        Some("backend is not connected")
    );

    assert_eq!(
        next_retry(&store, account_id).await.unwrap(),
        Some(now + backoff(1))
    );

    // Not due yet, nothing is replayed.
    let report = drain(&mut sender, &store, account_id, now + 1, DrainMode::Due)
        .await
        .unwrap();
    assert!(report.is_empty());

    // Once the backoff elapses the entry goes out.
    let report = drain(
        &mut sender,
        &store,
        account_id,
        now + backoff(1),
        DrainMode::Due,
    )
    .await
    .unwrap();
    assert_eq!(report.sent, 1);
    let record = store.outbox(account_id).await.unwrap().remove(0);
    assert_eq!(record.state, OutboxState::Sent);
}

#[tokio::test]
async fn entries_stop_after_max_attempts() {
    let (store, dir, account_id) = seeded().await;
    let raw = message(&["grace@navy.dev"]);
    let id = store
        .enqueue_outbox(OutboxRecord {
            attempts: MAX_ATTEMPTS - 1,
            ..entry(account_id, written_raw(dir.path(), &raw))
        })
        .await
        .unwrap();
    let mut sender = FakeSender::failing();

    let report = drain(
        &mut sender,
        &store,
        account_id,
        1_700_000_100,
        DrainMode::Due,
    )
    .await
    .unwrap();
    assert_eq!(report.retried, 0);
    assert_eq!(report.failed, 1);

    let record = store.outbox(account_id).await.unwrap().remove(0);
    assert_eq!(record.id, Some(id));
    assert_eq!(record.state, OutboxState::Failed);
    assert_eq!(record.attempts, MAX_ATTEMPTS);
    assert_eq!(record.send_after, None);
    assert!(next_retry(&store, account_id).await.unwrap().is_none());
}

#[tokio::test]
async fn a_missing_raw_file_fails_permanently() {
    let (store, dir, account_id) = seeded().await;
    store
        .enqueue_outbox(entry(
            account_id,
            dir.path().join("gone.eml").to_string_lossy().into_owned(),
        ))
        .await
        .unwrap();

    let mut sender = FakeSender::default();
    let report = drain(
        &mut sender,
        &store,
        account_id,
        1_700_000_100,
        DrainMode::Due,
    )
    .await
    .unwrap();
    assert_eq!(report.failed, 1);
    assert!(sender.sent.is_empty());

    let record = store.outbox(account_id).await.unwrap().remove(0);
    assert_eq!(record.state, OutboxState::Failed);
    assert!(
        record
            .last_error
            .as_deref()
            .unwrap()
            .contains("cannot read")
    );
}

#[tokio::test]
async fn messages_without_recipients_fail_permanently() {
    let (store, dir, account_id) = seeded().await;
    let raw = message(&[]);
    let id = enqueue(&store, account_id, &raw, dir.path()).await.unwrap();

    let mut sender = FakeSender::default();
    let report = drain(
        &mut sender,
        &store,
        account_id,
        1_700_000_100,
        DrainMode::Due,
    )
    .await
    .unwrap();
    assert_eq!(report.failed, 1);
    assert!(sender.sent.is_empty());

    let record = store.outbox(account_id).await.unwrap().remove(0);
    assert_eq!(record.id, Some(id));
    assert_eq!(record.state, OutboxState::Failed);
    assert!(
        record
            .last_error
            .as_deref()
            .unwrap()
            .contains("no usable recipients")
    );
}

#[tokio::test]
async fn forced_drain_bypasses_the_backoff() {
    let (store, dir, account_id) = seeded().await;
    let raw = message(&["grace@navy.dev"]);
    enqueue(&store, account_id, &raw, dir.path()).await.unwrap();
    let mut sender = FakeSender::failing();
    let now = 1_700_000_100;
    drain(&mut sender, &store, account_id, now, DrainMode::Due)
        .await
        .unwrap();

    let mut sender = FakeSender::default();
    let report = drain(&mut sender, &store, account_id, now, DrainMode::Force)
        .await
        .unwrap();
    assert_eq!(report.sent, 1);
    let record = store.outbox(account_id).await.unwrap().remove(0);
    assert_eq!(record.state, OutboxState::Sent);
}

#[tokio::test]
async fn the_worker_sends_queued_mail() {
    let (store, dir, account_id) = seeded().await;
    let raw = message(&["grace@navy.dev"]);
    let (worker, mut events) = SendWorker::spawn(
        FakeSender::default(),
        store.clone(),
        SendWorkerConfig {
            account_id,
            raw_dir: dir.path().to_path_buf(),
        },
    );
    worker.send(SendCommand::Send { raw });

    match tokio::time::timeout(std::time::Duration::from_secs(10), events.recv()).await {
        Ok(Some(SendEvent::Drained(report))) => {
            assert_eq!(report.sent, 1);
            assert_eq!(report.retried, 0);
            assert_eq!(report.failed, 0);
        }
        other => panic!("expected a drain event, got {other:?}"),
    }

    let records = store.outbox(account_id).await.unwrap();
    assert_eq!(records[0].state, OutboxState::Sent);
    let _ = worker;
}

#[tokio::test]
async fn the_worker_retries_failed_mail() {
    let (store, dir, account_id) = seeded().await;
    let raw = message(&["grace@navy.dev"]);
    let (worker, mut events) = SendWorker::spawn(
        FakeSender::failing(),
        store.clone(),
        SendWorkerConfig {
            account_id,
            raw_dir: dir.path().to_path_buf(),
        },
    );
    worker.send(SendCommand::Send { raw });

    match tokio::time::timeout(std::time::Duration::from_secs(10), events.recv()).await {
        Ok(Some(SendEvent::Drained(report))) => {
            assert_eq!(report.sent, 0);
            assert_eq!(report.retried, 1);
        }
        other => panic!("expected a drain event, got {other:?}"),
    }

    let record = store.outbox(account_id).await.unwrap().remove(0);
    assert_eq!(record.state, OutboxState::Queued);
    assert_eq!(record.attempts, 1);
    let _ = worker;
}

#[tokio::test]
async fn pruning_an_account_drops_its_outbox() {
    let (store, dir, account_id) = seeded().await;
    let raw = message(&["grace@navy.dev"]);
    enqueue(&store, account_id, &raw, dir.path()).await.unwrap();

    store.prune_accounts(&[]).await.unwrap();

    assert!(store.accounts().await.unwrap().is_empty());
    assert!(store.outbox(account_id).await.unwrap().is_empty());
}

#[test]
fn backoff_grows_capped() {
    assert_eq!(backoff(1), BASE_BACKOFF_SECS);
    assert_eq!(backoff(7), BASE_BACKOFF_SECS * 64);
    assert_eq!(backoff(8), 60 * 60);
    assert!(backoff(10) <= 60 * 60);
}
