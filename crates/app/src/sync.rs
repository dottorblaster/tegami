use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use accounts::AccountManager;
use accounts::CredentialWorker;
use imap::ImapBackend;
use mail_core::account::AccountConfig;
use mail_core::store::Store;
use mail_core::sync::EnvelopeWindow;
use mail_core::worker::{AccountWorker, WorkerCommand, WorkerConfig, WorkerEvent};
use relm4::prelude::*;
use tracing::{debug, warn};

use crate::config;

#[derive(Debug, Clone)]
pub struct ServiceAccount {
    pub account_id: i64,
    pub config: AccountConfig,
    pub credential: mail_core::Credential,
}

#[derive(Debug)]
pub enum SyncServiceMsg {
    Start,
    AccountsLoaded(Vec<ServiceAccount>),
    WorkerEvent {
        account_id: i64,
        event: WorkerEvent,
    },
    WatchFolder {
        account_id: i64,
        folder: String,
    },
    FetchBody {
        folder_id: i64,
        uid: u32,
    },
    FetchBodyResolved {
        account_id: i64,
        folder: String,
        uid: u32,
    },
    Failed {
        detail: String,
    },
}

#[derive(Debug)]
pub enum SyncServiceOutput {
    AccountsChanged,
    FolderChanged { folder_id: i64 },
    BodyFetched { message_id: i64 },
    Error { detail: String },
}

pub struct SyncService {
    store: Arc<Store>,
    body_dir: PathBuf,
    workers: HashMap<i64, AccountWorker>,
    watching: bool,
}

impl SyncService {
    fn spawn_worker(&mut self, account: ServiceAccount, sender: &ComponentSender<SyncService>) {
        let (worker, events) = AccountWorker::spawn(
            ImapBackend::new(),
            self.store.as_ref().clone(),
            WorkerConfig {
                account_id: account.account_id,
                account: account.config,
                credential: account.credential,
                window: EnvelopeWindow::default(),
                body_dir: self.body_dir.clone(),
            },
        );
        let account_id = account.account_id;
        self.workers.insert(account_id, worker);
        let bridge = sender.clone();
        relm4::spawn(async move {
            let mut events = events;
            while let Some(event) = events.recv().await {
                bridge.input(SyncServiceMsg::WorkerEvent { account_id, event });
            }
        });
    }

    fn reconcile(&mut self, accounts: Vec<ServiceAccount>, sender: &ComponentSender<SyncService>) {
        let mut seen: HashSet<i64> = HashSet::new();
        for account in accounts {
            seen.insert(account.account_id);
            if self.workers.contains_key(&account.account_id) {
                continue;
            }
            self.spawn_worker(account, sender);
        }
        self.workers.retain(|account_id, worker| {
            if seen.contains(account_id) {
                true
            } else {
                debug!(account_id, "account removed, shutting down worker");
                worker.shutdown();
                false
            }
        });
        let _ = sender.output(SyncServiceOutput::AccountsChanged);
    }

    fn handle_event(
        &mut self,
        account_id: i64,
        event: WorkerEvent,
        sender: &ComponentSender<SyncService>,
    ) {
        match event {
            WorkerEvent::FolderSynced(report) => {
                let _ = sender.output(SyncServiceOutput::FolderChanged {
                    folder_id: report.folder_id,
                });
            }
            WorkerEvent::AccountSynced { .. } => {
                let _ = sender.output(SyncServiceOutput::AccountsChanged);
            }
            WorkerEvent::Failed { operation, detail } => {
                warn!(account_id, operation, detail, "sync worker failure");
                let _ = sender.output(SyncServiceOutput::Error {
                    detail: format!("{operation}: {detail}"),
                });
            }
            WorkerEvent::BodyFetched { message_id, .. } => {
                let _ = sender.output(SyncServiceOutput::BodyFetched { message_id });
            }
            WorkerEvent::Connected
            | WorkerEvent::Disconnected
            | WorkerEvent::FlagsChanged { .. }
            | WorkerEvent::MessagesMoved { .. }
            | WorkerEvent::OpsReplayed(_)
            | WorkerEvent::OfflineQueued { .. }
            | WorkerEvent::Idle(_) => {}
        }
    }

    fn watch(&mut self, account_id: i64, folder: String) {
        if let Some(worker) = self.workers.get(&account_id) {
            worker.send(WorkerCommand::Watch { folder });
        }
    }
}

