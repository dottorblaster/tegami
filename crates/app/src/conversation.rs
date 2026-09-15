// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Conversation view.
//!
//! Renders every message of a thread as a card. Read messages start
//! collapsed, unread ones expand, and the quoted tail of a plain-text body is
//! hidden behind a toggle. Bodies are fetched lazily and rendered as HTML in a
//! WebKitGTK view when available, falling back to plain text.

use std::collections::HashSet;
use std::sync::Arc;

use mail_core::store::{BodyState, MessageRecord, Store, bits_to_flags};
use relm4::adw;
use relm4::factory::{DynamicIndex, FactoryComponent, FactorySender, FactoryVecDeque};
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::prelude::*;
use tracing::debug;
use webkit6::WebView;
use webkit6::prelude::WebViewExt;

use crate::html_view::{self, InlinePart};
use crate::message_text::{display_sender, display_subject, format_timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyStatus {
    Pending,
    Ready,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyView {
    Html,
    Plain,
}

#[derive(Debug, Clone)]
pub struct BodyFields {
    status: BodyStatus,
    head: String,
    quote: String,
    has_quote: bool,
    html: Option<String>,
    has_remote: bool,
    inline: Vec<InlinePart>,
}

impl BodyFields {
    fn pending() -> Self {
        Self {
            status: BodyStatus::Pending,
            head: String::new(),
            quote: String::new(),
            has_quote: false,
            html: None,
            has_remote: false,
            inline: Vec::new(),
        }
    }

    fn unavailable() -> Self {
        Self {
            status: BodyStatus::Unavailable,
            head: String::new(),
            quote: String::new(),
            has_quote: false,
            html: None,
            has_remote: false,
            inline: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MessageCard {
    message_id: i64,
    folder_id: i64,
    uid: u32,
    sender: String,
    sender_addr: Option<String>,
    date: String,
    subject: String,
    unread: bool,
    has_attach: bool,
    expanded: bool,
    quote_visible: bool,
    view: BodyView,
    remote_allowed: bool,
    body: BodyFields,
}

#[derive(Debug)]
pub enum MessageCardMsg {
    Toggle,
    ToggleQuote,
    ToggleView,
    Refresh,
    LoadRemoteOnce,
    AllowSender,
    SetBody { body: BodyFields, has_attach: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageCardOutput {
    AllowSender { sender: String },
}

#[relm4::factory(pub)]
impl FactoryComponent for MessageCard {
    type Init = MessageCard;
    type Input = MessageCardMsg;
    type Output = MessageCardOutput;
    type CommandOutput = ();
    type ParentWidget = gtk::Box;

    view! {
        root = gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            #[watch]
            set_css_classes: &self.css_classes(),

            gtk::Button {
                add_css_class: "flat",
                add_css_class: "conversation-header",
                set_hexpand: true,
                connect_clicked[sender] => move |_| sender.input(MessageCardMsg::Toggle),

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 10,
                    set_margin_all: 8,

                    gtk::Label {
                        set_label: "•",
                        set_valign: gtk::Align::Center,
                        add_css_class: "conversation-unread-dot",
                        #[watch]
                        set_visible: self.unread,
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 2,
                        set_hexpand: true,

                        gtk::Label {
                            set_xalign: 0.0,
                            set_ellipsize: gtk::pango::EllipsizeMode::End,
                            add_css_class: "conversation-sender",
                            #[watch]
                            set_label: &self.sender,
                        },

                        gtk::Label {
                            set_xalign: 0.0,
                            set_ellipsize: gtk::pango::EllipsizeMode::End,
                            add_css_class: "dim-label",
                            #[watch]
                            set_label: &self.subject,
                        },
                    },

                    gtk::Image {
                        set_icon_name: Some("mail-attachment-symbolic"),
                        set_valign: gtk::Align::Center,
                        add_css_class: "dim-label",
                        #[watch]
                        set_visible: self.has_attach,
                    },

                    gtk::Label {
                        set_valign: gtk::Align::Center,
                        add_css_class: "conversation-date",
                        #[watch]
                        set_label: &self.date,
                    },

                    gtk::Image {
                        set_valign: gtk::Align::Center,
                        add_css_class: "dim-label",
                        #[watch]
                        set_icon_name: Some(self.chevron()),
                    },
                },
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
                set_margin_start: 14,
                set_margin_end: 14,
                set_margin_bottom: 12,
                #[watch]
                set_visible: self.expanded,

                gtk::Stack {
                    set_hexpand: true,
                    #[watch(skip_init)]
                    set_visible_child_name: self.body_page(),

                    add_named[Some("pending")] = &gtk::Box {
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,

                        gtk::Spinner {
                            set_spinning: true,
                        },
                    },

                    add_named[Some("unavailable")] = &gtk::Label {
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_selectable: true,
                        set_label: "No body available.",
                        add_css_class: "dim-label",
                    },

                    add_named[Some("plain")] = &gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 6,
                        set_hexpand: true,

                        gtk::Label {
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_selectable: true,
                            add_css_class: "conversation-body",
                            #[watch]
                            set_label: &self.body.head,
                        },

                        gtk::Label {
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_selectable: true,
                            add_css_class: "conversation-quote",
                            #[watch]
                            set_label: &self.body.quote,
                            #[watch]
                            set_visible: self.body.has_quote && self.quote_visible,
                        },

                        gtk::Button {
                            set_halign: gtk::Align::Start,
                            set_label: "Show quoted text",
                            add_css_class: "flat",
                            add_css_class: "conversation-quote-toggle",
                            #[watch]
                            set_visible: self.body.has_quote && !self.quote_visible,
                            connect_clicked[sender] => move |_| sender.input(MessageCardMsg::ToggleQuote),
                        },

                        gtk::Button {
                            set_halign: gtk::Align::Start,
                            set_label: "Show rich text",
                            add_css_class: "flat",
                            add_css_class: "conversation-view-toggle",
                            #[watch]
                            set_visible: self.body.html.is_some(),
                            connect_clicked[sender] => move |_| sender.input(MessageCardMsg::ToggleView),
                        },
                    },

                    add_named[Some("html")] = &gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 6,
                        set_hexpand: true,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 6,
                            set_margin_bottom: 6,
                            add_css_class: "conversation-remote-banner",
                            #[watch]
                            set_visible: self.show_remote_banner(),

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 8,

                                gtk::Image {
                                    set_icon_name: Some("dialog-warning-symbolic"),
                                    set_valign: gtk::Align::Start,
                                    add_css_class: "dim-label",
                                },

                                gtk::Label {
                                    set_xalign: 0.0,
                                    set_wrap: true,
                                    set_hexpand: true,
                                    add_css_class: "dim-label",
                                    set_label: "Remote content is blocked to protect your privacy.",
                                },
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 6,
                                set_halign: gtk::Align::Start,

                                gtk::Button {
                                    set_label: "Load once",
                                    add_css_class: "flat",
                                    add_css_class: "conversation-view-toggle",
                                    connect_clicked[sender] => move |_| sender.input(MessageCardMsg::LoadRemoteOnce),
                                },

                                gtk::Button {
                                    set_label: "Always from this sender",
                                    add_css_class: "flat",
                                    add_css_class: "conversation-view-toggle",
                                    connect_clicked[sender] => move |_| sender.input(MessageCardMsg::AllowSender),
                                },
                            },
                        },

                        #[name(html_slot)]
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_hexpand: true,
                        },

                        gtk::Button {
                            set_halign: gtk::Align::Start,
                            set_label: "Show plain text",
                            add_css_class: "flat",
                            add_css_class: "conversation-view-toggle",
                            connect_clicked[sender] => move |_| sender.input(MessageCardMsg::ToggleView),
                        },
                    },
                },
            },
        }
    }

    fn init_model(init: Self::Init, _index: &DynamicIndex, _sender: FactorySender<Self>) -> Self {
        init
    }

    fn update(&mut self, msg: Self::Input, sender: FactorySender<Self>) {
        match msg {
            MessageCardMsg::Toggle => self.expanded = !self.expanded,
            MessageCardMsg::ToggleQuote => self.quote_visible = !self.quote_visible,
            MessageCardMsg::ToggleView => self.view = self.view.other(),
            MessageCardMsg::Refresh => {}
            MessageCardMsg::LoadRemoteOnce => self.remote_allowed = true,
            MessageCardMsg::AllowSender => {
                self.remote_allowed = true;
                if let Some(sender_addr) = self.sender_addr.clone() {
                    let _ = sender.output(MessageCardOutput::AllowSender {
                        sender: sender_addr,
                    });
                }
            }
            MessageCardMsg::SetBody { body, has_attach } => {
                self.view = if body.html.is_some() {
                    BodyView::Html
                } else {
                    BodyView::Plain
                };
                self.body = body;
                self.has_attach = has_attach;
            }
        }
    }

    fn post_view(&self, _sender: FactorySender<Self>) {
        if !self.expanded || self.view != BodyView::Html || self.body.status != BodyStatus::Ready {
            return;
        }
        let Some(html) = self.body.html.as_deref() else {
            return;
        };
        let current = html_slot.first_child();
        let up_to_date = current
            .as_ref()
            .and_then(|child| child.downcast_ref::<WebView>())
            .is_some_and(|webview| {
                webview.default_content_security_policy().is_none() == self.remote_allowed
            });
        if up_to_date {
            return;
        }
        if let Some(child) = current {
            html_slot.remove(&child);
        }
        html_slot.append(&html_view::new_webview(
            html,
            &self.body.inline,
            self.remote_allowed,
        ));
    }
}

