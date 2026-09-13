// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use accounts::goa::{AccountProxy, MailProxy, OAuth2BasedProxy, ObjectManagerProxy};
use zbus::connection::{Builder, Connection};
use zbus::interface;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const SERVICE: &str = "org.gnome.OnlineAccounts";
const MANAGER_PATH: &str = "/org/gnome/OnlineAccounts";
const ACCOUNT_PATH: &str = "/org/gnome/OnlineAccounts/Accounts/1";

static BUS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct MockAccount {
    provider_type: String,
    id: String,
    identity: String,
    presentation_identity: String,
    is_temporary: bool,
}

#[interface(name = "org.gnome.OnlineAccounts.Account")]
impl MockAccount {
    #[zbus(property)]
    fn provider_type(&self) -> String {
        self.provider_type.clone()
    }

    #[zbus(property)]
    fn id(&self) -> String {
        self.id.clone()
    }

    #[zbus(property)]
    fn identity(&self) -> String {
        self.identity.clone()
    }

    #[zbus(property)]
    fn presentation_identity(&self) -> String {
        self.presentation_identity.clone()
    }

    #[zbus(property)]
    fn is_temporary(&self) -> bool {
        self.is_temporary
    }

    #[zbus(property)]
    fn set_is_temporary(&mut self, value: bool) {
        self.is_temporary = value;
    }

    #[zbus(property)]
    fn mail_disabled(&self) -> bool {
        false
    }

    fn remove(&self) {}

    fn ensure_credentials(&self) -> i32 {
        3600
    }
}

struct MockMail {
    email_address: String,
    name: String,
    imap_supported: bool,
    smtp_supported: bool,
}

#[interface(name = "org.gnome.OnlineAccounts.Mail")]
impl MockMail {
    #[zbus(property)]
    fn email_address(&self) -> String {
        self.email_address.clone()
    }

    #[zbus(property)]
    fn name(&self) -> String {
        self.name.clone()
    }

    #[zbus(property)]
    fn imap_supported(&self) -> bool {
        self.imap_supported
    }

