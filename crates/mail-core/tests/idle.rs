// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::FakeBackend;
use mail_core::sync::{IdleEvent, IdleWorker};
use tokio::sync::mpsc;

#[tokio::test]
async fn idle_worker_emits_changes() {
    let mut backend = FakeBackend::new();
    backend.idle_supported = true;
    let (sender, mut receiver) = mpsc::channel(4);
    let worker = IdleWorker::spawn(backend, "INBOX", sender);

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
    let worker = IdleWorker::spawn(FakeBackend::new(), "INBOX", sender);

    assert_eq!(
        receiver.recv().await.unwrap(),
        IdleEvent::Unsupported {
            folder: "INBOX".to_string()
        }
    );
    assert!(receiver.try_recv().is_err());
    worker.abort();
}
