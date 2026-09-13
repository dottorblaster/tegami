// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use relm4::adw::prelude::*;
use relm4::gtk::gio;
use relm4::prelude::*;

const APP_ID: &str = "it.dottorblaster.tegami";

struct App;

#[relm4::component]
impl SimpleComponent for App {
    type Init = ();
    type Input = ();
    type Output = ();

    view! {
        #[root]
        adw::ApplicationWindow {
            set_title: Some("Tegami"),
            set_default_size: (720, 480),

            #[wrap(Some)]
            set_content = &adw::ToolbarView {
                add_top_bar = &adw::HeaderBar {},

                #[wrap(Some)]
                set_content = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Label {
                        set_label: "Hello, Tegami!",
                        add_css_class: "title-1",
                    },
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        window: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = App;
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, _msg: Self::Input, _sender: ComponentSender<Self>) {}
}

fn main() {
    let bundle = include_bytes!(concat!(env!("TEGAMI_RESOURCE_BUNDLE")));
    gio::resources_register(
        &gio::Resource::from_data(&relm4::gtk::glib::Bytes::from_static(bundle))
            .expect("failed to register resources"),
    );

    let app = RelmApp::new(APP_ID);
    app.run::<App>(());
}