impl BodyView {
    fn other(&self) -> Self {
        match self {
            Self::Html => Self::Plain,
            Self::Plain => Self::Html,
        }
    }
}

impl MessageCard {
    fn css_classes(&self) -> Vec<&'static str> {
        if self.unread {
            vec!["conversation-message", "unread"]
        } else {
            vec!["conversation-message"]
        }
    }

    fn chevron(&self) -> &'static str {
        if self.expanded {
            "pan-up-symbolic"
        } else {
            "pan-down-symbolic"
        }
    }

    fn body_page(&self) -> &'static str {
        page_for(&self.body, self.view)
    }

    fn show_remote_banner(&self) -> bool {
        remote_banner_visible(self)
    }
}

fn remote_banner_visible(card: &MessageCard) -> bool {
    card.expanded
        && card.view == BodyView::Html
        && card.body.status == BodyStatus::Ready
        && card.body.has_remote
        && !card.remote_allowed
}

fn page_for(body: &BodyFields, view: BodyView) -> &'static str {
    match body.status {
        BodyStatus::Pending => "pending",
        BodyStatus::Unavailable => "unavailable",
        BodyStatus::Ready if view == BodyView::Html && body.html.is_some() => "html",
        BodyStatus::Ready => "plain",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewState {
    Idle,
    Loading,
    Empty,
    Ready,
    Error,
}

impl ViewState {
    fn page_name(&self) -> &'static str {
        match self {
            Self::Idle | Self::Empty => "empty",
            Self::Loading => "loading",
            Self::Ready => "messages",
            Self::Error => "error",
        }
    }
}

