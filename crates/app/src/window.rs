// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::cell::Cell;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use gtk::gio;
use mail_core::compose::ReplyMode;
use mail_core::envelope::FlagChange;
use mail_core::store::{FLAG_FLAGGED, FLAG_SEEN, FolderRecord, SpecialUse, Store};
use relm4::MessageBroker;
use relm4::actions::RelmAction;
use relm4::adw::prelude::*;
use relm4::gtk::glib;
use relm4::prelude::*;
use tracing::{debug, warn};

use crate::application::{About, Quit};
use crate::composer::{self, Composer, ComposerInit, ComposerMsg, ComposerOutput, MessageDraft};
use crate::config;
use crate::conversation::{AnchorState, Conversation, ConversationMsg, ConversationOutput};
use crate::message_list::{MessageList, MessageListMsg, MessageListOutput};
use crate::notify::{self, MailNotice};
use crate::search::{SearchMsg, SearchOutput, SearchView};
use crate::sidebar::{FolderKey, FolderTree, FolderTreeMsg, FolderTreeOutput};
use crate::sync::{MessageAction, SyncService, SyncServiceMsg, SyncServiceOutput};

const COLLAPSE_FOLDERS_WIDTH: f64 = 860.0;
const COLLAPSE_READING_WIDTH: f64 = 500.0;

pub static WINDOW_BROKER: MessageBroker<WindowMsg> = MessageBroker::new();

pub struct Window {
    store: Arc<Store>,
    window: adw::ApplicationWindow,
    folder_tree: Controller<FolderTree>,
    message_list: Controller<MessageList>,
    search_view: Controller<SearchView>,
    conversation: Controller<Conversation>,
    sync_service: Controller<SyncService>,
    split_view: Option<adw::NavigationSplitView>,
    toast_overlay: Option<adw::ToastOverlay>,
    search_bar: Option<gtk::SearchBar>,
    search_entry: Option<gtk::SearchEntry>,
    searching: bool,
    mailbox_page_title: String,
    selected_folder_id: Option<i64>,
    anchor: Option<AnchorState>,
    account_folders: Vec<FolderRecord>,
    move_menu: Option<gio::Menu>,
    move_actions: Option<gio::SimpleActionGroup>,
    move_menu_dirty: Cell<bool>,
    reload_pending: HashSet<i64>,
    reload_scheduled: bool,
    send_receive_button: Option<gtk::Button>,
    sync_spinner: Option<gtk::Spinner>,
    composer: Option<Controller<Composer>>,
    pending_draft: Option<composer::DraftRef>,
}

#[derive(Debug)]
pub enum WindowMsg {
    FolderSelected {
        key: FolderKey,
        title: String,
    },
    MessageSelected {
        folder_id: i64,
        uid: u32,
    },
    FetchBody {
        folder_id: i64,
        uid: u32,
    },
    BodyFetched {
        message_id: i64,
    },
    AccountsChanged,
    SendReceive,
    SyncStarted,
    SyncFinished {
        synced: usize,
        failed: usize,
    },
    FolderChanged {
        folder_id: i64,
    },
    SyncError {
        detail: String,
    },
    Anchor(Option<AnchorState>),
    Viewed {
        folder_id: i64,
        uids: Vec<u32>,
    },
    FoldersLoaded {
        account_id: i64,
        folders: Vec<FolderRecord>,
    },
    FlushFolderReloads,
    MessagesChanged {
        folder_id: i64,
    },
    NewMail {
        folder_title: String,
        notices: Vec<MailNotice>,
    },
    OpenMessage {
        folder_id: i64,
        uid: u32,
    },
    ToggleFlagged,
    DeleteMessage,
    MoveMessage {
        target_folder_id: i64,
    },
    Compose,
    FocusSearch,
    SearchMode {
        enabled: bool,
    },
    Reply,
    ReplyAll,
    Forward,
    ReplyDraft(MessageDraft),
    EditDraft,
    SaveDraft(MessageDraft),
    DraftSaved {
        folder: String,
        uid: u32,
    },
    DraftReady(composer::MessageDraft),
    SendResult {
        sent: bool,
    },
}

#[relm4::component(pub)]
impl SimpleComponent for Window {
    type Init = ();
    type Input = WindowMsg;
    type Output = ();

