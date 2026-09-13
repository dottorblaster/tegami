// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Tests over the captured GOA `GetManagedObjects` sample.
//!
//! The fixture under `fixtures/goa/managed-objects.json` was captured
//! from a live `org.gnome.OnlineAccounts` daemon and records the object
//! paths with their interface names; it mixes mail and non-mail objects
//! so the object filter can be exercised against real data.

use std::collections::HashMap;

use accounts::goa::enumerate::is_mail_object;
use serde_json::Value;
use zbus::zvariant::OwnedValue;

const MANAGED_OBJECTS: &str = include_str!("fixtures/goa/managed-objects.json");

fn interfaces(names: &Value) -> HashMap<String, HashMap<String, OwnedValue>> {
    names
        .as_array()
        .unwrap()
        .iter()
        .map(|name| (name.as_str().unwrap().to_string(), HashMap::new()))
        .collect()
}

#[test]
fn captured_managed_objects_mail_filter() {
    let objects: Value = serde_json::from_str(MANAGED_OBJECTS).unwrap();
    let objects = objects.as_object().unwrap();

    let mail_accounts: Vec<&str> = objects
        .iter()
        .filter(|(_, names)| is_mail_object(&interfaces(names)))
        .map(|(path, _)| path.as_str())
        .collect();

    assert_eq!(
        mail_accounts,
        vec![
            "/org/gnome/OnlineAccounts/Accounts/account_1767350577_0",
            "/org/gnome/OnlineAccounts/Accounts/account_1783762501_0",
        ]
    );
}

#[test]
fn captured_managed_objects_oauth2_detection() {
    let objects: Value = serde_json::from_str(MANAGED_OBJECTS).unwrap();
    let objects = objects.as_object().unwrap();

    let password_based = objects
        .get("/org/gnome/OnlineAccounts/Accounts/account_1767350577_0")
        .unwrap();
    assert!(!interfaces(password_based).contains_key("org.gnome.OnlineAccounts.OAuth2Based"));

    let oauth2 = objects
        .get("/org/gnome/OnlineAccounts/Accounts/account_1783762501_0")
        .unwrap();
    assert!(interfaces(oauth2).contains_key("org.gnome.OnlineAccounts.OAuth2Based"));
}

#[test]
fn captured_managed_objects_excludes_non_mail() {
    let objects: Value = serde_json::from_str(MANAGED_OBJECTS).unwrap();
    let objects = objects.as_object().unwrap();

    let contacts_only = objects
        .get("/org/gnome/OnlineAccounts/Accounts/account_1711276543_15")
        .unwrap();
    assert!(!is_mail_object(&interfaces(contacts_only)));

    let manager = objects.get("/org/gnome/OnlineAccounts/Manager").unwrap();
    assert!(!is_mail_object(&interfaces(manager)));
}
