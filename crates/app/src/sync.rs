use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use accounts::AccountManager;
use accounts::CredentialWorker;
use mail_core::account::AccountConfig;
use mail_core::envelope::FlagChange;
use mail_core::imap::ImapBackend;
use mail_core::store::{SpecialUse, Store, bits_to_flags};
use mail_core::sync::EnvelopeWindow;
use mail_core::worker::{AccountWorker, WorkerCommand, WorkerConfig, WorkerEvent};
use relm4::prelude::*;
use tracing::{debug, warn};

use crate::config;
use crate::message_text::{display_sender, display_subject};
use crate::notify::MailNotice;

#[derive(Debug, Clone)]
pub struct ServiceAccount {
    pub account_id: i64,
    pub config: AccountConfig,
    pub credential: mail_core::Credential,
}

#[derive(Debug, Clone)]
pub enum MessageAction {
    SetFlags {
        folder_id: i64,
        uids: Vec<u32>,
        change: FlagChange,
    },
    Delete {
        folder_id: i64,
        uids: Vec<u32>,
    },
    Move {
        folder_id: i64,
        target_folder_id: i64,
        uids: Vec<u32>,
    },
}

#[derive(Debug)]
pub enum ResolvedAction {
    SetFlags {
        account_id: i64,
        folder: String,
        uids: Vec<u32>,
        change: FlagChange,
    },
    Delete {
        account_id: i64,
        folder: String,
        uids: Vec<u32>,
    },
    Move {
        account_id: i64,
        from: String,
        to: String,
        uids: Vec<u32>,
    },
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
    MessageAction(MessageAction),
    MessageActionResolved(ResolvedAction),
    Failed {
        detail: String,
    },
}

#[derive(Debug)]
pub enum SyncServiceOutput {
    AccountsChanged,
    FolderChanged {
        folder_id: i64,
    },
    BodyFetched {
        message_id: i64,
    },
    NewMail {
        folder_title: String,
        notices: Vec<MailNotice>,
    },
    Error {
        detail: String,
    },
}

pub struct SyncService {
    store: Arc<Store>,
    body_dir: PathBuf,
    workers: HashMap<i64, AccountWorker>,
    watching: bool,
    synced_folders: HashSet<i64>,
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
                // The first sync of a folder only populates it; new mail is
                // worth notifying about afterwards.
                let populated = !self.synced_folders.insert(report.folder_id);
                if populated && !report.new_uids.is_empty() {
                    self.check_new_mail(report.folder_id, report.new_uids, sender);
                }
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

    fn check_new_mail(&self, folder_id: i64, uids: Vec<u32>, sender: &ComponentSender<Self>) {
        let store = self.store.clone();
        let notice_sender = sender.clone();
        sender.oneshot_command(async move {
            if let Some((folder_title, notices)) = new_mail_notices(&store, folder_id, &uids).await
            {
                let _ = notice_sender.output(SyncServiceOutput::NewMail {
                    folder_title,
                    notices,
                });
            }
        });
    }

    fn dispatch(&self, action: ResolvedAction) {
        let (account_id, command) = match action {
            ResolvedAction::SetFlags {
                account_id,
                folder,
                uids,
                change,
            } => (
                account_id,
                WorkerCommand::SetFlags {
                    folder,
                    uids,
                    change,
                },
            ),
            ResolvedAction::Delete {
                account_id,
                folder,
                uids,
            } => (account_id, WorkerCommand::Delete { folder, uids }),
            ResolvedAction::Move {
                account_id,
                from,
                to,
                uids,
            } => (account_id, WorkerCommand::Move { from, to, uids }),
        };
        if let Some(worker) = self.workers.get(&account_id) {
            worker.send(command);
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
            synced_folders: HashSet::new(),
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
            SyncServiceMsg::MessageAction(action) => {
                let store = self.store.clone();
                let action_sender = sender.clone();
                sender.oneshot_command(async move {
                    match resolve_action(&store, action).await {
                        Ok(action) => {
                            action_sender.input(SyncServiceMsg::MessageActionResolved(action))
                        }
                        Err(detail) => debug!(detail, "cannot resolve message action"),
                    }
                });
            }
            SyncServiceMsg::MessageActionResolved(action) => {
                self.dispatch(action);
            }
            SyncServiceMsg::Failed { detail } => {
                warn!(detail, "account discovery failed");
                let _ = sender.output(SyncServiceOutput::Error { detail });
            }
        }
    }
}

