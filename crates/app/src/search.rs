// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Full-text search over the local store.
//!
//! Renders the ranked FTS5 hits with a context snippet. Selecting a row
//! emits the folder and UID of the underlying message so the shell can
//! open it in the reading pane, no matter which folder it lives in.

use std::sync::Arc;

use mail_core::store::{SearchHit, Store, fts_query};
use relm4::adw;
use relm4::gtk;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::prelude::*;
use relm4::typed_view::list::{RelmListItem, TypedListView};
use tracing::debug;

use crate::message_text::{display_sender, display_subject, format_timestamp};

const SEARCH_LIMIT: i64 = 200;

#[derive(Debug, Clone)]
pub struct SearchRow {
    pub folder_id: i64,
    pub uid: u32,
    subject: String,
    sender: String,
    snippet: String,
    date: String,
}

impl SearchRow {
    fn from_hit(hit: &SearchHit, now: &glib::DateTime) -> Self {
        let record = &hit.message;
        let subject = display_subject(&record.subject);
        let timestamp = record.date_sent.or(record.date_recv);
        Self {
            folder_id: record.folder_id,
            uid: record.uid,
            sender: display_sender(record),
            snippet: snippet_text(&hit.snippet, &subject),
            subject,
            date: timestamp
                .map(|timestamp| format_timestamp(timestamp, now))
                .unwrap_or_default(),
        }
    }
}

pub struct SearchRowWidgets {
    sender: gtk::Label,
    date: gtk::Label,
    subject: gtk::Label,
    snippet: gtk::Label,
}

impl RelmListItem for SearchRow {
    type Root = gtk::Box;
    type Widgets = SearchRowWidgets;

    fn setup(_item: &gtk::ListItem) -> (Self::Root, Self::Widgets) {
        relm4::view! {
            root = gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 2,
                add_css_class: "message-row",

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

                #[name = "subject"]
                gtk::Label {
                    set_xalign: 0.0,
                    set_ellipsize: gtk::pango::EllipsizeMode::End,
                    add_css_class: "message-row-subject",
                },

                #[name = "snippet"]
                gtk::Label {
                    set_xalign: 0.0,
                    set_ellipsize: gtk::pango::EllipsizeMode::End,
                    add_css_class: "search-row-snippet",
                },
            }
        }

        let widgets = SearchRowWidgets {
            sender,
            date,
            subject,
            snippet,
        };

        (root, widgets)
    }

    fn bind(&mut self, widgets: &mut Self::Widgets, _root: &mut Self::Root) {
        widgets.sender.set_label(&self.sender);
        widgets.date.set_label(&self.date);
        widgets.subject.set_label(&self.subject);
        widgets.snippet.set_label(&self.snippet);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SearchState {
    Idle,
    Loading,
    Empty,
    Ready,
    Error(String),
}

impl SearchState {
    fn page_name(&self) -> &'static str {
        match self {
            Self::Idle | Self::Empty => "empty",
            Self::Loading => "loading",
            Self::Ready => "results",
            Self::Error(_) => "error",
        }
    }
}

#[derive(Debug)]
pub enum SearchMsg {
    Search {
        query: String,
    },
    Loaded {
        query: String,
        result: Result<Vec<SearchRow>, String>,
    },
    Clear,
    Retry,
    SelectionChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchOutput {
    Selected { folder_id: i64, uid: u32 },
}

pub struct SearchView {
    store: Arc<Store>,
    list: TypedListView<SearchRow, gtk::SingleSelection>,
    query: String,
    selected: Option<(i64, u32)>,
    restoring: bool,
    state: SearchState,
}

#[relm4::component(pub)]
impl SimpleComponent for SearchView {
    type Init = Arc<Store>;
    type Input = SearchMsg;
    type Output = SearchOutput;

