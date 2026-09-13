// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Password lookup for EDS sources in the Secret Service.
//!
//! EDS persists source passwords through libsecret under the schema
//! `org.gnome.Evolution.Data.Source` (see `e-secret-store.c`), keyed by
//! the source UID in the `e-source-uid` attribute, with the schema name
//! recorded in the `xdg:schema` attribute.

use oo7::Keyring;

/// The secret schema EDS declares for source passwords.
pub const SOURCE_SCHEMA: &str = "org.gnome.Evolution.Data.Source";

/// The attribute carrying the ESource UID.
pub const SOURCE_UID_ATTRIBUTE: &str = "e-source-uid";

/// The attribute libsecret stores the schema name into.
pub const SCHEMA_ATTRIBUTE: &str = "xdg:schema";

#[allow(clippy::result_large_err)]
pub async fn password_for_source(keyring: &Keyring, uid: &str) -> oo7::Result<Option<String>> {
    let items = keyring
        .search_items(&[
            (SCHEMA_ATTRIBUTE, SOURCE_SCHEMA),
            (SOURCE_UID_ATTRIBUTE, uid),
        ])
        .await?;
    for item in items {
        let secret = item.secret().await?;
        if let Ok(password) = String::from_utf8(secret.as_bytes().to_vec()) {
            return Ok(Some(password));
        }
    }
    Ok(None)
}
