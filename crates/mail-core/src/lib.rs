// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Core mail engine for Tegami.
//!
//! The UI-free data layer: the account, envelope and folder models, the
//! `MailBackend` trait that connects accounts to IMAP (and later JMAP),
//! the SQLite [`store`], MIME [`mime`] parsing and the [`sync`] engine.

pub mod account;
pub mod backend;
pub mod envelope;
pub mod folder;
pub mod mime;
pub mod store;
pub mod sync;
pub mod threading;
pub mod worker;

pub use backend::{Credential, MailBackend, MailError};
pub use envelope::{Address, Envelope, FlagChange, MessageFlags};
pub use folder::{Folder, FolderDelta, FolderRole, FolderState};
pub use worker::{AccountWorker, WorkerCommand, WorkerConfig, WorkerEvent};
