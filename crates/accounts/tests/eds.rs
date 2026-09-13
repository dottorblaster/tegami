// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use accounts::eds::SourceData;
use accounts::eds::enumerate::{Source, enumerate_mail_accounts, mail_accounts};
use zbus::connection::{Builder, Connection};
use zbus::interface;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const SERVICE: &str = "org.gnome.evolution.dataserver.Sources5";
const MANAGER_PATH: &str = "/org/gnome/evolution/dataserver/SourceManager";

static BUS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const FASTMAIL_IMAP: &str = "\
[Data Source]
DisplayName=alessio@dottorblaster.it
Enabled=true
Parent=collection-fastmail

[Mail Account]
BackendName=imapx
IdentityUid=fastmail-identity

[Authentication]
Host=imap.fastmail.com
Method=none
Port=993
User=alessio@dottorblaster.it

[Security]
Method=ssl-on-alternate-port
";

const FASTMAIL_IDENTITY: &str = "\
[Data Source]
DisplayName=alessio@dottorblaster.it
Enabled=true
Parent=collection-fastmail

[Mail Submission]
TransportUid=fastmail-smtp

[Mail Identity]
Address=alessio@dottorblaster.it
Name=Alessio Biancalana
";

const FASTMAIL_SMTP: &str = "\
[Data Source]
DisplayName=alessio@dottorblaster.it
Enabled=true
Parent=collection-fastmail

[Authentication]
Host=smtp.fastmail.com
Method=PLAIN
Port=465
User=alessio@dottorblaster.it

[Security]
Method=ssl-on-alternate-port

[Mail Transport]
BackendName=smtp
";

const GMAIL_IMAP: &str = "\
[Data Source]
DisplayName=alessio.biancalana@suse.com
Enabled=true
Parent=collection-google

[Mail Account]
BackendName=imapx
IdentityUid=gmail-identity

[Authentication]
Host=imap.gmail.com
Method=XOAUTH2
Port=993
User=alessio.biancalana@suse.com

[Security]
Method=ssl-on-alternate-port
";

const GMAIL_IDENTITY: &str = "\
[Data Source]
DisplayName=alessio.biancalana@suse.com
Enabled=true
Parent=collection-google

[Mail Submission]
TransportUid=gmail-smtp

[Mail Identity]
Address=alessio.biancalana@suse.com
Name=Suse Gmail
";

const GMAIL_SMTP: &str = "\
[Data Source]
DisplayName=alessio.biancalana@suse.com
Enabled=true
Parent=collection-google

[Authentication]
Host=smtp.gmail.com
Method=XOAUTH2
Port=465
User=alessio.biancalana@suse.com

[Security]
Method=ssl-on-alternate-port

[Mail Transport]
BackendName=smtp
";

const CALENDAR: &str = "\
[Data Source]
DisplayName=Personal
Enabled=true
Parent=collection-local

[Calendar]
BackendName=local
";

const DISABLED_IMAP: &str = "\
[Data Source]
DisplayName=old@example.org
Enabled=false
Parent=collection-local

[Mail Account]
BackendName=imapx
IdentityUid=disabled-identity

[Authentication]
Host=imap.example.org
Method=PLAIN
Port=993
User=old@example.org

[Security]
Method=ssl-on-alternate-port
";

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

fn sources() -> Vec<Source> {
    vec![
        source("fastmail-imap", FASTMAIL_IMAP),
        source("fastmail-identity", FASTMAIL_IDENTITY),
        source("fastmail-smtp", FASTMAIL_SMTP),
        source("gmail-imap", GMAIL_IMAP),
        source("gmail-identity", GMAIL_IDENTITY),
        source("gmail-smtp", GMAIL_SMTP),
        source("personal-calendar", CALENDAR),
        source("disabled-imap", DISABLED_IMAP),
    ]
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
    let fixtures: Vec<(&str, &str)> = vec![
        ("fastmail-imap", FASTMAIL_IMAP),
        ("fastmail-identity", FASTMAIL_IDENTITY),
        ("fastmail-smtp", FASTMAIL_SMTP),
        ("gmail-imap", GMAIL_IMAP),
        ("gmail-identity", GMAIL_IDENTITY),
        ("gmail-smtp", GMAIL_SMTP),
        ("personal-calendar", CALENDAR),
        ("disabled-imap", DISABLED_IMAP),
    ];
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

    let fastmail = &accounts[0];
    assert_eq!(fastmail.id, "fastmail-imap");
    assert_eq!(fastmail.name, "Alessio Biancalana");
    assert_eq!(fastmail.email_address, "alessio@dottorblaster.it");
    assert!(fastmail.provider_type.is_none());
    assert!(!fastmail.is_temporary);
    let imap = fastmail.imap.as_ref().unwrap();
    assert_eq!(imap.host, "imap.fastmail.com");
    assert_eq!(imap.user_name, "alessio@dottorblaster.it");
    assert!(imap.use_ssl);
    assert!(!imap.use_tls);
    assert!(!imap.accept_ssl_errors);
    let smtp = fastmail.smtp.as_ref().unwrap();
    assert_eq!(smtp.host, "smtp.fastmail.com");
    assert_eq!(smtp.user_name, "alessio@dottorblaster.it");
    assert!(smtp.use_auth);
    assert!(smtp.auth_plain);
    assert!(!smtp.auth_login);
    assert!(!smtp.auth_xoauth2);
    assert!(smtp.use_ssl);

    let gmail = &accounts[1];
    assert_eq!(gmail.id, "gmail-imap");
    assert_eq!(gmail.name, "Suse Gmail");
    assert_eq!(gmail.email_address, "alessio.biancalana@suse.com");
    assert_eq!(gmail.imap.as_ref().unwrap().host, "imap.gmail.com");
    let smtp = gmail.smtp.as_ref().unwrap();
    assert_eq!(smtp.host, "smtp.gmail.com");
    assert!(smtp.use_auth);
    assert!(smtp.auth_xoauth2);
    assert!(!smtp.auth_plain);
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
        Some("imap.fastmail.com")
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
    assert_eq!(accounts[0].id, "fastmail-imap");
    assert_eq!(accounts[1].id, "gmail-imap");
    assert!(accounts.iter().all(|account| account.id != "disabled-imap"));
}
