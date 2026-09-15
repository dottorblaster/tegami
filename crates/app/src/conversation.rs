// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Conversation view.
//!
//! Renders every message of a thread as a card. Read messages start
//! collapsed, unread ones expand, and the quoted tail of a body is hidden
//! behind a toggle. Bodies are fetched lazily and rendered as plain text;
//! the rich HTML renderer replaces [`body_text`] later.

use std::sync::Arc;

use mail_core::store::{BodyState, MessageRecord, Store, bits_to_flags};
use relm4::adw;
use relm4::factory::{DynamicIndex, FactoryComponent, FactorySender, FactoryVecDeque};
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::prelude::*;
use tracing::debug;

use crate::message_text::{display_sender, display_subject, format_timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyStatus {
    Pending,
    Ready,
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct BodyFields {
    status: BodyStatus,
    head: String,
    quote: String,
    has_quote: bool,
}

impl BodyFields {
    fn pending() -> Self {
        Self {
            status: BodyStatus::Pending,
            head: String::new(),
            quote: String::new(),
            has_quote: false,
        }
    }

    fn unavailable() -> Self {
        Self {
            status: BodyStatus::Unavailable,
            head: String::new(),
            quote: String::new(),
            has_quote: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MessageCard {
    message_id: i64,
    folder_id: i64,
    uid: u32,
    sender: String,
    date: String,
    subject: String,
    unread: bool,
    has_attach: bool,
    expanded: bool,
    quote_visible: bool,
    body: BodyFields,
}

#[derive(Debug)]
pub enum MessageCardMsg {
    Toggle,
    ToggleQuote,
    SetBody { body: BodyFields, has_attach: bool },
}

#[relm4::factory(pub)]
impl FactoryComponent for MessageCard {
    type Init = MessageCard;
    type Input = MessageCardMsg;
    type Output = ();
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

                gtk::Spinner {
                    set_spinning: true,
                    set_halign: gtk::Align::Center,
                    #[watch]
                    set_visible: self.body.status == BodyStatus::Pending,
                },

                gtk::Label {
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_selectable: true,
                    set_label: "No plain-text body available.",
                    add_css_class: "dim-label",
                    #[watch]
                    set_visible: self.body.status == BodyStatus::Unavailable,
                },

                gtk::Label {
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_selectable: true,
                    add_css_class: "conversation-body",
                    #[watch]
                    set_label: &self.body.head,
                    #[watch]
                    set_visible: self.body.status == BodyStatus::Ready,
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
            },
        }
    }

    fn init_model(init: Self::Init, _index: &DynamicIndex, _sender: FactorySender<Self>) -> Self {
        init
    }

    fn update(&mut self, msg: Self::Input, _sender: FactorySender<Self>) {
        match msg {
            MessageCardMsg::Toggle => self.expanded = !self.expanded,
            MessageCardMsg::ToggleQuote => self.quote_visible = !self.quote_visible,
            MessageCardMsg::SetBody { body, has_attach } => {
                self.body = body;
                self.has_attach = has_attach;
            }
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
        let list = FactoryVecDeque::builder().launch_default().detach();
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
    let now = glib::DateTime::now_local()
        .or_else(|_| glib::DateTime::from_unix_utc(0))
        .expect("the Unix epoch is always a valid timestamp");
    let mut cards = Vec::with_capacity(records.len());
    for record in &records {
        cards.push(message_card(record, &now).await);
    }
    Ok(cards)
}

async fn message_card(record: &MessageRecord, now: &glib::DateTime) -> MessageCard {
    let flags = bits_to_flags(record.flags);
    let unread = !flags.seen;
    let timestamp = record.date_sent.or(record.date_recv);
    MessageCard {
        message_id: record.id.unwrap_or_default(),
        folder_id: record.folder_id,
        uid: record.uid,
        sender: display_sender(record),
        date: timestamp
            .map(|timestamp| format_timestamp(timestamp, now))
            .unwrap_or_default(),
        subject: display_subject(&record.subject),
        unread,
        has_attach: record.has_attach,
        expanded: unread,
        quote_visible: false,
        body: body_fields(record).await,
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
        Some(parsed) => match parsed.text {
            Some(text) if !text.trim().is_empty() => {
                let (head, quote) = split_quote(&text);
                let has_quote = quote.is_some();
                BodyFields {
                    status: BodyStatus::Ready,
                    head,
                    quote: quote.unwrap_or_default(),
                    has_quote,
                }
            }
            _ => BodyFields::unavailable(),
        },
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
}
