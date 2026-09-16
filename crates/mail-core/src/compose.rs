// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Outgoing message building.
//!
//! Turns a composed message into the raw MIME bytes to hand to a transport.
//! Bodies are wrapped the way other mail clients do it: a
//! `multipart/alternative` for the text and HTML renderings, a
//! `multipart/related` when inline images carry a `Content-ID` and a
//! `multipart/mixed` once regular attachments join in.

use mail_builder::MessageBuilder;
use mail_builder::headers::message_id::MessageId;
use mail_builder::mime::MimePart;
use mail_parser::{Address as ParsedAddress, MessageParser};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub name: Option<String>,
    pub email: String,
}

impl Address {
    pub fn new(name: Option<String>, email: impl Into<String>) -> Self {
        Self {
            name: name.filter(|name| !name.trim().is_empty()),
            email: email.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingAttachment {
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineImage {
    pub content_id: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutgoingMessage {
    pub from: Option<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    pub text: Option<String>,
    pub html: Option<String>,
    pub attachments: Vec<OutgoingAttachment>,
    pub inline: Vec<InlineImage>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

#[derive(Debug)]
pub enum ComposeError {
    Io(std::io::Error),
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "failed to build the message: {err}"),
        }
    }
}

impl std::error::Error for ComposeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
        }
    }
}

impl From<std::io::Error> for ComposeError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Parses a header value such as `Ada Lovelace <ada@example.org>, grace@navy.dev`
/// into a list of addresses.
pub fn parse_addresses(value: &str) -> Vec<Address> {
    if value.trim().is_empty() {
        return Vec::new();
    }
    let raw = format!("To: {value}\r\n\r\n");
    let Some(message) = MessageParser::default().parse(raw.as_bytes()) else {
        return Vec::new();
    };
    message.to().map(parsed_addresses).unwrap_or_default()
}

/// Converts a parsed address header into the address model.
pub(crate) fn parsed_addresses(parsed: &ParsedAddress<'_>) -> Vec<Address> {
    let mut addresses = Vec::new();
    match parsed {
        ParsedAddress::List(items) => {
            items.iter().for_each(|item| push(item, &mut addresses));
        }
        ParsedAddress::Group(groups) => {
            for group in groups {
                group
                    .addresses
                    .iter()
                    .for_each(|item| push(item, &mut addresses));
            }
        }
    }
    addresses
}

fn push(item: &mail_parser::Addr<'_>, addresses: &mut Vec<Address>) {
    let Some(email) = item
        .address
        .as_deref()
        .map(str::trim)
        .filter(|email| !email.is_empty())
    else {
        return;
    };
    addresses.push(Address::new(
        item.name.as_deref().map(str::to_string),
        email,
    ));
}

pub fn build(message: &OutgoingMessage) -> Result<Vec<u8>, ComposeError> {
    let mut builder = MessageBuilder::new();

    if let Some(from) = &message.from {
        builder = builder.from(header_address(from));
    }
    if !message.to.is_empty() {
        builder = builder.to(addresses(&message.to));
    }
    if !message.cc.is_empty() {
        builder = builder.cc(addresses(&message.cc));
    }
    if !message.bcc.is_empty() {
        builder = builder.bcc(addresses(&message.bcc));
    }
    builder = builder.subject(message.subject.as_str());
    if let Some(in_reply_to) = &message.in_reply_to {
        builder = builder.in_reply_to(MessageId::new(trim_message_id(in_reply_to)));
    }
    if !message.references.is_empty() {
        builder = builder.references(MessageId::new_list(
            message.references.iter().map(|id| trim_message_id(id)),
        ));
    }

    Ok(builder.body(body(message)).write_to_vec()?)
}

fn trim_message_id(value: &str) -> &str {
    value.trim().trim_start_matches('<').trim_end_matches('>')
}

fn addresses(list: &[Address]) -> mail_builder::headers::address::Address<'_> {
    mail_builder::headers::address::Address::List(list.iter().map(header_address).collect())
}

fn header_address(address: &Address) -> mail_builder::headers::address::Address<'_> {
    match &address.name {
        Some(name) => mail_builder::headers::address::Address::new_address(
            Some(name.as_str()),
            address.email.as_str(),
        ),
        None => mail_builder::headers::address::Address::from(address.email.as_str()),
    }
}

