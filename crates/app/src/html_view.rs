// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Rendering of message HTML bodies inside a hardened WebKitGTK view.
//!
//! Remote subresources are blocked through a default Content-Security-Policy
//! unless the reader explicitly opts in. Inline parts referenced through
//! `cid:` URLs are inlined as `data:` URIs so they stay available.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use mail_core::mime::Attachment;
use relm4::gtk;
use relm4::gtk::gio;
use relm4::gtk::glib::prelude::*;
use relm4::gtk::prelude::*;
use webkit6::prelude::*;
use webkit6::{
    NavigationPolicyDecision, NavigationType, PolicyDecision, PolicyDecisionType, Settings, WebView,
};

const BODY_HEIGHT: i32 = 360;
const BLOCKED_CSP: &str =
    "default-src 'none'; base-uri 'none'; img-src data:; style-src 'unsafe-inline'";

const REMOTE_MARKERS: [&str; 6] = [
    "src=",
    "srcset=",
    "background=",
    "poster=",
    "url(",
    "@import",
];

#[derive(Debug, Clone)]
pub(crate) struct InlinePart {
    content_id: String,
    mime_type: String,
    data: Vec<u8>,
}

pub(crate) fn inline_parts(attachments: Vec<Attachment>) -> Vec<InlinePart> {
    attachments
        .into_iter()
        .filter(crate::attachment::is_inline_image)
        .filter_map(|attachment| {
            let content_id = attachment.content_id?;
            Some(InlinePart {
                content_id,
                mime_type: attachment.mime_type,
                data: attachment.data,
            })
        })
        .collect()
}

pub(crate) fn has_remote_content(html: &str) -> bool {
    let haystack = html.to_ascii_lowercase();
    REMOTE_MARKERS
        .iter()
        .any(|marker| has_remote_url_after(&haystack, marker))
}

fn has_remote_url_after(haystack: &str, marker: &str) -> bool {
    let mut rest = haystack;
    while let Some(position) = rest.find(marker) {
        let after = &rest[position + marker.len()..];
        let value = after.trim_start_matches([' ', '\t', '\n', '\r', '"', '\'']);
        if value.starts_with("http://") || value.starts_with("https://") {
            return true;
        }
        rest = after;
    }
    false
}

fn inline_remote_parts(html: &str, parts: &[InlinePart]) -> String {
    parts.iter().fold(html.to_string(), |html, part| {
        let reference = format!("cid:{}", part.content_id);
        if !html.contains(&reference) {
            return html;
        }
        html.replace(&reference, &data_uri(&part.mime_type, &part.data))
    })
}

fn data_uri(mime_type: &str, data: &[u8]) -> String {
    format!("data:{mime_type};base64,{}", STANDARD.encode(data))
}

pub(crate) fn new_webview(html: &str, parts: &[InlinePart], allow_remote: bool) -> WebView {
    let settings = Settings::builder()
        .enable_javascript(false)
        .enable_javascript_markup(false)
        .enable_developer_extras(false)
        .enable_webgl(false)
        .enable_webaudio(false)
        .enable_media(false)
        .enable_media_stream(false)
        .enable_mediasource(false)
        .enable_html5_database(false)
        .enable_html5_local_storage(false)
        .enable_dns_prefetching(false)
        .javascript_can_access_clipboard(false)
        .javascript_can_open_windows_automatically(false)
        .allow_modal_dialogs(false)
        .allow_universal_access_from_file_urls(false)
        .allow_file_access_from_file_urls(false)
        .build();

    let mut builder = WebView::builder().settings(&settings);
    if !allow_remote {
        builder = builder.default_content_security_policy(BLOCKED_CSP);
    }

    let webview = builder.build();
    webview.set_hexpand(true);
    webview.set_height_request(BODY_HEIGHT);
    webview.connect_decide_policy(|webview, decision, decision_type| {
        handle_policy(webview, decision, decision_type)
    });
    webview.load_html(&inline_remote_parts(html, parts), None);
    webview
}

fn handle_policy(
    webview: &WebView,
    decision: &PolicyDecision,
    decision_type: PolicyDecisionType,
) -> bool {
    match decision_type {
        PolicyDecisionType::NavigationAction => {
            let Some(decision) = decision.downcast_ref::<NavigationPolicyDecision>() else {
                return false;
            };
            let Some(action) = decision.navigation_action() else {
                return false;
            };
            match action.navigation_type() {
                NavigationType::LinkClicked => {
                    let Some(uri) = action.request().and_then(|request| request.uri()) else {
                        return false;
                    };
                    open_externally(webview, &uri);
                    decision.ignore();
                    true
                }
                NavigationType::FormSubmitted | NavigationType::FormResubmitted => {
                    decision.ignore();
                    true
                }
                _ => false,
            }
        }
        PolicyDecisionType::NewWindowAction => {
            decision.ignore();
            true
        }
        PolicyDecisionType::Response => false,
        _ => false,
    }
}

fn open_externally(webview: &WebView, uri: &str) {
    let parent = webview.root().and_downcast::<gtk::Window>();
    let launcher = gtk::UriLauncher::new(uri);
    launcher.launch(parent.as_ref(), gio::Cancellable::NONE, |_| {});
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(mime_type: &str, content_id: Option<&str>, data: &[u8]) -> Attachment {
        Attachment {
            part_id: "1".to_string(),
            filename: None,
            mime_type: mime_type.to_string(),
            size: data.len(),
            content_id: content_id.map(str::to_string),
            data: data.to_vec(),
        }
    }

    #[test]
    fn inline_parts_keep_only_images_with_a_content_id() {
        let parts = inline_parts(vec![
            attachment("image/png", Some("logo@example.org"), b"png"),
            attachment("image/jpeg", None, b"jpeg"),
            attachment("application/pdf", Some("doc@example.org"), b"pdf"),
        ]);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].content_id, "logo@example.org");
        assert_eq!(parts[0].mime_type, "image/png");
        assert_eq!(parts[0].data, b"png");
    }
    #[test]
    fn detects_remote_resources() {
        assert!(has_remote_content(
            "<img src=\"https://tracker.example.org/pixel.png\">"
        ));
        assert!(has_remote_content("<img src=https://example.org/logo.png>"));
        assert!(has_remote_content(
            "<body background=\"http://example.org/bg.png\">"
        ));
        assert!(has_remote_content(
            "<style>body { background: url('https://example.org/bg.png'); }</style>"
        ));
        assert!(has_remote_content(
            "<style>@import \"https://example.org/style.css\";</style>"
        ));
        assert!(has_remote_content(
            "<img srcset=\"https://example.org/2x.png 2x\">"
        ));
    }

    #[test]
    fn ignores_relative_and_inlined_resources() {
        assert!(!has_remote_content(
            "<img src=\"cid:logo@example.org\"><img src=\"data:image/png;base64,AAAA\">"
        ));
        assert!(!has_remote_content(
            "<p>See <a href=\"https://example.org\">the site</a>.</p>"
        ));
        assert!(!has_remote_content("<p>Plain body</p>"));
    }

    #[test]
    fn inlines_cid_references_as_data_uris() {
        let parts = vec![InlinePart {
            content_id: "logo@example.org".to_string(),
            mime_type: "image/png".to_string(),
            data: b"hi".to_vec(),
        }];
        let html = "<img src=\"cid:logo@example.org\"><img src=\"cid:other@example.org\">";
        let inlined = inline_remote_parts(html, &parts);
        assert!(inlined.contains("src=\"data:image/png;base64,aGk=\""));
        assert!(inlined.contains("src=\"cid:other@example.org\""));
    }
}
