// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::MailBackend;

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
                match backend.idle(&folder).await {
                    Ok(()) => {
                        let event = IdleEvent::Changed {
                            folder: folder.clone(),
                        };
                        if sender.send(event).await.is_err() {
                            break;
                        }
                    }
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