    view! {
        #[root]
        adw::ApplicationWindow {
            set_title: Some("Tegami"),
            set_default_size: (1080, 720),
            set_width_request: 360,
            set_height_request: 294,

            #[name(toast_overlay)]
            adw::ToastOverlay {
                #[name(outer_view)]
                adw::OverlaySplitView {
                set_max_sidebar_width: 260.0,
                set_sidebar_width_fraction: 0.179,

                #[wrap(Some)]
                set_sidebar = &adw::ToolbarView {
                    add_top_bar = &adw::HeaderBar {
                        #[name(menu_button)]
                        pack_end = &gtk::MenuButton {
                            set_icon_name: "open-menu-symbolic",
                        },
                    },

                    #[wrap(Some)]
                    set_content = &gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        set_vexpand: true,

                        #[local_ref]
                        folder_tree -> adw::ViewStack {
                            set_vexpand: true,
                            set_hexpand: true,
                        },
                    },
                },

                #[name(inner_view)]
                #[wrap(Some)]
                set_content = &adw::NavigationSplitView {
                    set_min_sidebar_width: 290.0,
                    set_max_sidebar_width: 520.0,
                    set_sidebar_width_fraction: 0.355,

                    #[name(mailbox_page)]
                    #[wrap(Some)]
                    set_sidebar = &adw::NavigationPage {
                        #[watch]
                        set_title: &model.mailbox_page_title,
                        set_tag: Some("mailbox"),

                        #[wrap(Some)]
                        #[name(mailbox_toolbar)]
                        set_child = &adw::ToolbarView {
                            add_top_bar = &adw::HeaderBar {
                                #[name(show_sidebar_button)]
                                pack_start = &gtk::ToggleButton {
                                    set_icon_name: "sidebar-show-symbolic",
                                    set_tooltip_text: Some("Show folders"),
                                    set_active: true,
                                },

                                #[name(send_receive_button)]
                                pack_start = &gtk::Button {
                                    set_icon_name: "mail-send-receive-symbolic",
                                    set_tooltip_text: Some("Send and receive mail (F9)"),
                                    connect_clicked[sender] => move |_| sender.input(WindowMsg::SendReceive),
                                },

                                #[name(sync_spinner)]
                                pack_start = &gtk::Spinner {
                                    set_visible: false,
                                    set_valign: gtk::Align::Center,
                                },

                                pack_end = &gtk::Button {
                                    set_icon_name: "mail-message-new-symbolic",
                                    set_tooltip_text: Some("New Message"),
                                    connect_clicked[sender] => move |_| sender.input(WindowMsg::Compose),
                                },

                                #[name(search_button)]
                                pack_end = &gtk::ToggleButton {
                                    set_icon_name: "system-search-symbolic",
                                    set_tooltip_text: Some("Search mail"),
                                },
                            },

                            #[wrap(Some)]
                            set_content = &gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_hexpand: true,
                                set_vexpand: true,

                                #[local_ref]
                                message_list -> adw::ViewStack {
                                    set_vexpand: true,
                                    set_hexpand: true,
                                },

                                #[local_ref]
                                search_view_widget -> adw::ViewStack {
                                    set_vexpand: true,
                                    set_hexpand: true,
                                    set_visible: false,
                                },
                            },
                        },
                    },

                    #[wrap(Some)]
                    set_content = &adw::NavigationPage {
                        set_title: "Message",
                        set_tag: Some("message"),

                        #[wrap(Some)]
                        set_child = &adw::ToolbarView {
                            add_top_bar = &adw::HeaderBar {
                                #[name(edit_draft_button)]
                                pack_start = &gtk::Button {
                                    set_icon_name: "document-edit-symbolic",
                                    set_tooltip_text: Some("Edit draft"),
                                    set_visible: false,
                                    connect_clicked[sender] => move |_| sender.input(WindowMsg::EditDraft),
                                },

                                #[name(reply_button)]
                                pack_start = &gtk::Button {
                                    set_icon_name: "mail-reply-sender-symbolic",
                                    set_tooltip_text: Some("Reply"),
                                    set_sensitive: false,
                                    connect_clicked[sender] => move |_| sender.input(WindowMsg::Reply),
                                },

                                #[name(reply_all_button)]
                                pack_start = &gtk::Button {
                                    set_icon_name: "mail-reply-all-symbolic",
                                    set_tooltip_text: Some("Reply to all"),
                                    set_sensitive: false,
                                    connect_clicked[sender] => move |_| sender.input(WindowMsg::ReplyAll),
                                },

                                #[name(forward_button)]
                                pack_start = &gtk::Button {
                                    set_icon_name: "mail-forward-symbolic",
                                    set_tooltip_text: Some("Forward"),
                                    set_sensitive: false,
                                    connect_clicked[sender] => move |_| sender.input(WindowMsg::Forward),
                                },

                                #[name(flag_button)]
                                pack_start = &gtk::Button {
                                    set_icon_name: "non-starred-symbolic",
                                    set_tooltip_text: Some("Star this message"),
                                    set_sensitive: false,
                                    connect_clicked[sender] => move |_| sender.input(WindowMsg::ToggleFlagged),
                                },

                                #[name(delete_button)]
                                pack_start = &gtk::Button {
                                    set_icon_name: "user-trash-symbolic",
                                    set_tooltip_text: Some("Delete this message"),
                                    set_sensitive: false,
                                    connect_clicked[sender] => move |_| sender.input(WindowMsg::DeleteMessage),
                                },

                                #[name(move_button)]
                                pack_start = &gtk::MenuButton {
                                    set_icon_name: "folder-symbolic",
                                    set_tooltip_text: Some("Move this message"),
                                    set_sensitive: false,
                                },
                            },

                            #[wrap(Some)]
                            set_content = &gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_hexpand: true,
                                set_vexpand: true,

                                #[local_ref]
                                conversation -> adw::ViewStack {
                                    set_vexpand: true,
                                    set_hexpand: true,
                                },
                            },
                        },
                    },
                },
            },
            },

            add_breakpoint = collapse_breakpoint(COLLAPSE_FOLDERS_WIDTH, &outer_view),
            add_breakpoint = collapse_breakpoint(COLLAPSE_READING_WIDTH, &inner_view),
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let store = Arc::new(Store::open(config::store_path()).expect("failed to open mail store"));
        let folder_tree =
            FolderTree::builder()
                .launch(store.clone())
                .forward(sender.input_sender(), |msg| match msg {
                    FolderTreeOutput::Selected { key, title } => {
                        WindowMsg::FolderSelected { key, title }
                    }
                });
        let message_list =
            MessageList::builder()
                .launch(store.clone())
                .forward(sender.input_sender(), |msg| match msg {
                    MessageListOutput::Selected { folder_id, uid } => {
                        WindowMsg::MessageSelected { folder_id, uid }
                    }
                });
        let search_view =
            SearchView::builder()
                .launch(store.clone())
                .forward(sender.input_sender(), |msg| match msg {
                    SearchOutput::Selected { folder_id, uid } => {
                        WindowMsg::MessageSelected { folder_id, uid }
                    }
                });
        let conversation =
            Conversation::builder()
                .launch(store.clone())
                .forward(sender.input_sender(), |msg| match msg {
                    ConversationOutput::FetchBody { folder_id, uid } => {
                        WindowMsg::FetchBody { folder_id, uid }
                    }
                    ConversationOutput::Anchor(anchor) => WindowMsg::Anchor(anchor),
                    ConversationOutput::Viewed { folder_id, uids } => {
                        WindowMsg::Viewed { folder_id, uids }
                    }
                });
        let sync_service =
            SyncService::builder()
                .launch(store.clone())
                .forward(sender.input_sender(), |msg| match msg {
                    SyncServiceOutput::AccountsChanged => WindowMsg::AccountsChanged,
                    SyncServiceOutput::SyncStarted => WindowMsg::SyncStarted,
                    SyncServiceOutput::SyncFinished { synced, failed } => {
                        WindowMsg::SyncFinished { synced, failed }
                    }
                    SyncServiceOutput::FolderChanged { folder_id } => {
                        WindowMsg::FolderChanged { folder_id }
                    }
                    SyncServiceOutput::BodyFetched { message_id } => {
                        WindowMsg::BodyFetched { message_id }
                    }
                    SyncServiceOutput::NewMail {
                        folder_title,
                        notices,
                    } => WindowMsg::NewMail {
                        folder_title,
                        notices,
                    },
                    SyncServiceOutput::SendResult { sent } => WindowMsg::SendResult { sent },
                    SyncServiceOutput::DraftSaved { folder, uid } => {
                        WindowMsg::DraftSaved { folder, uid }
                    }
                    SyncServiceOutput::Error { detail } => WindowMsg::SyncError { detail },
                });
        let mut model = Window {
            store,
            window: root.clone(),
            folder_tree,
            message_list,
            search_view,
            conversation,
            sync_service,
            split_view: None,
            toast_overlay: None,
            search_bar: None,
            search_entry: None,
            searching: false,
            mailbox_page_title: "Inbox".to_string(),
            selected_folder_id: None,
            anchor: None,
            account_folders: Vec::new(),
            move_menu: None,
            move_actions: None,
            move_menu_dirty: Cell::new(false),
            reload_pending: HashSet::new(),
            reload_scheduled: false,
            send_receive_button: None,
            sync_spinner: None,
            composer: None,
            pending_draft: None,
        };

        let folder_tree = model.folder_tree.widget();
        let message_list = model.message_list.widget();
        let search_view_widget = model.search_view.widget();
        let conversation = model.conversation.widget();
        let widgets = view_output!();
        model.split_view = Some(widgets.inner_view.clone());
        model.toast_overlay = Some(widgets.toast_overlay.clone());
        model.send_receive_button = Some(widgets.send_receive_button.clone());
        model.sync_spinner = Some(widgets.sync_spinner.clone());

        let search_entry = gtk::SearchEntry::new();
        let search_bar = gtk::SearchBar::builder().child(&search_entry).build();
        widgets.mailbox_toolbar.add_top_bar(&search_bar);
        widgets
            .search_button
            .bind_property("active", &search_bar, "search-mode-enabled")
            .bidirectional()
            .sync_create()
            .build();
        search_bar.connect_search_mode_enabled_notify({
            let search_sender = sender.clone();
            move |bar| {
                search_sender.input(WindowMsg::SearchMode {
                    enabled: bar.is_search_mode(),
                });
            }
        });
        search_entry.connect_search_changed({
            let search_sender = model.search_view.sender().clone();
            move |entry| {
                search_sender.emit(SearchMsg::Search {
                    query: entry.text().to_string(),
                });
            }
        });
        model.search_bar = Some(search_bar);
        model.search_entry = Some(search_entry);

        let menu = gio::Menu::new();
        menu.append_item(&gio::MenuItem::new(
            Some("New Message"),
            Some("app.compose"),
        ));
        menu.append_item(&gio::MenuItem::new(
            Some("Send/Receive"),
            Some("app.send-receive"),
        ));
        menu.append_item(&RelmAction::<About>::to_menu_item("About Tegami"));
        menu.append_item(&RelmAction::<Quit>::to_menu_item("Quit"));
        widgets.menu_button.set_menu_model(Some(&menu));

        widgets
            .outer_view
            .bind_property("collapsed", &widgets.show_sidebar_button, "visible")
            .sync_create()
            .build();
        widgets
            .show_sidebar_button
            .bind_property("active", &widgets.outer_view, "show-sidebar")
            .bidirectional()
            .sync_create()
            .build();

        debug!("main window initialized");

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            WindowMsg::FolderSelected { key, title } => {
                self.mailbox_page_title = title;
                self.selected_folder_id = Some(key.folder_id);
                self.message_list.emit(MessageListMsg::Load {
                    folder_id: key.folder_id,
                });
                self.sync_service.emit(SyncServiceMsg::WatchFolder {
                    account_id: key.account_id,
                    folder: key.name.clone(),
                });
                debug!(account = key.account_id, folder = %key.name, "folder selected");
            }
            WindowMsg::MessageSelected { folder_id, uid } => {
                self.conversation
                    .emit(ConversationMsg::Load { folder_id, uid });
                if let Some(split_view) = &self.split_view {
                    split_view.set_show_content(true);
                }
                debug!(folder_id, uid, "message selected");
            }
            WindowMsg::FetchBody { folder_id, uid } => {
                self.sync_service
                    .emit(SyncServiceMsg::FetchBody { folder_id, uid });
            }
            WindowMsg::BodyFetched { message_id } => {
                self.conversation
                    .emit(ConversationMsg::BodyFetched { message_id });
            }
            WindowMsg::AccountsChanged => {
                self.folder_tree.emit(FolderTreeMsg::Reload);
            }
            WindowMsg::SendReceive => {
                self.sync_service.emit(SyncServiceMsg::SendReceive);
            }
            WindowMsg::SyncStarted => {
                self.set_syncing(true);
            }
            WindowMsg::SyncFinished { synced, failed } => {
                self.set_syncing(false);
                if failed > 0 {
                    self.show_toast("Some accounts could not be reached");
                } else if synced > 0 {
                    self.show_toast("Mailbox is up to date");
                }
            }
            WindowMsg::FolderChanged { folder_id } => {
                self.queue_folder_reload(folder_id, &sender);
            }
            WindowMsg::MessagesChanged { folder_id } => {
                self.queue_folder_reload(folder_id, &sender);
            }
            WindowMsg::FlushFolderReloads => {
                self.flush_pending_reloads();
            }
            WindowMsg::Anchor(anchor) => {
                let previous = self.anchor.as_ref().map(|anchor| anchor.account_id);
                let previous_folder = self.anchor.as_ref().map(|anchor| anchor.folder_id);
                let account = anchor.as_ref().map(|anchor| anchor.account_id);
                let folder = anchor.as_ref().map(|anchor| anchor.folder_id);
                self.anchor = anchor;
                if account != previous {
                    self.account_folders.clear();
                    match account {
                        Some(account_id) => {
                            let store = self.store.clone();
                            let folders_sender = sender.clone();
                            sender.oneshot_command(async move {
                                let folders = store.folders(account_id).await.unwrap_or_default();
                                folders_sender.input(WindowMsg::FoldersLoaded {
                                    account_id,
                                    folders,
                                });
                            });
                        }
                        None => {
                            self.move_menu = None;
                            self.move_actions = None;
                            self.move_menu_dirty.set(true);
                        }
                    }
                } else if folder != previous_folder && !self.account_folders.is_empty() {
                    self.rebuild_move_menu(&sender);
                }
            }
            WindowMsg::Viewed { folder_id, uids } => {
                self.apply_flag_change(
                    folder_id,
                    uids,
                    FLAG_SEEN,
                    0,
                    FlagChange {
                        seen: Some(true),
                        ..FlagChange::default()
                    },
                    &sender,
                );
            }
            WindowMsg::FoldersLoaded {
                account_id,
                folders,
            } => {
                if self.anchor.as_ref().map(|anchor| anchor.account_id) != Some(account_id) {
                    return;
                }
                self.account_folders = folders;
                self.rebuild_move_menu(&sender);
            }
            WindowMsg::ToggleFlagged => {
                let Some(anchor) = self.anchor.clone() else {
                    return;
                };
                let flagged = !anchor.flagged;
                self.anchor = Some(AnchorState {
                    flagged,
                    ..anchor.clone()
                });
                let (set, clear) = if flagged {
                    (FLAG_FLAGGED, 0)
                } else {
                    (0, FLAG_FLAGGED)
                };
                self.apply_flag_change(
                    anchor.folder_id,
                    vec![anchor.uid],
                    set,
                    clear,
                    FlagChange {
                        flagged: Some(flagged),
                        ..FlagChange::default()
                    },
                    &sender,
                );
            }
            WindowMsg::DeleteMessage => {
                self.delete_message(&sender);
            }
            WindowMsg::NewMail {
                folder_title,
                notices,
            } => {
                notify::send(&notify::notifications(&folder_title, &notices));
            }
            WindowMsg::OpenMessage { folder_id, uid } => {
                self.window.present();
                self.selected_folder_id = Some(folder_id);
                self.message_list.emit(MessageListMsg::Load { folder_id });
                self.message_list.emit(MessageListMsg::Select { uid });
                if let Some(split_view) = &self.split_view {
                    split_view.set_show_content(true);
                }
                debug!(folder_id, uid, "opening message from notification");
            }
            WindowMsg::MoveMessage { target_folder_id } => {
                self.move_message(target_folder_id, &sender);
            }
            WindowMsg::Compose => {
                if let Some(composer) = &self.composer
                    && composer.widget().is_visible()
                {
                    composer.widget().present();
                } else {
                    self.open_composer(None, &sender);
                }
            }
            WindowMsg::FocusSearch => {
                if let Some(bar) = &self.search_bar {
                    bar.set_search_mode(true);
                }
                if let Some(entry) = &self.search_entry {
                    entry.grab_focus();
                }
            }
            WindowMsg::SearchMode { enabled } => {
                self.searching = enabled;
                if enabled {
                    if let Some(split_view) = &self.split_view {
                        split_view.set_show_content(false);
                    }
                } else {
                    self.search_view.emit(SearchMsg::Clear);
                }
            }
            WindowMsg::Reply => self.start_reply(ReplyMode::Reply, &sender),
            WindowMsg::ReplyAll => self.start_reply(ReplyMode::ReplyAll, &sender),
            WindowMsg::Forward => self.start_reply(ReplyMode::Forward, &sender),
            WindowMsg::ReplyDraft(draft) => {
                self.open_composer(Some(draft), &sender);
            }
            WindowMsg::EditDraft => self.start_edit_draft(&sender),
            WindowMsg::SaveDraft(draft) => {
                let Some(account_id) = draft.identity.as_ref().map(|identity| identity.account_id)
                else {
                    warn!("draft has no sender identity");
                    return;
                };
                let replace_uid = draft.draft.as_ref().map(|draft| draft.uid);
                match mail_core::compose::build(&composer::outgoing(&draft)) {
                    Ok(raw) => self.sync_service.emit(SyncServiceMsg::SaveDraft {
                        account_id,
                        raw,
                        replace_uid,
                    }),
                    Err(err) => warn!(detail = %err, "failed to build draft"),
                }
            }
            WindowMsg::DraftSaved { folder, uid } => {
                if let Some(composer) = &self.composer {
                    composer.emit(ComposerMsg::DraftSaved { folder, uid });
                }
            }
            WindowMsg::DraftReady(draft) => {
                self.pending_draft = draft.draft.clone();
                let Some(account_id) = draft.identity.as_ref().map(|identity| identity.account_id)
                else {
                    warn!("draft has no sender identity");
                    return;
                };
                let message = composer::outgoing(&draft);
                match mail_core::compose::build(&message) {
                    Ok(raw) => {
                        debug!(
                            bytes = raw.len(),
                            to = %draft.to,
                            subject = %draft.subject,
                            "enqueuing composed message"
                        );
                        self.sync_service
                            .emit(SyncServiceMsg::Send { account_id, raw });
                    }
                    Err(err) => {
                        warn!(detail = %err, "failed to build composed message");
                    }
                }
            }
            WindowMsg::SendResult { sent } => {
                if let Some(composer) = &self.composer {
                    composer.widget().close();
                    self.toast(sent);
                }
                if sent && let Some(draft) = self.pending_draft.take() {
                    self.sync_service.emit(SyncServiceMsg::DeleteDraft {
                        account_id: draft.account_id,
                        folder: draft.folder,
                        uid: draft.uid,
                    });
                }
            }
            WindowMsg::SyncError { detail } => {
                debug!(detail, "sync error");
            }
        }
    }

    fn post_view(&self, _widgets: &mut Self::Widgets) {
        let enabled = self.anchor.is_some();
        let flagged = self.anchor.as_ref().is_some_and(|anchor| anchor.flagged);
        let drafts = self.anchor.as_ref().is_some_and(|anchor| {
            self.account_folders.iter().any(|folder| {
                folder.id == Some(anchor.folder_id)
                    && folder.special_use == Some(SpecialUse::Drafts)
            })
        });
        edit_draft_button.set_visible(drafts);
        self.message_list.widget().set_visible(!self.searching);
        self.search_view.widget().set_visible(self.searching);
        reply_button.set_sensitive(enabled);
        reply_all_button.set_sensitive(enabled);
        forward_button.set_sensitive(enabled);
        flag_button.set_sensitive(enabled);
        delete_button.set_sensitive(enabled);
        move_button.set_sensitive(enabled);
        flag_button.set_icon_name(if flagged {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        });
        flag_button.set_tooltip_text(Some(if flagged {
            "Remove star"
        } else {
            "Star this message"
        }));
        if self.move_menu_dirty.replace(false) {
            move_button.set_menu_model(self.move_menu.as_ref());
            if let Some(group) = &self.move_actions {
                move_button.insert_action_group("move", Some(group));
            }
        }
    }
}