    #[zbus(property)]
    fn imap_accept_ssl_errors(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn imap_host(&self) -> String {
        "imap.example.org".to_string()
    }

    #[zbus(property)]
    fn imap_use_ssl(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn imap_use_tls(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn imap_user_name(&self) -> String {
        self.email_address.clone()
    }

    #[zbus(property)]
    fn smtp_supported(&self) -> bool {
        self.smtp_supported
    }

    #[zbus(property)]
    fn smtp_accept_ssl_errors(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn smtp_host(&self) -> String {
        "smtp.example.org".to_string()
    }

    #[zbus(property)]
    fn smtp_use_auth(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn smtp_auth_login(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn smtp_auth_plain(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn smtp_auth_xoauth2(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn smtp_use_ssl(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn smtp_use_tls(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn smtp_user_name(&self) -> String {
        self.email_address.clone()
    }
}

struct MockOAuth2;

#[interface(name = "org.gnome.OnlineAccounts.OAuth2Based")]
impl MockOAuth2 {
    #[zbus(property)]
    fn client_id(&self) -> String {
        "mock-client".to_string()
    }

    #[zbus(property)]
    fn client_secret(&self) -> String {
        "mock-secret".to_string()
    }

    fn get_access_token(&self) -> (String, i32) {
        ("mock-token".to_string(), 3600)
    }
}

struct MockObjectManager;

#[interface(name = "org.freedesktop.DBus.ObjectManager")]
impl MockObjectManager {
    fn get_managed_objects(
        &self,
    ) -> HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> {
        let mut account = HashMap::new();
        account.insert(
            "org.gnome.OnlineAccounts.Account".to_string(),
            HashMap::new(),
        );
        let mut account_with_mail = account.clone();
        account_with_mail.insert("org.gnome.OnlineAccounts.Mail".to_string(), HashMap::new());
        account_with_mail.insert(
            "org.gnome.OnlineAccounts.OAuth2Based".to_string(),
            HashMap::new(),
        );
        let mut account_imap_less = account.clone();
        account_imap_less.insert("org.gnome.OnlineAccounts.Mail".to_string(), HashMap::new());

        let mut objects = HashMap::new();
        objects.insert(
            OwnedObjectPath::try_from(ACCOUNT_PATH).unwrap(),
            account_with_mail,
        );
        objects.insert(
            OwnedObjectPath::try_from("/org/gnome/OnlineAccounts/Accounts/2").unwrap(),
            account,
        );
        objects.insert(
            OwnedObjectPath::try_from("/org/gnome/OnlineAccounts/Accounts/3").unwrap(),
            account_imap_less,
        );
        objects
    }
}

async fn session_connection() -> Option<Connection> {
    let builder = match Builder::session() {
        Ok(builder) => builder,
        Err(_) => {
            eprintln!("no session bus available; run via `dbus-run-session -- cargo test`");
            return None;
        }
    };
    builder.name(SERVICE).ok()?.build().await.ok()
}

fn account(id: &str, provider_type: &str, identity: &str) -> MockAccount {
    MockAccount {
        provider_type: provider_type.to_string(),
        id: id.to_string(),
        identity: identity.to_string(),
        presentation_identity: format!("User <{identity}>"),
        is_temporary: false,
    }
}

fn mail(email_address: &str, imap_supported: bool) -> MockMail {
    MockMail {
        email_address: email_address.to_string(),
        name: email_address.to_string(),
        imap_supported,
        smtp_supported: true,
    }
}

async fn serve(conn: &Connection) -> Option<()> {
    let server = conn.object_server();
    server.at(MANAGER_PATH, MockObjectManager).await.ok()?;
    server
        .at(ACCOUNT_PATH, account("1", "imap_smtp", "mock@example.org"))
        .await
        .ok()?;
    server
        .at(ACCOUNT_PATH, mail("mock@example.org", true))
        .await
        .ok()?;
    server.at(ACCOUNT_PATH, MockOAuth2).await.ok()?;
    server
        .at(
            "/org/gnome/OnlineAccounts/Accounts/2",
            account("2", "exchange", "other@example.org"),
        )
        .await
        .ok()?;
    server
        .at(
            "/org/gnome/OnlineAccounts/Accounts/3",
            account("3", "google", "gmail@example.org"),
        )
        .await
        .ok()?;
    server
        .at(
            "/org/gnome/OnlineAccounts/Accounts/3",
            mail("gmail@example.org", false),
        )
        .await
        .ok()?;
    Some(())
}

async fn setup() -> Option<Connection> {
    let conn = session_connection().await?;
    serve(&conn).await?;
    Some(conn)
}

async fn busy_setup() -> Option<(Connection, tokio::sync::MutexGuard<'static, ()>)> {
    let _guard = BUS_LOCK.lock().await;
    let conn = setup().await?;
    Some((conn, _guard))
}

#[tokio::test]
async fn account_interface() {
    let Some((conn, _guard)) = busy_setup().await else {
        return;
    };

    let account = AccountProxy::builder(&conn)
        .path(ACCOUNT_PATH)
        .unwrap()
        .build()
        .await
        .unwrap();

    assert_eq!(account.provider_type().await.unwrap(), "imap_smtp");
    assert_eq!(account.id().await.unwrap(), "1");
    assert_eq!(account.identity().await.unwrap(), "mock@example.org");
    assert_eq!(
        account.presentation_identity().await.unwrap(),
        "User <mock@example.org>"
    );
    assert!(!account.is_temporary().await.unwrap());
    account.set_is_temporary(true).await.unwrap();
    assert!(account.is_temporary().await.unwrap());
    assert_eq!(account.ensure_credentials().await.unwrap(), 3600);
}

#[tokio::test]
async fn mail_interface() {
    let Some((conn, _guard)) = busy_setup().await else {
        return;
    };

    let mail = MailProxy::builder(&conn)
        .path(ACCOUNT_PATH)
        .unwrap()
        .build()
        .await
        .unwrap();

    assert_eq!(mail.email_address().await.unwrap(), "mock@example.org");
    assert_eq!(mail.name().await.unwrap(), "mock@example.org");
    assert!(mail.imap_supported().await.unwrap());
    assert_eq!(mail.imap_host().await.unwrap(), "imap.example.org");
    assert!(mail.imap_use_ssl().await.unwrap());
    assert_eq!(mail.imap_user_name().await.unwrap(), "mock@example.org");
    assert_eq!(mail.smtp_host().await.unwrap(), "smtp.example.org");
}

#[tokio::test]
async fn oauth2_interface() {
    let Some((conn, _guard)) = busy_setup().await else {
        return;
    };

    let oauth2 = OAuth2BasedProxy::builder(&conn)
        .path(ACCOUNT_PATH)
        .unwrap()
        .build()
        .await
        .unwrap();

    assert_eq!(oauth2.client_id().await.unwrap(), "mock-client");
    assert_eq!(oauth2.client_secret().await.unwrap(), "mock-secret");
    assert_eq!(
        oauth2.get_access_token().await.unwrap(),
        ("mock-token".to_string(), 3600)
    );
}

#[tokio::test]
async fn object_manager() {
    let Some((conn, _guard)) = busy_setup().await else {
        return;
    };

    let manager = ObjectManagerProxy::builder(&conn).build().await.unwrap();

    let objects = manager.get_managed_objects().await.unwrap();
    assert!(objects.contains_key(&OwnedObjectPath::try_from(ACCOUNT_PATH).unwrap()));
    assert!(
        objects.contains_key(
            &OwnedObjectPath::try_from("/org/gnome/OnlineAccounts/Accounts/2").unwrap()
        )
    );
}

#[tokio::test]
async fn enumerate_mail_accounts() {
    let Some((conn, _guard)) = busy_setup().await else {
        return;
    };

    let accounts = accounts::goa::enumerate_mail_accounts(&conn).await.unwrap();

    assert_eq!(accounts.len(), 2);

    let first = &accounts[0];
    assert_eq!(first.id, "1");
    assert_eq!(first.name, "mock@example.org");
    assert_eq!(first.email_address, "mock@example.org");
    assert_eq!(first.provider_type.as_deref(), Some("imap_smtp"));
    assert!(!first.is_temporary);
    let imap = first.imap.as_ref().unwrap();
    assert_eq!(imap.host, "imap.example.org");
    assert_eq!(imap.user_name, "mock@example.org");
    assert!(imap.use_ssl);
    assert!(!imap.use_tls);
    assert!(!imap.accept_ssl_errors);
    let smtp = first.smtp.as_ref().unwrap();
    assert_eq!(smtp.host, "smtp.example.org");
    assert_eq!(smtp.user_name, "mock@example.org");
    assert!(smtp.use_auth);
    assert!(smtp.auth_xoauth2);
    assert!(!smtp.auth_plain);
    assert!(smtp.use_ssl);
    assert!(!smtp.use_tls);

    let second = &accounts[1];
    assert_eq!(second.id, "3");
    assert_eq!(second.name, "gmail@example.org");
    assert_eq!(second.email_address, "gmail@example.org");
    assert_eq!(second.provider_type.as_deref(), Some("google"));
    assert!(second.imap.is_none());
    assert!(second.smtp.is_some());
}
