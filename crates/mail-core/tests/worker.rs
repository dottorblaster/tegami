mod common;

use std::time::Duration;

use common::FakeBackend;
use mail_core::account::AccountConfig;
use mail_core::envelope::FlagChange;
use mail_core::store::{AccountRecord, AccountSource, AuthKind, BodyState, FLAG_SEEN, Store};
use mail_core::sync::{EnvelopeWindow, IdleEvent, ReplayReport, queue_set_flags};
use mail_core::worker::{AccountWorker, WorkerCommand, WorkerConfig, WorkerEvent};
use tempfile::TempDir;
use tokio::sync::mpsc;

fn account() -> AccountRecord {
    AccountRecord {
        id: None,
        source: AccountSource::Goa,
        external_id: "account_1".to_string(),
        email: "me@example.org".to_string(),
        display_name: None,
        imap_host: None,
        imap_port: None,
        imap_security: None,
        smtp_host: None,
        smtp_port: None,
        smtp_security: None,
        auth_kind: AuthKind::Password,
        username: None,
    }
}

fn worker_config(account_id: i64, body_dir: &std::path::Path) -> WorkerConfig {
    WorkerConfig {
        account_id,
        account: AccountConfig {
            id: "account_1".to_string(),
            name: "Test".to_string(),
            email_address: "me@example.org".to_string(),
            provider_type: None,
            is_temporary: false,
            imap: None,
            smtp: None,
        },
        credential: mail_core::Credential::Password("secret".to_string()),
        window: EnvelopeWindow::new(10),
        body_dir: body_dir.to_path_buf(),
    }
}

async fn recv(events: &mut mpsc::UnboundedReceiver<WorkerEvent>) -> WorkerEvent {
    tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("timed out waiting for worker event")
        .expect("worker event channel closed")
}

async fn folder_id(store: &Store, account_id: i64, name: &str) -> i64 {
    store
        .folders(account_id)
        .await
        .unwrap()
        .into_iter()
        .find(|folder| folder.name == name)
        .unwrap()
        .id
        .unwrap()
}

#[tokio::test]
async fn worker_connects_syncs_and_shuts_down() {
    let backend = FakeBackend::with_folders(&[("INBOX", 3)]);
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let temp = TempDir::new().unwrap();
    let (mut worker, mut events) = AccountWorker::spawn(
        backend,
        store.clone(),
        worker_config(account_id, temp.path()),
    );

    assert_eq!(recv(&mut events).await, WorkerEvent::Connected);
    assert_eq!(
        recv(&mut events).await,
        WorkerEvent::OpsReplayed(ReplayReport {
            replayed: 0,
            dropped: 0,
            remaining: 0,
        })
    );
    let synced = recv(&mut events).await;
    let inbox_id = folder_id(&store, account_id, "INBOX").await;
    assert!(matches!(
        synced,
        WorkerEvent::FolderSynced(report) if report.folder_id == inbox_id && report.changed == 3
    ));
    assert_eq!(
        recv(&mut events).await,
        WorkerEvent::AccountSynced { folders: 1 }
    );

    assert_eq!(store.messages(inbox_id).await.unwrap().len(), 3);

    worker.shutdown();
    assert_eq!(recv(&mut events).await, WorkerEvent::Disconnected);
}

#[tokio::test]
async fn worker_syncs_single_folder_on_command() {
    let backend = FakeBackend::with_folders(&[("INBOX", 2)]);
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let temp = TempDir::new().unwrap();
    let (mut worker, mut events) = AccountWorker::spawn(
        backend,
        store.clone(),
        worker_config(account_id, temp.path()),
    );

    let mut drain = 0;
    while drain < 4 {
        recv(&mut events).await;
        drain += 1;
    }
    let inbox_id = folder_id(&store, account_id, "INBOX").await;
    assert!(worker.send(WorkerCommand::SyncFolder {
        folder: "INBOX".to_string(),
    }));

    let report = recv(&mut events).await;
    assert!(
        matches!(report, WorkerEvent::FolderSynced(report) if report.folder_id == inbox_id && report.changed == 0)
    );
    worker.shutdown();
}