impl Window {
    fn open_composer(&mut self, initial: Option<MessageDraft>, sender: &ComponentSender<Self>) {
        let builder = Composer::builder();
        relm4::main_application().add_window(&builder.root);
        let composer = builder
            .launch(ComposerInit {
                store: self.store.clone(),
                initial,
            })
            .forward(sender.input_sender(), |msg| match msg {
                ComposerOutput::Send(draft) => WindowMsg::DraftReady(draft),
                ComposerOutput::SaveDraft(draft) => WindowMsg::SaveDraft(draft),
            });
        self.composer = Some(composer);
    }

    fn start_reply(&self, mode: ReplyMode, sender: &ComponentSender<Self>) {
        let Some(anchor) = self.anchor.clone() else {
            return;
        };
        let store = self.store.clone();
        let reply_sender = sender.clone();
        sender.oneshot_command(async move {
            match reply_draft(&store, anchor.folder_id, anchor.uid, mode).await {
                Ok(draft) => reply_sender.input(WindowMsg::ReplyDraft(draft)),
                Err(detail) => debug!(detail, "cannot compose reply"),
            }
        });
    }

    fn start_edit_draft(&self, sender: &ComponentSender<Self>) {
        let Some(anchor) = self.anchor.clone() else {
            return;
        };
        let store = self.store.clone();
        let edit_sender = sender.clone();
        sender.oneshot_command(async move {
            match edit_draft(&store, anchor.folder_id, anchor.uid).await {
                Ok(draft) => edit_sender.input(WindowMsg::ReplyDraft(draft)),
                Err(detail) => debug!(detail, "cannot open draft"),
            }
        });
    }