#[derive(Debug)]
pub enum ConversationMsg {
    Load {
        folder_id: i64,
        uid: u32,
    },
    Loaded {
        folder_id: i64,
        uid: u32,
        result: Result<Vec<MessageCard>, String>,
    },
    BodyFetched {
        message_id: i64,
    },
    BodyLoaded {
        message_id: i64,
        body: BodyFields,
        has_attach: bool,
    },
    AllowSender {
        sender: String,
    },
    Retry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationOutput {
    FetchBody { folder_id: i64, uid: u32 },
}

pub struct Conversation {
    store: Arc<Store>,
    list: FactoryVecDeque<MessageCard>,
    selection: Option<(i64, u32)>,
    state: ViewState,
}

#[relm4::component(pub)]
impl SimpleComponent for Conversation {
    type Init = Arc<Store>;
    type Input = ConversationMsg;
    type Output = ConversationOutput;

    view! {
        #[root]
        adw::ViewStack {
            set_vexpand: true,
            set_hexpand: true,

            add_named[Some("messages")] = &gtk::ScrolledWindow {
                set_vexpand: true,
                set_hexpand: true,
                set_policy: (gtk::PolicyType::Never, gtk::PolicyType::Automatic),

                #[local_ref]
                messages -> gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    set_margin_all: 12,
                    add_css_class: "conversation",
                },
            },

            add_named[Some("loading")] = &adw::StatusPage {
                set_title: "Loading conversation…",

                #[wrap(Some)]
                set_child = &adw::Spinner {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                },
            },

            add_named[Some("empty")] = &adw::StatusPage {
                set_icon_name: Some("mail-symbolic"),
                set_title: "Nothing to read",
                set_description: Some("Select a message to open its conversation."),
            },

            add_named[Some("error")] = &adw::StatusPage {
                set_icon_name: Some("dialog-warning-symbolic"),
                set_title: "Couldn't load the conversation",

                #[wrap(Some)]
                set_child = &gtk::Button {
                    set_label: "Try Again",
                    set_halign: gtk::Align::Center,
                    add_css_class: "pill",
                    connect_clicked[sender] => move |_| sender.input(ConversationMsg::Retry),
                },
            },

            #[watch]
            set_visible_child_name: model.state.page_name(),
        }
    }

