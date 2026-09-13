// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Parsing of the key-file `Data` property of EDS sources.
//!
//! EDS serializes every source as a GKeyFile-formatted string; the same
//! format is used on disk for `~/.config/evolution/sources/*.source`, so
//! this parser doubles as the on-disk fallback backend.

use glib::KeyFile;

#[derive(Clone, Debug)]
pub struct SourceData {
    keyfile: KeyFile,
}

impl SourceData {
    pub fn parse(data: &str) -> Option<Self> {
        let keyfile = KeyFile::new();
        keyfile
            .load_from_data(data, glib::KeyFileFlags::NONE)
            .ok()?;
        Some(Self { keyfile })
    }

    pub fn has_group(&self, group: &str) -> bool {
        self.keyfile.has_group(group)
    }

    pub fn string(&self, group: &str, key: &str) -> Option<String> {
        self.keyfile
            .string(group, key)
            .ok()
            .map(|value| value.to_string())
    }

    pub fn boolean(&self, group: &str, key: &str) -> Option<bool> {
        self.keyfile.boolean(group, key).ok()
    }
}
