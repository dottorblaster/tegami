// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Parser unit tests over the captured EDS `.source` samples.
//!
//! The fixtures under `fixtures/eds/` were captured from a live system
//! (`~/.config/evolution/sources/`) and redacted of personal data; the
//! structure, keys and cross-source references are preserved verbatim.

use accounts::eds::SourceData;
use accounts::eds::enumerate::{Source, mail_accounts};

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
const FASTMAIL_COLLECTION: &str =
    include_str!("fixtures/eds/8c0a229b1ed98a15a2b85b77e332396f082e57ef.source");
const GOOGLE_COLLECTION: &str =
    include_str!("fixtures/eds/ac118492bbfdbb3f5e8f9dbc04de89ce6ab75bd0.source");
const CALENDAR: &str = include_str!("fixtures/eds/system-calendar.source");
const DISABLED_IMAP: &str = include_str!("fixtures/eds/imap-disabled.source");

fn source(uid: &str, data: &str) -> Source {
    Source {
        uid: uid.to_string(),
        data: SourceData::parse(data).unwrap(),
    }
}

fn captured_sources() -> Vec<Source> {
    vec![
        source("c68d7f1afb422dd27c3f4cd098ba2be01b514e58", FASTMAIL_IMAP),
        source(
            "5f17d8b5541804eee23b8902151f9cd17f3a9f97",
            FASTMAIL_IDENTITY,
        ),
        source("07333698cc03117632160cb8de2cc6bf11df61cb", FASTMAIL_SMTP),
        source("6065d502a22c5cde5f8d8d9f390718534b450c4a", GMAIL_IMAP),
        source("0b176a8664eba323b9ee3fef1638b94826bb5e8b", GMAIL_IDENTITY),
        source("d0669aa7f547a31b7460f04a991409ebae129940", GMAIL_SMTP),
        source(
            "8c0a229b1ed98a15a2b85b77e332396f082e57ef",
            FASTMAIL_COLLECTION,
        ),
        source(
            "ac118492bbfdbb3f5e8f9dbc04de89ce6ab75bd0",
            GOOGLE_COLLECTION,
        ),
        source("system-calendar", CALENDAR),
        source("disabled-imap", DISABLED_IMAP),
    ]
}

#[test]
fn captured_imap_account_source() {
    let data = SourceData::parse(FASTMAIL_IMAP).unwrap();
    assert!(data.has_group("Mail Account"));
    assert_eq!(
        data.string("Mail Account", "BackendName").as_deref(),
        Some("imapx")
    );
    assert_eq!(
        data.string("Mail Account", "IdentityUid").as_deref(),
        Some("5f17d8b5541804eee23b8902151f9cd17f3a9f97")
    );
    assert_eq!(
        data.string("Authentication", "Host").as_deref(),
        Some("imap.fastmail.example.org")
    );
    assert_eq!(
        data.string("Authentication", "Method").as_deref(),
        Some("none")
    );
    assert_eq!(
        data.string("Authentication", "Port").as_deref(),
        Some("993")
    );
    assert_eq!(
        data.string("Authentication", "User").as_deref(),
        Some("user@fastmail.example.org")
    );
    assert_eq!(
        data.string("Security", "Method").as_deref(),
        Some("ssl-on-alternate-port")
    );
    assert!(data.has_group("Imapx Backend"));
}

#[test]
fn captured_oauth2_account_source() {
    let data = SourceData::parse(GMAIL_IMAP).unwrap();
    assert_eq!(
        data.string("Authentication", "Method").as_deref(),
        Some("XOAUTH2")
    );
    assert_eq!(
        data.string("Authentication", "Host").as_deref(),
        Some("imap.gmail.example.org")
    );
}

#[test]
fn captured_identity_source() {
    let data = SourceData::parse(FASTMAIL_IDENTITY).unwrap();
    assert!(data.has_group("Mail Identity"));
    assert_eq!(
        data.string("Mail Identity", "Address").as_deref(),
        Some("user@fastmail.example.org")
    );
    assert_eq!(
        data.string("Mail Identity", "Name").as_deref(),
        Some("Fixture User")
    );
    assert_eq!(
        data.string("Mail Submission", "TransportUid").as_deref(),
        Some("07333698cc03117632160cb8de2cc6bf11df61cb")
    );
}

#[test]
fn captured_smtp_transport_source() {
    let data = SourceData::parse(FASTMAIL_SMTP).unwrap();
    assert!(data.has_group("Mail Transport"));
    assert_eq!(
        data.string("Mail Transport", "BackendName").as_deref(),
        Some("smtp")
    );
    assert_eq!(
        data.string("Authentication", "Method").as_deref(),
        Some("PLAIN")
    );
}

#[test]
fn captured_non_mail_sources_are_filtered() {
    for fixture in [FASTMAIL_COLLECTION, GOOGLE_COLLECTION, CALENDAR] {
        let data = SourceData::parse(fixture).unwrap();
        assert!(
            !data.has_group("Mail Account"),
            "unexpected mail account section"
        );
    }
    let disabled = SourceData::parse(DISABLED_IMAP).unwrap();
    assert_eq!(disabled.boolean("Data Source", "Enabled"), Some(false));
}

#[test]
fn captured_set_maps_to_accounts() {
    let accounts = mail_accounts(&captured_sources());

    assert_eq!(accounts.len(), 2);
    let gmail = &accounts[0];
    assert_eq!(gmail.id, "6065d502a22c5cde5f8d8d9f390718534b450c4a");
    assert_eq!(gmail.name, "user@googlemail.example.org");
    assert_eq!(gmail.email_address, "user@googlemail.example.org");
    assert_eq!(gmail.imap.as_ref().unwrap().host, "imap.gmail.example.org");
    let smtp = gmail.smtp.as_ref().unwrap();
    assert_eq!(smtp.host, "smtp.gmail.example.org");
    assert!(smtp.auth_xoauth2);
    assert!(!smtp.auth_plain);

    let fastmail = &accounts[1];
    assert_eq!(fastmail.id, "c68d7f1afb422dd27c3f4cd098ba2be01b514e58");
    assert_eq!(fastmail.name, "Fixture User");
    assert_eq!(fastmail.email_address, "user@fastmail.example.org");
    assert_eq!(
        fastmail.imap.as_ref().unwrap().host,
        "imap.fastmail.example.org"
    );
    let smtp = fastmail.smtp.as_ref().unwrap();
    assert_eq!(smtp.host, "smtp.fastmail.example.org");
    assert!(smtp.auth_plain);
    assert!(!smtp.auth_xoauth2);
}
