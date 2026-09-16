// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use mail_core::account::SmtpConfig;
use mail_core::compose::{Address, OutgoingMessage};

pub fn smtp_config(host: &str, port: u16, use_auth: bool) -> SmtpConfig {
    SmtpConfig {
        accept_ssl_errors: false,
        host: host.to_string(),
        port: Some(port),
        use_auth,
        auth_login: true,
        auth_plain: true,
        auth_xoauth2: false,
        use_ssl: false,
        use_tls: false,
        user_name: "user".to_string(),
    }
}

pub fn message() -> OutgoingMessage {
    OutgoingMessage {
        from: Some(Address::new(Some("Tegami".to_string()), "user@localhost")),
        to: vec![Address::new(None, "user@localhost")],
        subject: "Tegami SMTP test".to_string(),
        text: Some("Hello from the SMTP test.\n".to_string()),
        ..OutgoingMessage::default()
    }
}
