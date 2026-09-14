// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use mail_parser::{ContentType, MessageParser, MessagePart, MimeHeaders};

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
        html: message.body_html(0).map(|body| body.into_owned()),
        attachments: message
            .attachments()
            .enumerate()
            .map(|(index, part)| attachment(index, part))
            .collect(),
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
        assert!(
            parsed
                .html
                .as_deref()
                .is_some_and(|html| html.contains("Hello world"))
        );
        assert_eq!(parsed.attachments.len(), 1);

        let attachment = &parsed.attachments[0];
        assert_eq!(attachment.part_id, "0");
        assert_eq!(attachment.filename.as_deref(), Some("doc.pdf"));
        assert_eq!(attachment.mime_type, "application/pdf");
        assert_eq!(attachment.size, 5);
        assert_eq!(attachment.data, b"Hello");
        assert_eq!(attachment.content_id, None);
    }

    #[test]
    fn parses_plain_text() {
        let raw = "From: a@example.org\r\nSubject: hi\r\n\r\nbody line\r\n";
        let parsed = parse(raw.as_bytes()).unwrap();
        assert_eq!(parsed.text.as_deref().map(str::trim), Some("body line"));
        assert!(parsed.attachments.is_empty());
    }
}