    fn init(
        store: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let list =
            FactoryVecDeque::builder()
                .launch_default()
                .forward(sender.input_sender(), |output| match output {
                    MessageCardOutput::AllowSender { sender } => {
                        ConversationMsg::AllowSender { sender }
                    }
                });
        let model = Conversation {
            store,
            list,
            selection: None,
            state: ViewState::Idle,
        };
        let messages = model.list.widget();
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            ConversationMsg::Load { folder_id, uid } => {
                self.selection = Some((folder_id, uid));
                self.state = ViewState::Loading;
                self.list.guard().clear();
                let store = self.store.clone();
                let load_sender = sender.clone();
                sender.oneshot_command(async move {
                    let result = load_conversation(&store, folder_id, uid).await;
                    load_sender.input(ConversationMsg::Loaded {
                        folder_id,
                        uid,
                        result,
                    });
                });
            }
            ConversationMsg::Loaded {
                folder_id,
                uid,
                result,
            } => {
                if self.selection != Some((folder_id, uid)) {
                    return;
                }
                self.list.guard().clear();
                match result {
                    Ok(cards) if cards.is_empty() => {
                        self.state = ViewState::Empty;
                    }
                    Ok(cards) => {
                        let pending: Vec<(i64, u32)> = cards
                            .iter()
                            .filter(|card| card.body.status == BodyStatus::Pending)
                            .map(|card| (card.folder_id, card.uid))
                            .collect();
                        let mut guard = self.list.guard();
                        for card in cards {
                            guard.push_back(card);
                        }
                        drop(guard);
                        for index in 0..self.list.len() {
                            self.list.send(index, MessageCardMsg::Refresh);
                        }
                        self.state = ViewState::Ready;
                        for (folder_id, uid) in pending {
                            let _ = sender.output(ConversationOutput::FetchBody { folder_id, uid });
                        }
                    }
                    Err(detail) => {
                        debug!(detail, "failed to load conversation");
                        self.state = ViewState::Error;
                    }
                }
            }
            ConversationMsg::BodyFetched { message_id } => {
                let Some((_, folder_id, uid)) = self.find(message_id) else {
                    return;
                };
                let store = self.store.clone();
                let load_sender = sender.clone();
                sender.oneshot_command(async move {
                    let (body, has_attach) = match store.message(folder_id, uid).await {
                        Ok(Some(record)) => (body_fields(&record).await, record.has_attach),
                        _ => (BodyFields::unavailable(), false),
                    };
                    load_sender.input(ConversationMsg::BodyLoaded {
                        message_id,
                        body,
                        has_attach,
                    });
                });
            }
            ConversationMsg::BodyLoaded {
                message_id,
                body,
                has_attach,
            } => {
                if let Some((index, _, _)) = self.find(message_id) {
                    self.list
                        .send(index, MessageCardMsg::SetBody { body, has_attach });
                }
            }
            ConversationMsg::AllowSender { sender: address } => {
                let store = self.store.clone();
                sender.oneshot_command(async move {
                    if let Err(err) = store.allow_remote_content(&address).await {
                        debug!(detail = %err, "failed to store remote content sender");
                    }
                });
            }
            ConversationMsg::Retry => {
                if let Some((folder_id, uid)) = self.selection {
                    sender.input(ConversationMsg::Load { folder_id, uid });
                }
            }
        }
    }
}