#[tokio::test]
async fn worker_has_backend_visible_to_external_mutations() {
    let backend = FakeBackend::with_folders(&[("INBOX", 3)]);
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let temp = TempDir::new().unwrap();
    let (mut worker, mut events) = AccountWorker::spawn(
        backend,
        store.clone(),
        worker_config(account_id, temp.path()),
    );
    let mut drain = 0;
    while drain < 4 {
        recv(&mut events).await;
        drain += 1;
    }
    let inbox_id = folder_id(&store, account_id, "INBOX").await;
    assert!(worker.send(WorkerCommand::SetFlags {
        folder: "INBOX".to_string(),
        uids: vec![1],
        change: FlagChange {
            seen: Some(true),
            ..FlagChange::default()
        },
    }));

    assert_eq!(
        recv(&mut events).await,
        WorkerEvent::FlagsChanged {
            folder: "INBOX".to_string(),
            uids: vec![1],
        }
    );
    let report = recv(&mut events).await;
    assert!(matches!(report, WorkerEvent::FolderSynced(report) if report.changed == 1));
    assert!(store.message(inbox_id, 1).await.unwrap().unwrap().flags & FLAG_SEEN != 0);
    worker.shutdown();
}

#[tokio::test]
async fn worker_fetches_body_into_cache() {
    let mut backend = FakeBackend::with_folders(&[("INBOX", 1)]);
    backend.set_body("INBOX", 1, b"Subject: hello\r\n\r\ngreeting\r\n");
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let temp = TempDir::new().unwrap();
    let (mut worker, mut events) = AccountWorker::spawn(
        backend,
        store.clone(),
        worker_config(account_id, temp.path()),
    );
    let mut drain = 0;
    while drain < 4 {
        recv(&mut events).await;
        drain += 1;
    }
    let inbox_id = folder_id(&store, account_id, "INBOX").await;
    assert!(worker.send(WorkerCommand::FetchBody {
        folder: "INBOX".to_string(),
        uid: 1,
    }));

    let fetched = recv(&mut events).await;
    assert!(matches!(
        fetched,
        WorkerEvent::BodyFetched { message_id, .. } if message_id > 0
    ));
    let message = store.message(inbox_id, 1).await.unwrap().unwrap();
    assert_eq!(message.body_state, BodyState::Full);
    assert!(message.raw_path.is_some());
    worker.shutdown();
}

#[tokio::test]
async fn worker_replays_pending_ops_before_sync() {
    let mut backend = FakeBackend::with_folders(&[("INBOX", 2)]);
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    mail_core::sync::sync_account(&mut backend, &store, account_id, EnvelopeWindow::new(10))
        .await
        .unwrap();
    let inbox_id = folder_id(&store, account_id, "INBOX").await;
    queue_set_flags(
        &store,
        account_id,
        inbox_id,
        1,
        FlagChange {
            seen: Some(true),
            ..FlagChange::default()
        },
    )
    .await
    .unwrap();

    let temp = TempDir::new().unwrap();
    let (mut worker, mut events) = AccountWorker::spawn(
        backend,
        store.clone(),
        worker_config(account_id, temp.path()),
    );

    assert_eq!(recv(&mut events).await, WorkerEvent::Connected);
    assert_eq!(
        recv(&mut events).await,
        WorkerEvent::OpsReplayed(ReplayReport {
            replayed: 1,
            dropped: 0,
            remaining: 0,
        })
    );
    assert!(store.pending_ops(account_id).await.unwrap().is_empty());
    worker.shutdown();
}

#[tokio::test]
async fn worker_watches_inbox_and_resyncs_on_idle_signal() {
    let mut backend = FakeBackend::with_folders(&[("INBOX", 2)]);
    backend.idle_supported = true;
    backend.idle_delay = Duration::from_millis(5);
    let store = Store::open(":memory:").unwrap();
    let account_id = store.upsert_account(account()).await.unwrap();
    let temp = TempDir::new().unwrap();
    let (mut worker, mut events) = AccountWorker::spawn(
        backend,
        store.clone(),
        worker_config(account_id, temp.path()),
    );

    let mut saw_idle_change = false;
    let mut syncs = 0;
    loop {
        let event = recv(&mut events).await;
        match event {
            WorkerEvent::Idle(IdleEvent::Changed { .. }) => {
                saw_idle_change = true;
            }
            WorkerEvent::FolderSynced(_) => {
                syncs += 1;
            }
            _ => {}
        }
        if saw_idle_change && syncs > 0 {
            break;
        }
    }
    assert!(saw_idle_change);
    worker.shutdown();
}
