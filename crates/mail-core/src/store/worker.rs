// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The store worker.
//!
//! A dedicated thread owns the SQLite connection and answers typed
//! commands sent over a channel, so callers never block the async
//! runtime on database I/O. The [`Store`] handle is the async facade.

use std::path::Path;

use crate::folder::{Folder, FolderState};
use rusqlite::{Connection, OptionalExtension};
use tokio::sync::{mpsc, oneshot};

use super::error::{StoreError, StoreResult};
use super::models::{
    ACCOUNT_COLUMNS, ATTACHMENT_COLUMNS, AccountRecord, AttachmentRecord, BodyState,
    FOLDER_COLUMNS, FolderRecord, MESSAGE_COLUMNS, MessageRecord, PENDING_OP_COLUMNS,
    PendingOpRecord, SearchHit,
};
use super::schema;

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
    SyncFolders {
        account_id: i64,
        folders: Vec<Folder>,
        reply: oneshot::Sender<StoreResult<Vec<FolderRecord>>>,
    },
    SetFolderState {
        folder_id: i64,
        state: FolderState,
        reply: oneshot::Sender<StoreResult<()>>,
    },
    MessageUids {
        folder_id: i64,
        reply: oneshot::Sender<StoreResult<Vec<u32>>>,
    },
    DeleteMessages {
        folder_id: i64,
        uids: Vec<u32>,
        reply: oneshot::Sender<StoreResult<()>>,
    },
    ClearMessages {
        folder_id: i64,
        reply: oneshot::Sender<StoreResult<()>>,
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
    UpsertMessages {
        messages: Vec<MessageRecord>,
        reply: oneshot::Sender<StoreResult<()>>,
    },
    SetMessageBody {
        folder_id: i64,
        uid: u32,
        raw_path: String,
        body_state: BodyState,
        has_attach: bool,
        reply: oneshot::Sender<StoreResult<()>>,
    },
    ReplaceAttachments {
        message_id: i64,
        attachments: Vec<AttachmentRecord>,
        reply: oneshot::Sender<StoreResult<()>>,
    },
    Attachments {
        message_id: i64,
        reply: oneshot::Sender<StoreResult<Vec<AttachmentRecord>>>,
    },
    EnqueueOp {
        op: PendingOpRecord,
        reply: oneshot::Sender<StoreResult<i64>>,
    },
    PendingOps {
        account_id: i64,
        reply: oneshot::Sender<StoreResult<Vec<PendingOpRecord>>>,
    },
    DeleteOp {
        id: i64,
        reply: oneshot::Sender<StoreResult<()>>,
    },
    IndexBody {
        message_id: i64,
        text: String,
        reply: oneshot::Sender<StoreResult<()>>,
    },
    Search {
        query: String,
        limit: i64,
        reply: oneshot::Sender<StoreResult<Vec<SearchHit>>>,
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

    pub async fn sync_folders(
        &self,
        account_id: i64,
        folders: &[Folder],
    ) -> StoreResult<Vec<FolderRecord>> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::SyncFolders {
            account_id,
            folders: folders.to_vec(),
            reply,
        })
        .await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn set_folder_state(&self, folder_id: i64, state: FolderState) -> StoreResult<()> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::SetFolderState {
            folder_id,
            state,
            reply,
        })
        .await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn message_uids(&self, folder_id: i64) -> StoreResult<Vec<u32>> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::MessageUids { folder_id, reply }).await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn delete_messages(&self, folder_id: i64, uids: &[u32]) -> StoreResult<()> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::DeleteMessages {
            folder_id,
            uids: uids.to_vec(),
            reply,
        })
        .await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn clear_messages(&self, folder_id: i64) -> StoreResult<()> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::ClearMessages { folder_id, reply })
            .await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn set_message_body(
        &self,
        folder_id: i64,
        uid: u32,
        raw_path: String,
        body_state: BodyState,
        has_attach: bool,
    ) -> StoreResult<()> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::SetMessageBody {
            folder_id,
            uid,
            raw_path,
            body_state,
            has_attach,
            reply,
        })
        .await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn replace_attachments(
        &self,
        message_id: i64,
        attachments: Vec<AttachmentRecord>,
    ) -> StoreResult<()> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::ReplaceAttachments {
            message_id,
            attachments,
            reply,
        })
        .await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn attachments(&self, message_id: i64) -> StoreResult<Vec<AttachmentRecord>> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Attachments { message_id, reply })
            .await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn enqueue_op(&self, op: PendingOpRecord) -> StoreResult<i64> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::EnqueueOp { op, reply }).await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn pending_ops(&self, account_id: i64) -> StoreResult<Vec<PendingOpRecord>> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::PendingOps { account_id, reply }).await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn delete_op(&self, id: i64) -> StoreResult<()> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::DeleteOp { id, reply }).await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn index_body(&self, message_id: i64, text: String) -> StoreResult<()> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::IndexBody {
            message_id,
            text,
            reply,
        })
        .await?;
        receiver.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn search(&self, query: &str, limit: i64) -> StoreResult<Vec<SearchHit>> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Search {
            query: query.to_string(),
            limit,
            reply,
        })
        .await?;
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

    pub async fn upsert_messages(&self, messages: Vec<MessageRecord>) -> StoreResult<()> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::UpsertMessages { messages, reply })
            .await?;
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
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
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
            Command::SyncFolders {
                account_id,
                folders,
                reply,
            } => reply_send(reply, sync_folders(&mut connection, account_id, &folders)),
            Command::SetFolderState {
                folder_id,
                state,
                reply,
            } => reply_send(reply, set_folder_state(&connection, folder_id, &state)),
            Command::MessageUids { folder_id, reply } => {
                reply_send(reply, message_uids(&connection, folder_id))
            }
            Command::DeleteMessages {
                folder_id,
                uids,
                reply,
            } => reply_send(reply, delete_messages(&mut connection, folder_id, &uids)),
            Command::ClearMessages { folder_id, reply } => {
                reply_send(reply, clear_messages(&mut connection, folder_id))
            }
            Command::SetMessageBody {
                folder_id,
                uid,
                raw_path,
                body_state,
                has_attach,
                reply,
            } => reply_send(
                reply,
                set_message_body(
                    &connection,
                    folder_id,
                    uid,
                    &raw_path,
                    body_state,
                    has_attach,
                ),
            ),
            Command::ReplaceAttachments {
                message_id,
                attachments,
                reply,
            } => reply_send(
                reply,
                replace_attachments(&mut connection, message_id, &attachments),
            ),
            Command::Attachments { message_id, reply } => {
                reply_send(reply, attachments(&connection, message_id))
            }
            Command::EnqueueOp { op, reply } => reply_send(reply, enqueue_op(&connection, &op)),
            Command::PendingOps { account_id, reply } => {
                reply_send(reply, pending_ops(&connection, account_id))
            }
            Command::DeleteOp { id, reply } => reply_send(reply, delete_op(&connection, id)),
            Command::IndexBody {
                message_id,
                text,
                reply,
            } => reply_send(reply, index_body(&connection, message_id, &text)),
            Command::Search {
                query,
                limit,
                reply,
            } => reply_send(reply, search(&connection, &query, limit)),
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
            Command::UpsertMessages { messages, reply } => {
                reply_send(reply, upsert_messages(&mut connection, &messages))
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

fn sync_folders(
    connection: &mut Connection,
    account_id: i64,
    discovered: &[Folder],
) -> StoreResult<Vec<FolderRecord>> {
    let transaction = connection.transaction()?;
    for folder in discovered {
        upsert_discovered_folder(&transaction, &FolderRecord::from_folder(account_id, folder))?;
    }
    delete_missing_folders(&transaction, account_id, discovered)?;
    transaction.commit()?;
    folders(connection, account_id)
}

fn upsert_discovered_folder(connection: &Connection, folder: &FolderRecord) -> StoreResult<()> {
    connection.execute(
        "INSERT INTO folder (account_id, name, display_name, special_use)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(account_id, name) DO UPDATE SET
           display_name = excluded.display_name, special_use = excluded.special_use",
        rusqlite::params![
            folder.account_id,
            folder.name,
            folder.display_name,
            folder.special_use.map(|value| value.as_str()),
        ],
    )?;
    Ok(())
}

fn delete_missing_folders(
    connection: &Connection,
    account_id: i64,
    discovered: &[Folder],
) -> StoreResult<()> {
    if discovered.is_empty() {
        connection.execute("DELETE FROM folder WHERE account_id = ?1", [account_id])?;
        return Ok(());
    }
    let placeholders = discovered.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!("DELETE FROM folder WHERE account_id = ?1 AND name NOT IN ({placeholders})");
    let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(discovered.len() + 1);
    params.push(&account_id);
    for folder in discovered {
        params.push(&folder.id);
    }
    connection.execute(&sql, rusqlite::params_from_iter(params.iter()))?;
    Ok(())
}

fn set_folder_state(
    connection: &Connection,
    folder_id: i64,
    state: &FolderState,
) -> StoreResult<()> {
    connection.execute(
        "UPDATE folder SET uidvalidity = ?1, uidnext = ?2, highestmodseq = ?3, unread_count = ?4, total_count = ?5 WHERE id = ?6",
        rusqlite::params![
            i64::from(state.uid_validity),
            i64::from(state.uid_next),
            state
                .highest_modseq
                .and_then(|modseq| i64::try_from(modseq).ok()),
            i64::from(state.unseen.unwrap_or(0)),
            i64::from(state.exists),
            folder_id,
        ],
    )?;
    Ok(())
}

fn message_uids(connection: &Connection, folder_id: i64) -> StoreResult<Vec<u32>> {
    let mut statement =
        connection.prepare("SELECT uid FROM message WHERE folder_id = ?1 ORDER BY uid")?;
    let rows = statement
        .query_map([folder_id], |row| row.get::<_, u32>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn delete_messages(connection: &mut Connection, folder_id: i64, uids: &[u32]) -> StoreResult<()> {
    if uids.is_empty() {
        return Ok(());
    }
    let placeholders = uids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let fts_sql = format!(
        "DELETE FROM message_fts WHERE rowid IN (SELECT id FROM message WHERE folder_id = ?1 AND uid IN ({placeholders}))"
    );
    let message_sql =
        format!("DELETE FROM message WHERE folder_id = ?1 AND uid IN ({placeholders})");
    let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(uids.len() + 1);
    params.push(&folder_id);
    for uid in uids {
        params.push(uid);
    }
    let transaction = connection.transaction()?;
    transaction.execute(&fts_sql, rusqlite::params_from_iter(params.iter()))?;
    transaction.execute(&message_sql, rusqlite::params_from_iter(params.iter()))?;
    transaction.commit()?;
    Ok(())
}

fn clear_messages(connection: &mut Connection, folder_id: i64) -> StoreResult<()> {
    let transaction = connection.transaction()?;
    transaction.execute(
        "DELETE FROM message_fts WHERE rowid IN (SELECT id FROM message WHERE folder_id = ?1)",
        [folder_id],
    )?;
    transaction.execute("DELETE FROM message WHERE folder_id = ?1", [folder_id])?;
    transaction.commit()?;
    Ok(())
}

fn set_message_body(
    connection: &Connection,
    folder_id: i64,
    uid: u32,
    raw_path: &str,
    body_state: BodyState,
    has_attach: bool,
) -> StoreResult<()> {
    connection.execute(
        "UPDATE message SET raw_path = ?1, body_state = ?2, has_attach = ?3 WHERE folder_id = ?4 AND uid = ?5",
        rusqlite::params![raw_path, body_state.as_str(), has_attach, folder_id, uid],
    )?;
    Ok(())
}

fn replace_attachments(
    connection: &mut Connection,
    message_id: i64,
    attachments: &[AttachmentRecord],
) -> StoreResult<()> {
    let transaction = connection.transaction()?;
    transaction.execute("DELETE FROM attachment WHERE message_id = ?1", [message_id])?;
    for attachment in attachments {
        upsert_attachment(&transaction, attachment)?;
    }
    transaction.commit()?;
    Ok(())
}

fn upsert_attachment(connection: &Connection, attachment: &AttachmentRecord) -> StoreResult<i64> {
    connection.execute(
        "INSERT INTO attachment (message_id, part_id, filename, mime_type, size, content_id, disk_path)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            attachment.message_id,
            attachment.part_id,
            attachment.filename,
            attachment.mime_type,
            attachment.size,
            attachment.content_id,
            attachment.disk_path,
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

fn attachments(connection: &Connection, message_id: i64) -> StoreResult<Vec<AttachmentRecord>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {ATTACHMENT_COLUMNS} FROM attachment WHERE message_id = ?1 ORDER BY id"
    ))?;
    let rows = statement
        .query_map([message_id], AttachmentRecord::from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn enqueue_op(connection: &Connection, op: &PendingOpRecord) -> StoreResult<i64> {
    let created_at = op.created_at.unwrap_or_else(now);
    connection.execute(
        "INSERT INTO pending_op (account_id, op_kind, folder_id, target_folder_id, uid, payload, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            op.account_id,
            op.kind.as_str(),
            op.folder_id,
            op.target_folder_id,
            op.uid,
            op.payload,
            created_at,
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

fn pending_ops(connection: &Connection, account_id: i64) -> StoreResult<Vec<PendingOpRecord>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {PENDING_OP_COLUMNS} FROM pending_op WHERE account_id = ?1 ORDER BY id"
    ))?;
    let rows = statement
        .query_map([account_id], PendingOpRecord::from_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn delete_op(connection: &Connection, id: i64) -> StoreResult<()> {
    connection.execute("DELETE FROM pending_op WHERE id = ?1", [id])?;
    Ok(())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
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
    let message_id = connection.query_row(
        "INSERT INTO message (folder_id, uid, modseq, message_id, thread_id, subject, from_addr, from_name, to_addrs, cc_addrs, date_sent, date_recv, in_reply_to, refs, flags, has_attach, size, structure, raw_path, body_state)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
         ON CONFLICT(folder_id, uid) DO UPDATE SET
           modseq = excluded.modseq, message_id = excluded.message_id, thread_id = excluded.thread_id,
           subject = excluded.subject, from_addr = excluded.from_addr, from_name = excluded.from_name,
           to_addrs = excluded.to_addrs, cc_addrs = excluded.cc_addrs,
           date_sent = excluded.date_sent, date_recv = excluded.date_recv,
           in_reply_to = excluded.in_reply_to, refs = excluded.refs,
           flags = excluded.flags, has_attach = excluded.has_attach, size = excluded.size,
           structure = excluded.structure, raw_path = excluded.raw_path, body_state = excluded.body_state
         RETURNING id",
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
        |row| row.get(0),
    )?;
    index_message(connection, message, message_id)?;
    Ok(message_id)
}

fn index_message(
    connection: &Connection,
    message: &MessageRecord,
    message_id: i64,
) -> StoreResult<()> {
    let from_text = header_text([message.from_name.as_deref(), message.from_addr.as_deref()]);
    let to_text = header_text([message.to_addrs.as_deref(), message.cc_addrs.as_deref()]);
    let updated = connection.execute(
        "UPDATE message_fts SET subject = ?1, from_text = ?2, to_text = ?3 WHERE rowid = ?4",
        rusqlite::params![message.subject, from_text, to_text, message_id],
    )?;
    if updated == 0 {
        connection.execute(
            "INSERT INTO message_fts (rowid, subject, from_text, to_text, body_text) VALUES (?1, ?2, ?3, ?4, '')",
            rusqlite::params![message_id, message.subject, from_text, to_text],
        )?;
    }
    Ok(())
}

fn header_text<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> String {
    values.into_iter().flatten().collect::<Vec<_>>().join(" ")
}

fn index_body(connection: &Connection, message_id: i64, body_text: &str) -> StoreResult<()> {
    let updated = connection.execute(
        "UPDATE message_fts SET body_text = ?1 WHERE rowid = ?2",
        rusqlite::params![body_text, message_id],
    )?;
    if updated == 0 {
        return Err(StoreError::Protocol(format!(
            "no fts row for message {message_id}"
        )));
    }
    Ok(())
}

fn search(connection: &Connection, query: &str, limit: i64) -> StoreResult<Vec<SearchHit>> {
    let columns = MESSAGE_COLUMNS
        .split(", ")
        .map(|column| format!("message.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {columns}, snippet(message_fts, -1, '[', ']', '...', 12)
         FROM message_fts
         JOIN message ON message.id = message_fts.rowid
         WHERE message_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2"
    );
    let mut statement = connection.prepare(&sql)?;
    let hits = statement
        .query_map(rusqlite::params![query, limit], |row| {
            let message = MessageRecord::from_row(row)?;
            let snippet: String = row.get(21)?;
            Ok(SearchHit { message, snippet })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(hits)
}

fn upsert_messages(connection: &mut Connection, messages: &[MessageRecord]) -> StoreResult<()> {
    let transaction = connection.transaction()?;
    for message in messages {
        upsert_message(&transaction, message)?;
    }
    transaction.commit()?;
    Ok(())
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