fn body(message: &OutgoingMessage) -> MimePart<'_> {
    let inline: Vec<MimePart<'_>> = message
        .inline
        .iter()
        .map(|image| {
            MimePart::new(image.mime_type.as_str(), image.data.as_slice())
                .inline()
                .cid(image.content_id.as_str())
        })
        .collect();
    let attachments: Vec<MimePart<'_>> = message
        .attachments
        .iter()
        .map(|attachment| {
            MimePart::new(attachment.mime_type.as_str(), attachment.data.as_slice())
                .attachment(attachment.filename.as_str())
        })
        .collect();

    let alternative = match (message.text.as_deref(), message.html.as_deref()) {
        (Some(text), Some(html)) => Some(MimePart::new(
            "multipart/alternative",
            vec![
                MimePart::new("text/plain", text),
                MimePart::new("text/html", html),
            ],
        )),
        (Some(text), None) => Some(MimePart::new("text/plain", text)),
        (None, Some(html)) => Some(MimePart::new("text/html", html)),
        (None, None) => None,
    };

    let content = match (alternative, inline.is_empty()) {
        (Some(alternative), true) => Some(alternative),
        (Some(alternative), false) => {
            let mut parts = vec![alternative];
            parts.extend(inline);
            Some(MimePart::new("multipart/related", parts))
        }
        (None, true) => None,
        (None, false) => Some(MimePart::new("multipart/related", inline)),
    };

    match (content, attachments.is_empty()) {
        (Some(content), true) => content,
        (Some(content), false) => {
            let mut parts = vec![content];
            parts.extend(attachments);
            MimePart::new("multipart/mixed", parts)
        }
        (None, false) => MimePart::new("multipart/mixed", attachments),
        (None, true) => MimePart::new("text/plain", ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(message: &OutgoingMessage) -> String {
        String::from_utf8(build(message).unwrap()).unwrap()
    }

    fn parsed_addresses(value: Option<&mail_parser::Address<'_>>) -> Vec<String> {
        match value {
            Some(mail_parser::Address::List(items)) => items
                .iter()
                .filter_map(|item| item.address.as_deref().map(str::to_string))
                .collect(),
            Some(mail_parser::Address::Group(groups)) => groups
                .iter()
                .flat_map(|group| group.addresses.iter())
                .filter_map(|item| item.address.as_deref().map(str::to_string))
                .collect(),
            None => Vec::new(),
        }
    }

    fn base() -> OutgoingMessage {
        OutgoingMessage {
            from: Some(Address::new(
                Some("Ada Lovelace".to_string()),
                "ada@lovelace.dev",
            )),
            to: parse_addresses("grace@navy.dev, \"Hopper, G\" <g@navy.dev>"),
            subject: "Greetings".to_string(),
            text: Some("Hello there".to_string()),
            ..OutgoingMessage::default()
        }
    }

    #[test]
    fn parses_address_lists() {
        let addresses = parse_addresses("Ada <ada@lovelace.dev>, grace@navy.dev");
        assert_eq!(addresses.len(), 2);
        assert_eq!(addresses[0].name.as_deref(), Some("Ada"));
        assert_eq!(addresses[0].email, "ada@lovelace.dev");
        assert_eq!(addresses[1].name, None);
        assert_eq!(addresses[1].email, "grace@navy.dev");

        assert!(parse_addresses("   ").is_empty());
    }

    #[test]
    fn parses_quoted_names_and_groups() {
        let addresses = parse_addresses("\"Hopper, Grace\" <grace@navy.dev>");
        assert_eq!(addresses.len(), 1);
        assert_eq!(addresses[0].name.as_deref(), Some("Hopper, Grace"));
        assert_eq!(addresses[0].email, "grace@navy.dev");

        let grouped = parse_addresses("Team: ada@lovelace.dev, grace@navy.dev;");
        assert_eq!(grouped.len(), 2);
    }

    #[test]
    fn builds_a_plain_text_message() {
        let raw = rendered(&base());
        assert!(raw.contains("From: \"Ada Lovelace\" <ada@lovelace.dev>"));
        assert!(raw.contains("To: <grace@navy.dev>, \"Hopper, G\" <g@navy.dev>"));
        assert!(raw.contains("Subject: Greetings"));
        assert!(raw.contains("Content-Type: text/plain; charset=\"utf-8\""));
        assert!(raw.contains("Hello there"));
        assert!(!raw.contains("multipart"));
    }

    #[test]
    fn builds_a_text_and_html_alternative() {
        let mut message = base();
        message.html = Some("<p>Hello there</p>".to_string());
        let raw = rendered(&message);
        assert!(raw.contains("Content-Type: multipart/alternative"));
        assert!(raw.contains("Content-Type: text/plain; charset=\"utf-8\""));
        assert!(raw.contains("Content-Type: text/html; charset=\"utf-8\""));
        assert!(raw.contains("<p>Hello there</p>"));
        assert!(!raw.contains("multipart/mixed"));
    }

    #[test]
    fn keeps_inline_images_inside_a_related_part() {
        let mut message = base();
        message.html = Some("<img src=\"cid:logo@example.org\">".to_string());
        message.inline.push(InlineImage {
            content_id: "logo@example.org".to_string(),
            mime_type: "image/png".to_string(),
            data: vec![0x89, 0x50, 0x4e, 0x47],
        });
        let raw = rendered(&message);
        let related = raw.find("Content-Type: multipart/related").unwrap();
        let alternative = raw.find("Content-Type: multipart/alternative").unwrap();
        assert!(related < alternative);
        assert!(raw.contains("Content-ID: <logo@example.org>"));
        assert!(raw.contains("Content-Disposition: inline"));
        assert!(raw.contains("Content-Type: image/png"));
        assert!(!raw.contains("multipart/mixed"));
    }

    #[test]
    fn wraps_attachments_in_a_mixed_part() {
        let mut message = base();
        message.attachments.push(OutgoingAttachment {
            filename: "report.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
            data: b"%PDF-1.4".to_vec(),
        });
        let raw = rendered(&message);
        assert!(raw.contains("Content-Type: multipart/mixed"));
        assert!(raw.contains("Content-Type: application/pdf"));
        assert!(raw.contains("Content-Disposition: attachment; filename=\"report.pdf\""));
        assert!(raw.contains("Content-Transfer-Encoding: base64"));
    }

    #[test]
    fn stacks_mixed_related_and_alternative_for_full_messages() {
        let mut message = base();
        message.html = Some("<img src=\"cid:logo@example.org\">".to_string());
        message.inline.push(InlineImage {
            content_id: "logo@example.org".to_string(),
            mime_type: "image/png".to_string(),
            data: vec![1, 2, 3],
        });
        message.attachments.push(OutgoingAttachment {
            filename: "report.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
            data: b"%PDF-1.4".to_vec(),
        });
        let raw = rendered(&message);
        let mixed = raw.find("Content-Type: multipart/mixed").unwrap();
        let related = raw.find("Content-Type: multipart/related").unwrap();
        let alternative = raw.find("Content-Type: multipart/alternative").unwrap();
        assert!(mixed < related && related < alternative);
    }

    #[test]
    fn carries_threading_headers() {
        let mut message = base();
        message.in_reply_to = Some("<one@example.org>".to_string());
        message.references = vec![
            "<zero@example.org>".to_string(),
            "<one@example.org>".to_string(),
        ];
        let raw = rendered(&message);
        assert!(raw.contains("In-Reply-To: <one@example.org>"));
        assert!(raw.contains("References: <zero@example.org> <one@example.org>"));
    }

    #[test]
    fn round_trips_through_the_parser() {
        let mut message = base();
        message.cc = parse_addresses("cc@example.org");
        message.bcc = parse_addresses("bcc@example.org");
        message.html = Some("<p>Hello there</p>".to_string());
        message.attachments.push(OutgoingAttachment {
            filename: "notes.txt".to_string(),
            mime_type: "text/plain".to_string(),
            data: b"notes".to_vec(),
        });
        let raw = build(&message).unwrap();
        let parsed = MessageParser::default().parse(&raw).unwrap();

        assert_eq!(parsed.subject(), Some("Greetings"));
        assert_eq!(
            parsed_addresses(parsed.from()),
            vec!["ada@lovelace.dev".to_string()]
        );
        assert_eq!(
            parsed_addresses(parsed.to()),
            vec!["grace@navy.dev".to_string(), "g@navy.dev".to_string()]
        );
        assert_eq!(
            parsed_addresses(parsed.cc()),
            vec!["cc@example.org".to_string()]
        );
        assert_eq!(
            parsed_addresses(parsed.bcc()),
            vec!["bcc@example.org".to_string()]
        );
        assert_eq!(parsed.text_bodies().count(), 1);
        assert_eq!(parsed.html_bodies().count(), 1);
        assert_eq!(parsed.attachments().count(), 1);
        use mail_parser::MimeHeaders;
        assert_eq!(
            parsed.attachment(0).unwrap().attachment_name(),
            Some("notes.txt")
        );
    }
}
