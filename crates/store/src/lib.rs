// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! SQLite account and mailbox store.
//!
//! The schema mirrors the accounts resolved from GOA/EDS and the
//! synchronized mailbox data: folders, messages, attachments, the
//! offline outbox, the pending-operation queue and the FTS index. All
//! database access goes through a dedicated worker thread exposed as
//! the async [`Store`] handle.

mod error;
mod models;
mod schema;
mod worker;

pub use error::{StoreError, StoreResult};
pub use models::{
    AccountRecord, AccountSource, AuthKind, BodyState, FolderRecord, MessageRecord, Security,
    SpecialUse,
};
pub use worker::Store;

pub const FLAG_SEEN: i64 = 1 << 0;
pub const FLAG_ANSWERED: i64 = 1 << 1;
pub const FLAG_FLAGGED: i64 = 1 << 2;
pub const FLAG_DRAFT: i64 = 1 << 3;
pub const FLAG_DELETED: i64 = 1 << 4;

pub fn flags_to_bits(flags: mail_core::envelope::MessageFlags) -> i64 {
    let mut bits = 0;
    if flags.seen {
        bits |= FLAG_SEEN;
    }
    if flags.answered {
        bits |= FLAG_ANSWERED;
    }
    if flags.flagged {
        bits |= FLAG_FLAGGED;
    }
    if flags.draft {
        bits |= FLAG_DRAFT;
    }
    if flags.deleted {
        bits |= FLAG_DELETED;
    }
    bits
}

pub fn bits_to_flags(bits: i64) -> mail_core::envelope::MessageFlags {
    mail_core::envelope::MessageFlags {
        seen: bits & FLAG_SEEN != 0,
        answered: bits & FLAG_ANSWERED != 0,
        flagged: bits & FLAG_FLAGGED != 0,
        draft: bits & FLAG_DRAFT != 0,
        deleted: bits & FLAG_DELETED != 0,
    }
}
