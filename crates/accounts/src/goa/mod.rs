// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Typesafe proxies for the GNOME Online Accounts (GOA) D-Bus API.
//!
//! GOA exposes its account store on the session bus under the
//! `org.gnome.OnlineAccounts` name. The root object at
//! `/org/gnome/OnlineAccounts` implements the standard
//! `org.freedesktop.DBus.ObjectManager` interface; account objects live
//! at `/org/gnome/OnlineAccounts/Accounts/{id}` and carry the
//! `org.gnome.OnlineAccounts.Account` interface together with the provider
//! interfaces the account supports (Mail, OAuth2Based, ...).

pub mod account;
pub mod enumerate;
pub mod mail;
pub mod oauth2;
pub mod object_manager;

pub use account::AccountProxy;
pub use enumerate::enumerate_mail_accounts;
pub use mail::MailProxy;
pub use oauth2::OAuth2BasedProxy;
pub use object_manager::ObjectManagerProxy;
