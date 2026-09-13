// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! EDS (Evolution Data Server) source-based account discovery.
//!
//! EDS exposes its account store through the Sources5 D-Bus service
//! (`org.gnome.evolution.dataserver.Sources5`). The SourceManager object
//! at `/org/gnome/evolution/dataserver/SourceManager` implements the
//! standard `org.freedesktop.DBus.ObjectManager` interface; every source
//! object below it carries the `org.gnome.evolution.dataserver.Source`
//! interface with a `UID` and a `Data` property holding the serialized
//! key-file contents of the source.

pub mod discover;
pub mod enumerate;
pub mod parse;
pub mod secret;
pub mod source;

pub use discover::{default_sources_dir, discover_mail_accounts, mail_accounts_from_dir};
pub use enumerate::enumerate_mail_accounts;
pub use parse::SourceData;
pub use secret::{SCHEMA_ATTRIBUTE, SOURCE_SCHEMA, SOURCE_UID_ATTRIBUTE, password_for_source};
pub use source::{ObjectManagerProxy, SourceProxy};
