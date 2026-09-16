// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Message list.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use mail_core::store::{MessageRecord, Store, bits_to_flags};
use relm4::adw;
use relm4::gtk;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::prelude::*;
use relm4::typed_view::list::{RelmListItem, TypedListView};
use tracing::debug;

use crate::message_text::{display_sender, display_subject, format_timestamp};

#[derive(Debug, Clone)]
pub struct MessageRow {
    pub uid: u32,
    subject: String,
    sender: String,
    date: String,
    timestamp: Option<i64>,
    unread: bool,
    flagged: bool,
    has_attach: bool,
}

impl MessageRow {
    pub fn from_record(record: &MessageRecord, now: &glib::DateTime) -> Self {
        let flags = bits_to_flags(record.flags);
        let timestamp = record.date_sent.or(record.date_recv);
        Self {
            uid: record.uid,
            subject: display_subject(&record.subject),
            sender: display_sender(record),
            date: timestamp
                .map(|timestamp| format_timestamp(timestamp, now))
                .unwrap_or_default(),
            timestamp,
            unread: !flags.seen,
            flagged: flags.flagged,
            has_attach: record.has_attach,
        }
    }

    fn same_ui(&self, other: &Self) -> bool {
        self.uid == other.uid
            && self.subject == other.subject
            && self.sender == other.sender
            && self.date == other.date
            && self.unread == other.unread
            && self.flagged == other.flagged
            && self.has_attach == other.has_attach
    }
}

impl Ord for MessageRow {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .timestamp
            .cmp(&self.timestamp)
            .then_with(|| other.uid.cmp(&self.uid))
    }
}

impl PartialOrd for MessageRow {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for MessageRow {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for MessageRow {}

pub struct MessageRowWidgets {
    unread_dot: gtk::Label,
    sender: gtk::Label,
    subject: gtk::Label,
    date: gtk::Label,
    attachment: gtk::Image,
    flagged: gtk::Image,
}

impl RelmListItem for MessageRow {
    type Root = gtk::Box;
    type Widgets = MessageRowWidgets;

    fn setup(_item: &gtk::ListItem) -> (Self::Root, Self::Widgets) {
        relm4::view! {
            root = gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                add_css_class: "message-row",

                #[name = "unread_dot"]
                gtk::Label {
                    set_label: "•",
                    set_valign: gtk::Align::Center,
                    add_css_class: "message-row-unread-dot",
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 2,
                    set_hexpand: true,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 8,

                        #[name = "sender"]
                        gtk::Label {
                            set_xalign: 0.0,
                            set_hexpand: true,
                            set_ellipsize: gtk::pango::EllipsizeMode::End,
                            add_css_class: "message-row-sender",
                        },

                        #[name = "date"]
                        gtk::Label {
                            set_xalign: 1.0,
                            add_css_class: "message-row-date",
                        },
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 6,

                        #[name = "subject"]
                        gtk::Label {
                            set_xalign: 0.0,
                            set_hexpand: true,
                            set_ellipsize: gtk::pango::EllipsizeMode::End,
                            add_css_class: "message-row-subject",
                        },

                        #[name = "attachment"]
                        gtk::Image {
                            set_icon_name: Some("mail-attachment-symbolic"),
                            set_valign: gtk::Align::Center,
                            add_css_class: "dim-label",
                        },

                        #[name = "flagged"]
                        gtk::Image {
                            set_icon_name: Some("starred-symbolic"),
                            set_valign: gtk::Align::Center,
                            add_css_class: "dim-label",
                        },
                    },
                },
            }
        }

        let widgets = MessageRowWidgets {
            unread_dot,
            sender,
            subject,
            date,
            attachment,
            flagged,
        };

        (root, widgets)
    }

