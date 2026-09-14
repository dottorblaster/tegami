// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Typed rows mirroring the store schema.

use crate::folder::{Folder, FolderRole};
use rusqlite::types::Type;
use rusqlite::{Error, Row};

fn bad_enum(column: usize, value: &str) -> Error {
    Error::FromSqlConversionFailure(column, Type::Text, value.to_string().into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountSource {
    Goa,
    Eds,
}

impl AccountSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Goa => "goa",
            Self::Eds => "eds",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    OAuth2,
    Password,
}

impl AuthKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OAuth2 => "oauth2",
            Self::Password => "password",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    Ssl,
    StartTls,
    None,
}

impl Security {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ssl => "ssl",
            Self::StartTls => "starttls",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialUse {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
    Important,
    All,
    Flagged,
}

impl SpecialUse {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Sent => "sent",
            Self::Drafts => "drafts",
            Self::Trash => "trash",
            Self::Junk => "junk",
            Self::Archive => "archive",
            Self::Important => "important",
            Self::All => "all",
            Self::Flagged => "flagged",
        }
    }

    pub fn from_role(role: FolderRole) -> Option<Self> {
        match role {
            FolderRole::Inbox => Some(Self::Inbox),
            FolderRole::Sent => Some(Self::Sent),
            FolderRole::Drafts => Some(Self::Drafts),
            FolderRole::Trash => Some(Self::Trash),
            FolderRole::Junk => Some(Self::Junk),
            FolderRole::Archive => Some(Self::Archive),
            FolderRole::Important => Some(Self::Important),
            FolderRole::All => Some(Self::All),
            FolderRole::Flagged => Some(Self::Flagged),
            FolderRole::Other => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyState {
    None,
    Headers,
    Full,
}

impl BodyState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Headers => "headers",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AccountRecord {
    /// Populated by the database on insert; ignored on upsert.
    pub id: Option<i64>,
    pub source: AccountSource,
    pub external_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub imap_host: Option<String>,
    pub imap_port: Option<i64>,
    pub imap_security: Option<Security>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub smtp_security: Option<Security>,
    pub auth_kind: AuthKind,
    pub username: Option<String>,
}

impl AccountRecord {
    pub fn from_row(row: &Row<'_>) -> Result<Self, Error> {
        let source: String = row.get(1)?;
        let auth_kind: String = row.get(11)?;
        Ok(Self {
            id: Some(row.get(0)?),
            source: match source.as_str() {
                "goa" => AccountSource::Goa,
                "eds" => AccountSource::Eds,
                other => {
                    return Err(bad_enum(1, other));
                }
            },
            external_id: row.get(2)?,
            email: row.get(3)?,
            display_name: row.get(4)?,
            imap_host: row.get(5)?,
            imap_port: row.get(6)?,
            imap_security: row
                .get::<_, Option<String>>(7)?
                .map(|value| parse_security(&value))
                .transpose()?,
            smtp_host: row.get(8)?,
            smtp_port: row.get(9)?,
            smtp_security: row
                .get::<_, Option<String>>(10)?
                .map(|value| parse_security(&value))
                .transpose()?,
            auth_kind: match auth_kind.as_str() {
                "oauth2" => AuthKind::OAuth2,
                "password" => AuthKind::Password,
                other => return Err(bad_enum(11, other)),
            },
            username: row.get(12)?,
        })
    }
}

pub const ACCOUNT_COLUMNS: &str = "id, source, external_id, email, display_name, imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, auth_kind, username";
pub const FOLDER_COLUMNS: &str = "id, account_id, name, display_name, special_use, uidvalidity, uidnext, highestmodseq, unread_count, total_count, subscribed";
pub const MESSAGE_COLUMNS: &str = "id, folder_id, uid, modseq, message_id, thread_id, subject, from_addr, from_name, to_addrs, cc_addrs, date_sent, date_recv, in_reply_to, refs, flags, has_attach, size, structure, raw_path, body_state";
pub const ATTACHMENT_COLUMNS: &str =
    "id, message_id, part_id, filename, mime_type, size, content_id, disk_path";
pub const PENDING_OP_COLUMNS: &str =
    "id, account_id, op_kind, folder_id, target_folder_id, uid, payload, created_at";

#[derive(Debug, Clone, PartialEq)]
pub struct FolderRecord {
    pub id: Option<i64>,
    pub account_id: i64,
    pub name: String,
    pub display_name: Option<String>,
    pub special_use: Option<SpecialUse>,
    pub uidvalidity: Option<i64>,
    pub uidnext: Option<i64>,
    pub highestmodseq: Option<i64>,
    pub unread_count: i64,
    pub total_count: i64,
    pub subscribed: bool,
}

impl FolderRecord {
    pub fn from_folder(account_id: i64, folder: &Folder) -> Self {
        Self {
            id: None,
            account_id,
            name: folder.id.clone(),
            display_name: Some(folder.name.clone()).filter(|name| !name.is_empty()),
            special_use: SpecialUse::from_role(folder.role),
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            unread_count: 0,
            total_count: 0,
            subscribed: true,
        }
    }

    pub fn from_row(row: &Row<'_>) -> Result<Self, Error> {
        Ok(Self {
            id: Some(row.get(0)?),
            account_id: row.get(1)?,
            name: row.get(2)?,
            display_name: row.get(3)?,
            special_use: row
                .get::<_, Option<String>>(4)?
                .map(|value| parse_special_use(&value))
                .transpose()?,
            uidvalidity: row.get(5)?,
            uidnext: row.get(6)?,
            highestmodseq: row.get(7)?,
            unread_count: row.get(8)?,
            total_count: row.get(9)?,
            subscribed: row.get(10)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageRecord {
    pub id: Option<i64>,
    pub folder_id: i64,
    pub uid: u32,
    pub modseq: Option<i64>,
    pub message_id: Option<String>,
    pub thread_id: Option<i64>,
    pub subject: String,
    pub from_addr: Option<String>,
    pub from_name: Option<String>,
    pub to_addrs: Option<String>,
    pub cc_addrs: Option<String>,
    pub date_sent: Option<i64>,
    pub date_recv: Option<i64>,
    pub in_reply_to: Option<String>,
    pub refs: Option<String>,
    pub flags: i64,
    pub has_attach: bool,
    pub size: Option<i64>,
    pub structure: Option<String>,
    pub raw_path: Option<String>,
    pub body_state: BodyState,
}

impl MessageRecord {
    pub fn from_row(row: &Row<'_>) -> Result<Self, Error> {
        let body_state: String = row.get(20)?;
        Ok(Self {
            id: Some(row.get(0)?),
            folder_id: row.get(1)?,
            uid: row.get(2)?,
            modseq: row.get(3)?,
            message_id: row.get(4)?,
            thread_id: row.get(5)?,
            subject: row.get(6)?,
            from_addr: row.get(7)?,
            from_name: row.get(8)?,
            to_addrs: row.get(9)?,
            cc_addrs: row.get(10)?,
            date_sent: row.get(11)?,
            date_recv: row.get(12)?,
            in_reply_to: row.get(13)?,
            refs: row.get(14)?,
            flags: row.get(15)?,
            has_attach: row.get(16)?,
            size: row.get(17)?,
            structure: row.get(18)?,
            raw_path: row.get(19)?,
            body_state: match body_state.as_str() {
                "none" => BodyState::None,
                "headers" => BodyState::Headers,
                "full" => BodyState::Full,
                other => return Err(bad_enum(20, other)),
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRecord {
    pub id: Option<i64>,
    pub message_id: i64,
    pub part_id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
    pub content_id: Option<String>,
    pub disk_path: Option<String>,
}

impl AttachmentRecord {
    pub fn from_row(row: &Row<'_>) -> Result<Self, Error> {
        Ok(Self {
            id: Some(row.get(0)?),
            message_id: row.get(1)?,
            part_id: row.get(2)?,
            filename: row.get(3)?,
            mime_type: row.get(4)?,
            size: row.get(5)?,
            content_id: row.get(6)?,
            disk_path: row.get(7)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    SetFlags,
    Move,
    Delete,
}

impl OpKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SetFlags => "set_flags",
            Self::Move => "move",
            Self::Delete => "delete",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOpRecord {
    pub id: Option<i64>,
    pub account_id: i64,
    pub kind: OpKind,
    pub folder_id: Option<i64>,
    pub target_folder_id: Option<i64>,
    pub uid: Option<u32>,
    pub payload: Option<String>,
    pub created_at: Option<i64>,
}

impl PendingOpRecord {
    pub fn from_row(row: &Row<'_>) -> Result<Self, Error> {
        let kind: String = row.get(2)?;
        Ok(Self {
            id: Some(row.get(0)?),
            account_id: row.get(1)?,
            kind: match kind.as_str() {
                "set_flags" => OpKind::SetFlags,
                "move" => OpKind::Move,
                "delete" => OpKind::Delete,
                other => return Err(bad_enum(2, other)),
            },
            folder_id: row.get(3)?,
            target_folder_id: row.get(4)?,
            uid: row.get(5)?,
            payload: row.get(6)?,
            created_at: row.get(7)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub message: MessageRecord,
    pub snippet: String,
}

fn parse_security(value: &str) -> Result<Security, Error> {
    match value {
        "ssl" => Ok(Security::Ssl),
        "starttls" => Ok(Security::StartTls),
        "none" => Ok(Security::None),
        other => Err(bad_enum(4, other)),
    }
}

fn parse_special_use(value: &str) -> Result<SpecialUse, Error> {
    match value {
        "inbox" => Ok(SpecialUse::Inbox),
        "sent" => Ok(SpecialUse::Sent),
        "drafts" => Ok(SpecialUse::Drafts),
        "trash" => Ok(SpecialUse::Trash),
        "junk" => Ok(SpecialUse::Junk),
        "archive" => Ok(SpecialUse::Archive),
        "important" => Ok(SpecialUse::Important),
        "all" => Ok(SpecialUse::All),
        "flagged" => Ok(SpecialUse::Flagged),
        other => Err(bad_enum(4, other)),
    }
}
