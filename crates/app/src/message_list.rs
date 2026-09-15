// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Message list.

use std::cmp::Ordering;
use std::sync::Arc;

use mail_core::store::{MessageRecord, Store, bits_to_flags};
use relm4::gtk;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::prelude::*;
use relm4::typed_view::list::{RelmListItem, TypedListView};
use tracing::debug;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

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

#[derive(Debug)]
pub enum MessageListMsg {
    Load {
        folder_id: i64,
    },
    Loaded {
        folder_id: i64,
        rows: Vec<MessageRow>,
    },
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
}

#[relm4::component(pub)]
impl SimpleComponent for MessageList {
    type Init = Arc<Store>;
    type Input = MessageListMsg;
    type Output = MessageListOutput;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vexpand: true,
            set_hexpand: true,
            set_policy: (gtk::PolicyType::Never, gtk::PolicyType::Automatic),

            #[local_ref]
            list_view -> gtk::ListView {
                add_css_class: "message-list",
            },
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
        };
        let list_view = &model.list.view;
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            MessageListMsg::Load { folder_id } => {
                self.folder_id = Some(folder_id);
                self.list.clear();
                let store = self.store.clone();
                let load_sender = sender.clone();
                sender.oneshot_command(async move {
                    let rows = load_rows(&store, folder_id).await;
                    load_sender.input(MessageListMsg::Loaded { folder_id, rows });
                });
            }
            MessageListMsg::Loaded { folder_id, rows } => {
                if self.folder_id != Some(folder_id) {
                    return;
                }
                self.list.clear();
                self.list.extend_from_iter(rows);
            }
            MessageListMsg::SelectionChanged => {
                let Some(folder_id) = self.folder_id else {
                    return;
                };
                let position = self.list.selection_model.selected();
                let Some(item) = self.list.get_visible(position) else {
                    return;
                };
                let uid = item.borrow().uid;
                let _ = sender.output(MessageListOutput::Selected { folder_id, uid });
            }
        }
    }
}

async fn load_rows(store: &Store, folder_id: i64) -> Vec<MessageRow> {
    let messages = match store.messages(folder_id).await {
        Ok(messages) => messages,
        Err(err) => {
            debug!("failed to load messages for folder {folder_id}: {err}");
            return Vec::new();
        }
    };
    let now = glib::DateTime::now_local()
        .or_else(|_| glib::DateTime::from_unix_utc(0))
        .expect("the Unix epoch is always a valid timestamp");
    messages
        .iter()
        .map(|message| MessageRow::from_record(message, &now))
        .collect()
}

fn display_subject(subject: &str) -> String {
    let subject = subject.trim();
    if subject.is_empty() {
        "(no subject)".to_string()
    } else {
        subject.to_string()
    }
}

fn display_sender(record: &MessageRecord) -> String {
    if let Some(name) = non_empty(record.from_name.as_deref()) {
        return name.to_string();
    }
    if let Some(address) = non_empty(record.from_addr.as_deref()) {
        return address.to_string();
    }
    "(unknown sender)".to_string()
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn format_timestamp(timestamp: i64, now: &glib::DateTime) -> String {
    match glib::DateTime::from_unix_local(timestamp) {
        Ok(date) => format_relative(&date, now),
        Err(_) => String::new(),
    }
}

fn format_relative(date: &glib::DateTime, now: &glib::DateTime) -> String {
    if date.year() == now.year()
        && date.month() == now.month()
        && date.day_of_month() == now.day_of_month()
    {
        format!("{:02}:{:02}", date.hour(), date.minute())
    } else if date.year() == now.year() {
        format!("{} {}", date.day_of_month(), month_name(date.month()))
    } else {
        format!(
            "{:04}-{:02}-{:02}",
            date.year(),
            date.month(),
            date.day_of_month()
        )
    }
}

fn month_name(month: i32) -> &'static str {
    let index = (month - 1).clamp(0, 11) as usize;
    MONTHS[index]
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
    fn display_subject_falls_back_when_blank() {
        assert_eq!(display_subject("  "), "(no subject)");
        assert_eq!(display_subject(" Meeting "), "Meeting");
    }

    #[test]
    fn display_sender_prefers_name_then_address() {
        let mut record = record(1, 0, false);
        assert_eq!(display_sender(&record), "Ada Lovelace");

        record.from_name = Some("   ".to_string());
        assert_eq!(display_sender(&record), "ada@lovelace.dev");

        record.from_addr = None;
        assert_eq!(display_sender(&record), "(unknown sender)");
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
    fn format_relative_picks_day_year_and_clock() {
        let now = utc(1_700_000_000);

        assert_eq!(format_relative(&utc(1_699_990_000), &now), "19:26");
        assert_eq!(format_relative(&utc(1_699_000_000), &now), "3 Nov");
        assert_eq!(format_relative(&utc(1_600_000_000), &now), "2020-09-13");
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
}