    fn bind(&mut self, widgets: &mut Self::Widgets, root: &mut Self::Root) {
        widgets.unread_dot.set_visible(self.unread);
        widgets.sender.set_label(&self.sender);
        widgets.date.set_label(&self.date);
        widgets.subject.set_label(&self.subject);
        widgets.attachment.set_visible(self.has_attach);
        widgets.flagged.set_visible(self.flagged);

        if self.unread {
            root.add_css_class("unread");
        } else {
            root.remove_css_class("unread");
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ListState {
    Idle,
    Loading,
    Empty,
    Ready,
    Error(String),
}

impl ListState {
    fn page_name(&self) -> &'static str {
        match self {
            Self::Idle | Self::Empty => "empty",
            Self::Loading => "loading",
            Self::Ready => "messages",
            Self::Error(_) => "error",
        }
    }
}

#[derive(Debug)]
pub enum MessageListMsg {
    Load {
        folder_id: i64,
    },
    Loaded {
        folder_id: i64,
        seq: u64,
        result: Result<Vec<MessageRow>, String>,
    },
    Select {
        uid: u32,
    },
    Retry,
    SelectionChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageListOutput {
    Selected { folder_id: i64, uid: u32 },
}

pub struct MessageList {
    store: Arc<Store>,
    list: TypedListView<MessageRow, gtk::SingleSelection>,
    folder_id: Option<i64>,
    selected_uid: Option<u32>,
    pending_select: Option<u32>,
    restoring: bool,
    load_seq: u64,
    state: ListState,
}

#[relm4::component(pub)]
impl SimpleComponent for MessageList {
    type Init = Arc<Store>;
    type Input = MessageListMsg;
    type Output = MessageListOutput;

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
                list_view -> gtk::ListView {
                    add_css_class: "message-list",
                },
            },

            add_named[Some("loading")] = &adw::StatusPage {
                set_title: "Loading messages…",

                #[wrap(Some)]
                set_child = &adw::Spinner {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                },
            },

            add_named[Some("empty")] = &adw::StatusPage {
                set_icon_name: Some("mail-symbolic"),
                #[watch]
                set_title: model.empty_title(),
                #[watch]
                set_description: Some(model.empty_description()),
            },

            add_named[Some("error")] = &adw::StatusPage {
                set_icon_name: Some("dialog-warning-symbolic"),
                set_title: "Couldn't load messages",
                #[watch]
                set_description: Some(model.error_message()),

                #[wrap(Some)]
                set_child = &gtk::Button {
                    set_label: "Try Again",
                    set_halign: gtk::Align::Center,
                    add_css_class: "pill",

                    connect_clicked[sender] => move |_| {
                        sender.input(MessageListMsg::Retry);
                    },
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
        let list: TypedListView<MessageRow, gtk::SingleSelection> = TypedListView::with_sorting();
        list.selection_model.connect_selection_changed({
            let sender = sender.clone();
            move |_, _, _| sender.input(MessageListMsg::SelectionChanged)
        });

        let model = MessageList {
            store,
            list,
            folder_id: None,
            selected_uid: None,
            pending_select: None,
            restoring: false,
            load_seq: 0,
            state: ListState::Idle,
        };
        let list_view = &model.list.view;
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            MessageListMsg::Load { folder_id } => {
                let switched = self.folder_id != Some(folder_id);
                if switched {
                    self.selected_uid = None;
                    self.list.clear();
                    self.state = ListState::Loading;
                } else if matches!(self.state, ListState::Idle | ListState::Error(_)) {
                    self.state = ListState::Loading;
                }
                self.folder_id = Some(folder_id);
                self.load_seq = self.load_seq.wrapping_add(1);
                let seq = self.load_seq;
                let store = self.store.clone();
                let load_sender = sender.clone();
                sender.oneshot_command(async move {
                    let result = load_rows(&store, folder_id).await;
                    load_sender.input(MessageListMsg::Loaded {
                        folder_id,
                        seq,
                        result,
                    });
                });
            }
            MessageListMsg::Loaded {
                folder_id,
                seq,
                result,
            } => {
                if self.folder_id != Some(folder_id) || seq != self.load_seq {
                    return;
                }
                self.restoring = true;
                match result {
                    Ok(rows) if rows.is_empty() => {
                        self.selected_uid = None;
                        self.list.clear();
                        self.state = ListState::Empty;
                    }
                    Ok(rows) => {
                        if self.state == ListState::Ready {
                            self.apply_update(rows);
                        } else {
                            self.list.clear();
                            self.list.extend_from_iter(rows);
                        }
                        self.state = ListState::Ready;
                        self.restore_selection();
                    }
                    Err(detail) => {
                        debug!(folder_id, detail, "failed to load messages");
                        self.selected_uid = None;
                        self.list.clear();
                        self.state = ListState::Error(detail);
                    }
                }
                let reemit = self.selected_uid.is_none() && self.state == ListState::Ready;
                self.restoring = false;
                if reemit {
                    sender.input(MessageListMsg::SelectionChanged);
                }
                if let Some(uid) = self.pending_select.take()
                    && self.state == ListState::Ready
                {
                    self.select_uid(uid);
                }
            }
            MessageListMsg::Select { uid } => {
                self.selected_uid = None;
                if self.state == ListState::Ready {
                    self.select_uid(uid);
                } else {
                    self.pending_select = Some(uid);
                }
            }
            MessageListMsg::Retry => {
                if let Some(folder_id) = self.folder_id {
                    sender.input(MessageListMsg::Load { folder_id });
                }
            }
            MessageListMsg::SelectionChanged => {
                if self.restoring {
                    return;
                }
                let Some(folder_id) = self.folder_id else {
                    return;
                };
                let position = self.list.selection_model.selected();
                let Some(item) = self.list.get_visible(position) else {
                    self.selected_uid = None;
                    return;
                };
                let uid = item.borrow().uid;
                if self.selected_uid == Some(uid) {
                    return;
                }
                self.selected_uid = Some(uid);
                let _ = sender.output(MessageListOutput::Selected { folder_id, uid });
            }
        }
    }
}

impl MessageList {
    fn restore_selection(&mut self) {
        let Some(uid) = self.selected_uid else {
            return;
        };
        if let Some(position) = self.visible_position(uid) {
            self.list.selection_model.set_selected(position);
        } else {
            self.selected_uid = None;
        }
    }

    fn select_uid(&mut self, uid: u32) {
        if let Some(position) = self.visible_position(uid) {
            self.selected_uid = None;
            self.list.selection_model.set_selected(position);
        }
    }

    fn visible_position(&self, uid: u32) -> Option<u32> {
        let mut position = 0;
        while let Some(item) = self.list.get_visible(position) {
            if item.borrow().uid == uid {
                return Some(position);
            }
            position += 1;
        }
        None
    }

    fn apply_update(&mut self, rows: Vec<MessageRow>) {
        let current = self.rows_snapshot();
        let plan = plan_update(&current, &rows);
        for uid in plan.removes {
            if let Some(position) = self.list.find(|row| row.uid == uid) {
                self.list.remove(position);
            }
        }
        for row in plan.inserts {
            self.list.append(row);
        }
    }

    fn rows_snapshot(&self) -> Vec<MessageRow> {
        (0..self.list.len())
            .filter_map(|position| self.list.get(position))
            .map(|item| item.borrow().clone())
            .collect()
    }

    fn empty_title(&self) -> &'static str {
        if self.folder_id.is_some() {
            "No messages"
        } else {
            "No folder selected"
        }
    }

    fn empty_description(&self) -> &'static str {
        if self.folder_id.is_some() {
            "This folder is empty."
        } else {
            "Choose a folder from the sidebar to read your mail."
        }
    }

