// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Proxy for the GOA `org.gnome.OnlineAccounts.Account` interface.

#[zbus::proxy(
    interface = "org.gnome.OnlineAccounts.Account",
    default_service = "org.gnome.OnlineAccounts"
)]
pub trait Account {
    #[zbus(property)]
    fn provider_type(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn provider_name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn provider_icon(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn is_locked(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn is_temporary(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_is_temporary(&self, value: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn attention_needed(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn identity(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn presentation_identity(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn mail_disabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_mail_disabled(&self, value: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn calendar_disabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_calendar_disabled(&self, value: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn contacts_disabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_contacts_disabled(&self, value: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn chat_disabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_chat_disabled(&self, value: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn documents_disabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_documents_disabled(&self, value: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn maps_disabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_maps_disabled(&self, value: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn music_disabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_music_disabled(&self, value: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn printers_disabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_printers_disabled(&self, value: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn photos_disabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_photos_disabled(&self, value: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn files_disabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_files_disabled(&self, value: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn ticketing_disabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_ticketing_disabled(&self, value: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn todo_disabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_todo_disabled(&self, value: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn read_later_disabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_read_later_disabled(&self, value: bool) -> zbus::Result<()>;

    fn remove(&self) -> zbus::Result<()>;

    fn ensure_credentials(&self) -> zbus::Result<i32>;
}
