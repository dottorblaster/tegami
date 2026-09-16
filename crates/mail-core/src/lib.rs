// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Core mail engine for Tegami.
//!
//! The UI-free data layer: the account, envelope and folder models, the
//! [`MailBackend`] and [`send::MailSender`] traits, their bundled [`imap`]
//! and [`smtp`] implementations, the SQLite [`store`], MIME [`mime`]
//! parsing, [`compose`] message building and the [`sync`] engine.

pub mod account;
pub mod backend;
pub mod compose;
pub mod envelope;
pub mod folder;
pub mod imap;
pub mod mime;
pub mod send;
pub mod smtp;
pub mod store;
pub mod sync;
pub mod threading;
pub mod worker;

pub use backend::{Credential, MailBackend, MailError};
pub use envelope::{Address, Envelope, FlagChange, MessageFlags};
pub use folder::{Folder, FolderDelta, FolderRole, FolderState};
pub use worker::{AccountWorker, WorkerCommand, WorkerConfig, WorkerEvent};