    fn rebuild_move_menu(&mut self, sender: &ComponentSender<Self>) {
        let current = self.anchor.as_ref().map(|anchor| anchor.folder_id);
        let mut targets: Vec<FolderRecord> = self
            .account_folders
            .iter()
            .filter(|folder| folder.id != current)
            .cloned()
            .collect();
        targets.sort_by_key(folder_title);
        let targets: Vec<&FolderRecord> = targets.iter().collect();
        let (menu, group) = build_move_menu(&targets, sender);
        self.move_menu = Some(menu);
        self.move_actions = Some(group);
        self.move_menu_dirty.set(true);
    }

    fn toast(&self, sent: bool) {
        let label = if sent {
            "Message sent"
        } else {
            "Message queued for delivery"
        };
        self.show_toast(label);
    }

    fn show_toast(&self, label: &str) {
        if let Some(overlay) = &self.toast_overlay {
            overlay.add_toast(adw::Toast::new(label));
        }
    }

    fn set_syncing(&self, syncing: bool) {
        if let Some(button) = &self.send_receive_button {
            button.set_sensitive(!syncing);
        }
        if let Some(spinner) = &self.sync_spinner {
            spinner.set_spinning(syncing);
            spinner.set_visible(syncing);
        }
    }

    fn queue_folder_reload(&mut self, folder_id: i64, sender: &ComponentSender<Self>) {
        self.reload_pending.insert(folder_id);
        if self.reload_scheduled {
            return;
        }
        self.reload_scheduled = true;
        let flush_sender = sender.clone();
        glib::timeout_add_local_once(Duration::from_millis(80), move || {
            flush_sender.input(WindowMsg::FlushFolderReloads);
        });
    }

