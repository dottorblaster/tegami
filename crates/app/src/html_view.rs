// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Rendering of message HTML bodies inside a hardened WebKitGTK view.

use relm4::gtk;
use relm4::gtk::gio;
use relm4::gtk::glib::prelude::*;
use relm4::gtk::prelude::*;
use webkit6::prelude::*;
use webkit6::{
    NavigationPolicyDecision, NavigationType, PolicyDecision, PolicyDecisionType, Settings, WebView,
};

const BODY_HEIGHT: i32 = 360;

pub(crate) fn new_webview(html: &str) -> WebView {
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

    let webview = WebView::builder().settings(&settings).build();
    webview.set_hexpand(true);
    webview.set_height_request(BODY_HEIGHT);
    webview.connect_decide_policy(|webview, decision, decision_type| {
        handle_policy(webview, decision, decision_type)
    });
    webview.load_html(html, None);
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
