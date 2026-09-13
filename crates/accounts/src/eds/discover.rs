// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! On-disk ESource fallback discovery.
//!
//! Evolution persists every source as a key-file in
//! `~/.config/evolution/sources/<uid>.source`, in the exact format
//! served over D-Bus. When the Sources5 service is unavailable — for
//! example inside a container or a headless session without the EDS
//! daemon — the same sources can be read from disk with the same
//! parsing and mapping pipeline.

use std::path::Path;

use zbus::connection::Connection;
use zbus::fdo;

use mail_core::account::AccountConfig;

use crate::eds::enumerate::{Source, mail_accounts};
use crate::eds::parse::SourceData;

pub fn default_sources_dir() -> std::path::PathBuf {
    glib::user_config_dir().join("evolution").join("sources")
}

pub async fn discover_mail_accounts(
    connection: &Connection,
    sources_dir: impl AsRef<Path>,
) -> zbus::Result<Vec<AccountConfig>> {
    match crate::eds::enumerate::enumerate_mail_accounts(connection).await {
        Ok(accounts) => Ok(accounts),
        Err(err) if service_unavailable(&err) => Ok(mail_accounts_from_dir(sources_dir)),
        Err(err) => Err(err),
    }
}

pub fn mail_accounts_from_dir(dir: impl AsRef<Path>) -> Vec<AccountConfig> {
    let sources = match std::fs::read_dir(dir.as_ref()) {
        Ok(sources) => sources
            .flatten()
            .filter_map(|entry| source_file(entry.path()))
            .collect(),
        Err(_) => Vec::new(),
    };
    mail_accounts(&sources)
}

fn source_file(path: std::path::PathBuf) -> Option<Source> {
    let uid = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".source"))
        .map(str::to_string)?;
    let data = std::fs::read_to_string(&path).ok()?;
    Some(Source {
        uid,
        data: SourceData::parse(&data)?,
    })
}

fn service_unavailable(err: &zbus::Error) -> bool {
    matches!(
        err,
        zbus::Error::MethodError(name, _, _)
            if name.as_str() == "org.freedesktop.DBus.Error.ServiceUnknown"
    ) || matches!(
        err,
        zbus::Error::FDO(err) if matches!(err.as_ref(), fdo::Error::ServiceUnknown(_))
    )
}
