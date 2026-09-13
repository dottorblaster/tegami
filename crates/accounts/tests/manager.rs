// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use accounts::goa::MailAccount;
use accounts::manager::merge;
use accounts::model::AccountChange;
use mail_core::account::{AccountConfig, ImapConfig, SmtpConfig};
use zbus::connection::{Builder, Connection};
use zbus::interface;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const GOA_SERVICE: &str = "org.gnome.OnlineAccounts";
const GOA_MANAGER_PATH: &str = "/org/gnome/OnlineAccounts";
const GOA_ACCOUNT_PATH: &str = "/org/gnome/OnlineAccounts/Accounts";
const EDS_SERVICE: &str = "org.gnome.evolution.dataserver.Sources5";
const EDS_MANAGER_PATH: &str = "/org/gnome/evolution/dataserver/SourceManager";

static BUS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Debug, Default)]
struct GoaEntry {
    id: String,
    email: String,
    provider: String,
    oauth2: bool,
}

#[derive(Clone, Debug, Default)]
struct EdsEntry {
    uid: String,
    email: String,
    host: String,
}

#[derive(Default)]
struct MockState {
    goa: HashMap<String, GoaEntry>,
    eds: HashMap<String, EdsEntry>,
}

fn goa_entry(id: &str, email: &str, provider: &str, oauth2: bool) -> GoaEntry {
    GoaEntry {
        id: id.to_string(),
        email: email.to_string(),
        provider: provider.to_string(),
        oauth2,
    }
}

fn eds_entry(uid: &str, email: &str, host: &str) -> EdsEntry {
    EdsEntry {
        uid: uid.to_string(),
        email: email.to_string(),
        host: host.to_string(),
    }
}

fn goa_entry_state(state: &Mutex<MockState>, id: &str) -> GoaEntry {
    state.lock().unwrap().goa.get(id).cloned().unwrap()
}

struct MockGoaManager {
    state: Arc<Mutex<MockState>>,
}

type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

#[interface(name = "org.freedesktop.DBus.ObjectManager")]
impl MockGoaManager {
    fn get_managed_objects(&self) -> ManagedObjects {
        let state = self.state.lock().unwrap();
        state
            .goa
            .iter()
            .map(|(id, entry)| goa_object(id, entry.oauth2))
            .collect()
    }
}

fn goa_object(
    id: &str,
    oauth2: bool,
) -> (
    OwnedObjectPath,
    HashMap<String, HashMap<String, OwnedValue>>,
) {
    let path = OwnedObjectPath::try_from(format!("{GOA_ACCOUNT_PATH}/{id}")).unwrap();
    let mut interfaces = HashMap::new();
    interfaces.insert(
        "org.gnome.OnlineAccounts.Account".to_string(),
        HashMap::new(),
    );
    interfaces.insert("org.gnome.OnlineAccounts.Mail".to_string(), HashMap::new());
    if oauth2 {
        interfaces.insert(
            "org.gnome.OnlineAccounts.OAuth2Based".to_string(),
            HashMap::new(),
        );
    }
    (path, interfaces)
}

struct MockEdsManager {
    state: Arc<Mutex<MockState>>,
}

#[interface(name = "org.freedesktop.DBus.ObjectManager")]
impl MockEdsManager {
    fn get_managed_objects(&self) -> ManagedObjects {
        let state = self.state.lock().unwrap();
        state
            .eds
            .keys()
            .map(|uid| {
                let path =
                    OwnedObjectPath::try_from(format!("{EDS_MANAGER_PATH}/Source_{uid}")).unwrap();
                let mut interfaces = HashMap::new();
                interfaces.insert(
                    "org.gnome.evolution.dataserver.Source".to_string(),
                    HashMap::new(),
                );
                (path, interfaces)
            })
            .collect()
    }
}

struct MockGoaAccount {
    state: Arc<Mutex<MockState>>,
    id: String,
}

#[interface(name = "org.gnome.OnlineAccounts.Account")]
impl MockGoaAccount {
    #[zbus(property)]
    fn provider_type(&self) -> String {
        goa_entry_state(&self.state, &self.id).provider
    }