impl Conversation {
    fn find(&self, message_id: i64) -> Option<(usize, i64, u32)> {
        (0..self.list.len()).find_map(|index| {
            let card = self.list.get(index)?;
            (card.message_id == message_id).then_some((index, card.folder_id, card.uid))
        })
    }
}

async fn load_conversation(
    store: &Store,
    folder_id: i64,
    uid: u32,
) -> Result<Vec<MessageCard>, String> {
    let anchor = store
        .message(folder_id, uid)
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "message not found".to_string())?;
    let records = match anchor.thread_id {
        Some(thread_id) => store
            .thread_messages(thread_id)
            .await
            .map_err(|err| err.to_string())?,
        None => vec![anchor],
    };
    let allowed: HashSet<String> = store
        .remote_content_senders()
        .await
        .map_err(|err| err.to_string())?
        .into_iter()
        .collect();
    let now = glib::DateTime::now_local()
        .or_else(|_| glib::DateTime::from_unix_utc(0))
        .expect("the Unix epoch is always a valid timestamp");
    let mut cards = Vec::with_capacity(records.len());
    for record in &records {
        cards.push(message_card(record, &now, &allowed).await);
    }
    Ok(cards)
}

async fn message_card(
    record: &MessageRecord,
    now: &glib::DateTime,
    allowed: &HashSet<String>,
) -> MessageCard {
    let flags = bits_to_flags(record.flags);
    let unread = !flags.seen;
    let timestamp = record.date_sent.or(record.date_recv);
    let body = body_fields(record).await;
    let sender_addr = record.from_addr.clone();
    let remote_allowed = sender_addr
        .as_deref()
        .is_some_and(|address| allowed.contains(&address.trim().to_ascii_lowercase()));
    MessageCard {
        message_id: record.id.unwrap_or_default(),
        folder_id: record.folder_id,
        uid: record.uid,
        sender: display_sender(record),
        sender_addr,
        date: timestamp
            .map(|timestamp| format_timestamp(timestamp, now))
            .unwrap_or_default(),
        subject: display_subject(&record.subject),
        unread,
        has_attach: record.has_attach,
        expanded: unread,
        quote_visible: false,
        view: if body.html.is_some() {
            BodyView::Html
        } else {
            BodyView::Plain
        },
        remote_allowed,
        body,
    }
}

async fn body_fields(record: &MessageRecord) -> BodyFields {
    if record.body_state != BodyState::Full {
        return BodyFields::pending();
    }
    let Some(path) = record.raw_path.as_deref() else {
        return BodyFields::unavailable();
    };
    let Ok(raw) = tokio::fs::read(path).await else {
        return BodyFields::unavailable();
    };
    match mail_core::mime::parse(&raw) {
        Some(parsed) => {
            let html = parsed.html.filter(|html| !html.trim().is_empty());
            let has_remote = html.as_deref().is_some_and(html_view::has_remote_content);
            let inline = if html.is_some() {
                html_view::inline_parts(parsed.attachments)
            } else {
                Vec::new()
            };
            match parsed.text {
                Some(text) if !text.trim().is_empty() => {
                    let (head, quote) = split_quote(&text);
                    let has_quote = quote.is_some();
                    BodyFields {
                        status: BodyStatus::Ready,
                        head,
                        quote: quote.unwrap_or_default(),
                        has_quote,
                        html,
                        has_remote,
                        inline,
                    }
                }
                _ if html.is_some() => BodyFields {
                    status: BodyStatus::Ready,
                    head: String::new(),
                    quote: String::new(),
                    has_quote: false,
                    html,
                    has_remote,
                    inline,
                },
                _ => BodyFields::unavailable(),
            }
        }
        None => BodyFields::unavailable(),
    }
}