    fn flush_pending_reloads(&mut self) {
        self.reload_scheduled = false;
        if self.reload_pending.is_empty() {
            return;
        }
        self.folder_tree.emit(FolderTreeMsg::Reload);
        if let Some(selected) = self.selected_folder_id
            && self.reload_pending.contains(&selected)
        {
            self.message_list.emit(MessageListMsg::Load { folder_id: selected });
        }
        self.reload_pending.clear();
    }

    fn apply_flag_change(
        &self,
        folder_id: i64,
        uids: Vec<u32>,
        set: i64,
        clear: i64,
        change: FlagChange,
        sender: &ComponentSender<Self>,
    ) {
        let store = self.store.clone();
        let refresh_sender = sender.clone();
        let stored = uids.clone();
        sender.oneshot_command(async move {
            if let Err(err) = store
                .update_message_flags(folder_id, &stored, set, clear)
                .await
            {
                debug!(detail = %err, "failed to update message flags");
            }
            refresh_sender.input(WindowMsg::MessagesChanged { folder_id });
        });
        self.sync_service
            .emit(SyncServiceMsg::MessageAction(MessageAction::SetFlags {
                folder_id,
                uids,
                change,
            }));
    }

    fn remove_message(&self, folder_id: i64, uid: u32, sender: &ComponentSender<Self>) {
        let store = self.store.clone();
        let refresh_sender = sender.clone();
        sender.oneshot_command(async move {
            if let Err(err) = store.delete_messages(folder_id, &[uid]).await {
                debug!(detail = %err, "failed to remove message");
            }
            refresh_sender.input(WindowMsg::MessagesChanged { folder_id });
        });
    }

