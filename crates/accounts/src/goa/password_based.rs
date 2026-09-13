// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Proxy for the GOA `org.gnome.OnlineAccounts.PasswordBased` interface.

#[zbus::proxy(
    interface = "org.gnome.OnlineAccounts.PasswordBased",
    default_service = "org.gnome.OnlineAccounts"
)]
pub trait PasswordBased {
    fn get_password(&self, id: &str) -> zbus::Result<String>;
}
