use std::collections::VecDeque;
use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::MailBackend;
use crate::account::AccountConfig;
use crate::backend::Credential;
use crate::envelope::FlagChange;
use crate::store::{SpecialUse, Store};
use crate::sync::{
    EnvelopeWindow, FolderSync, IdleEvent, ReplayReport, fetch_body as fetch_message_body,
    queue_delete, queue_move, queue_set_flags, replay_pending, sync_account, sync_folder,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    pub account_id: i64,
    pub account: AccountConfig,
    pub credential: Credential,
    pub window: EnvelopeWindow,
    pub body_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerCommand {
    Sync,
    SyncFolder {
        folder: String,
    },
    FetchBody {
        folder: String,
        uid: u32,
    },
    SetFlags {
        folder: String,
        uids: Vec<u32>,
        change: FlagChange,
    },
    Move {
        from: String,
        to: String,
        uids: Vec<u32>,
    },
    Copy {
        from: String,
        to: String,
        uids: Vec<u32>,
    },
    Delete {
        folder: String,
        uids: Vec<u32>,
    },
    Watch {
        folder: String,
    },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerEvent {
    Connected,
    Disconnected,
    FolderSynced(FolderSync),
    AccountSynced {
        folders: usize,
    },
    BodyFetched {
        message_id: i64,
        attachments: usize,
    },
    FlagsChanged {
        folder: String,
        uids: Vec<u32>,
    },
    MessagesMoved {
        from: String,
        to: String,
        uids: Vec<u32>,
    },
    OpsReplayed(ReplayReport),
    OfflineQueued {
        folder: String,
        uids: Vec<u32>,
    },
    Idle(IdleEvent),
    Failed {
        operation: &'static str,
        detail: String,
    },
}

#[derive(Debug)]
pub struct AccountWorker {
    commands: mpsc::UnboundedSender<WorkerCommand>,
}

impl AccountWorker {
    pub fn spawn<B>(
        backend: B,
        store: Store,
        config: WorkerConfig,
    ) -> (Self, mpsc::UnboundedReceiver<WorkerEvent>)
    where
        B: MailBackend + Send + 'static,
    {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name(format!("sync-{}", config.account_id))
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(1)
                    .build();
                match runtime {
                    Ok(runtime) => {
                        runtime.block_on(run(backend, store, config, command_rx, event_tx))
                    }
                    Err(err) => {
                        let _ = event_tx.send(WorkerEvent::Failed {
                            operation: "runtime",
                            detail: err.to_string(),
                        });
                    }
                }
            })
            .expect("failed to spawn sync worker thread");
        (
            Self {
                commands: command_tx,
            },
            event_rx,
        )
    }

    pub fn send(&self, command: WorkerCommand) -> bool {
        self.commands.send(command).is_ok()
    }

    pub fn shutdown(&mut self) {
        let _ = self.commands.send(WorkerCommand::Shutdown);
    }
}

impl Drop for AccountWorker {
    fn drop(&mut self) {
        let _ = self.commands.send(WorkerCommand::Shutdown);
    }
}

struct Worker<B> {
    backend: B,
    store: Store,
    config: WorkerConfig,
    events: mpsc::UnboundedSender<WorkerEvent>,
    queue: VecDeque<WorkerCommand>,
    connected: bool,
    watch: Option<String>,
    resync_pending: bool,
}

impl<B: MailBackend> Worker<B> {
    fn emit(&self, event: WorkerEvent) -> bool {
        self.events.send(event).is_ok()
    }

    async fn ensure_connected(&mut self) -> bool {
        if self.connected {
            return true;
        }
        match self
            .backend
            .connect(&self.config.account, &self.config.credential)
            .await
        {
            Ok(()) => {
                self.connected = true;
                let _ = self.emit(WorkerEvent::Connected);
                true
            }
            Err(err) => {
                let _ = self.emit(WorkerEvent::Failed {
                    operation: "connect",
                    detail: err.to_string(),
                });
                false
            }
        }
    }