    fn move_message(&mut self, target_folder_id: i64, sender: &ComponentSender<Self>) {
        let Some(anchor) = self.anchor.clone() else {
            return;
        };
        self.remove_message(anchor.folder_id, anchor.uid, sender);
        self.sync_service
            .emit(SyncServiceMsg::MessageAction(MessageAction::Move {
                folder_id: anchor.folder_id,
                target_folder_id,
                uids: vec![anchor.uid],
            }));
        self.anchor = None;
        self.conversation.emit(ConversationMsg::Clear);
    }

    fn delete_message(&mut self, sender: &ComponentSender<Self>) {
        let Some(anchor) = self.anchor.clone() else {
            return;
        };
        let trash = self
            .account_folders
            .iter()
            .find(|folder| folder.special_use == Some(SpecialUse::Trash))
            .and_then(|folder| folder.id)
            .filter(|id| *id != anchor.folder_id);
        match trash {
            Some(target) => self.move_message(target, sender),
            None => {
                self.remove_message(anchor.folder_id, anchor.uid, sender);
                self.sync_service
                    .emit(SyncServiceMsg::MessageAction(MessageAction::Delete {
                        folder_id: anchor.folder_id,
                        uids: vec![anchor.uid],
                    }));
                self.anchor = None;
                self.conversation.emit(ConversationMsg::Clear);
            }
        }
    }
}

fn build_move_menu(
    targets: &[&FolderRecord],
    sender: &ComponentSender<Window>,
) -> (gio::Menu, gio::SimpleActionGroup) {
    let menu = gio::Menu::new();
    let group = gio::SimpleActionGroup::new();
    for folder in targets {
        let Some(id) = folder.id else {
            continue;
        };
        let action = gio::SimpleAction::new(&format!("to-{id}"), None);
        let action_sender = sender.clone();
        action.connect_activate(move |_, _| {
            action_sender.input(WindowMsg::MoveMessage {
                target_folder_id: id,
            });
        });
        group.add_action(&action);
        menu.append_item(&gio::MenuItem::new(
            Some(&folder_title(folder)),
            Some(&format!("move.to-{id}")),
        ));
    }
    (menu, group)
}

fn folder_title(folder: &FolderRecord) -> String {
    folder
        .display_name
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| folder.name.clone())
}

