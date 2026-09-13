// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Proxies for the Sources5 D-Bus interfaces.

use std::collections::HashMap;

use zbus::zvariant::{OwnedObjectPath, OwnedValue};

#[zbus::proxy(
    interface = "org.gnome.evolution.dataserver.Source",
    default_service = "org.gnome.evolution.dataserver.Sources5"
)]
pub trait Source {
    #[zbus(property, name = "UID")]
    fn uid(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn data(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "org.freedesktop.DBus.ObjectManager",
    default_service = "org.gnome.evolution.dataserver.Sources5",
    default_path = "/org/gnome/evolution/dataserver/SourceManager"
)]
pub trait ObjectManager {
    fn get_managed_objects(
        &self,
    ) -> zbus::Result<HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>>;
}
