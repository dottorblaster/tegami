// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The store worker.
//!
//! A dedicated thread owns the SQLite connection and answers typed
//! commands sent over a channel, so callers never block the async
//! runtime on database I/O. The [`Store`] handle is the async facade.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use tokio::sync::{mpsc, oneshot};

use crate::error::{StoreError, StoreResult};
use crate::models::{
    ACCOUNT_COLUMNS, AccountRecord, FOLDER_COLUMNS, FolderRecord, MESSAGE_COLUMNS, MessageRecord,
};
use crate::schema;

enum Command {
    Accounts(oneshot::Sender<StoreResult<Vec<AccountRecord>>>),
    Account {
        source: String,
        external_id: String,
        reply: oneshot::Sender<StoreResult<Option<AccountRecord>>>,
    },
    UpsertAccount {
        account: AccountRecord,
        reply: oneshot::Sender<StoreResult<i64>>,
    },
    Folders {
        account_id: i64,
        reply: oneshot::Sender<StoreResult<Vec<FolderRecord>>>,
    },
    UpsertFolder {
        folder: FolderRecord,
        reply: oneshot::Sender<StoreResult<i64>>,
    },
    Messages {
        folder_id: i64,
        reply: oneshot::Sender<StoreResult<Vec<MessageRecord>>>,
    },
    Message {
        folder_id: i64,
        uid: u32,
        reply: oneshot::Sender<StoreResult<Option<MessageRecord>>>,
    },
    UpsertMessage {
        message: MessageRecord,
        reply: oneshot::Sender<StoreResult<i64>>,
    },
    SetMessageFlags {
        folder_id: i64,
        uids: Vec<u32>,
        flags: i64,
        reply: oneshot::Sender<StoreResult<()>>,
    },
    Shutdown,
}

pub struct Store {
    sender: mpsc::Sender<Command>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        let path = path.as_ref().to_path_buf();
        let (sender, receiver) = mpsc::channel(64);
        std::thread::Builder::new()
            .name("store-worker".to_string())
            .spawn(move || {
                let result = worker(receiver, &path);
                if let Err(err) = result {
                    eprintln!("store worker failed: {err}");
                }
            })
            .map_err(|err| StoreError::Protocol(err.to_string()))?;
        Ok(Self { sender })
    }

    pub async fn accounts(&self) -> StoreResult<Vec<AccountRecord>> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Accounts(reply)).await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn account(
        &self,
        source: &str,
        external_id: &str,
    ) -> StoreResult<Option<AccountRecord>> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Account {
            source: source.to_string(),
            external_id: external_id.to_string(),
            reply,
        })
        .await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn upsert_account(&self, account: AccountRecord) -> StoreResult<i64> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::UpsertAccount { account, reply }).await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn folders(&self, account_id: i64) -> StoreResult<Vec<FolderRecord>> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Folders { account_id, reply }).await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn upsert_folder(&self, folder: FolderRecord) -> StoreResult<i64> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::UpsertFolder { folder, reply }).await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn messages(&self, folder_id: i64) -> StoreResult<Vec<MessageRecord>> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Messages { folder_id, reply }).await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn message(&self, folder_id: i64, uid: u32) -> StoreResult<Option<MessageRecord>> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Message {
            folder_id,
            uid,
            reply,
        })
        .await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn upsert_message(&self, message: MessageRecord) -> StoreResult<i64> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::UpsertMessage { message, reply }).await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn set_message_flags(
        &self,
        folder_id: i64,
        uids: &[u32],
        flags: i64,
    ) -> StoreResult<()> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::SetMessageFlags {
            folder_id,
            uids: uids.to_vec(),
            flags,
            reply,
        })
        .await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    async fn send(&self, command: Command) -> StoreResult<()> {
        self.sender
            .send(command)
            .await
            .map_err(|_| StoreError::Closed)
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let _ = self.sender.try_send(Command::Shutdown);
    }
}

fn worker(mut receiver: mpsc::Receiver<Command>, path: &Path) -> StoreResult<()> {
    let mut connection = Connection::open(path)?;
    schema::migrate(&mut connection)?;
    while let Some(command) = receiver.blocking_recv() {
        match command {
            Command::Accounts(reply) => reply_send(reply, accounts(&connection)),
            Command::Account {
                source,
                external_id,
                reply,
            } => reply_send(reply, account(&connection, &source, &external_id)),
            Command::UpsertAccount { account, reply } => {
                reply_send(reply, upsert_account(&connection, &account))
            }
            Command::Folders { account_id, reply } => {
                reply_send(reply, folders(&connection, account_id))
            }
            Command::UpsertFolder { folder, reply } => {
                reply_send(reply, upsert_folder(&connection, &folder))
            }
            Command::Messages { folder_id, reply } => {
                reply_send(reply, messages(&connection, folder_id))
            }
            Command::Message {
                folder_id,
                uid,
                reply,
            } => reply_send(reply, message(&connection, folder_id, uid)),
            Command::UpsertMessage { message, reply } => {
                reply_send(reply, upsert_message(&connection, &message))
            }
            Command::SetMessageFlags {
                folder_id,
                uids,
                flags,
                reply,
            } => reply_send(
                reply,
                set_message_flags(&connection, folder_id, &uids, flags),
            ),
            Command::Shutdown => break,
        }
    }
    Ok(())
}

fn reply_send<T>(reply: oneshot::Sender<T>, value: T) {
    let _ = reply.send(value);
}

