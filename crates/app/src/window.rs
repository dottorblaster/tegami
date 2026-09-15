// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Arc;

use gtk::gio;
use mail_core::store::Store;
use relm4::actions::RelmAction;
use relm4::adw::prelude::*;
use relm4::gtk::glib;
use relm4::prelude::*;
use tracing::debug;

use crate::application::{About, Quit};
use crate::config;
use crate::conversation::{Conversation, ConversationMsg, ConversationOutput};
use crate::message_list::{MessageList, MessageListMsg, MessageListOutput};
use crate::sidebar::{FolderKey, FolderTree, FolderTreeMsg, FolderTreeOutput};
use crate::sync::{SyncService, SyncServiceMsg, SyncServiceOutput};

const COLLAPSE_FOLDERS_WIDTH: f64 = 860.0;
const COLLAPSE_READING_WIDTH: f64 = 500.0;

pub struct Window {
    folder_tree: Controller<FolderTree>,
    message_list: Controller<MessageList>,
    conversation: Controller<Conversation>,
    sync_service: Controller<SyncService>,
    split_view: Option<adw::NavigationSplitView>,
    mailbox_page_title: String,
    selected_folder_id: Option<i64>,
}

#[derive(Debug)]
pub enum WindowMsg {
    FolderSelected { key: FolderKey, title: String },
    MessageSelected { folder_id: i64, uid: u32 },
    FetchBody { folder_id: i64, uid: u32 },
    BodyFetched { message_id: i64 },
    AccountsChanged,
    FolderChanged { folder_id: i64 },
    SyncError { detail: String },
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
                            add_top_bar = &adw::HeaderBar {},

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
        _root: Self::Root,
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
                });
        let sync_service =
            SyncService::builder()
                .launch(store)
                .forward(sender.input_sender(), |msg| match msg {
                    SyncServiceOutput::AccountsChanged => WindowMsg::AccountsChanged,
                    SyncServiceOutput::FolderChanged { folder_id } => {
                        WindowMsg::FolderChanged { folder_id }
                    }
                    SyncServiceOutput::BodyFetched { message_id } => {
                        WindowMsg::BodyFetched { message_id }
                    }
                    SyncServiceOutput::Error { detail } => WindowMsg::SyncError { detail },
                });
        let mut model = Window {
            folder_tree,
            message_list,
            conversation,
            sync_service,
            split_view: None,
            mailbox_page_title: "Inbox".to_string(),
            selected_folder_id: None,
        };

        let folder_tree = model.folder_tree.widget();
        let message_list = model.message_list.widget();
        let conversation = model.conversation.widget();
        let widgets = view_output!();
        model.split_view = Some(widgets.inner_view.clone());

        let menu = gio::Menu::new();
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

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>) {
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
                self.folder_tree.emit(FolderTreeMsg::Reload);
                if self.selected_folder_id == Some(folder_id) {
                    self.message_list.emit(MessageListMsg::Load { folder_id });
                }
            }
            WindowMsg::SyncError { detail } => {
                debug!(detail, "sync error");
            }
        }
    }
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
