// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Proxy for the GOA `org.gnome.OnlineAccounts.OAuth2Based` interface.

#[zbus::proxy(
    interface = "org.gnome.OnlineAccounts.OAuth2Based",
    default_service = "org.gnome.OnlineAccounts"
)]
pub trait OAuth2Based {
    #[zbus(property)]
    fn client_id(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn client_secret(&self) -> zbus::Result<String>;

    fn get_access_token(&self) -> zbus::Result<(String, i32)>;
}