fn accounts(connection: &Connection) -> StoreResult<Vec<AccountRecord>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {ACCOUNT_COLUMNS} FROM account ORDER BY id"
    ))?;
    let rows = statement
        .query_map([], AccountRecord::from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn account(
    connection: &Connection,
    source: &str,
    external_id: &str,
) -> StoreResult<Option<AccountRecord>> {
    connection
        .query_row(
            &format!(
                "SELECT {ACCOUNT_COLUMNS} FROM account WHERE source = ?1 AND external_id = ?2"
            ),
            (source, external_id),
            AccountRecord::from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn upsert_account(connection: &Connection, account: &AccountRecord) -> StoreResult<i64> {
    connection.execute(
        "INSERT INTO account (source, external_id, email, display_name, imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, auth_kind, username)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(source, external_id) DO UPDATE SET
           email = excluded.email, display_name = excluded.display_name,
           imap_host = excluded.imap_host, imap_port = excluded.imap_port, imap_security = excluded.imap_security,
           smtp_host = excluded.smtp_host, smtp_port = excluded.smtp_port, smtp_security = excluded.smtp_security,
           auth_kind = excluded.auth_kind, username = excluded.username",
        rusqlite::params![
            account.source.as_str(),
            account.external_id,
            account.email,
            account.display_name,
            account.imap_host,
            account.imap_port,
            account.imap_security.map(|value| value.as_str()),
            account.smtp_host,
            account.smtp_port,
            account.smtp_security.map(|value| value.as_str()),
            account.auth_kind.as_str(),
            account.username,
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

fn folders(connection: &Connection, account_id: i64) -> StoreResult<Vec<FolderRecord>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {FOLDER_COLUMNS} FROM folder WHERE account_id = ?1 ORDER BY name"
    ))?;
    let rows = statement
        .query_map([account_id], FolderRecord::from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn upsert_folder(connection: &Connection, folder: &FolderRecord) -> StoreResult<i64> {
    connection.execute(
        "INSERT INTO folder (account_id, name, display_name, special_use, uidvalidity, uidnext, highestmodseq, unread_count, total_count, subscribed)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(account_id, name) DO UPDATE SET
           display_name = excluded.display_name, special_use = excluded.special_use,
           uidvalidity = excluded.uidvalidity, uidnext = excluded.uidnext, highestmodseq = excluded.highestmodseq,
           unread_count = excluded.unread_count, total_count = excluded.total_count, subscribed = excluded.subscribed",
        rusqlite::params![
            folder.account_id,
            folder.name,
            folder.display_name,
            folder.special_use.map(|value| value.as_str()),
            folder.uidvalidity,
            folder.uidnext,
            folder.highestmodseq,
            folder.unread_count,
            folder.total_count,
            folder.subscribed,
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

fn messages(connection: &Connection, folder_id: i64) -> StoreResult<Vec<MessageRecord>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {MESSAGE_COLUMNS} FROM message WHERE folder_id = ?1 ORDER BY uid"
    ))?;
    let rows = statement
        .query_map([folder_id], MessageRecord::from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn message(
    connection: &Connection,
    folder_id: i64,
    uid: u32,
) -> StoreResult<Option<MessageRecord>> {
    connection
        .query_row(
            &format!("SELECT {MESSAGE_COLUMNS} FROM message WHERE folder_id = ?1 AND uid = ?2"),
            (folder_id, uid),
            MessageRecord::from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn upsert_message(connection: &Connection, message: &MessageRecord) -> StoreResult<i64> {
    connection.execute(
        "INSERT INTO message (folder_id, uid, modseq, message_id, thread_id, subject, from_addr, from_name, to_addrs, cc_addrs, date_sent, date_recv, in_reply_to, refs, flags, has_attach, size, structure, raw_path, body_state)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
         ON CONFLICT(folder_id, uid) DO UPDATE SET
           modseq = excluded.modseq, message_id = excluded.message_id, thread_id = excluded.thread_id,
           subject = excluded.subject, from_addr = excluded.from_addr, from_name = excluded.from_name,
           to_addrs = excluded.to_addrs, cc_addrs = excluded.cc_addrs,
           date_sent = excluded.date_sent, date_recv = excluded.date_recv,
           in_reply_to = excluded.in_reply_to, refs = excluded.refs,
           flags = excluded.flags, has_attach = excluded.has_attach, size = excluded.size,
           structure = excluded.structure, raw_path = excluded.raw_path, body_state = excluded.body_state",
        rusqlite::params![
            message.folder_id,
            message.uid,
            message.modseq,
            message.message_id,
            message.thread_id,
            message.subject,
            message.from_addr,
            message.from_name,
            message.to_addrs,
            message.cc_addrs,
            message.date_sent,
            message.date_recv,
            message.in_reply_to,
            message.refs,
            message.flags,
            message.has_attach,
            message.size,
            message.structure,
            message.raw_path,
            message.body_state.as_str(),
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

fn set_message_flags(
    connection: &Connection,
    folder_id: i64,
    uids: &[u32],
    flags: i64,
) -> StoreResult<()> {
    if uids.is_empty() {
        return Ok(());
    }
    let placeholders = uids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql =
        format!("UPDATE message SET flags = ?1 WHERE folder_id = ?2 AND uid IN ({placeholders})");
    let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(uids.len() + 2);
    params.push(&flags);
    params.push(&folder_id);
    for uid in uids {
        params.push(uid);
    }
    connection.execute(&sql, rusqlite::params_from_iter(params.iter()))?;
    Ok(())
}
