// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::FakeBackend;
use mail_core::account::AccountConfig;
use mail_core::envelope::{FlagChange, MessageFlags};
use mail_core::folder::FolderRole;
use mail_core::{Credential, MailBackend};

#[tokio::test]
async fn mail_backend_round_trip() {
    let mut backend = FakeBackend::with_folders(&[("INBOX", 0), ("Archive", 0)]);
    backend.uid_validity = 42;
    backend.idle_supported = true;
    let draft_uid = backend.add_message("INBOX", "hello", b"Subject: hello\r\n\r\nbody");
    backend.add_message("INBOX", "unread", b"Subject: unread\r\n\r\nbody");

    let config = AccountConfig {
        id: "test".to_string(),
        name: "Test".to_string(),
        email_address: "me@example.org".to_string(),
        provider_type: None,
        is_temporary: false,
        imap: None,
        smtp: None,
    };

    backend
        .connect(&config, &Credential::Password("hunter2".to_string()))
        .await
        .unwrap();
    let folders = backend.folders().await.unwrap();
    assert_eq!(folders.len(), 2);
    assert_eq!(folders[0].role, FolderRole::Other);
    assert_eq!(folders[1].id, "INBOX");
    assert_eq!(folders[1].role, FolderRole::Inbox);

    let state = backend.select("INBOX").await.unwrap();
    assert_eq!(state.uid_validity, 42);
    assert_eq!(state.exists, 2);

    let backend_uids = backend.uids("INBOX").await.unwrap();
    assert_eq!(backend_uids, vec![draft_uid, draft_uid + 1]);

    let envelopes = backend
        .fetch_envelopes("INBOX", &backend_uids)
        .await
        .unwrap();
    assert_eq!(envelopes.len(), 2);
    assert_eq!(envelopes[0].subject, "hello");
    assert_eq!(
        envelopes[0].from[0].address.as_deref(),
        Some("sender@example.org")
    );
    assert!(envelopes[0].date.is_some());

    let body = backend.fetch_message("INBOX", draft_uid).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("Subject: hello"));

    let change = FlagChange {
        seen: Some(true),
        flagged: Some(true),
        ..FlagChange::default()
    };
    backend
        .set_flags("INBOX", &[draft_uid], change)
        .await
        .unwrap();
    let envelopes = backend
        .fetch_envelopes("INBOX", &backend_uids)
        .await
        .unwrap();
    let draft = envelopes.iter().find(|e| e.uid == draft_uid).unwrap();
    assert!(draft.flags.seen);
    assert!(draft.flags.flagged);

    backend
        .copy_messages("INBOX", "Archive", &[draft_uid])
        .await
        .unwrap();

    backend
        .move_messages("INBOX", "Archive", &[draft_uid])
        .await
        .unwrap();

    let appended = backend
        .append(
            "Archive",
            MessageFlags {
                draft: true,
                ..MessageFlags::default()
            },
            b"Subject: draft\r\n\r\nwork in progress",
        )
        .await
        .unwrap();
    assert!(appended > 0);

    backend.idle("INBOX").await.unwrap();
    assert_eq!(backend.idle_calls, 1);
    backend.disconnect().await.unwrap();
}
