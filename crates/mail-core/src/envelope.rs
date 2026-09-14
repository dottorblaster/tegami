// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Mail envelope model.

use std::time::SystemTime;

/// Message flags as carried by the IMAP `\Seen`, `\Answered`, `\Flagged`,
/// `\Deleted` and `\Draft` system flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MessageFlags {
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub deleted: bool,
    pub draft: bool,
}

/// A per-flag instruction: `Some(true)` adds, `Some(false)` removes and
/// `None` leaves the flag untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FlagChange {
    pub seen: Option<bool>,
    pub answered: Option<bool>,
    pub flagged: Option<bool>,
    pub deleted: Option<bool>,
    pub draft: Option<bool>,
}

/// A mailbox address with an optional display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub name: Option<String>,
    pub address: Option<String>,
}

/// The metadata of one message in a folder, keyed by its numeric UID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub uid: u32,
    pub flags: MessageFlags,
    pub size: u32,
    pub subject: String,
    pub from: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub date: Option<SystemTime>,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}
