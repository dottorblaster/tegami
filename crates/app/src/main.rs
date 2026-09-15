// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

mod application;
mod config;
mod logging;
mod sidebar;
mod window;

use relm4::gtk::gio;
use tracing::info;

fn main() {
    logging::init();

    let bundle = include_bytes!(concat!(env!("TEGAMI_RESOURCE_BUNDLE")));
    gio::resources_register(
        &gio::Resource::from_data(&relm4::gtk::glib::Bytes::from_static(bundle))
            .expect("failed to register resources"),
    );

    info!(version = config::VERSION, "starting Tegami");

    application::run();
}