async fn new_mail_notices(
    store: &Store,
    folder_id: i64,
    uids: &[u32],
) -> Option<(String, Vec<MailNotice>)> {
    let folder = store.folder(folder_id).await.ok()??;
    if folder.special_use != Some(SpecialUse::Inbox) {
        return None;
    }
    let folder_title = folder
        .display_name
        .filter(|name| !name.is_empty())
        .unwrap_or(folder.name);
    let mut notices = Vec::new();
    for uid in uids {
        let Ok(Some(record)) = store.message(folder_id, *uid).await else {
            continue;
        };
        if bits_to_flags(record.flags).seen {
            continue;
        }
        notices.push(MailNotice {
            folder_id,
            uid: *uid,
            sender: display_sender(&record),
            subject: display_subject(&record.subject),
        });
    }
    (!notices.is_empty()).then_some((folder_title, notices))
}

async fn resolve_folder(store: &Store, folder_id: i64) -> Result<(i64, String), String> {
    match store.folder(folder_id).await {
        Ok(Some(folder)) => Ok((folder.account_id, folder.name)),
        Ok(None) => Err(format!("unknown folder {folder_id}")),
        Err(err) => Err(err.to_string()),
    }
}

async fn resolve_action(store: &Store, action: MessageAction) -> Result<ResolvedAction, String> {
    match action {
        MessageAction::SetFlags {
            folder_id,
            uids,
            change,
        } => {
            let (account_id, folder) = resolve_folder(store, folder_id).await?;
            Ok(ResolvedAction::SetFlags {
                account_id,
                folder,
                uids,
                change,
            })
        }
        MessageAction::Delete { folder_id, uids } => {
            let (account_id, folder) = resolve_folder(store, folder_id).await?;
            Ok(ResolvedAction::Delete {
                account_id,
                folder,
                uids,
            })
        }
        MessageAction::Move {
            folder_id,
            target_folder_id,
            uids,
        } => {
            let (account_id, from) = resolve_folder(store, folder_id).await?;
            let (_, to) = resolve_folder(store, target_folder_id).await?;
            Ok(ResolvedAction::Move {
                account_id,
                from,
                to,
                uids,
            })
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
    let accounts = manager.accounts().await;
    let mut seen = Vec::with_capacity(accounts.len());
    for account in &accounts {
        let record = account.to_record();
        seen.push((
            record.source.as_str().to_string(),
            record.external_id.clone(),
        ));
        let account_id = store
            .upsert_account(record)
            .await
            .map_err(|err| err.to_string())?;
        let Some(credential) = credential_worker
            .credentials(&connection, account)
            .await
            .map_err(|err| format!("{err:?}"))?
        else {
            warn!(account = account.id(), "no credential available");
            continue;
        };
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
    if !seen.is_empty() {
        store
            .prune_accounts(&seen)
            .await
            .map_err(|err| err.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::store::{
        AccountRecord, AccountSource, AuthKind, BodyState, FLAG_SEEN, FolderRecord, MessageRecord,
    };

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
            smtp_host: None,
            smtp_port: None,
            smtp_security: None,
            auth_kind: AuthKind::Password,
            username: None,
        }
    }

    fn folder(account_id: i64, name: &str) -> FolderRecord {
        FolderRecord {
            id: None,
            account_id,
            name: name.to_string(),
            display_name: None,
            special_use: None,
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            unread_count: 0,
            total_count: 0,
            subscribed: true,
        }
    }

    async fn seeded() -> (Store, i64, i64, i64) {
        let store = Store::open(":memory:").unwrap();
        let account_id = store.upsert_account(account()).await.unwrap();
        let inbox = store
            .upsert_folder(folder(account_id, "INBOX"))
            .await
            .unwrap();
        let archive = store
            .upsert_folder(folder(account_id, "Archive"))
            .await
            .unwrap();
        (store, account_id, inbox, archive)
    }

    #[tokio::test]
    async fn resolves_flags_move_and_delete_actions() {
        let (store, account_id, inbox, archive) = seeded().await;

        let action = MessageAction::SetFlags {
            folder_id: inbox,
            uids: vec![3],
            change: FlagChange {
                flagged: Some(true),
                ..FlagChange::default()
            },
        };
        match resolve_action(&store, action).await.unwrap() {
            ResolvedAction::SetFlags {
                account_id: resolved,
                folder,
                uids,
                change,
            } => {
                assert_eq!(resolved, account_id);
                assert_eq!(folder, "INBOX");
                assert_eq!(uids, vec![3]);
                assert_eq!(change.flagged, Some(true));
            }
            other => panic!("unexpected action {other:?}"),
        }

        let action = MessageAction::Move {
            folder_id: inbox,
            target_folder_id: archive,
            uids: vec![4],
        };
        match resolve_action(&store, action).await.unwrap() {
            ResolvedAction::Move {
                account_id: resolved,
                from,
                to,
                uids,
            } => {
                assert_eq!(resolved, account_id);
                assert_eq!(from, "INBOX");
                assert_eq!(to, "Archive");
                assert_eq!(uids, vec![4]);
            }
            other => panic!("unexpected action {other:?}"),
        }

        let action = MessageAction::Delete {
            folder_id: inbox,
            uids: vec![5],
        };
        match resolve_action(&store, action).await.unwrap() {
            ResolvedAction::Delete {
                account_id: resolved,
                folder,
                uids,
            } => {
                assert_eq!(resolved, account_id);
                assert_eq!(folder, "INBOX");
                assert_eq!(uids, vec![5]);
            }
            other => panic!("unexpected action {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_unknown_folders() {
        let (store, _, _, _) = seeded().await;
        let action = MessageAction::Delete {
            folder_id: 999,
            uids: vec![1],
        };
        assert!(resolve_action(&store, action).await.is_err());
    }

    fn message(folder_id: i64, uid: u32) -> MessageRecord {
        MessageRecord {
            id: None,
            folder_id,
            uid,
            modseq: None,
            message_id: Some(format!("<{uid}@example.org>")),
            thread_id: None,
            subject: format!("Subject {uid}"),
            from_addr: Some("sender@example.org".to_string()),
            from_name: Some(format!("Sender {uid}")),
            to_addrs: None,
            cc_addrs: None,
            date_sent: Some(1_700_000_000),
            date_recv: Some(1_700_000_001),
            in_reply_to: None,
            refs: None,
            flags: 0,
            has_attach: false,
            size: None,
            structure: None,
            raw_path: None,
            body_state: BodyState::None,
        }
    }

    #[tokio::test]
    async fn notices_cover_only_unread_inbox_mail() {
        let store = Store::open(":memory:").unwrap();
        let account_id = store.upsert_account(account()).await.unwrap();
        let mut inbox = folder(account_id, "INBOX");
        inbox.display_name = Some("Inbox".to_string());
        inbox.special_use = Some(SpecialUse::Inbox);
        let inbox = store.upsert_folder(inbox).await.unwrap();
        let archive = store
            .upsert_folder(folder(account_id, "Archive"))
            .await
            .unwrap();

        let mut unread = message(inbox, 4);
        unread.subject = "Hello there".to_string();
        let mut seen = message(inbox, 5);
        seen.flags = FLAG_SEEN;
        store.upsert_messages(vec![unread, seen]).await.unwrap();

        let (title, notices) = new_mail_notices(&store, inbox, &[4, 5]).await.unwrap();
        assert_eq!(title, "Inbox");
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].uid, 4);
        assert_eq!(notices[0].sender, "Sender 4");
        assert_eq!(notices[0].subject, "Hello there");

        assert!(new_mail_notices(&store, archive, &[4]).await.is_none());
        assert!(new_mail_notices(&store, 404, &[4]).await.is_none());
        assert!(new_mail_notices(&store, inbox, &[]).await.is_none());
    }
}