    #[zbus(property)]
    fn id(&self) -> String {
        goa_entry_state(&self.state, &self.id).id
    }

    #[zbus(property)]
    fn is_temporary(&self) -> bool {
        false
    }
}

struct MockGoaMail {
    state: Arc<Mutex<MockState>>,
    id: String,
}

#[interface(name = "org.gnome.OnlineAccounts.Mail")]
impl MockGoaMail {
    #[zbus(property)]
    fn email_address(&self) -> String {
        goa_entry_state(&self.state, &self.id).email
    }

    #[zbus(property)]
    fn name(&self) -> String {
        goa_entry_state(&self.state, &self.id).email
    }

    #[zbus(property)]
    fn imap_supported(&self) -> bool {
        true
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
        goa_entry_state(&self.state, &self.id).email
    }

    #[zbus(property)]
    fn smtp_supported(&self) -> bool {
        true
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
        !self.oauth2()
    }

    #[zbus(property)]
    fn smtp_auth_xoauth2(&self) -> bool {
        self.oauth2()
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
        goa_entry_state(&self.state, &self.id).email
    }
}

impl MockGoaMail {
    fn oauth2(&self) -> bool {
        goa_entry_state(&self.state, &self.id).oauth2
    }
}

struct MockEdsSource {
    state: Arc<Mutex<MockState>>,
    uid: String,
}

#[interface(name = "org.gnome.evolution.dataserver.Source")]
impl MockEdsSource {
    #[zbus(property, name = "UID")]
    fn uid(&self) -> String {
        self.state
            .lock()
            .unwrap()
            .eds
            .get(&self.uid)
            .cloned()
            .unwrap()
            .uid
    }

    #[zbus(property)]
    fn data(&self) -> String {
        let entry = self
            .state
            .lock()
            .unwrap()
            .eds
            .get(&self.uid)
            .cloned()
            .unwrap();
        format!(
            "[Data Source]\nDisplayName={email}\nEnabled=true\nParent=\n\n[Mail Account]\nBackendName=imapx\nIdentityUid=\n\n[Authentication]\nHost={host}\nMethod=PLAIN\nPort=993\nUser={email}\n\n[Security]\nMethod=ssl-on-alternate-port\n",
            email = entry.email,
            host = entry.host,
        )
    }
}

async fn session(
    goa: &[GoaEntry],
    eds: &[EdsEntry],
) -> Option<(Connection, Arc<Mutex<MockState>>)> {
    let builder = match Builder::session() {
        Ok(builder) => builder,
        Err(_) => {
            eprintln!("no session bus available; run via `dbus-run-session -- cargo test`");
            return None;
        }
    };
    let conn = builder
        .name(GOA_SERVICE)
        .ok()?
        .name(EDS_SERVICE)
        .ok()?
        .build()
        .await
        .ok()?;

    let state = Arc::new(Mutex::new(MockState {
        goa: goa
            .iter()
            .map(|entry| (entry.id.clone(), entry.clone()))
            .collect(),
        eds: eds
            .iter()
            .map(|entry| (entry.uid.clone(), entry.clone()))
            .collect(),
    }));
    let server = conn.object_server();
    server
        .at(
            GOA_MANAGER_PATH,
            MockGoaManager {
                state: Arc::clone(&state),
            },
        )
        .await
        .ok()?;
    server
        .at(
            EDS_MANAGER_PATH,
            MockEdsManager {
                state: Arc::clone(&state),
            },
        )
        .await
        .ok()?;
    for entry in goa {
        serve_goa_account(&conn, &state, &entry.id).await?;
    }
    for entry in eds {
        serve_eds_source(&conn, &state, &entry.uid).await?;
    }
    Some((conn, state))
}

async fn serve_goa_account(
    conn: &Connection,
    state: &Arc<Mutex<MockState>>,
    id: &str,
) -> Option<()> {
    let path = format!("{GOA_ACCOUNT_PATH}/{id}");
    conn.object_server()
        .at(
            path.as_str(),
            MockGoaAccount {
                state: Arc::clone(state),
                id: id.to_string(),
            },
        )
        .await
        .ok()?;
    conn.object_server()
        .at(
            path.as_str(),
            MockGoaMail {
                state: Arc::clone(state),
                id: id.to_string(),
            },
        )
        .await
        .ok()?;
    Some(())
}

