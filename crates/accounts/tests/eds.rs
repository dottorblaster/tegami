// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use accounts::eds::SourceData;
use accounts::eds::discover::{discover_mail_accounts, mail_accounts_from_dir};
use accounts::eds::enumerate::{Source, enumerate_mail_accounts, mail_accounts};
use tempfile::TempDir;
use zbus::connection::{Builder, Connection};
use zbus::interface;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const SERVICE: &str = "org.gnome.evolution.dataserver.Sources5";
const MANAGER_PATH: &str = "/org/gnome/evolution/dataserver/SourceManager";

type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

static BUS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const FASTMAIL_IMAP: &str =
    include_str!("fixtures/eds/c68d7f1afb422dd27c3f4cd098ba2be01b514e58.source");
const FASTMAIL_IDENTITY: &str =
    include_str!("fixtures/eds/5f17d8b5541804eee23b8902151f9cd17f3a9f97.source");
const FASTMAIL_SMTP: &str =
    include_str!("fixtures/eds/07333698cc03117632160cb8de2cc6bf11df61cb.source");
const GMAIL_IMAP: &str =
    include_str!("fixtures/eds/6065d502a22c5cde5f8d8d9f390718534b450c4a.source");
const GMAIL_IDENTITY: &str =
    include_str!("fixtures/eds/0b176a8664eba323b9ee3fef1638b94826bb5e8b.source");
const GMAIL_SMTP: &str =
    include_str!("fixtures/eds/d0669aa7f547a31b7460f04a991409ebae129940.source");
const CALENDAR: &str = include_str!("fixtures/eds/system-calendar.source");
const DISABLED_IMAP: &str = include_str!("fixtures/eds/imap-disabled.source");

const FASTMAIL_IMAP_UID: &str = "c68d7f1afb422dd27c3f4cd098ba2be01b514e58";
const FASTMAIL_IDENTITY_UID: &str = "5f17d8b5541804eee23b8902151f9cd17f3a9f97";
const FASTMAIL_SMTP_UID: &str = "07333698cc03117632160cb8de2cc6bf11df61cb";
const GMAIL_IMAP_UID: &str = "6065d502a22c5cde5f8d8d9f390718534b450c4a";
const GMAIL_IDENTITY_UID: &str = "0b176a8664eba323b9ee3fef1638b94826bb5e8b";
const GMAIL_SMTP_UID: &str = "d0669aa7f547a31b7460f04a991409ebae129940";

struct MockSource {
    uid: String,
    data: String,
}

#[interface(name = "org.gnome.evolution.dataserver.Source")]
impl MockSource {
    #[zbus(property, name = "UID")]
    fn uid(&self) -> String {
        self.uid.clone()
    }

    #[zbus(property)]
    fn data(&self) -> String {
        self.data.clone()
    }
}

struct MockObjectManager {
    paths: Vec<String>,
}

#[interface(name = "org.freedesktop.DBus.ObjectManager")]
impl MockObjectManager {
    fn get_managed_objects(
        &self,
    ) -> HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> {
        self.paths
            .iter()
            .map(|path| {
                let mut interfaces = HashMap::new();
                interfaces.insert(
                    "org.gnome.evolution.dataserver.Source".to_string(),
                    HashMap::new(),
                );
                (
                    OwnedObjectPath::try_from(path.as_str()).unwrap(),
                    interfaces,
                )
            })
            .collect()
    }
}

fn source(uid: &str, data: &str) -> Source {
    Source {
        uid: uid.to_string(),
        data: SourceData::parse(data).unwrap(),
    }
}

fn write_sources(dir: &std::path::Path, fixtures: &[(&str, &str)]) {
    for (uid, data) in fixtures {
        std::fs::write(dir.join(format!("{uid}.source")), data).unwrap();
    }
}

fn disk_fixtures() -> Vec<(&'static str, &'static str)> {
    vec![
        (FASTMAIL_IMAP_UID, FASTMAIL_IMAP),
        (FASTMAIL_IDENTITY_UID, FASTMAIL_IDENTITY),
        (FASTMAIL_SMTP_UID, FASTMAIL_SMTP),
        (GMAIL_IMAP_UID, GMAIL_IMAP),
        (GMAIL_IDENTITY_UID, GMAIL_IDENTITY),
        (GMAIL_SMTP_UID, GMAIL_SMTP),
        ("personal-calendar", CALENDAR),
        ("disabled-imap", DISABLED_IMAP),
    ]
}

fn assert_fastmail(account: &mail_core::account::AccountConfig) {
    assert_eq!(account.id, FASTMAIL_IMAP_UID);
    assert_eq!(account.name, "Fixture User");
    assert_eq!(account.email_address, "user@fastmail.example.org");
    let imap = account.imap.as_ref().unwrap();
    assert_eq!(imap.host, "imap.fastmail.example.org");
    assert_eq!(imap.user_name, "user@fastmail.example.org");
    assert!(imap.use_ssl);
    assert!(!imap.use_tls);
    let smtp = account.smtp.as_ref().unwrap();
    assert_eq!(smtp.host, "smtp.fastmail.example.org");
    assert!(smtp.use_auth);
    assert!(smtp.auth_plain);
    assert!(!smtp.auth_xoauth2);
    assert!(smtp.use_ssl);
}

fn assert_gmail(account: &mail_core::account::AccountConfig) {
    assert_eq!(account.id, GMAIL_IMAP_UID);
    assert_eq!(account.name, "user@googlemail.example.org");
    assert_eq!(account.email_address, "user@googlemail.example.org");
    assert!(account.imap.as_ref().unwrap().use_ssl);
    let smtp = account.smtp.as_ref().unwrap();
    assert_eq!(smtp.host, "smtp.gmail.example.org");
    assert!(smtp.use_auth);
    assert!(smtp.auth_xoauth2);
    assert!(!smtp.auth_plain);
}

