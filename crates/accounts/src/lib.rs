// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Account discovery and credential handling for Tegami.
//!
//! Built on top of GNOME Online Accounts (GOA): it enumerates the mail
//! accounts exposed by GOA's D-Bus API, maps them to an
//! [`AccountConfig`] model and resolves passwords through the Secret
//! Service. UI-free on purpose, so the logic can be tested headless.

pub mod goa;