    async fn replay(&mut self) {
        match replay_pending(&mut self.backend, &self.store, self.config.account_id).await {
            Ok(report) => {
                let _ = self.emit(WorkerEvent::OpsReplayed(report));
            }
            Err(err) => {
                self.connected = false;
                let _ = self.emit(WorkerEvent::Failed {
                    operation: "replay",
                    detail: err.to_string(),
                });
            }
        }
    }

    async fn sync_account(&mut self) {
        match sync_account(
            &mut self.backend,
            &self.store,
            self.config.account_id,
            self.config.window,
        )
        .await
        {
            Ok(reports) => {
                let folders = reports.len();
                for report in reports {
                    let _ = self.emit(WorkerEvent::FolderSynced(report));
                }
                let _ = self.emit(WorkerEvent::AccountSynced { folders });
            }
            Err(err) => {
                self.connected = false;
                let _ = self.emit(WorkerEvent::Failed {
                    operation: "sync account",
                    detail: err.to_string(),
                });
            }
        }
    }

    async fn sync_folder(&mut self, name: &str) {
        let folder = match self.store.folders(self.config.account_id).await {
            Ok(folders) => folders.into_iter().find(|folder| folder.name == name),
            Err(err) => {
                let _ = self.emit(WorkerEvent::Failed {
                    operation: "sync folder",
                    detail: err.to_string(),
                });
                return;
            }
        };
        let Some(folder) = folder else {
            let _ = self.emit(WorkerEvent::Failed {
                operation: "sync folder",
                detail: format!("unknown folder {name}"),
            });
            return;
        };
        match sync_folder(&mut self.backend, &self.store, &folder, self.config.window).await {
            Ok(report) => {
                let _ = self.emit(WorkerEvent::FolderSynced(report));
            }
            Err(err) => {
                self.connected = false;
                let _ = self.emit(WorkerEvent::Failed {
                    operation: "sync folder",
                    detail: err.to_string(),
                });
            }
        }
    }

    async fn adopt_default_watch(&mut self) {
        if self.watch.is_some() || !self.backend.supports_idle() {
            return;
        }
        let Ok(folders) = self.store.folders(self.config.account_id).await else {
            return;
        };
        let Some(inbox) = folders
            .iter()
            .find(|folder| folder.special_use == Some(SpecialUse::Inbox))
        else {
            return;
        };
        self.watch = Some(inbox.name.clone());
    }

    async fn fetch_body(&mut self, folder: &str, uid: u32) {
        let Some(folder_id) = self.folder_id(folder).await else {
            let _ = self.emit(WorkerEvent::Failed {
                operation: "fetch body",
                detail: format!("unknown folder {folder}"),
            });
            return;
        };
        match fetch_message_body(
            &mut self.backend,
            &self.store,
            folder,
            folder_id,
            uid,
            &self.config.body_dir,
        )
        .await
        {
            Ok(fetch) => {
                let _ = self.emit(WorkerEvent::BodyFetched {
                    message_id: fetch.message_id,
                    attachments: fetch.attachments,
                });
            }
            Err(err) => {
                self.connected = false;
                let _ = self.emit(WorkerEvent::Failed {
                    operation: "fetch body",
                    detail: err.to_string(),
                });
            }
        }
    }

    async fn set_flags(&mut self, folder: &str, uids: &[u32], change: FlagChange) {
        if uids.is_empty() {
            return;
        }
        match self.backend.set_flags(folder, uids, change).await {
            Ok(()) => {
                let _ = self.emit(WorkerEvent::FlagsChanged {
                    folder: folder.to_string(),
                    uids: uids.to_vec(),
                });
                self.queue.push_back(WorkerCommand::SyncFolder {
                    folder: folder.to_string(),
                });
            }
            Err(_) => self.queue_flag_ops(folder, uids, change).await,
        }
    }

    async fn move_messages(&mut self, from: &str, to: &str, uids: &[u32]) {
        if uids.is_empty() {
            return;
        }
        match self.backend.move_messages(from, to, uids).await {
            Ok(()) => {
                let _ = self.emit(WorkerEvent::MessagesMoved {
                    from: from.to_string(),
                    to: to.to_string(),
                    uids: uids.to_vec(),
                });
                self.queue.push_back(WorkerCommand::SyncFolder {
                    folder: from.to_string(),
                });
                self.queue.push_back(WorkerCommand::SyncFolder {
                    folder: to.to_string(),
                });
            }
            Err(_) => self.queue_moves(from, to, uids).await,
        }
    }

