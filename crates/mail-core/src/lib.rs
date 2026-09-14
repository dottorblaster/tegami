// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Core mail model and backend abstractions for Tegami.
//!
//! This crate hosts the UI-free data layer: envelopes and flags,
//! conversation/thread grouping, and the `MailBackend` trait that
//! connects accounts to IMAP (and later JMAP). No GTK, no I/O
//! specifics here, so it stays easy to unit test in isolation.

pub mod account;
pub mod backend;
pub mod envelope;
pub mod folder;

pub use backend::{Credential, MailBackend, MailError};
pub use envelope::{Address, Envelope, FlagChange, MessageFlags};
pub use folder::{Folder, FolderRole, FolderState};
