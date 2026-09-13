// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use accounts::goa::{AccountProxy, MailProxy, OAuth2BasedProxy, ObjectManagerProxy};
use zbus::connection::{Builder, Connection};
use zbus::interface;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const SERVICE: &str = "org.gnome.OnlineAccounts";
const ACCOUNT_PATH: &str = "/org/gnome/OnlineAccounts/Accounts/1";

static BUS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct MockAccount {
    is_temporary: bool,
}

#[interface(name = "org.gnome.OnlineAccounts.Account")]
impl MockAccount {
    #[zbus(property)]
    fn provider_type(&self) -> String {
        "imap_smtp".to_string()
    }

    #[zbus(property)]
    fn id(&self) -> String {
        "1".to_string()
    }

    #[zbus(property)]
    fn identity(&self) -> String {
        "mock@example.org".to_string()
    }

    #[zbus(property)]
    fn presentation_identity(&self) -> String {
        "Mock User <mock@example.org>".to_string()
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

struct MockMail;

#[interface(name = "org.gnome.OnlineAccounts.Mail")]
impl MockMail {
    #[zbus(property)]
    fn email_address(&self) -> String {
        "mock@example.org".to_string()
    }

    #[zbus(property)]
    fn name(&self) -> String {
        "Mock User".to_string()
    }

    #[zbus(property)]
    fn imap_supported(&self) -> bool {
        true
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
    fn imap_user_name(&self) -> String {
        "mock@example.org".to_string()
    }

    #[zbus(property)]
    fn smtp_host(&self) -> String {
        "smtp.example.org".to_string()
    }

    #[zbus(property)]
    fn smtp_user_name(&self) -> String {
        "mock@example.org".to_string()
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
        let mut mail = HashMap::new();
        mail.insert("org.gnome.OnlineAccounts.Mail".to_string(), HashMap::new());
        let mut objects = HashMap::new();
        objects.insert(OwnedObjectPath::try_from(ACCOUNT_PATH).unwrap(), account);
        objects.insert(
            OwnedObjectPath::try_from(format!("{ACCOUNT_PATH}/mail")).unwrap(),
            mail,
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

async fn serve(conn: &Connection) -> Option<()> {
    let server = conn.object_server();
    server
        .at("/org/gnome/OnlineAccounts/Manager", MockObjectManager)
        .await
        .ok()?;
    server
        .at(
            ACCOUNT_PATH,
            MockAccount {
                is_temporary: false,
            },
        )
        .await
        .ok()?;
    server
        .at(format!("{ACCOUNT_PATH}/mail"), MockMail)
        .await
        .ok()?;
    server
        .at(format!("{ACCOUNT_PATH}/oauth2"), MockOAuth2)
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
        "Mock User <mock@example.org>"
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
        .path(format!("{ACCOUNT_PATH}/mail"))
        .unwrap()
        .build()
        .await
        .unwrap();

    assert_eq!(mail.email_address().await.unwrap(), "mock@example.org");
    assert_eq!(mail.name().await.unwrap(), "Mock User");
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
        .path(format!("{ACCOUNT_PATH}/oauth2"))
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
        objects.contains_key(&OwnedObjectPath::try_from(format!("{ACCOUNT_PATH}/mail")).unwrap())
    );
}