    fn error_message(&self) -> &str {
        match &self.state {
            ListState::Error(detail) => detail,
            _ => "Something went wrong while syncing this folder.",
        }
    }
}

async fn load_rows(store: &Store, folder_id: i64) -> Result<Vec<MessageRow>, String> {
    let messages = store
        .messages(folder_id)
        .await
        .map_err(|err| err.to_string())?;
    let now = glib::DateTime::now_local()
        .or_else(|_| glib::DateTime::from_unix_utc(0))
        .expect("the Unix epoch is always a valid timestamp");
    Ok(messages
        .iter()
        .map(|message| MessageRow::from_record(message, &now))
        .collect())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct UpdatePlan {
    removes: Vec<u32>,
    inserts: Vec<MessageRow>,
}

fn plan_update(current: &[MessageRow], next: &[MessageRow]) -> UpdatePlan {
    let current_by_uid: HashMap<u32, &MessageRow> =
        current.iter().map(|row| (row.uid, row)).collect();
    let next_by_uid: HashMap<u32, &MessageRow> = next.iter().map(|row| (row.uid, row)).collect();
    let removes = current
        .iter()
        .filter(|row| next_by_uid.get(&row.uid).is_none_or(|next| !row.same_ui(next)))
        .map(|row| row.uid)
        .collect();
    let inserts = next
        .iter()
        .filter(|row| current_by_uid.get(&row.uid).is_none_or(|current| !current.same_ui(row)))
        .cloned()
        .collect();
    UpdatePlan { removes, inserts }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::store::{BodyState, FLAG_FLAGGED, FLAG_SEEN};

    fn record(uid: u32, flags: i64, has_attach: bool) -> MessageRecord {
        MessageRecord {
            id: Some(i64::from(uid)),
            folder_id: 1,
            uid,
            modseq: None,
            message_id: None,
            thread_id: None,
            subject: "Hello".to_string(),
            from_addr: Some("ada@lovelace.dev".to_string()),
            from_name: Some("Ada Lovelace".to_string()),
            to_addrs: None,
            cc_addrs: None,
            date_sent: Some(1_700_000_000),
            date_recv: None,
            in_reply_to: None,
            refs: None,
            flags,
            has_attach,
            size: None,
            structure: None,
            raw_path: None,
            body_state: BodyState::None,
        }
    }

    fn utc(timestamp: i64) -> glib::DateTime {
        glib::DateTime::from_unix_utc(timestamp).expect("valid timestamp")
    }

    #[test]
    fn from_record_maps_flags() {
        let now = utc(1_700_000_000);

        let unread = MessageRow::from_record(&record(1, 0, true), &now);
        assert!(unread.unread);
        assert!(!unread.flagged);
        assert!(unread.has_attach);

        let read_flagged =
            MessageRow::from_record(&record(2, FLAG_SEEN | FLAG_FLAGGED, false), &now);
        assert!(!read_flagged.unread);
        assert!(read_flagged.flagged);
        assert!(!read_flagged.has_attach);
    }

    #[test]
    fn rows_sort_newest_first_and_tie_break_on_uid() {
        let now = utc(1_700_000_000);
        let mut older = record(1, 0, false);
        older.date_sent = Some(1_699_000_000);
        let mut newer = record(2, 0, false);
        newer.date_sent = Some(1_700_000_000);
        let newer_high_uid = record(9, 0, false);

        let older = MessageRow::from_record(&older, &now);
        let newer = MessageRow::from_record(&newer, &now);
        let newer_high_uid = MessageRow::from_record(&newer_high_uid, &now);

        let mut rows = vec![older.clone(), newer.clone(), newer_high_uid.clone()];
        rows.sort();

        assert_eq!(rows, vec![newer_high_uid, newer, older]);
    }

    #[test]
    fn missing_date_sorts_last() {
        let now = utc(1_700_000_000);
        let mut undated = record(1, 0, false);
        undated.date_sent = None;
        undated.date_recv = None;
        let dated = record(2, 0, false);

        let undated = MessageRow::from_record(&undated, &now);
        let dated = MessageRow::from_record(&dated, &now);

        assert!(dated < undated);
        assert_eq!(undated.date, "");
    }

    #[test]
    fn list_state_maps_to_stack_page() {
        assert_eq!(ListState::Idle.page_name(), "empty");
        assert_eq!(ListState::Empty.page_name(), "empty");
        assert_eq!(ListState::Loading.page_name(), "loading");
        assert_eq!(ListState::Ready.page_name(), "messages");
        assert_eq!(ListState::Error("boom".to_string()).page_name(), "error");
    }

    #[test]
    fn plan_update_is_a_noop_for_identical_rows() {
        let now = utc(1_700_000_000);
        let rows = vec![
            MessageRow::from_record(&record(2, FLAG_SEEN, false), &now),
            MessageRow::from_record(&record(1, 0, false), &now),
        ];
        let plan = plan_update(&rows, &rows);
        assert!(plan.removes.is_empty());
        assert!(plan.inserts.is_empty());
    }

    #[test]
    fn plan_update_drops_vanished_and_adds_new_rows() {
        let now = utc(1_700_000_000);
        let current = vec![MessageRow::from_record(&record(3, 0, false), &now)];
        let next = vec![MessageRow::from_record(&record(4, 0, false), &now)];
        let plan = plan_update(&current, &next);
        assert_eq!(plan.removes, vec![3]);
        assert_eq!(plan.inserts.len(), 1);
        assert_eq!(plan.inserts[0].uid, 4);
    }

    #[test]
    fn plan_update_replaces_rows_whose_ui_changed() {
        let now = utc(1_700_000_000);
        let current = vec![MessageRow::from_record(&record(1, 0, false), &now)];
        let mut seen = record(1, 0, false);
        seen.flags = FLAG_SEEN;
        let next = vec![MessageRow::from_record(&seen, &now)];
        let plan = plan_update(&current, &next);
        assert_eq!(plan.removes, vec![1]);
        assert_eq!(plan.inserts.len(), 1);
        assert!(!plan.inserts[0].unread);
    }

    #[test]
    fn plan_update_catches_content_changes_beyond_ordering() {
        let now = utc(1_700_000_000);
        let current = vec![MessageRow::from_record(&record(1, 0, false), &now)];
        let mut renamed = record(1, 0, false);
        renamed.subject = "Different".to_string();
        let next = vec![MessageRow::from_record(&renamed, &now)];
        let plan = plan_update(&current, &next);
        assert_eq!(plan.removes, vec![1]);
        assert_eq!(plan.inserts.len(), 1);
        assert_eq!(plan.inserts[0].subject, "Different");
    }
}