async fn reply_draft(
    store: &Store,
    folder_id: i64,
    uid: u32,
    mode: ReplyMode,
) -> Result<MessageDraft, String> {
    let message = store
        .message(folder_id, uid)
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "message not found".to_string())?;
    let raw_path = message
        .raw_path
        .as_deref()
        .ok_or_else(|| "message body is not cached".to_string())?;
    let raw = tokio::fs::read(raw_path)
        .await
        .map_err(|err| err.to_string())?;

    let account_id = store
        .folder(folder_id)
        .await
        .map_err(|err| err.to_string())?
        .map(|folder| folder.account_id);
    let accounts = store.accounts().await.map_err(|err| err.to_string())?;
    let identity = account_id.and_then(|account_id| {
        accounts
            .into_iter()
            .find(|account| account.id == Some(account_id))
    });
    let identity = identity.map(|account| composer::Identity {
        account_id: account.id.unwrap_or_default(),
        name: account.display_name.clone().unwrap_or_default(),
        email: account.email.clone(),
    });
    let me = identity.as_ref().map(|identity| {
        mail_core::compose::Address::new(Some(identity.name.clone()), identity.email.clone())
    });

    let outgoing = mail_core::compose::reply(&raw, me.as_ref(), mode)
        .ok_or_else(|| "cannot parse the message".to_string())?;
    Ok(MessageDraft {
        identity,
        to: composer::render_addresses(&outgoing.to),
        cc: composer::render_addresses(&outgoing.cc),
        bcc: composer::render_addresses(&outgoing.bcc),
        subject: outgoing.subject,
        body: outgoing.text.unwrap_or_default(),
        attachments: outgoing.attachments,
        in_reply_to: outgoing.in_reply_to,
        references: outgoing.references,
        draft: None,
    })
}

async fn edit_draft(store: &Store, folder_id: i64, uid: u32) -> Result<MessageDraft, String> {
    let message = store
        .message(folder_id, uid)
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "message not found".to_string())?;
    let raw_path = message
        .raw_path
        .as_deref()
        .ok_or_else(|| "message body is not cached".to_string())?;
    let raw = tokio::fs::read(raw_path)
        .await
        .map_err(|err| err.to_string())?;
    let folder = store
        .folder(folder_id)
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "folder not found".to_string())?;
    let accounts = store.accounts().await.map_err(|err| err.to_string())?;
    let identity = accounts
        .into_iter()
        .find(|account| account.id == Some(folder.account_id))
        .map(|account| composer::Identity {
            account_id: account.id.unwrap_or_default(),
            name: account.display_name.clone().unwrap_or_default(),
            email: account.email.clone(),
        });
    let outgoing = mail_core::compose::parse_outgoing(&raw)
        .ok_or_else(|| "cannot parse the draft".to_string())?;
    Ok(MessageDraft {
        identity,
        to: composer::render_addresses(&outgoing.to),
        cc: composer::render_addresses(&outgoing.cc),
        bcc: composer::render_addresses(&outgoing.bcc),
        subject: outgoing.subject,
        body: outgoing.text.unwrap_or_default(),
        attachments: outgoing.attachments,
        in_reply_to: outgoing.in_reply_to,
        references: outgoing.references,
        draft: Some(composer::DraftRef {
            account_id: folder.account_id,
            folder: folder.name,
            uid,
        }),
    })
}

