// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unified account model.
//!
//! A [`Account`] joins the transport configuration of an identity with
//! the discovery backends that back it. The same mailbox is typically
//! present both in GOA and in EDS, so the two are merged on the e-mail
//! address and both references are kept: the GOA object path drives
//! token and password retrieval through the daemon, the EDS source UID
//! drives Secret Service password lookups.

use mail_core::account::AccountConfig;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoaReference {
    pub account_path: String,
    pub provider_type: String,
    pub oauth2: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdsReference {
    pub uid: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub config: AccountConfig,
    pub goa: Option<GoaReference>,
    pub eds: Option<EdsReference>,
}

impl Account {
    pub fn id(&self) -> &str {
        &self.config.id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountChange {
    Added(Account),
    Removed(String),
    Modified(Account),
}