impl SimpleComponent for SyncService {
    type Init = Arc<Store>;
    type Input = SyncServiceMsg;
    type Output = SyncServiceOutput;
    type Root = ();
    type Widgets = ();

    fn init_root() {}

    fn init(
        store: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = SyncService {
            store,
            body_dir: config::body_dir(),
            workers: HashMap::new(),
            watching: false,
        };
        sender.input(SyncServiceMsg::Start);
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            SyncServiceMsg::Start => {
                let store = self.store.clone();
                let start_sender = sender.clone();
                sender.oneshot_command(async move {
                    match discover(&store).await {
                        Ok(accounts) => {
                            start_sender.input(SyncServiceMsg::AccountsLoaded(accounts))
                        }
                        Err(err) => start_sender.input(SyncServiceMsg::Failed { detail: err }),
                    }
                });
            }
            SyncServiceMsg::AccountsLoaded(accounts) => {
                self.reconcile(accounts, &sender);
                if !self.watching {
                    self.watching = true;
                    start_watcher(self.store.clone(), &sender);
                }
            }
            SyncServiceMsg::WorkerEvent { account_id, event } => {
                self.handle_event(account_id, event, &sender);
            }
            SyncServiceMsg::WatchFolder { account_id, folder } => {
                self.watch(account_id, folder);
            }
            SyncServiceMsg::FetchBody { folder_id, uid } => {
                let store = self.store.clone();
                let fetch_sender = sender.clone();
                sender.oneshot_command(async move {
                    match store.folder(folder_id).await {
                        Ok(Some(folder)) => fetch_sender.input(SyncServiceMsg::FetchBodyResolved {
                            account_id: folder.account_id,
                            folder: folder.name,
                            uid,
                        }),
                        Ok(None) => debug!(folder_id, uid, "cannot fetch body for unknown folder"),
                        Err(err) => debug!(folder_id, uid, err = %err, "cannot resolve folder"),
                    }
                });
            }
            SyncServiceMsg::FetchBodyResolved {
                account_id,
                folder,
                uid,
            } => {
                if let Some(worker) = self.workers.get(&account_id) {
                    worker.send(WorkerCommand::FetchBody { folder, uid });
                }
            }
            SyncServiceMsg::Failed { detail } => {
                warn!(detail, "account discovery failed");
                let _ = sender.output(SyncServiceOutput::Error { detail });
            }
        }
    }
}

async fn discover(store: &Store) -> Result<Vec<ServiceAccount>, String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|err| err.to_string())?;
    let manager = AccountManager::new(connection.clone());
    let credential_worker = CredentialWorker::new()
        .await
        .map_err(|err| err.to_string())?;
    let mut discovered = Vec::new();
    for account in manager.accounts().await {
        let Some(credential) = credential_worker
            .credentials(&connection, &account)
            .await
            .map_err(|err| format!("{err:?}"))?
        else {
            warn!(account = account.id(), "no credential available");
            continue;
        };
        let account_id = store
            .upsert_account(account.to_record())
            .await
            .map_err(|err| err.to_string())?;
        discovered.push(ServiceAccount {
            account_id,
            config: account.config.clone(),
            credential: match credential {
                accounts::Credentials::Password(password) => {
                    mail_core::Credential::Password(password)
                }
                accounts::Credentials::OAuth2(token) => mail_core::Credential::OAuth2(token),
            },
        });
    }
    Ok(discovered)
}

fn start_watcher(store: Arc<Store>, sender: &ComponentSender<SyncService>) {
    let watch_sender = sender.clone();
    sender.oneshot_command(async move {
        let connection = match zbus::Connection::session().await {
            Ok(connection) => connection,
            Err(err) => {
                watch_sender.input(SyncServiceMsg::Failed {
                    detail: err.to_string(),
                });
                return;
            }
        };
        let manager = AccountManager::new(connection.clone());
        let mut changes = match manager.changes().await {
            Ok(changes) => changes,
            Err(err) => {
                watch_sender.input(SyncServiceMsg::Failed {
                    detail: err.to_string(),
                });
                return;
            }
        };
        while changes.recv().await.is_some() {
            match discover(&store).await {
                Ok(accounts) => watch_sender.input(SyncServiceMsg::AccountsLoaded(accounts)),
                Err(err) => watch_sender.input(SyncServiceMsg::Failed { detail: err }),
            }
        }
    });
}
