// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The IMAP backend: a [`crate::MailBackend`] implementation on top of
//! `async-imap`, with implicit TLS and STARTTLS, LOGIN and XOAUTH2
//! authentication and post-login capability detection.

pub mod auth;
mod backend;

pub use backend::ImapBackend;
