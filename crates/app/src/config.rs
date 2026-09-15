// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

pub const APP_ID: &str = env!("TEGAMI_APP_ID");
pub const VERSION: &str = env!("TEGAMI_VERSION");

/// The store backing the UI shell's account and folder state.
pub fn store_path() -> std::path::PathBuf {
    let dir = relm4::gtk::glib::user_data_dir().join("tegami");
    std::fs::create_dir_all(&dir).expect("failed to create data directory");
    dir.join("mail.sqlite3")
}

/// The directory caching raw MIME bodies and attachments.
pub fn body_dir() -> std::path::PathBuf {
    let dir = relm4::gtk::glib::user_data_dir()
        .join("tegami")
        .join("bodies");
    std::fs::create_dir_all(&dir).expect("failed to create body cache directory");
    dir
}
