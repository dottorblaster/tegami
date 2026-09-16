// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The mail backend abstraction.
//!
//! Backends translate the application's mailbox operations onto a
//! transport protocol, starting with IMAP through `async-imap` and
//! later JMAP. All folder-scoped calls take the folder path explicitly,
//! so a backend never depends on hidden selection state. Bodies are
//! fetched lazily as raw MIME bytes; parsing happens above the backend.

use std::future::Future;

use crate::account::AccountConfig;
use crate::envelope::{Envelope, FlagChange, MessageFlags};
use crate::folder::{Folder, FolderDelta, FolderState};

/// Untyped error surfaced by a [`MailBackend`].
#[derive(Debug)]
pub enum MailError {
    Io(std::io::Error),
    Disconnected,
    Protocol(String),
}

impl std::fmt::Display for MailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "backend I/O error: {err}"),
            Self::Disconnected => write!(f, "backend is not connected"),
            Self::Protocol(detail) => write!(f, "backend protocol error: {detail}"),
        }
    }
}

impl std::error::Error for MailError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for MailError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

pub type Result<T> = std::result::Result<T, MailError>;

/// The secret used to authenticate against a mail store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Credential {
    Password(String),
    OAuth2(String),
}

/// A connection to a mail store.
///
/// Every method returns a `Send` future so backends can be driven from
/// spawned tasks.
pub trait MailBackend {
    /// Establishes the session for the given account, negotiates the
    /// encryption layer and authenticates with the given credential.
    fn connect(
        &mut self,
        config: &AccountConfig,
        credential: &Credential,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Terminates the session gracefully.
    fn disconnect(&mut self) -> impl Future<Output = Result<()>> + Send;

    fn supports_idle(&self) -> bool;

    fn supports_condstore(&self) -> bool;

    fn supports_qresync(&self) -> bool;

    /// Lists the folders of the account with their SPECIAL-USE roles.
    fn folders(&mut self) -> impl Future<Output = Result<Vec<Folder>>> + Send;

    /// Selects a folder and reports its state.
    fn select(&mut self, folder: &str) -> impl Future<Output = Result<FolderState>> + Send;

    fn uids(&mut self, folder: &str) -> impl Future<Output = Result<Vec<u32>>> + Send;

    fn fetch_envelopes(
        &mut self,
        folder: &str,
        uids: &[u32],
    ) -> impl Future<Output = Result<Vec<Envelope>>> + Send;

    fn fetch_delta(
        &mut self,
        folder: &str,
        since_modseq: u64,
    ) -> impl Future<Output = Result<FolderDelta>> + Send;

    /// Fetches the raw MIME body of a message, identified by its UID.
    fn fetch_message(
        &mut self,
        folder: &str,
        uid: u32,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send;

    /// Applies a per-flag change to a set of messages.
    fn set_flags(
        &mut self,
        folder: &str,
        uids: &[u32],
        change: FlagChange,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Moves a set of messages to another folder.
    fn move_messages(
        &mut self,
        from: &str,
        to: &str,
        uids: &[u32],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Copies a set of messages to another folder.
    fn copy_messages(
        &mut self,
        from: &str,
        to: &str,
        uids: &[u32],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Appends a raw message to a folder, returning the assigned UID when
    /// the server supports it.
    fn append(
        &mut self,
        folder: &str,
        flags: MessageFlags,
        raw: &[u8],
    ) -> impl Future<Output = Result<u32>> + Send;

    /// Removes messages from a folder outright: marks them `\Deleted` and
    /// expunges them. Uses UIDPLUS (UID EXPUNGE) when available so only the
    /// requested messages are expunged.
    fn delete_permanently(
        &mut self,
        folder: &str,
        uids: &[u32],
    ) -> impl Future<Output = Result<()>> + Send;

    /// Waits for a change in the given folder, returning when the server
    /// signals one; the caller re-fetches to observe it.
    fn idle(&mut self, folder: &str) -> impl Future<Output = Result<()>> + Send;
}
