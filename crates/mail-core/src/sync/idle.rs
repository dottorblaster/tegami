// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Arc;

use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

use crate::{IdleOutcome, MailBackend};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdleEvent {
    Changed { folder: String },
    Unsupported { folder: String },
    Failed { folder: String, detail: String },
}

pub struct IdleWorker {
    handle: JoinHandle<()>,
}

impl IdleWorker {
    pub fn spawn<B>(
        mut backend: B,
        folder: impl Into<String>,
        sender: mpsc::Sender<IdleEvent>,
    ) -> Self
    where
        B: MailBackend + Send + 'static,
    {
        let folder = folder.into();
        let handle = tokio::spawn(async move {
            if !backend.supports_idle() {
                let _ = sender.send(IdleEvent::Unsupported { folder }).await;
                return;
            }
            loop {
                let interrupt = Arc::new(Notify::new());
                match backend.idle(&folder, interrupt).await {
                    Ok(IdleOutcome::Changed) => {
                        let event = IdleEvent::Changed {
                            folder: folder.clone(),
                        };
                        if sender.send(event).await.is_err() {
                            break;
                        }
                    }
                    Ok(IdleOutcome::Interrupted) => {}
                    Err(err) => {
                        let event = IdleEvent::Failed {
                            folder: folder.clone(),
                            detail: err.to_string(),
                        };
                        let _ = sender.send(event).await;
                        break;
                    }
                }
            }
        });
        Self { handle }
    }

    pub fn abort(&self) {
        self.handle.abort();
    }

    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}
