// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use mail_parser::{ContentType, Message, MessageParser, MessagePart, MimeHeaders, PartType};

pub struct Attachment {
    pub part_id: String,
    pub filename: Option<String>,
    pub mime_type: String,
    pub size: usize,
    pub content_id: Option<String>,
    pub data: Vec<u8>,
}

pub struct ParsedMessage {
    pub text: Option<String>,
    pub html: Option<String>,
    pub attachments: Vec<Attachment>,
}

pub fn parse(raw: &[u8]) -> Option<ParsedMessage> {
    let message = MessageParser::default().parse(raw)?;
    Some(ParsedMessage {
        text: message.body_text(0).map(|body| body.into_owned()),
        html: genuine_html(&message),
        attachments: message
            .attachments()
            .enumerate()
            .map(|(index, part)| attachment(index, part))
            .collect(),
    })
}

fn genuine_html(message: &Message<'_>) -> Option<String> {
    message
        .html_body
        .first()
        .and_then(|&index| message.parts.get(index as usize))
        .and_then(|part| match &part.body {
            PartType::Html(html) => Some(html.as_ref().to_owned()),
            _ => None,
        })
}

fn attachment(index: usize, part: &MessagePart<'_>) -> Attachment {
    Attachment {
        part_id: index.to_string(),
        filename: part.attachment_name().map(str::to_string),
        mime_type: part
            .content_type()
            .map(content_type)
            .unwrap_or_else(|| "application/octet-stream".to_string()),
        size: part.contents().len(),
        content_id: part.content_id().map(str::to_string),
        data: part.contents().to_vec(),
    }
}

fn content_type(content_type: &ContentType<'_>) -> String {
    match &content_type.c_subtype {
        Some(subtype) => format!("{}/{}", content_type.c_type, subtype),
        None => content_type.c_type.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    const MULTIPART: &str = concat!(
        "From: Sender <sender@example.org>\r\n",
        "To: me@example.org\r\n",
        "Subject: greeting\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"BOUND\"\r\n",
        "\r\n",
        "--BOUND\r\n",
        "Content-Type: text/plain; charset=\"utf-8\"\r\n",
        "\r\n",
        "Hello world\r\n",
        "--BOUND\r\n",
        "Content-Type: application/pdf; name=\"doc.pdf\"\r\n",
        "Content-Disposition: attachment; filename=\"doc.pdf\"\r\n",
        "Content-Transfer-Encoding: base64\r\n",
        "\r\n",
        "SGVsbG8=\r\n",
        "--BOUND--\r\n",
    );

    #[test]
    fn parses_bodies_and_attachments() {
        let parsed = parse(MULTIPART.as_bytes()).unwrap();
        assert_eq!(parsed.text.as_deref().map(str::trim), Some("Hello world"));
        assert_eq!(parsed.attachments.len(), 1);

        let attachment = &parsed.attachments[0];
        assert_eq!(attachment.part_id, "0");
        assert_eq!(attachment.filename.as_deref(), Some("doc.pdf"));
        assert_eq!(attachment.mime_type, "application/pdf");
        assert_eq!(attachment.size, 5);
        assert_eq!(attachment.data, b"Hello");
        assert_eq!(attachment.content_id, None);
        assert!(parsed.html.is_none());
    }

    const HTML_MESSAGE: &str = concat!(
        "From: Sender <sender@example.org>\r\n",
        "To: me@example.org\r\n",
        "Subject: rich message\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/alternative; boundary=\"ALT\"\r\n",
        "\r\n",
        "--ALT\r\n",
        "Content-Type: text/plain; charset=\"utf-8\"\r\n",
        "\r\n",
        "Hello world\r\n",
        "--ALT\r\n",
        "Content-Type: text/html; charset=\"utf-8\"\r\n",
        "\r\n",
        "<html><body><p>Hello <b>world</b></p></body></html>\r\n",
        "--ALT--\r\n",
    );

    #[test]
    fn keeps_only_the_genuine_html_part() {
        let parsed = parse(HTML_MESSAGE.as_bytes()).unwrap();
        assert_eq!(parsed.text.as_deref().map(str::trim), Some("Hello world"));
        assert_eq!(
            parsed.html.as_deref().map(str::trim),
            Some("<html><body><p>Hello <b>world</b></p></body></html>")
        );
    }

    const INLINE_MESSAGE: &str = concat!(
        "From: Sender <sender@example.org>\r\n",
        "To: me@example.org\r\n",
        "Subject: inline\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/related; boundary=\"REL\"\r\n",
        "\r\n",
        "--REL\r\n",
        "Content-Type: text/html; charset=\"utf-8\"\r\n",
        "\r\n",
        "<p><img src=\"cid:logo@example.org\"></p>\r\n",
        "--REL\r\n",
        "Content-Type: image/png\r\n",
        "Content-Transfer-Encoding: base64\r\n",
        "Content-ID: <logo@example.org>\r\n",
        "\r\n",
        "aGk=\r\n",
        "--REL--\r\n",
    );

    #[test]
    fn captures_inline_parts_with_their_content_id() {
        let parsed = parse(INLINE_MESSAGE.as_bytes()).unwrap();
        assert_eq!(parsed.attachments.len(), 1);

        let inline = &parsed.attachments[0];
        assert_eq!(inline.content_id.as_deref(), Some("logo@example.org"));
        assert_eq!(inline.mime_type, "image/png");
        assert_eq!(inline.data, b"hi");
        assert!(
            parsed
                .html
                .as_deref()
                .is_some_and(|html| html.contains("cid:logo@example.org"))
        );
    }

    #[test]
    fn parses_plain_text() {
        let raw = "From: a@example.org\r\nSubject: hi\r\n\r\nbody line\r\n";
        let parsed = parse(raw.as_bytes()).unwrap();
        assert_eq!(parsed.text.as_deref().map(str::trim), Some("body line"));
        assert!(parsed.html.is_none());
        assert!(parsed.attachments.is_empty());
    }
}
