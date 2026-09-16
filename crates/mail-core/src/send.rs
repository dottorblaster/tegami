// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Outgoing mail transport.
//!
//! A [`MailSender`] submits an already built message: the envelope carries the
//! return path and the recipients, the raw bytes carry the MIME message. The
//! split keeps the transport free of any MIME knowledge and lets workers
//! resubmit a message straight from its cached raw form.

use std::future::Future;

use mail_parser::MessageParser;

use crate::backend::MailError;
use crate::compose::{Address, parsed_addresses};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MailEnvelope {
    pub from: Option<Address>,
    pub recipients: Vec<Address>,
}

/// A transport able to submit a prebuilt message.
pub trait MailSender {
    fn send(
        &mut self,
        envelope: &MailEnvelope,
        raw: &[u8],
    ) -> impl Future<Output = Result<(), MailError>> + Send;
}

/// Derives the SMTP envelope from a raw message.
///
/// Returns [`None`] when the message cannot be parsed at all; a message
/// without recipients yields an envelope with an empty recipient list.
pub fn envelope(raw: &[u8]) -> Option<MailEnvelope> {
    let message = MessageParser::default().parse(raw)?;
    let from = message
        .from()
        .and_then(|address| parsed_addresses(address).into_iter().next());
    let mut recipients = Vec::new();
    for header in [message.to(), message.cc(), message.bcc()]
        .into_iter()
        .flatten()
    {
        recipients.extend(parsed_addresses(header));
    }
    Some(MailEnvelope { from, recipients })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::{OutgoingMessage, build, parse_addresses};

    fn message() -> OutgoingMessage {
        OutgoingMessage {
            from: Some(Address::new(
                Some("Ada Lovelace".to_string()),
                "ada@lovelace.dev",
            )),
            to: parse_addresses("grace@navy.dev"),
            cc: parse_addresses("cc@example.org"),
            bcc: parse_addresses("bcc@example.org"),
            subject: "Greetings".to_string(),
            text: Some("Hello there".to_string()),
            ..OutgoingMessage::default()
        }
    }

    #[test]
    fn extracts_the_envelope_from_a_built_message() {
        let raw = build(&message()).unwrap();
        let envelope = envelope(&raw).unwrap();
        assert_eq!(envelope.from.unwrap().email, "ada@lovelace.dev");
        assert_eq!(
            envelope
                .recipients
                .iter()
                .map(|address| address.email.as_str())
                .collect::<Vec<_>>(),
            vec!["grace@navy.dev", "cc@example.org", "bcc@example.org"]
        );
    }

    #[test]
    fn messages_without_recipients_still_parse() {
        let mut message = message();
        message.to.clear();
        message.cc.clear();
        message.bcc.clear();
        let raw = build(&message).unwrap();
        let envelope = envelope(&raw).unwrap();
        assert!(envelope.recipients.is_empty());
        assert_eq!(envelope.from.unwrap().email, "ada@lovelace.dev");
    }

    #[test]
    fn garbage_yields_an_empty_envelope() {
        let envelope = envelope(&[0xff, 0xfe, 0x00]).unwrap();
        assert!(envelope.from.is_none());
        assert!(envelope.recipients.is_empty());
    }
}