    view! {
        #[root]
        adw::ViewStack {
            set_vexpand: true,
            set_hexpand: true,

            add_named[Some("results")] = &gtk::ScrolledWindow {
                set_vexpand: true,
                set_hexpand: true,
                set_policy: (gtk::PolicyType::Never, gtk::PolicyType::Automatic),

                #[local_ref]
                list_view -> gtk::ListView {
                    add_css_class: "message-list",
                },
            },

            add_named[Some("loading")] = &adw::StatusPage {
                set_title: "Searching…",

                #[wrap(Some)]
                set_child = &adw::Spinner {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                },
            },

            add_named[Some("empty")] = &adw::StatusPage {
                set_icon_name: Some("system-search-symbolic"),
                #[watch]
                set_title: empty_title(&model.query),
                #[watch]
                set_description: Some(empty_description(&model.query).as_str()),
            },

            add_named[Some("error")] = &adw::StatusPage {
                set_icon_name: Some("dialog-warning-symbolic"),
                set_title: "Couldn't search your mail",
                #[watch]
                set_description: Some(model.error_message()),

                #[wrap(Some)]
                set_child = &gtk::Button {
                    set_label: "Try Again",
                    set_halign: gtk::Align::Center,
                    add_css_class: "pill",

                    connect_clicked[sender] => move |_| {
                        sender.input(SearchMsg::Retry);
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
        let list: TypedListView<SearchRow, gtk::SingleSelection> = TypedListView::new();
        list.selection_model.set_autoselect(false);
        list.selection_model.connect_selection_changed({
            let sender = sender.clone();
            move |_, _, _| sender.input(SearchMsg::SelectionChanged)
        });

        let model = SearchView {
            store,
            list,
            query: String::new(),
            selected: None,
            restoring: false,
            state: SearchState::Idle,
        };
        let list_view = &model.list.view;
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            SearchMsg::Search { query } => {
                let query = query.trim().to_string();
                if query.is_empty() {
                    self.reset();
                    return;
                }
                if query == self.query && self.state == SearchState::Ready {
                    return;
                }
                self.query = query.clone();
                self.selected = None;
                self.state = SearchState::Loading;
                self.list.clear();
                let store = self.store.clone();
                let load_sender = sender.clone();
                sender.oneshot_command(async move {
                    let result = search_rows(&store, &query).await;
                    load_sender.input(SearchMsg::Loaded { query, result });
                });
            }
            SearchMsg::Loaded { query, result } => {
                if self.query != query {
                    return;
                }
                self.restoring = true;
                self.list.clear();
                match result {
                    Ok(rows) if rows.is_empty() => {
                        self.state = SearchState::Empty;
                    }
                    Ok(rows) => {
                        self.state = SearchState::Ready;
                        self.list.extend_from_iter(rows);
                    }
                    Err(detail) => {
                        debug!(query, detail, "failed to search messages");
                        self.state = SearchState::Error(detail);
                    }
                }
                self.restoring = false;
            }
            SearchMsg::Clear => self.reset(),
            SearchMsg::Retry => {
                if !self.query.is_empty() {
                    sender.input(SearchMsg::Search {
                        query: self.query.clone(),
                    });
                }
            }
            SearchMsg::SelectionChanged => {
                if self.restoring {
                    return;
                }
                let position = self.list.selection_model.selected();
                let Some(item) = self.list.get_visible(position) else {
                    self.selected = None;
                    return;
                };
                let item = item.borrow();
                let key = (item.folder_id, item.uid);
                if self.selected == Some(key) {
                    return;
                }
                self.selected = Some(key);
                let _ = sender.output(SearchOutput::Selected {
                    folder_id: item.folder_id,
                    uid: item.uid,
                });
            }
        }
    }
}

impl SearchView {
    fn reset(&mut self) {
        self.query.clear();
        self.selected = None;
        self.state = SearchState::Idle;
        self.list.clear();
    }

    fn error_message(&self) -> &str {
        match &self.state {
            SearchState::Error(detail) => detail,
            _ => "Something went wrong while searching.",
        }
    }
}

async fn search_rows(store: &Store, query: &str) -> Result<Vec<SearchRow>, String> {
    let hits = store
        .search(&fts_query(query), SEARCH_LIMIT)
        .await
        .map_err(|err| err.to_string())?;
    let now = glib::DateTime::now_local()
        .or_else(|_| glib::DateTime::from_unix_utc(0))
        .expect("the Unix epoch is always a valid timestamp");
    Ok(hits
        .iter()
        .map(|hit| SearchRow::from_hit(hit, &now))
        .collect())
}

fn empty_title(query: &str) -> &'static str {
    if query.is_empty() {
        "Search your mail"
    } else {
        "No results"
    }
}

fn empty_description(query: &str) -> String {
    if query.is_empty() {
        "Search subjects, senders and message bodies.".to_string()
    } else {
        format!("No messages match “{}”.", query)
    }
}

fn snippet_text(snippet: &str, fallback: &str) -> String {
    let collapsed = collapse_whitespace(snippet);
    if collapsed.is_empty() {
        fallback.to_string()
    } else {
        collapsed
    }
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::store::{BodyState, MessageRecord};

    fn hit(snippet: &str) -> SearchHit {
        SearchHit {
            message: MessageRecord {
                id: Some(1),
                folder_id: 4,
                uid: 12,
                modseq: None,
                message_id: None,
                thread_id: None,
                subject: "Quarterly report".to_string(),
                from_addr: Some("ada@lovelace.dev".to_string()),
                from_name: Some("Ada Lovelace".to_string()),
                to_addrs: None,
                cc_addrs: None,
                date_sent: Some(1_700_000_000),
                date_recv: None,
                in_reply_to: None,
                refs: None,
                flags: 0,
                has_attach: false,
                size: None,
                structure: None,
                raw_path: None,
                body_state: BodyState::None,
            },
            snippet: snippet.to_string(),
        }
    }

    fn utc(timestamp: i64) -> glib::DateTime {
        glib::DateTime::from_unix_utc(timestamp).expect("valid timestamp")
    }

    #[test]
    fn row_carries_folder_and_uid_for_jumping() {
        let row = SearchRow::from_hit(&hit("the [report] is ready"), &utc(1_700_000_000));
        assert_eq!(row.folder_id, 4);
        assert_eq!(row.uid, 12);
        assert_eq!(row.sender, "Ada Lovelace");
        assert_eq!(row.subject, "Quarterly report");
        assert_eq!(row.snippet, "the [report] is ready");
    }

    #[test]
    fn row_collapses_snippet_whitespace() {
        let row = SearchRow::from_hit(&hit("line one\n\tline   two"), &utc(1_700_000_000));
        assert_eq!(row.snippet, "line one line two");
    }

    #[test]
    fn row_falls_back_to_subject_without_snippet() {
        let row = SearchRow::from_hit(&hit("   "), &utc(1_700_000_000));
        assert_eq!(row.snippet, "Quarterly report");
    }

    #[test]
    fn empty_query_without_a_search_shows_the_prompt() {
        assert_eq!(empty_title(""), "Search your mail");
        assert_eq!(
            empty_description(""),
            "Search subjects, senders and message bodies."
        );
        assert_eq!(empty_title("report"), "No results");
        assert_eq!(empty_description("report"), "No messages match “report”.");
    }

    #[test]
    fn search_state_maps_to_stack_page() {
        assert_eq!(SearchState::Idle.page_name(), "empty");
        assert_eq!(SearchState::Empty.page_name(), "empty");
        assert_eq!(SearchState::Loading.page_name(), "loading");
        assert_eq!(SearchState::Ready.page_name(), "results");
        assert_eq!(SearchState::Error("boom".to_string()).page_name(), "error");
    }
}
