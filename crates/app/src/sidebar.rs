// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Account/folder sidebar.
//!
//! The folder tree factory renders every account in the local store as a
//! section header followed by its folders as selectable rows carrying
//! unread badges. Row activation is emitted upwards so the shell can
//! switch the mailbox view.

use std::sync::Arc;

use mail_core::store::{AccountRecord, FolderRecord, SpecialUse, Store};
use relm4::adw;
use relm4::factory::{FactoryComponent, FactorySender, FactoryVecDeque};
use relm4::gtk::prelude::*;
use relm4::prelude::*;
use tracing::debug;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderKey {
    pub account_id: i64,
    pub folder_id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarRow {
    Account {
        name: String,
        email: String,
    },
    Folder {
        key: FolderKey,
        title: String,
        unread: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderTreeOutput {
    Selected { key: FolderKey, title: String },
}

#[derive(Debug)]
pub struct FolderRow {
    kind: SidebarRow,
    unread: u32,
}

#[relm4::factory(pub)]
impl FactoryComponent for FolderRow {
    type Init = SidebarRow;
    type Input = ();
    type Output = FolderTreeOutput;
    type CommandOutput = ();
    type ParentWidget = gtk::ListBox;

    view! {
        root = gtk::ListBoxRow {
            set_activatable: !self.is_header(),
            set_selectable: !self.is_header(),
            connect_activate => (),

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                set_margin_top: 6,
                set_margin_bottom: 6,
                set_margin_start: 12,
                set_margin_end: 12,

                #[name(title)]
                gtk::Label {
                    set_xalign: 0.0,
                    set_hexpand: true,
                    set_ellipsize: gtk::pango::EllipsizeMode::End,
                    set_label: &self.title(),
                    set_css_classes: &self.title_classes(),
                },

                #[name(badge)]
                gtk::Label {
                    set_valign: gtk::Align::Center,
                    set_css_classes: &["unread-badge"],
                    #[watch]
                    set_label: &self.badge_text(),
                    #[watch]
                    set_visible: self.unread > 0,
                },
            },
        }
    }

    fn init_model(init: Self::Init, _index: &DynamicIndex, _sender: FactorySender<Self>) -> Self {
        let unread = match &init {
            SidebarRow::Folder { unread, .. } => *unread,
            SidebarRow::Account { .. } => 0,
        };
        Self { kind: init, unread }
    }

    fn update(&mut self, _msg: Self::Input, sender: FactorySender<Self>) {
        if let SidebarRow::Folder { key, title, .. } = &self.kind {
            let _ = sender.output(FolderTreeOutput::Selected {
                key: key.clone(),
                title: title.clone(),
            });
        }
    }
}

impl FolderRow {
    fn is_header(&self) -> bool {
        matches!(self.kind, SidebarRow::Account { .. })
    }

    fn title(&self) -> String {
        match &self.kind {
            SidebarRow::Account { name, .. } => name.clone(),
            SidebarRow::Folder { title, .. } => title.clone(),
        }
    }

    fn title_classes(&self) -> Vec<&'static str> {
        if self.is_header() {
            vec![relm4::css::HEADING]
        } else {
            Vec::new()
        }
    }

    fn badge_text(&self) -> String {
        badge_text(self.unread)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SidebarState {
    Loading,
    Empty,
    Ready,
    Error(String),
}

impl SidebarState {
    fn page_name(&self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Empty => "empty",
            Self::Ready => "folders",
            Self::Error(_) => "error",
        }
    }
}

#[derive(Debug)]
pub enum FolderTreeMsg {
    Reload,
    Loaded(Result<Vec<SidebarRow>, String>),
    Selected { key: FolderKey, title: String },
}

pub struct FolderTree {
    rows: FactoryVecDeque<FolderRow>,
    store: Arc<Store>,
    state: SidebarState,
}

#[relm4::component(pub)]
impl SimpleComponent for FolderTree {
    type Init = Arc<Store>;
    type Input = FolderTreeMsg;
    type Output = FolderTreeOutput;

    view! {
        #[root]
        adw::ViewStack {
            set_vexpand: true,
            set_hexpand: true,

            add_named[Some("folders")] = &gtk::ScrolledWindow {
                set_vexpand: true,
                set_hexpand: true,
                set_policy: (gtk::PolicyType::Never, gtk::PolicyType::Automatic),

                #[local_ref]
                rows -> gtk::ListBox {
                    set_vexpand: true,
                    set_hexpand: true,
                    add_css_class: relm4::css::NAVIGATION_SIDEBAR,
                    set_selection_mode: gtk::SelectionMode::Single,
                },
            },

            add_named[Some("loading")] = &adw::StatusPage {
                set_title: "Loading accounts…",

                #[wrap(Some)]
                set_child = &adw::Spinner {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                },
            },

            add_named[Some("empty")] = &adw::StatusPage {
                set_icon_name: Some("mail-symbolic"),
                set_title: "No accounts",
                set_description: Some("Add a mail account in Settings to get started."),
            },

            add_named[Some("error")] = &adw::StatusPage {
                set_icon_name: Some("dialog-warning-symbolic"),
                set_title: "Couldn't load accounts",
                #[watch]
                set_description: Some(model.error_message()),

                #[wrap(Some)]
                set_child = &gtk::Button {
                    set_label: "Try Again",
                    set_halign: gtk::Align::Center,
                    add_css_class: "pill",

                    connect_clicked[sender] => move |_| {
                        sender.input(FolderTreeMsg::Reload);
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
        let rows =
            FactoryVecDeque::builder()
                .launch_default()
                .forward(sender.input_sender(), |msg| match msg {
                    FolderTreeOutput::Selected { key, title } => {
                        FolderTreeMsg::Selected { key, title }
                    }
                });
        let model = FolderTree {
            rows,
            store,
            state: SidebarState::Loading,
        };
        let rows = model.rows.widget();
        let widgets = view_output!();

        sender.input(FolderTreeMsg::Reload);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            FolderTreeMsg::Reload => {
                if self.state != SidebarState::Ready {
                    self.state = SidebarState::Loading;
                }
                let store = self.store.clone();
                let reload_sender = sender.clone();
                sender.oneshot_command(async move {
                    let result = load_rows(&store).await;
                    reload_sender.input(FolderTreeMsg::Loaded(result));
                });
            }
            FolderTreeMsg::Loaded(result) => match result {
                Ok(rows) if rows.is_empty() => {
                    self.state = SidebarState::Empty;
                }
                Ok(rows) => {
                    self.state = SidebarState::Ready;
                    let mut guard = self.rows.guard();
                    guard.clear();
                    for row in rows {
                        guard.push_back(row);
                    }
                }
                Err(detail) => {
                    debug!(detail, "failed to load accounts");
                    self.state = SidebarState::Error(detail);
                }
            },
            FolderTreeMsg::Selected { key, title } => {
                let _ = sender.output(FolderTreeOutput::Selected { key, title });
            }
        }
    }
}

impl FolderTree {
    fn error_message(&self) -> &str {
        match &self.state {
            SidebarState::Error(detail) => detail,
            _ => "Something went wrong while reading your accounts.",
        }
    }
}

async fn load_rows(store: &Store) -> Result<Vec<SidebarRow>, String> {
    let accounts = store.accounts().await.map_err(|err| err.to_string())?;
    let mut rows = Vec::new();
    for account in accounts {
        let account_id = account.id.unwrap_or_default();
        let folders = match store.folders(account_id).await {
            Ok(folders) => folders,
            Err(err) => {
                debug!("failed to load folders for account {account_id}: {err}");
                continue;
            }
        };
        rows.extend(account_rows(&account, folders));
    }
    Ok(rows)
}

fn account_rows(account: &AccountRecord, mut folders: Vec<FolderRecord>) -> Vec<SidebarRow> {
    let account_id = account.id.unwrap_or_default();
    let name = account
        .display_name
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| account.email.clone());
    let mut rows = Vec::with_capacity(folders.len() + 1);
    rows.push(SidebarRow::Account {
        name: name.clone(),
        email: account.email.clone(),
    });
    folders.sort_by(folder_order);
    for folder in folders {
        rows.push(SidebarRow::Folder {
            key: FolderKey {
                account_id,
                folder_id: folder.id.unwrap_or_default(),
                name: folder.name.clone(),
            },
            title: folder
                .display_name
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or(folder.name),
            unread: folder.unread_count.max(0) as u32,
        });
    }
    rows
}

fn folder_order(a: &FolderRecord, b: &FolderRecord) -> std::cmp::Ordering {
    role_rank(a.special_use)
        .cmp(&role_rank(b.special_use))
        .then_with(|| a.name.cmp(&b.name))
}

fn role_rank(role: Option<SpecialUse>) -> u8 {
    match role {
        Some(SpecialUse::Inbox) => 0,
        Some(SpecialUse::Drafts) => 1,
        Some(SpecialUse::Sent) => 2,
        Some(SpecialUse::Archive) => 3,
        Some(SpecialUse::Junk) => 4,
        Some(SpecialUse::Trash) => 5,
        Some(SpecialUse::Important) => 6,
        Some(SpecialUse::All) => 7,
        Some(SpecialUse::Flagged) => 8,
        None => 9,
    }
}

fn badge_text(unread: u32) -> String {
    if unread > 99 {
        "99+".to_string()
    } else {
        unread.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::store::{AccountSource, AuthKind};

    fn account() -> AccountRecord {
        AccountRecord {
            id: Some(1),
            source: AccountSource::Eds,
            external_id: "source-1".to_string(),
            email: "ada@lovelace.dev".to_string(),
            display_name: None,
            imap_host: None,
            imap_port: None,
            imap_security: None,
            smtp_host: None,
            smtp_port: None,
            smtp_security: None,
            auth_kind: AuthKind::Password,
            username: None,
        }
    }

    fn folder(name: &str, role: Option<SpecialUse>, unread: i64) -> FolderRecord {
        FolderRecord {
            id: None,
            account_id: 1,
            name: name.to_string(),
            display_name: None,
            special_use: role,
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            unread_count: unread,
            total_count: 0,
            subscribed: true,
        }
    }

    #[test]
    fn badge_text_caps_at_99_plus() {
        assert_eq!(badge_text(0), "0");
        assert_eq!(badge_text(42), "42");
        assert_eq!(badge_text(99), "99");
        assert_eq!(badge_text(100), "99+");
    }

    #[test]
    fn account_rows_header_uses_email_without_display_name() {
        let rows = account_rows(&account(), Vec::new());
        assert_eq!(
            rows,
            vec![SidebarRow::Account {
                name: "ada@lovelace.dev".to_string(),
                email: "ada@lovelace.dev".to_string(),
            }]
        );
    }

    #[test]
    fn account_rows_sort_folders_by_role_then_name() {
        let folders = vec![
            folder("Work/Receipts", None, 2),
            folder("Trash", Some(SpecialUse::Trash), 3),
            folder("Archive", Some(SpecialUse::Archive), 0),
            folder("INBOX", Some(SpecialUse::Inbox), 7),
        ];
        let rows = account_rows(&account(), folders);
        assert_eq!(
            rows,
            vec![
                SidebarRow::Account {
                    name: "ada@lovelace.dev".to_string(),
                    email: "ada@lovelace.dev".to_string(),
                },
                SidebarRow::Folder {
                    key: FolderKey {
                        account_id: 1,
                        folder_id: 0,
                        name: "INBOX".to_string(),
                    },
                    title: "INBOX".to_string(),
                    unread: 7,
                },
                SidebarRow::Folder {
                    key: FolderKey {
                        account_id: 1,
                        folder_id: 0,
                        name: "Archive".to_string(),
                    },
                    title: "Archive".to_string(),
                    unread: 0,
                },
                SidebarRow::Folder {
                    key: FolderKey {
                        account_id: 1,
                        folder_id: 0,
                        name: "Trash".to_string(),
                    },
                    title: "Trash".to_string(),
                    unread: 3,
                },
                SidebarRow::Folder {
                    key: FolderKey {
                        account_id: 1,
                        folder_id: 0,
                        name: "Work/Receipts".to_string(),
                    },
                    title: "Work/Receipts".to_string(),
                    unread: 2,
                },
            ]
        );
    }

    #[test]
    fn sidebar_state_maps_to_stack_page() {
        assert_eq!(SidebarState::Loading.page_name(), "loading");
        assert_eq!(SidebarState::Empty.page_name(), "empty");
        assert_eq!(SidebarState::Ready.page_name(), "folders");
        assert_eq!(SidebarState::Error("boom".to_string()).page_name(), "error");
    }
}
