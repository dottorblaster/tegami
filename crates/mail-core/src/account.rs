// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Account configuration model shared by every discovery backend.
//!
//! [`AccountConfig`] is the canonical representation of a mail account,
//! independent of where it was discovered: GOA D-Bus objects, EDS
//! sources, or the on-disk fallback all map into this model.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountConfig {
    pub id: String,
    pub name: String,
    pub email_address: String,
    pub provider_type: Option<String>,
    pub is_temporary: bool,
    pub imap: Option<ImapConfig>,
    pub smtp: Option<SmtpConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapConfig {
    pub accept_ssl_errors: bool,
    pub host: String,
    pub use_ssl: bool,
    pub use_tls: bool,
    pub user_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpConfig {
    pub accept_ssl_errors: bool,
    pub host: String,
    pub use_auth: bool,
    pub auth_login: bool,
    pub auth_plain: bool,
    pub auth_xoauth2: bool,
    pub use_ssl: bool,
    pub use_tls: bool,
    pub user_name: String,
}
