// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::cell::Cell;
use std::sync::Arc;

use gtk::gio;
use mail_core::envelope::FlagChange;
use mail_core::store::{FLAG_FLAGGED, FLAG_SEEN, FolderRecord, SpecialUse, Store};
use relm4::MessageBroker;
use relm4::actions::RelmAction;
use relm4::adw::prelude::*;
use relm4::gtk::glib;
use relm4::prelude::*;
use tracing::debug;

use crate::application::{About, Quit};
use crate::composer::{self, Composer, ComposerOutput};
use crate::config;
use crate::conversation::{AnchorState, Conversation, ConversationMsg, ConversationOutput};
use crate::message_list::{MessageList, MessageListMsg, MessageListOutput};
use crate::notify::{self, MailNotice};
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
    conversation: Controller<Conversation>,
    sync_service: Controller<SyncService>,
    split_view: Option<adw::NavigationSplitView>,
    mailbox_page_title: String,
    selected_folder_id: Option<i64>,
    anchor: Option<AnchorState>,
    account_folders: Vec<FolderRecord>,
    move_menu: Option<gio::Menu>,
    move_actions: Option<gio::SimpleActionGroup>,
    move_menu_dirty: Cell<bool>,
    composer: Option<Controller<Composer>>,
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
    DraftReady(composer::MessageDraft),
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
                        set_child = &adw::ToolbarView {
                            add_top_bar = &adw::HeaderBar {
                                #[name(show_sidebar_button)]
                                pack_start = &gtk::ToggleButton {
                                    set_icon_name: "sidebar-show-symbolic",
                                    set_tooltip_text: Some("Show folders"),
                                    set_active: true,
                                },

                                pack_end = &gtk::Button {
                                    set_icon_name: "mail-message-new-symbolic",
                                    set_tooltip_text: Some("New Message"),
                                    connect_clicked[sender] => move |_| sender.input(WindowMsg::Compose),
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
                    SyncServiceOutput::Error { detail } => WindowMsg::SyncError { detail },
                });
        let mut model = Window {
            store,
            window: root.clone(),
            folder_tree,
            message_list,
            conversation,
            sync_service,
            split_view: None,
            mailbox_page_title: "Inbox".to_string(),
            selected_folder_id: None,
            anchor: None,
            account_folders: Vec::new(),
            move_menu: None,
            move_actions: None,
            move_menu_dirty: Cell::new(false),
            composer: None,
        };

        let folder_tree = model.folder_tree.widget();
        let message_list = model.message_list.widget();
        let conversation = model.conversation.widget();
        let widgets = view_output!();
        model.split_view = Some(widgets.inner_view.clone());

        let menu = gio::Menu::new();
        menu.append_item(&gio::MenuItem::new(
            Some("New Message"),
            Some("app.compose"),
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
            WindowMsg::FolderChanged { folder_id } => {
                self.reload_folder(folder_id);
            }
            WindowMsg::MessagesChanged { folder_id } => {
                self.reload_folder(folder_id);
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
                    self.open_composer(&sender);
                }
            }
            WindowMsg::DraftReady(draft) => {
                debug!(
                    from = draft
                        .identity
                        .as_ref()
                        .map(|identity| identity.email.as_str())
                        .unwrap_or_default(),
                    to = %draft.to,
                    subject = %draft.subject,
                    body_len = draft.body.len(),
                    "composer draft ready"
                );
            }
            WindowMsg::SyncError { detail } => {
                debug!(detail, "sync error");
            }
        }
    }

    fn post_view(&self, _widgets: &mut Self::Widgets) {
        let enabled = self.anchor.is_some();
        let flagged = self.anchor.as_ref().is_some_and(|anchor| anchor.flagged);
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
    fn open_composer(&mut self, sender: &ComponentSender<Self>) {
        let builder = Composer::builder();
        relm4::main_application().add_window(&builder.root);
        let composer = builder
            .launch(self.store.clone())
            .forward(sender.input_sender(), |msg| match msg {
                ComposerOutput::Send(draft) => WindowMsg::DraftReady(draft),
            });
        self.composer = Some(composer);
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

    fn reload_folder(&self, folder_id: i64) {
        self.folder_tree.emit(FolderTreeMsg::Reload);
        if self.selected_folder_id == Some(folder_id) {
            self.message_list.emit(MessageListMsg::Load { folder_id });
        }
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

fn collapse_breakpoint(max_width: f64, widget: &impl IsA<glib::Object>) -> adw::Breakpoint {
    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        max_width,
        adw::LengthUnit::Sp,
    ));
    breakpoint.add_setter(widget, "collapsed", Some(&true.to_value()));
    breakpoint
}
