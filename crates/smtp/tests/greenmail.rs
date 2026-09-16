// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::{message, smtp_config};
use mail_core::Credential;
use mail_core::compose::build;
use mail_core::send::{MailSender, envelope};
use smtp::SmtpSender;

const GREENMAIL_SMTP_PORT: u16 = 3025;

fn greenmail_host() -> Option<String> {
    std::env::var("TEGAMI_GREENMAIL_HOST").ok()
}

#[tokio::test]
async fn sends_a_message_through_greenmail() {
    let Some(host) = greenmail_host() else {
        eprintln!("skipping: set TEGAMI_GREENMAIL_HOST to run GreenMail tests");
        return;
    };

    let raw = build(&message()).unwrap();
    let envelope = envelope(&raw).unwrap();
    let config = smtp_config(&host, GREENMAIL_SMTP_PORT, true);
    let mut sender =
        SmtpSender::new(&config, &Credential::Password("pass".to_string())).expect("transport");
    sender
        .send(&envelope, &raw)
        .await
        .expect("message accepted");
}