async fn serve_eds_source(
    conn: &Connection,
    state: &Arc<Mutex<MockState>>,
    uid: &str,
) -> Option<()> {
    conn.object_server()
        .at(
            format!("{EDS_MANAGER_PATH}/Source_{uid}"),
            MockEdsSource {
                state: Arc::clone(state),
                uid: uid.to_string(),
            },
        )
        .await
        .ok()?;
    Some(())
}

fn goa_mail_account(entry: &GoaEntry) -> MailAccount {
    MailAccount {
        config: AccountConfig {
            id: entry.id.clone(),
            name: entry.email.clone(),
            email_address: entry.email.clone(),
            provider_type: Some(entry.provider.clone()),
            is_temporary: false,
            imap: Some(ImapConfig {
                accept_ssl_errors: false,
                host: "imap.example.org".to_string(),
                use_ssl: true,
                use_tls: false,
                user_name: entry.email.clone(),
            }),
            smtp: Some(SmtpConfig {
                accept_ssl_errors: false,
                host: "smtp.example.org".to_string(),
                use_auth: true,
                auth_login: false,
                auth_plain: !entry.oauth2,
                auth_xoauth2: entry.oauth2,
                use_ssl: true,
                use_tls: false,
                user_name: entry.email.clone(),
            }),
        },
        path: OwnedObjectPath::try_from(format!("{GOA_ACCOUNT_PATH}/{}", entry.id)).unwrap(),
        oauth2: entry.oauth2,
    }
}

#[test]
fn merge_dedup_by_email() {
    let goa = vec![
        goa_mail_account(&goa_entry("1", "shared@example.org", "google", true)),
        goa_mail_account(&goa_entry("2", "goa-only@example.org", "imap_smtp", false)),
    ];
    let eds = vec![
        AccountConfig {
            id: "eds-shared".to_string(),
            name: "eds name".to_string(),
            email_address: "SHARED@example.org".to_string(),
            provider_type: None,
            is_temporary: false,
            imap: Some(ImapConfig {
                accept_ssl_errors: false,
                host: "imap.eds.org".to_string(),
                use_ssl: true,
                use_tls: false,
                user_name: "shared@example.org".to_string(),
            }),
            smtp: None,
        },
        AccountConfig {
            id: "eds-only".to_string(),
            name: "eds-only@example.org".to_string(),
            email_address: "eds-only@example.org".to_string(),
            provider_type: None,
            is_temporary: false,
            imap: Some(ImapConfig {
                accept_ssl_errors: false,
                host: "imap.eds.org".to_string(),
                use_ssl: true,
                use_tls: false,
                user_name: "eds-only@example.org".to_string(),
            }),
            smtp: None,
        },
    ];

    let accounts = merge(&goa, &eds);

    assert_eq!(accounts.len(), 3);
    let shared = &accounts[0];
    assert_eq!(shared.id(), "1");
    assert_eq!(shared.config.email_address, "SHARED@example.org");
    assert_eq!(shared.config.name, "eds name");
    assert_eq!(
        shared.goa.as_ref().unwrap().account_path,
        format!("{GOA_ACCOUNT_PATH}/1")
    );
    assert!(shared.goa.as_ref().unwrap().oauth2);
    assert_eq!(shared.eds.as_ref().unwrap().uid, "eds-shared");
    assert_eq!(shared.config.imap.as_ref().unwrap().host, "imap.eds.org");
    assert!(shared.config.smtp.as_ref().unwrap().auth_xoauth2);

    let goa_only = &accounts[1];
    assert_eq!(goa_only.id(), "2");
    assert!(goa_only.goa.is_some());
    assert!(goa_only.eds.is_none());

    let eds_only = &accounts[2];
    assert_eq!(eds_only.id(), "eds-only");
    assert!(eds_only.goa.is_none());
    assert_eq!(eds_only.eds.as_ref().unwrap().uid, "eds-only");
}

