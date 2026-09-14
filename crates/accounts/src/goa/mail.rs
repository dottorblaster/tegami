// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Proxy for the GOA `org.gnome.OnlineAccounts.Mail` interface.

#[zbus::proxy(
    interface = "org.gnome.OnlineAccounts.Mail",
    default_service = "org.gnome.OnlineAccounts"
)]
pub trait Mail {
    #[zbus(property)]
    fn email_address(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn imap_supported(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn imap_accept_ssl_errors(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn imap_host(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn imap_port(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn imap_use_ssl(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn imap_use_tls(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn imap_user_name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn smtp_supported(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn smtp_accept_ssl_errors(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn smtp_host(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn smtp_use_auth(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn smtp_auth_login(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn smtp_auth_plain(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn smtp_auth_xoauth2(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn smtp_use_ssl(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn smtp_use_tls(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn smtp_user_name(&self) -> zbus::Result<String>;
}
