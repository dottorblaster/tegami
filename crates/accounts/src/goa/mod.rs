// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Typesafe proxies for the GNOME Online Accounts (GOA) D-Bus API.
//!
//! GOA exposes its account store on the session bus under the
//! `org.gnome.OnlineAccounts` name. The manager object at
//! `/org/gnome/OnlineAccounts/Manager` implements the standard
//! `org.freedesktop.DBus.ObjectManager` interface; account objects live
//! at `/org/gnome/OnlineAccounts/Accounts/{id}` and provider interfaces
//! (Mail, OAuth2Based, ...) are exposed on child objects below each
//! account, e.g. `/org/gnome/OnlineAccounts/Accounts/{id}/mail`.

pub mod account;
pub mod mail;
pub mod oauth2;
pub mod object_manager;

pub use account::AccountProxy;
pub use mail::MailProxy;
pub use oauth2::OAuth2BasedProxy;
pub use object_manager::ObjectManagerProxy;