    async fn copy_messages(&mut self, from: &str, to: &str, uids: &[u32]) {
        if uids.is_empty() {
            return;
        }
        match self.backend.copy_messages(from, to, uids).await {
            Ok(()) => {
                self.queue.push_back(WorkerCommand::SyncFolder {
                    folder: to.to_string(),
                });
            }
            Err(err) => {
                let _ = self.emit(WorkerEvent::Failed {
                    operation: "copy",
                    detail: err.to_string(),
                });
            }
        }
    }

    async fn delete(&mut self, folder: &str, uids: &[u32]) {
        if uids.is_empty() {
            return;
        }
        let change = FlagChange {
            deleted: Some(true),
            ..FlagChange::default()
        };
        match self.backend.set_flags(folder, uids, change).await {
            Ok(()) => {
                let _ = self.emit(WorkerEvent::FlagsChanged {
                    folder: folder.to_string(),
                    uids: uids.to_vec(),
                });
                self.queue.push_back(WorkerCommand::SyncFolder {
                    folder: folder.to_string(),
                });
            }
            Err(_) => self.queue_deletes(folder, uids).await,
        }
    }

    async fn watch(&mut self, folder: &str) {
        if self.folder_id(folder).await.is_some() {
            self.sync_folder(folder).await;
        } else {
            self.sync_account().await;
        }
        if self.backend.supports_idle() {
            self.watch = Some(folder.to_string());
        } else {
            self.watch = None;
            let _ = self.emit(WorkerEvent::Idle(IdleEvent::Unsupported {
                folder: folder.to_string(),
            }));
        }
    }

    async fn queue_flag_ops(&mut self, folder: &str, uids: &[u32], change: FlagChange) {
        let Some(folder_id) = self.folder_id(folder).await else {
            let _ = self.emit(WorkerEvent::Failed {
                operation: "queue flags",
                detail: format!("unknown folder {folder}"),
            });
            return;
        };
        for uid in uids {
            if let Err(err) =
                queue_set_flags(&self.store, self.config.account_id, folder_id, *uid, change).await
            {
                let _ = self.emit(WorkerEvent::Failed {
                    operation: "queue flags",
                    detail: err.to_string(),
                });
                return;
            }
        }
        let _ = self.emit(WorkerEvent::OfflineQueued {
            folder: folder.to_string(),
            uids: uids.to_vec(),
        });
    }

    async fn queue_moves(&mut self, from: &str, to: &str, uids: &[u32]) {
        let (Some(from_id), Some(to_id)) = (self.folder_id(from).await, self.folder_id(to).await)
        else {
            let _ = self.emit(WorkerEvent::Failed {
                operation: "queue move",
                detail: format!("unknown folder {from} or {to}"),
            });
            return;
        };
        for uid in uids {
            if let Err(err) =
                queue_move(&self.store, self.config.account_id, from_id, *uid, to_id).await
            {
                let _ = self.emit(WorkerEvent::Failed {
                    operation: "queue move",
                    detail: err.to_string(),
                });
                return;
            }
        }
        let _ = self.emit(WorkerEvent::OfflineQueued {
            folder: from.to_string(),
            uids: uids.to_vec(),
        });
    }

    async fn queue_deletes(&mut self, folder: &str, uids: &[u32]) {
        let Some(folder_id) = self.folder_id(folder).await else {
            let _ = self.emit(WorkerEvent::Failed {
                operation: "queue delete",
                detail: format!("unknown folder {folder}"),
            });
            return;
        };
        for uid in uids {
            if let Err(err) =
                queue_delete(&self.store, self.config.account_id, folder_id, *uid).await
            {
                let _ = self.emit(WorkerEvent::Failed {
                    operation: "queue delete",
                    detail: err.to_string(),
                });
                return;
            }
        }
        let _ = self.emit(WorkerEvent::OfflineQueued {
            folder: folder.to_string(),
            uids: uids.to_vec(),
        });
    }

