// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::{message, smtp_config};
use mail_core::Credential;
use mail_core::compose::build;
use mail_core::send::{MailSender, envelope};
use smtp::SmtpSender;

#[tokio::test]
async fn reports_unreachable_servers() {
    let raw = build(&message()).unwrap();
    let envelope = envelope(&raw).unwrap();
    let config = smtp_config("127.0.0.1", 1, false);
    let mut sender =
        SmtpSender::new(&config, &Credential::Password("secret".to_string())).expect("transport");
    assert!(sender.send(&envelope, &raw).await.is_err());
}