fn collapse_breakpoint(max_width: f64, widget: &impl IsA<glib::Object>) -> adw::Breakpoint {
    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        max_width,
        adw::LengthUnit::Sp,
    ));
    breakpoint.add_setter(widget, "collapsed", Some(&true.to_value()));
    breakpoint
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::compose::OutgoingMessage;
    use mail_core::store::{
        AccountRecord, AccountSource, AuthKind, BodyState, FolderRecord, MessageRecord, SpecialUse,
    };
    use tempfile::TempDir;

    const RAW: &str = concat!(
        "From: Ada Lovelace <ada@lovelace.dev>\r\n",
        "To: grace@navy.dev\r\n",
        "Subject: Re: Greetings\r\n",
        "Date: Mon, 03 Mar 2025 10:20:30 +0000\r\n",
        "Message-ID: <abc123@lovelace.dev>\r\n",
        "\r\n",
        "Hello there\r\n",
    );

    fn account() -> AccountRecord {
        AccountRecord {
            id: None,
            source: AccountSource::Goa,
            external_id: "account_1".to_string(),
            email: "user@example.org".to_string(),
            display_name: Some("Example User".to_string()),
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

    fn folder(account_id: i64) -> FolderRecord {
        FolderRecord {
            id: None,
            account_id,
            name: "INBOX".to_string(),
            display_name: Some("Inbox".to_string()),
            special_use: Some(SpecialUse::Inbox),
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            unread_count: 0,
            total_count: 0,
            subscribed: true,
        }
    }

    fn message(folder_id: i64, raw_path: &str) -> MessageRecord {
        MessageRecord {
            id: None,
            folder_id,
            uid: 7,
            modseq: None,
            message_id: Some("<abc123@lovelace.dev>".to_string()),
            thread_id: None,
            subject: "Re: Greetings".to_string(),
            from_addr: Some("ada@lovelace.dev".to_string()),
            from_name: Some("Ada Lovelace".to_string()),
            to_addrs: None,
            cc_addrs: None,
            date_sent: Some(1741000000),
            date_recv: None,
            in_reply_to: None,
            refs: None,
            flags: 0,
            has_attach: false,
            size: None,
            structure: None,
            raw_path: Some(raw_path.to_string()),
            body_state: BodyState::Full,
        }
    }

    async fn seeded() -> (Store, TempDir) {
        let store = Store::open(":memory:").unwrap();
        let account_id = store.upsert_account(account()).await.unwrap();
        let folder_id = store.upsert_folder(folder(account_id)).await.unwrap();
        let dir = TempDir::new().unwrap();
        let raw_path = dir.path().join("raw.eml").to_string_lossy().into_owned();
        std::fs::write(&raw_path, RAW).unwrap();
        store
            .upsert_message(message(folder_id, &raw_path))
            .await
            .unwrap();
        (store, dir)
    }

    #[tokio::test]
    async fn reply_draft_builds_a_quoted_reply() {
        let (store, _dir) = seeded().await;
        let folder_id = store
            .folders(store.accounts().await.unwrap()[0].id.unwrap())
            .await
            .unwrap()[0]
            .id
            .unwrap();
        let draft = reply_draft(&store, folder_id, 7, ReplyMode::Reply)
            .await
            .unwrap();

        assert_eq!(draft.to, "Ada Lovelace <ada@lovelace.dev>");
        assert!(draft.cc.is_empty());
        assert_eq!(draft.subject, "Re: Greetings");
        assert_eq!(draft.in_reply_to.as_deref(), Some("abc123@lovelace.dev"));
        assert!(draft.body.contains("> Hello there"));
        assert_eq!(draft.identity.unwrap().email, "user@example.org");
    }

    #[tokio::test]
    async fn reply_draft_requires_a_cached_body() {
        let (store, _dir) = seeded().await;
        let folder_id = store
            .folders(store.accounts().await.unwrap()[0].id.unwrap())
            .await
            .unwrap()[0]
            .id
            .unwrap();
        store
            .set_message_body(folder_id, 7, String::new(), BodyState::None, false)
            .await
            .unwrap();

        assert!(
            reply_draft(&store, folder_id, 7, ReplyMode::Reply)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn edit_draft_reopens_a_saved_draft_for_editing() {
        let (store, dir) = seeded().await;
        let account_id = store.accounts().await.unwrap()[0].id.unwrap();
        let mut drafts = folder(account_id);
        drafts.name = "Drafts".to_string();
        drafts.display_name = Some("Drafts".to_string());
        drafts.special_use = Some(SpecialUse::Drafts);
        let drafts_id = store.upsert_folder(drafts).await.unwrap();

        let raw = concat!(
            "From: user@example.org\r\n",
            "To: \"Hopper, G\" <g@navy.dev>, grace@navy.dev\r\n",
            "Subject: Draft note\r\n",
            "Message-ID: <draft@example.org>\r\n",
            "In-Reply-To: <orig@example.org>\r\n",
            "\r\n",
            "Saved body\r\n",
        );
        let raw_path = dir.path().join("draft.eml").to_string_lossy().into_owned();
        std::fs::write(&raw_path, raw).unwrap();
        let mut message = message(drafts_id, &raw_path);
        message.uid = 9;
        store.upsert_message(message).await.unwrap();

        let draft = edit_draft(&store, drafts_id, 9).await.unwrap();
        assert_eq!(draft.to, "\"Hopper, G\" <g@navy.dev>, grace@navy.dev");
        assert_eq!(draft.subject, "Draft note");
        assert_eq!(draft.body.trim_end(), "Saved body");
        assert_eq!(draft.in_reply_to.as_deref(), Some("orig@example.org"));
        let draft_ref = draft.draft.unwrap();
        assert_eq!(draft_ref.folder, "Drafts");
        assert_eq!(draft_ref.uid, 9);
        assert_eq!(draft_ref.account_id, account_id);
    }

    #[test]
    fn draft_from_outgoing_carries_everything() {
        let outgoing = OutgoingMessage {
            from: None,
            to: vec![mail_core::compose::Address::new(
                Some("Ada Lovelace".to_string()),
                "ada@lovelace.dev",
            )],
            cc: vec![mail_core::compose::Address::new(
                Some("Hopper, G".to_string()),
                "g@navy.dev",
            )],
            subject: "Re: hi".to_string(),
            text: Some("> quoted".to_string()),
            attachments: vec![mail_core::compose::OutgoingAttachment {
                filename: "doc.pdf".to_string(),
                mime_type: "application/pdf".to_string(),
                data: b"%PDF".to_vec(),
            }],
            in_reply_to: Some("abc@example.org".to_string()),
            references: vec!["abc@example.org".to_string()],
            ..OutgoingMessage::default()
        };
        let draft = MessageDraft {
            identity: None,
            to: composer::render_addresses(&outgoing.to),
            cc: composer::render_addresses(&outgoing.cc),
            bcc: composer::render_addresses(&outgoing.bcc),
            subject: outgoing.subject,
            body: outgoing.text.unwrap_or_default(),
            attachments: outgoing.attachments,
            in_reply_to: outgoing.in_reply_to,
            references: outgoing.references,
            draft: None,
        };
        assert_eq!(draft.to, "Ada Lovelace <ada@lovelace.dev>");
        assert_eq!(draft.cc, "\"Hopper, G\" <g@navy.dev>");
        assert_eq!(draft.attachments.len(), 1);
        assert_eq!(draft.in_reply_to.as_deref(), Some("abc@example.org"));
        assert_eq!(draft.references, vec!["abc@example.org"]);
    }
}
