// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use relm4::actions::{AccelsPlus, RelmAction, RelmActionGroup};
use relm4::adw::prelude::*;
use relm4::prelude::*;
use tracing::debug;

use crate::config;
use crate::window::Window;

relm4::new_action_group!(pub AppGroup, "app");
relm4::new_stateless_action!(pub Quit, AppGroup, "quit");
relm4::new_stateless_action!(pub About, AppGroup, "about");

pub fn run() {
    let app = RelmApp::new(config::APP_ID);
    register_actions();
    load_style();
    debug!("application started");
    app.run::<Window>(());
    debug!("application stopped");
}

fn load_style() {
    let provider = gtk::CssProvider::new();
    provider.load_from_resource("/it/dottorblaster/tegami/style.css");
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn register_actions() {
    let mut group = RelmActionGroup::<AppGroup>::new();

    group.add_action(RelmAction::<Quit>::new_stateless(|_| {
        relm4::main_application().quit();
    }));
    group.add_action(RelmAction::<About>::new_stateless(|_| {
        show_about_dialog();
    }));
    group.register_for_main_application();

    let app = relm4::main_application();
    app.set_accelerators_for_action::<Quit>(&["<primary>q"]);
}

fn show_about_dialog() {
    let dialog = adw::AboutDialog::builder()
        .application_name("Tegami")
        .application_icon(config::APP_ID)
        .version(config::VERSION)
        .developers(vec!["Alessio Biancalana"])
        .website("https://github.com/dottorblaster/tegami")
        .issue_url("https://github.com/dottorblaster/tegami/issues")
        .copyright("© 2026 Alessio Biancalana")
        .license_type(gtk::License::Gpl30)
        .build();
    dialog.present(relm4::main_application().active_window().as_ref());
}