fn split_quote(text: &str) -> (String, Option<String>) {
    let mut head = Vec::new();
    let mut quote = Vec::new();
    let mut quoting = false;
    for line in text.lines() {
        if !quoting && line.trim_start().starts_with('>') {
            quoting = true;
        }
        if quoting {
            quote.push(line);
        } else {
            head.push(line);
        }
    }
    let head = head.join("\n").trim_end().to_string();
    let quote = (!quote.is_empty()).then(|| quote.join("\n").trim_end().to_string());
    (head, quote)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_quote_separates_the_quoted_tail() {
        let (head, quote) = split_quote("Hello\n\n> quoted one\n> quoted two\n");
        assert_eq!(head, "Hello");
        assert_eq!(quote.as_deref(), Some("> quoted one\n> quoted two"));
    }

    #[test]
    fn split_quote_without_a_quote_keeps_everything() {
        let (head, quote) = split_quote("Just a body.\nSecond line.");
        assert_eq!(head, "Just a body.\nSecond line.");
        assert!(quote.is_none());
    }

    #[test]
    fn split_quote_handles_indented_markers_and_attribution() {
        let (head, quote) = split_quote("On Monday, Ada wrote:\n  > old text");
        assert_eq!(head, "On Monday, Ada wrote:");
        assert_eq!(quote.as_deref(), Some("  > old text"));
    }

    #[test]
    fn split_quote_of_a_quote_only_body_has_no_head() {
        let (head, quote) = split_quote("> only the quote");
        assert!(head.is_empty());
        assert_eq!(quote.as_deref(), Some("> only the quote"));
    }

    #[test]
    fn view_state_maps_to_stack_page() {
        assert_eq!(ViewState::Idle.page_name(), "empty");
        assert_eq!(ViewState::Loading.page_name(), "loading");
        assert_eq!(ViewState::Empty.page_name(), "empty");
        assert_eq!(ViewState::Ready.page_name(), "messages");
        assert_eq!(ViewState::Error.page_name(), "error");
    }

    fn ready_body(html: Option<&str>) -> BodyFields {
        BodyFields {
            status: BodyStatus::Ready,
            head: "Hello".to_string(),
            quote: String::new(),
            has_quote: false,
            html: html.map(str::to_string),
            has_remote: false,
            inline: Vec::new(),
        }
    }

    fn card(body: BodyFields) -> MessageCard {
        MessageCard {
            message_id: 1,
            folder_id: 1,
            uid: 1,
            sender: "Ada Lovelace".to_string(),
            sender_addr: Some("ada@lovelace.dev".to_string()),
            date: String::new(),
            subject: "Hello".to_string(),
            unread: true,
            has_attach: false,
            expanded: true,
            quote_visible: false,
            view: BodyView::Html,
            remote_allowed: false,
            body,
        }
    }

    #[test]
    fn remote_banner_shows_only_for_blocked_remote_html() {
        let mut body = ready_body(Some("<img src=\"https://example.org/a.png\">"));
        body.has_remote = true;
        let mut card = card(body);
        assert!(remote_banner_visible(&card));

        card.remote_allowed = true;
        assert!(!remote_banner_visible(&card));

        card.remote_allowed = false;
        card.view = BodyView::Plain;
        assert!(!remote_banner_visible(&card));

        card.view = BodyView::Html;
        card.expanded = false;
        assert!(!remote_banner_visible(&card));

        card.expanded = true;
        card.body.has_remote = false;
        assert!(!remote_banner_visible(&card));
    }

    #[test]
    fn body_page_prefers_html_and_falls_back_to_plain() {
        assert_eq!(page_for(&ready_body(None), BodyView::Html), "plain");
        assert_eq!(page_for(&ready_body(None), BodyView::Plain), "plain");
        assert_eq!(
            page_for(&ready_body(Some("<p>hi</p>")), BodyView::Html),
            "html"
        );
        assert_eq!(
            page_for(&ready_body(Some("<p>hi</p>")), BodyView::Plain),
            "plain"
        );
    }

    #[test]
    fn body_page_covers_pending_and_unavailable() {
        assert_eq!(page_for(&BodyFields::pending(), BodyView::Html), "pending");
        assert_eq!(
            page_for(&BodyFields::unavailable(), BodyView::Plain),
            "unavailable"
        );
    }

    #[test]
    fn body_view_toggles_between_modes() {
        assert_eq!(BodyView::Html.other(), BodyView::Plain);
        assert_eq!(BodyView::Plain.other(), BodyView::Html);
    }
}
