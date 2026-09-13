// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use gtk::gio;
use relm4::actions::RelmAction;
use relm4::adw::prelude::*;
use relm4::prelude::*;
use tracing::debug;

use crate::application::{About, Quit};

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

            #[wrap(Some)]
            set_content = &adw::ToolbarView {
                add_top_bar = &adw::HeaderBar {
                    #[name(menu_button)]
                    pack_end = &gtk::MenuButton {
                        set_icon_name: "open-menu-symbolic",
                    },
                },

                #[wrap(Some)]
                set_content = &gtk::Box {
                    set_hexpand: true,
                    set_vexpand: true,
                },
            },
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

        debug!("main window initialized");

        ComponentParts {
            model: Window,
            widgets,
        }
    }

    fn update(&mut self, _msg: Self::Input, _sender: ComponentSender<Self>) {}
}
