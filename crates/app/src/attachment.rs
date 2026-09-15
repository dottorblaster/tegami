// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Attachment classification, presentation metadata and extraction.

use std::path::{Path, PathBuf};

use mail_core::mime::Attachment;
use relm4::gtk::glib;

use crate::config;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttachmentInfo {
    pub part_id: String,
    pub name: String,
    pub mime_type: String,
    pub size: usize,
}

pub(crate) fn is_inline_image(attachment: &Attachment) -> bool {
    attachment.content_id.is_some() && attachment.mime_type.starts_with("image/")
}

pub(crate) fn list(attachments: &[Attachment]) -> Vec<AttachmentInfo> {
    attachments
        .iter()
        .filter(|attachment| !is_inline_image(attachment))
        .map(info)
        .collect()
}

pub(crate) fn size_text(size: usize) -> String {
    glib::format_size(size as u64).to_string()
}

pub(crate) fn safe_filename(name: &str) -> String {
    let candidate = Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .trim()
        .trim_start_matches('.');
    if candidate.is_empty() {
        "attachment".to_string()
    } else {
        candidate.to_string()
    }
}

fn info(attachment: &Attachment) -> AttachmentInfo {
    AttachmentInfo {
        part_id: attachment.part_id.clone(),
        name: display_name(attachment.filename.as_deref(), &attachment.mime_type),
        mime_type: attachment.mime_type.clone(),
        size: attachment.size,
    }
}

fn display_name(filename: Option<&str>, mime_type: &str) -> String {
    if let Some(name) = filename.map(str::trim).filter(|name| !name.is_empty()) {
        return name.to_string();
    }
    match subtype(mime_type) {
        Some(subtype) => format!("attachment.{subtype}"),
        None => "attachment".to_string(),
    }
}

fn subtype(mime_type: &str) -> Option<&str> {
    let subtype = mime_type.split('/').nth(1)?.split(';').next()?.trim();
    (!subtype.is_empty()).then_some(subtype)
}

async fn bytes(raw_path: Option<&str>, part_id: &str) -> Result<Attachment, String> {
    let raw_path = raw_path.ok_or_else(|| "message body is not cached".to_string())?;
    let raw = tokio::fs::read(raw_path)
        .await
        .map_err(|err| err.to_string())?;
    mail_core::mime::find_attachment(&raw, part_id)
        .ok_or_else(|| "attachment is missing from the message".to_string())
}

pub(crate) async fn extract(
    message_id: i64,
    raw_path: Option<&str>,
    part_id: &str,
    name: &str,
) -> Result<PathBuf, String> {
    let attachment = bytes(raw_path, part_id).await?;
    let directory = config::attachment_dir().join(format!("{message_id}-{part_id}"));
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|err| err.to_string())?;
    let path = directory.join(safe_filename(name));
    tokio::fs::write(&path, &attachment.data)
        .await
        .map_err(|err| err.to_string())?;
    Ok(path)
}

pub(crate) async fn write(
    raw_path: Option<&str>,
    part_id: &str,
    destination: &Path,
) -> Result<(), String> {
    let attachment = bytes(raw_path, part_id).await?;
    tokio::fs::write(destination, &attachment.data)
        .await
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(filename: Option<&str>, mime_type: &str, content_id: Option<&str>) -> Attachment {
        Attachment {
            part_id: "0".to_string(),
            filename: filename.map(str::to_string),
            mime_type: mime_type.to_string(),
            size: 5,
            content_id: content_id.map(str::to_string),
            data: b"hello".to_vec(),
        }
    }

    #[test]
    fn list_skips_inline_images() {
        let attachments = vec![
            attachment(Some("report.pdf"), "application/pdf", None),
            attachment(None, "image/png", Some("logo@example.org")),
            attachment(Some("photo.jpg"), "image/jpeg", None),
        ];
        let listed = list(&attachments);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "report.pdf");
        assert_eq!(listed[1].name, "photo.jpg");
    }

    #[test]
    fn display_name_falls_back_to_the_mime_subtype() {
        assert_eq!(
            display_name(Some(" report.pdf "), "application/pdf"),
            "report.pdf"
        );
        assert_eq!(
            display_name(Some("  "), "application/pdf"),
            "attachment.pdf"
        );
        assert_eq!(
            display_name(None, "text/calendar; charset=utf-8"),
            "attachment.calendar"
        );
        assert_eq!(display_name(None, ""), "attachment");
    }

    #[test]
    fn safe_filename_strips_paths_and_hidden_names() {
        assert_eq!(safe_filename("report.pdf"), "report.pdf");
        assert_eq!(safe_filename("../../etc/passwd"), "passwd");
        assert_eq!(safe_filename("/tmp/evil.pdf"), "evil.pdf");
        assert_eq!(safe_filename(".bashrc"), "bashrc");
        assert_eq!(safe_filename(".."), "attachment");
        assert_eq!(safe_filename(""), "attachment");
    }
}
