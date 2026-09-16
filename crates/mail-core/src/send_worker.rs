// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Background sender for the offline outbox.
//!
//! The worker owns the transport and the retry loop: it drains the outbox on
//! startup, waits for the earliest backoff deadline and wakes up on command
//! (a freshly composed message or a restored connection) to try again.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;
use tokio::time::{Duration, Sleep, sleep};

use crate::outbox::{self, DrainMode, DrainReport};
use crate::send::MailSender;
use crate::store::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendWorkerConfig {
    pub account_id: i64,
    pub raw_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendCommand {
    Send { raw: Vec<u8> },
    Drain,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendEvent {
    Drained(DrainReport),
    Failed {
        operation: &'static str,
        detail: String,
    },
}

#[derive(Debug)]
pub struct SendWorker {
    commands: mpsc::UnboundedSender<SendCommand>,
}

impl SendWorker {
    pub fn spawn<M>(
        sender: M,
        store: Store,
        config: SendWorkerConfig,
    ) -> (Self, mpsc::UnboundedReceiver<SendEvent>)
    where
        M: MailSender + Send + 'static,
    {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name(format!("send-{}", config.account_id))
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => {
                        runtime.block_on(run(sender, store, config, command_rx, event_tx))
                    }
                    Err(err) => {
                        let _ = event_tx.send(SendEvent::Failed {
                            operation: "runtime",
                            detail: err.to_string(),
                        });
                    }
                }
            })
            .expect("failed to spawn send worker thread");
        (
            Self {
                commands: command_tx,
            },
            event_rx,
        )
    }

    pub fn send(&self, command: SendCommand) -> bool {
        self.commands.send(command).is_ok()
    }

    pub fn shutdown(&mut self) {
        let _ = self.commands.send(SendCommand::Shutdown);
    }
}

impl Drop for SendWorker {
    fn drop(&mut self) {
        let _ = self.commands.send(SendCommand::Shutdown);
    }
}

async fn wait_timer(timer: Option<Sleep>) {
    match timer {
        Some(timer) => timer.await,
        None => std::future::pending::<()>().await,
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

async fn run<M: MailSender>(
    mut sender: M,
    store: Store,
    config: SendWorkerConfig,
    mut commands: mpsc::UnboundedReceiver<SendCommand>,
    events: mpsc::UnboundedSender<SendEvent>,
) {
    let mut mode = DrainMode::Force;
    loop {
        match outbox::drain(&mut sender, &store, config.account_id, now(), mode).await {
            Ok(report) => {
                if !report.is_empty() {
                    let _ = events.send(SendEvent::Drained(report));
                }
            }
            Err(err) => {
                let _ = events.send(SendEvent::Failed {
                    operation: "send",
                    detail: err.to_string(),
                });
            }
        }
        mode = DrainMode::Due;

        let timer = match outbox::next_retry(&store, config.account_id).await {
            Ok(Some(at)) => Some(sleep(Duration::from_secs((at - now()).max(1) as u64))),
            Ok(None) => None,
            Err(err) => {
                let _ = events.send(SendEvent::Failed {
                    operation: "outbox",
                    detail: err.to_string(),
                });
                None
            }
        };

        tokio::select! {
            command = commands.recv() => match command {
                Some(SendCommand::Send { raw }) => {
                    if let Err(err) =
                        outbox::enqueue(&store, config.account_id, &raw, &config.raw_dir).await
                    {
                        let _ = events.send(SendEvent::Failed {
                            operation: "enqueue",
                            detail: err.to_string(),
                        });
                    }
                    mode = DrainMode::Force;
                }
                Some(SendCommand::Drain) => mode = DrainMode::Force,
                Some(SendCommand::Shutdown) | None => break,
            },
            () = wait_timer(timer) => {}
        }
    }
}