    async fn folder_id(&self, name: &str) -> Option<i64> {
        match self.store.folders(self.config.account_id).await {
            Ok(folders) => folders
                .into_iter()
                .find(|folder| folder.name == name)
                .and_then(|folder| folder.id),
            Err(err) => {
                let _ = self.emit(WorkerEvent::Failed {
                    operation: "folder lookup",
                    detail: err.to_string(),
                });
                None
            }
        }
    }

    async fn handle(&mut self, command: WorkerCommand) -> bool {
        match command {
            WorkerCommand::Shutdown => return false,
            WorkerCommand::Sync => {
                if self.ensure_connected().await {
                    self.replay().await;
                    self.sync_account().await;
                    self.adopt_default_watch().await;
                }
            }
            WorkerCommand::SyncFolder { folder } => {
                self.resync_pending = false;
                if self.ensure_connected().await {
                    self.sync_folder(&folder).await;
                }
            }
            WorkerCommand::FetchBody { folder, uid } => {
                if self.ensure_connected().await {
                    self.fetch_body(&folder, uid).await;
                }
            }
            WorkerCommand::SetFlags {
                folder,
                uids,
                change,
            } => {
                if self.ensure_connected().await {
                    self.set_flags(&folder, &uids, change).await;
                } else {
                    self.queue_flag_ops(&folder, &uids, change).await;
                }
            }
            WorkerCommand::Move { from, to, uids } => {
                if self.ensure_connected().await {
                    self.move_messages(&from, &to, &uids).await;
                } else {
                    self.queue_moves(&from, &to, &uids).await;
                }
            }
            WorkerCommand::Copy { from, to, uids } => {
                if self.ensure_connected().await {
                    self.copy_messages(&from, &to, &uids).await;
                } else {
                    let _ = self.emit(WorkerEvent::Failed {
                        operation: "copy",
                        detail: "backend is not connected".to_string(),
                    });
                }
            }
            WorkerCommand::Delete { folder, uids } => {
                if self.ensure_connected().await {
                    self.delete(&folder, &uids).await;
                } else {
                    self.queue_deletes(&folder, &uids).await;
                }
            }
            WorkerCommand::Watch { folder } => {
                if self.ensure_connected().await {
                    self.watch(&folder).await;
                }
            }
        }
        true
    }
}

async fn run<B: MailBackend>(
    backend: B,
    store: Store,
    config: WorkerConfig,
    mut commands: mpsc::UnboundedReceiver<WorkerCommand>,
    events: mpsc::UnboundedSender<WorkerEvent>,
) {
    let mut worker = Worker {
        backend,
        store,
        config,
        events,
        queue: VecDeque::from([WorkerCommand::Sync]),
        connected: false,
        watch: None,
        resync_pending: false,
    };

    'worker: loop {
        while let Some(command) = worker.queue.pop_front() {
            if !worker.handle(command).await {
                break 'worker;
            }
        }

        while let Ok(command) = commands.try_recv() {
            worker.queue.push_back(command);
        }
        if !worker.queue.is_empty() {
            continue;
        }

        match worker.watch.clone() {
            Some(folder) if worker.backend.supports_idle() => {
                match worker.backend.idle(&folder).await {
                    Ok(()) => {
                        let _ = worker.emit(WorkerEvent::Idle(IdleEvent::Changed {
                            folder: folder.clone(),
                        }));
                        if !worker.resync_pending {
                            worker.resync_pending = true;
                            worker.queue.push_back(WorkerCommand::SyncFolder { folder });
                        }
                    }
                    Err(err) => {
                        let _ = worker.emit(WorkerEvent::Idle(IdleEvent::Failed {
                            folder: folder.clone(),
                            detail: err.to_string(),
                        }));
                        worker.watch = None;
                        worker.connected = false;
                    }
                }
            }
            _ => match commands.recv().await {
                Some(command) => worker.queue.push_back(command),
                None => break 'worker,
            },
        }
    }

    if worker.connected {
        let _ = worker.backend.disconnect().await;
    }
    let _ = worker.emit(WorkerEvent::Disconnected);
}
