// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Mail folder model.

/// The SPECIAL-USE role of a folder, when the server or the discovery
/// layer can map it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderRole {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
    Important,
    All,
    Flagged,
    Other,
}

/// A mail folder, identified by its IMAP mailbox path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub role: FolderRole,
}

/// The state reported when a folder is selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderState {
    pub uid_validity: u32,
    pub uid_next: u32,
    pub exists: u32,
    pub recent: u32,
    pub unseen: Option<u32>,
}
