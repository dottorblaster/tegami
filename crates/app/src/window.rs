// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use gtk::gio;
use relm4::actions::RelmAction;
use relm4::adw::prelude::*;
use relm4::gtk::glib;
use relm4::prelude::*;
use tracing::debug;

use crate::application::{About, Quit};

const COLLAPSE_FOLDERS_WIDTH: f64 = 860.0;
const COLLAPSE_READING_WIDTH: f64 = 500.0;

pub struct Window;

#[relm4::component(pub)]
impl SimpleComponent for Window {
    type Init = ();
    type Input = ();
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
                    },
                },

                #[name(inner_view)]
                #[wrap(Some)]
                set_content = &adw::NavigationSplitView {
                    set_min_sidebar_width: 290.0,
                    set_max_sidebar_width: 520.0,
                    set_sidebar_width_fraction: 0.355,

                    #[wrap(Some)]
                    set_sidebar = &adw::NavigationPage {
                        set_title: "Inbox",
                        set_tag: Some("mailbox"),

                        #[wrap(Some)]
                        set_child = &adw::ToolbarView {
                            add_top_bar = &adw::HeaderBar {
                                #[name(show_sidebar_button)]
                                pack_start = &gtk::ToggleButton {
                                    set_icon_name: "sidebar-show-symbolic",
                                    set_tooltip_text: Some("Show folders"),
                                },
                            },

                            #[wrap(Some)]
                            set_content = &gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_hexpand: true,
                                set_vexpand: true,
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
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();

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

        ComponentParts {
            model: Window,
            widgets,
        }
    }

    fn update(&mut self, _msg: Self::Input, _sender: ComponentSender<Self>) {}
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