#[tokio::test]
async fn merged_accounts_from_services() {
    let _guard = BUS_LOCK.lock().await;
    let goa = vec![
        goa_entry("1", "shared@example.org", "google", true),
        goa_entry("2", "goa-only@example.org", "imap_smtp", false),
    ];
    let eds = vec![eds_entry(
        "eds-shared",
        "SHARED@example.org",
        "imap.example.org",
    )];
    let Some((conn, _state)) = session(&goa, &eds).await else {
        return;
    };

    let manager = accounts::AccountManager::new(conn);
    let accounts = manager.accounts().await;

    assert_eq!(accounts.len(), 3);
    assert_eq!(accounts[0].id(), "1");
    assert_eq!(accounts[0].eds.as_ref().unwrap().uid, "eds-shared");
    assert_eq!(accounts[0].config.email_address, "SHARED@example.org");
    assert_eq!(accounts[1].goa.as_ref().unwrap().provider_type, "imap_smtp");
    assert!(accounts[1].eds.is_none());
    assert_eq!(accounts[2].id(), "eds-only");
}

#[tokio::test]
async fn account_change_signals() {
    let _guard = BUS_LOCK.lock().await;
    let goa = vec![goa_entry("1", "change@example.org", "imap_smtp", false)];
    let Some((conn, state)) = session(&goa, &[]).await else {
        return;
    };

    let manager = accounts::AccountManager::new(conn.clone());
    let mut changes = manager.changes().await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let added = goa_entry("3", "added@example.org", "imap_smtp", false);
    state
        .lock()
        .unwrap()
        .goa
        .insert("3".to_string(), added.clone());
    serve_goa_account(&conn, &state, "3").await.unwrap();
    conn.emit_signal::<&str, &str, &str, &str, _>(
        None,
        GOA_MANAGER_PATH,
        "org.freedesktop.DBus.ObjectManager",
        "InterfacesAdded",
        &(
            OwnedObjectPath::try_from(format!("{GOA_ACCOUNT_PATH}/3")).unwrap(),
            goa_object("3", added.oauth2).1,
        ),
    )
    .await
    .unwrap();

    match next_change(&mut changes).await {
        AccountChange::Added(account) => assert_eq!(account.id(), "3"),
        other => panic!("expected Added, got {other:?}"),
    }

    state.lock().unwrap().goa.get_mut("1").unwrap().email = "modified@example.org".to_string();
    conn.emit_signal::<&str, String, &str, &str, _>(
        None,
        format!("{GOA_ACCOUNT_PATH}/1"),
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
        &(
            "org.gnome.OnlineAccounts.Mail".to_string(),
            HashMap::<String, OwnedValue>::new(),
            Vec::<String>::new(),
        ),
    )
    .await
    .unwrap();

    match next_change(&mut changes).await {
        AccountChange::Modified(account) => {
            assert_eq!(account.id(), "1");
            assert_eq!(account.config.email_address, "modified@example.org");
        }
        other => panic!("expected Modified, got {other:?}"),
    }

    state.lock().unwrap().goa.remove("1");
    conn.emit_signal::<&str, &str, &str, &str, _>(
        None,
        GOA_MANAGER_PATH,
        "org.freedesktop.DBus.ObjectManager",
        "InterfacesRemoved",
        &(
            OwnedObjectPath::try_from(format!("{GOA_ACCOUNT_PATH}/1")).unwrap(),
            vec!["org.gnome.OnlineAccounts.Mail".to_string()],
        ),
    )
    .await
    .unwrap();

    match next_change(&mut changes).await {
        AccountChange::Removed(id) => assert_eq!(id, "1"),
        other => panic!("expected Removed, got {other:?}"),
    }
}

async fn next_change(changes: &mut tokio::sync::mpsc::Receiver<AccountChange>) -> AccountChange {
    tokio::time::timeout(Duration::from_secs(5), changes.recv())
        .await
        .expect("timed out waiting for change event")
        .expect("change channel closed")
}