fn sources() -> Vec<Source> {
    vec![
        source(FASTMAIL_IMAP_UID, FASTMAIL_IMAP),
        source(FASTMAIL_IDENTITY_UID, FASTMAIL_IDENTITY),
        source(FASTMAIL_SMTP_UID, FASTMAIL_SMTP),
        source(GMAIL_IMAP_UID, GMAIL_IMAP),
        source(GMAIL_IDENTITY_UID, GMAIL_IDENTITY),
        source(GMAIL_SMTP_UID, GMAIL_SMTP),
        source("personal-calendar", CALENDAR),
        source("disabled-imap", DISABLED_IMAP),
    ]
}

struct UnavailableObjectManager;

#[interface(name = "org.freedesktop.DBus.ObjectManager")]
impl UnavailableObjectManager {
    fn get_managed_objects(&self) -> Result<ManagedObjects, zbus::fdo::Error> {
        Err(zbus::fdo::Error::ServiceUnknown(SERVICE.to_string()))
    }
}

struct BrokenObjectManager;

#[interface(name = "org.freedesktop.DBus.ObjectManager")]
impl BrokenObjectManager {
    fn get_managed_objects(&self) -> Result<ManagedObjects, zbus::fdo::Error> {
        Err(zbus::fdo::Error::Failed("broken".to_string()))
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
    let fixtures: Vec<(&str, &str)> = disk_fixtures();
    let paths = fixtures
        .iter()
        .enumerate()
        .map(|(index, _)| format!("{MANAGER_PATH}/Source_{index}"))
        .collect();
    server
        .at(MANAGER_PATH, MockObjectManager { paths })
        .await
        .ok()?;
    for (index, (uid, data)) in fixtures.into_iter().enumerate() {
        server
            .at(
                format!("{MANAGER_PATH}/Source_{index}"),
                MockSource {
                    uid: uid.to_string(),
                    data: data.to_string(),
                },
            )
            .await
            .ok()?;
    }
    Some(())
}

#[tokio::test]
async fn enumerate_mail_accounts_from_sources() {
    let _guard = BUS_LOCK.lock().await;
    let Some(conn) = session_connection().await else {
        return;
    };
    let Some(()) = serve(&conn).await else {
        panic!("failed to serve mock Sources5 service");
    };

    let accounts = enumerate_mail_accounts(&conn).await.unwrap();

    assert_eq!(accounts.len(), 2);
    assert_gmail(&accounts[0]);
    assert_fastmail(&accounts[1]);
}

#[test]
fn parse_source_data() {
    let data = SourceData::parse(FASTMAIL_IMAP).unwrap();
    assert!(data.has_group("Mail Account"));
    assert!(data.has_group("Authentication"));
    assert_eq!(
        data.string("Mail Account", "BackendName").as_deref(),
        Some("imapx")
    );
    assert_eq!(data.boolean("Data Source", "Enabled"), Some(true));
    assert_eq!(
        data.string("Authentication", "Host").as_deref(),
        Some("imap.fastmail.example.org")
    );
    assert_eq!(
        data.string("Authentication", "Port").as_deref(),
        Some("993")
    );
    assert_eq!(
        data.string("Security", "Method").as_deref(),
        Some("ssl-on-alternate-port")
    );
    assert!(data.string("Missing", "Key").is_none());
    assert!(SourceData::parse("not a key file").is_none());
}

#[test]
fn maps_sources_to_accounts() {
    let accounts = mail_accounts(&sources());

    assert_eq!(accounts.len(), 2);
    assert_eq!(accounts[0].id, GMAIL_IMAP_UID);
    assert_eq!(accounts[1].id, FASTMAIL_IMAP_UID);
    assert!(accounts.iter().all(|account| account.id != "disabled-imap"));
}

#[test]
fn mail_accounts_from_disk() {
    let dir = TempDir::new().unwrap();
    write_sources(dir.path(), &disk_fixtures());
    std::fs::write(dir.path().join("README"), "not a source").unwrap();

    let accounts = mail_accounts_from_dir(dir.path());

    assert_eq!(accounts.len(), 2);
    assert_gmail(&accounts[0]);
    assert_fastmail(&accounts[1]);
}

#[tokio::test]
async fn discover_falls_back_to_disk() {
    let _guard = BUS_LOCK.lock().await;
    let dir = TempDir::new().unwrap();
    write_sources(dir.path(), &disk_fixtures());
    let Some(conn) = session_connection().await else {
        return;
    };
    conn.object_server()
        .at(MANAGER_PATH, UnavailableObjectManager)
        .await
        .unwrap();

    let accounts = discover_mail_accounts(&conn, dir.path()).await.unwrap();

    assert_eq!(accounts.len(), 2);
    assert_gmail(&accounts[0]);
    assert_fastmail(&accounts[1]);
}

#[tokio::test]
async fn discover_propagates_other_errors() {
    let _guard = BUS_LOCK.lock().await;
    let dir = TempDir::new().unwrap();
    write_sources(dir.path(), &disk_fixtures());
    let Some(conn) = session_connection().await else {
        return;
    };
    conn.object_server()
        .at(MANAGER_PATH, BrokenObjectManager)
        .await
        .unwrap();

    assert!(discover_mail_accounts(&conn, dir.path()).await.is_err());
}
