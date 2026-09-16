// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The [`MailBackend`] implementation for IMAP.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_imap::extensions::idle::IdleResponse;
use async_imap::types::{Flag, NameAttribute};
use async_imap::{Client, Session};
use futures_util::StreamExt;
use imap_proto::{Response, ResponseCode, UidSetMember};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::account::AccountConfig;
use crate::backend::Result;
use crate::envelope::{Address, Envelope, FlagChange, MessageFlags};
use crate::{Credential, Folder, FolderDelta, FolderRole, FolderState, MailBackend, MailError};

use super::auth::Xoauth2;

const IDLE_CYCLE: Duration = Duration::from_secs(60);

trait BackendStream: AsyncRead + AsyncWrite + std::fmt::Debug {}
impl<T: AsyncRead + AsyncWrite + std::fmt::Debug + ?Sized> BackendStream for T {}

type Stream = Box<dyn BackendStream + Send + Unpin>;

fn tls_config() -> Result<Arc<rustls::ClientConfig>> {
    let mut roots = rustls::RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    if let Some(error) = native.errors.first() {
        return Err(MailError::Protocol(error.to_string()));
    }
    for cert in native.certs {
        roots
            .add(cert)
            .map_err(|err| MailError::Protocol(err.to_string()))?;
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// A connection to one IMAP server.
pub struct ImapBackend {
    session: Option<Session<Stream>>,
    supports_idle: bool,
    supports_move: bool,
    supports_condstore: bool,
    supports_qresync: bool,
    supports_uidplus: bool,
}

impl ImapBackend {
    pub fn new() -> Self {
        Self {
            session: None,
            supports_idle: false,
            supports_move: false,
            supports_condstore: false,
            supports_qresync: false,
            supports_uidplus: false,
        }
    }

    fn session(&mut self) -> Result<&mut Session<Stream>> {
        self.session.as_mut().ok_or(MailError::Disconnected)
    }

    async fn select_mailbox(&mut self, folder: &str) -> Result<async_imap::types::Mailbox> {
        let supports_condstore = self.supports_condstore;
        let folder = folder.to_string();
        let session = self.session()?;
        if supports_condstore {
            session.select_condstore(folder).await.map_err(map_err)
        } else {
            session.select(folder).await.map_err(map_err)
        }
    }

    async fn mark_deleted(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        let id_set = uid_set(uids);
        self.select_mailbox(folder).await?;
        let session = self.session()?;
        let mut messages = session
            .uid_store(&id_set, "+FLAGS.SILENT (\\Deleted)")
            .await
            .map_err(map_err)?;
        while let Some(message) = messages.next().await {
            message.map_err(map_err)?;
        }
        Ok(())
    }

    async fn expunge(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        let id_set = uid_set(uids);
        self.select_mailbox(folder).await?;
        let supports_uidplus = self.supports_uidplus;
        let session = self.session()?;
        if supports_uidplus
            && session
                .run_command_and_check_ok(format!("UID EXPUNGE {id_set}"))
                .await
                .is_ok()
        {
            return Ok(());
        }
        session
            .run_command_and_check_ok("EXPUNGE")
            .await
            .map_err(map_err)
    }
}

fn map_err(err: async_imap::error::Error) -> MailError {
    MailError::Protocol(err.to_string())
}

impl Default for ImapBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MailBackend for ImapBackend {
    async fn connect(&mut self, config: &AccountConfig, credential: &Credential) -> Result<()> {
        let imap = config
            .imap
            .as_ref()
            .ok_or_else(|| MailError::Protocol("no imap configuration".to_string()))?;
        let port = imap.port.unwrap_or(if imap.use_ssl { 993 } else { 143 });
        let tcp = TcpStream::connect((imap.host.as_str(), port)).await?;
        let connector = tokio_rustls::TlsConnector::from(tls_config()?);
        let server_name = rustls::pki_types::ServerName::try_from(imap.host.clone())
            .map_err(|err| MailError::Protocol(err.to_string()))?;

        let client = if imap.use_ssl {
            let stream = connector.connect(server_name, tcp).await?;
            let mut client = Client::new(Box::new(stream) as Stream);
            if client.read_response().await?.is_none() {
                return Err(MailError::Disconnected);
            }
            client
        } else {
            let mut client = Client::new(Box::new(tcp) as Stream);
            if client.read_response().await?.is_none() {
                return Err(MailError::Disconnected);
            }
            if imap.use_tls {
                client
                    .run_command_and_check_ok("STARTTLS", None)
                    .await
                    .map_err(map_err)?;
                let plain = client.into_inner();
                let stream = connector.connect(server_name, plain).await?;
                client = Client::new(Box::new(stream) as Stream);
            }
            client
        };

        let mut session = match credential {
            Credential::Password(password) => client
                .login(&imap.user_name, password)
                .await
                .map_err(|(err, _)| MailError::Protocol(err.to_string()))?,
            Credential::OAuth2(token) => client
                .authenticate(
                    "XOAUTH2",
                    Xoauth2::new(imap.user_name.clone(), token.clone()),
                )
                .await
                .map_err(|(err, _)| MailError::Protocol(err.to_string()))?,
        };
        let capabilities = session.capabilities().await.map_err(map_err)?;
        let has =
            |name: &str| capabilities.has(&async_imap::types::Capability::Atom(name.to_string()));
        self.supports_idle = has("IDLE");
        self.supports_move = has("MOVE");
        self.supports_condstore = has("CONDSTORE");
        self.supports_qresync = has("QRESYNC");
        self.supports_uidplus = has("UIDPLUS");
        if self.supports_qresync {
            session
                .run_command_and_check_ok("ENABLE QRESYNC")
                .await
                .map_err(map_err)?;
        }
        self.session = Some(session);
        Ok(())
    }

    fn supports_idle(&self) -> bool {
        self.supports_idle
    }

    fn supports_condstore(&self) -> bool {
        self.supports_condstore
    }

    fn supports_qresync(&self) -> bool {
        self.supports_qresync
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(mut session) = self.session.take() {
            session.logout().await.map_err(map_err)?;
        }
        Ok(())
    }

    async fn folders(&mut self) -> Result<Vec<Folder>> {
        let session = self.session()?;
        let names = session.list(Some(""), Some("*")).await.map_err(map_err)?;
        let mut folders = Vec::new();
        futures_util::pin_mut!(names);
        while let Some(name) = names.next().await {
            let name = name.map_err(map_err)?;
            if name
                .attributes()
                .iter()
                .any(|attribute| matches!(attribute, NameAttribute::NoSelect))
            {
                continue;
            }
            folders.push(Folder {
                id: name.name().to_string(),
                name: display_name(name.name()),
                role: folder_role(name.name(), name.attributes()),
            });
        }
        folders.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(folders)
    }

    async fn select(&mut self, folder: &str) -> Result<FolderState> {
        let mailboxes = self.select_mailbox(folder).await?;
        Ok(FolderState {
            uid_validity: mailboxes.uid_validity.unwrap_or(0),
            uid_next: mailboxes.uid_next.unwrap_or(0),
            exists: mailboxes.exists,
            recent: mailboxes.recent,
            unseen: mailboxes.unseen,
            highest_modseq: mailboxes.highest_modseq,
        })
    }

    async fn uids(&mut self, folder: &str) -> Result<Vec<u32>> {
        self.select_mailbox(folder).await?;
        let session = self.session()?;
        let mut uids: Vec<u32> = session
            .uid_search("ALL")
            .await
            .map_err(map_err)?
            .into_iter()
            .collect();
        uids.sort_unstable();
        Ok(uids)
    }

    async fn fetch_envelopes(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<Envelope>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let supports_condstore = self.supports_condstore;
        let id_set = uid_set(uids);
        let query = if supports_condstore {
            "(UID ENVELOPE FLAGS RFC822.SIZE INTERNALDATE MODSEQ)"
        } else {
            "(UID ENVELOPE FLAGS RFC822.SIZE INTERNALDATE)"
        };
        self.select_mailbox(folder).await?;
        let session = self.session()?;
        let messages = session.uid_fetch(id_set, query).await.map_err(map_err)?;
        let mut envelopes = Vec::new();
        futures_util::pin_mut!(messages);
        while let Some(message) = messages.next().await {
            envelopes.push(to_envelope(message.map_err(map_err)?));
        }
        envelopes.sort_by_key(|envelope| envelope.uid);
        Ok(envelopes)
    }

    async fn fetch_delta(&mut self, folder: &str, since_modseq: u64) -> Result<FolderDelta> {
        let query = delta_query(since_modseq, self.supports_qresync);
        self.select_mailbox(folder).await?;
        let session = self.session()?;
        let mut changed = Vec::new();
        {
            let messages = session.uid_fetch("1:*", query).await.map_err(map_err)?;
            futures_util::pin_mut!(messages);
            while let Some(message) = messages.next().await {
                changed.push(to_envelope(message.map_err(map_err)?));
            }
        }
        let mut vanished = Vec::new();
        while let Ok(response) = session.unsolicited_responses.try_recv() {
            if let async_imap::types::UnsolicitedResponse::Other(data) = response
                && let async_imap::imap_proto::Response::Vanished { uids, .. } = data.parsed()
            {
                for range in uids {
                    vanished.extend(range.clone());
                }
            }
        }
        changed.sort_by_key(|envelope| envelope.uid);
        vanished.sort_unstable();
        vanished.dedup();
        Ok(FolderDelta { changed, vanished })
    }

    async fn fetch_message(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        let folder = folder.to_string();
        let session = self.session()?;
        session.select(&folder).await.map_err(map_err)?;
        let messages = session
            .uid_fetch(uid.to_string(), "(RFC822)")
            .await
            .map_err(map_err)?;
        let mut body = None;
        futures_util::pin_mut!(messages);
        while let Some(message) = messages.next().await {
            if let Some(found) = message.map_err(map_err)?.body() {
                body = Some(found.to_vec());
            }
        }
        body.ok_or_else(|| MailError::Protocol(format!("no body for uid {uid}")))
    }

    async fn set_flags(&mut self, folder: &str, uids: &[u32], change: FlagChange) -> Result<()> {
        let folder = folder.to_string();
        let uids = uids.to_vec();
        if uids.is_empty() {
            return Ok(());
        }
        let id_set = uid_set(&uids);
        let session = self.session()?;
        session.select(folder).await.map_err(map_err)?;
        let mut additions = Vec::new();
        let mut removals = Vec::new();
        flag_add_remove(&mut additions, &mut removals, &change.seen, "\\Seen");
        flag_add_remove(
            &mut additions,
            &mut removals,
            &change.answered,
            "\\Answered",
        );
        flag_add_remove(&mut additions, &mut removals, &change.flagged, "\\Flagged");
        flag_add_remove(&mut additions, &mut removals, &change.deleted, "\\Deleted");
        flag_add_remove(&mut additions, &mut removals, &change.draft, "\\Draft");
        if !additions.is_empty() {
            let mut messages = session
                .uid_store(&id_set, format!("+FLAGS ({})", additions.join(" ")))
                .await
                .map_err(map_err)?;
            while let Some(message) = messages.next().await {
                message.map_err(map_err)?;
            }
        }
        if !removals.is_empty() {
            let mut messages = session
                .uid_store(&id_set, format!("-FLAGS ({})", removals.join(" ")))
                .await
                .map_err(map_err)?;
            while let Some(message) = messages.next().await {
                message.map_err(map_err)?;
            }
        }
        Ok(())
    }

    async fn move_messages(&mut self, from: &str, to: &str, uids: &[u32]) -> Result<()> {
        let from = from.to_string();
        let to = to.to_string();
        let uids = uids.to_vec();
        if uids.is_empty() {
            return Ok(());
        }
        let id_set = uid_set(&uids);
        let supports_move = self.supports_move;
        {
            let session = self.session()?;
            session.select(&from).await.map_err(map_err)?;
            if supports_move {
                session.uid_mv(to, &id_set).await.map_err(map_err)?;
                return Ok(());
            }
            session.uid_copy(to, &id_set).await.map_err(map_err)?;
            let mut messages = session
                .uid_store(&id_set, "+FLAGS.SILENT (\\Deleted)")
                .await
                .map_err(map_err)?;
            while let Some(message) = messages.next().await {
                message.map_err(map_err)?;
            }
        }
        self.expunge(&from, &uids).await
    }

    async fn copy_messages(&mut self, from: &str, to: &str, uids: &[u32]) -> Result<()> {
        let from = from.to_string();
        let to = to.to_string();
        let uids = uids.to_vec();
        if uids.is_empty() {
            return Ok(());
        }
        let session = self.session()?;
        session.select(from).await.map_err(map_err)?;
        session
            .uid_copy(to, uid_set(&uids))
            .await
            .map_err(map_err)?;
        Ok(())
    }

    async fn append(&mut self, folder: &str, flags: MessageFlags, raw: &[u8]) -> Result<u32> {
        append_uidplus(self.session()?, folder, &append_flags(flags), raw).await
    }

    async fn delete_permanently(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        self.mark_deleted(folder, uids).await?;
        self.expunge(folder, uids).await
    }

    async fn idle(&mut self, folder: &str) -> Result<()> {
        if !self.supports_idle {
            return Err(MailError::Protocol("IDLE not supported".to_string()));
        }
        self.select_mailbox(folder).await?;
        let session = self.session.take().ok_or(MailError::Disconnected)?;
        let mut idle = session.idle();
        idle.init().await.map_err(map_err)?;
        let (wait, stop_source) = idle.wait_with_timeout(IDLE_CYCLE);
        let response = wait.await.map_err(map_err)?;
        drop(stop_source);
        self.session = Some(idle.done().await.map_err(map_err)?);
        match response {
            IdleResponse::NewData(_) | IdleResponse::Timeout => Ok(()),
            IdleResponse::ManualInterrupt => {
                Err(MailError::Protocol("idle interrupted".to_string()))
            }
        }
    }
}

fn delta_query(since_modseq: u64, qresync: bool) -> String {
    let modifier = if qresync {
        format!("(CHANGEDSINCE {since_modseq} VANISHED)")
    } else {
        format!("(CHANGEDSINCE {since_modseq})")
    };
    format!("(UID ENVELOPE FLAGS RFC822.SIZE INTERNALDATE MODSEQ) {modifier}")
}

fn display_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

fn folder_role(name: &str, attributes: &[NameAttribute<'_>]) -> FolderRole {
    for attribute in attributes {
        let role = match attribute {
            NameAttribute::All => FolderRole::All,
            NameAttribute::Archive => FolderRole::Archive,
            NameAttribute::Drafts => FolderRole::Drafts,
            NameAttribute::Flagged => FolderRole::Flagged,
            NameAttribute::Junk => FolderRole::Junk,
            NameAttribute::Sent => FolderRole::Sent,
            NameAttribute::Trash => FolderRole::Trash,
            NameAttribute::Extension(extension)
                if extension.eq_ignore_ascii_case("\\important") =>
            {
                FolderRole::Important
            }
            _ => continue,
        };
        return role;
    }
    if name.eq_ignore_ascii_case("inbox") {
        return FolderRole::Inbox;
    }
    FolderRole::Other
}

fn to_envelope(fetch: async_imap::types::Fetch) -> Envelope {
    let (subject, in_reply_to, message_id, from, to, cc) = match fetch.envelope() {
        Some(envelope) => (
            cow_to_string(&envelope.subject),
            optional_cow(&envelope.in_reply_to),
            optional_cow(&envelope.message_id),
            addresses(&envelope.from),
            addresses(&envelope.to),
            addresses(&envelope.cc),
        ),
        None => Default::default(),
    };
    Envelope {
        uid: fetch.uid.unwrap_or(0),
        modseq: fetch.modseq,
        flags: flags(&fetch),
        size: fetch.size.unwrap_or(0),
        subject,
        from,
        to,
        cc,
        date: internal_date(&fetch),
        message_id,
        in_reply_to,
        references: Vec::new(),
    }
}

fn flags(fetch: &async_imap::types::Fetch) -> MessageFlags {
    let mut result = MessageFlags::default();
    for flag in fetch.flags() {
        match flag {
            Flag::Seen => result.seen = true,
            Flag::Answered => result.answered = true,
            Flag::Flagged => result.flagged = true,
            Flag::Deleted => result.deleted = true,
            Flag::Draft => result.draft = true,
            _ => {}
        }
    }
    result
}

fn addresses(addresses: &Option<Vec<async_imap::imap_proto::types::Address<'_>>>) -> Vec<Address> {
    addresses
        .as_ref()
        .map(|addresses| addresses.iter().map(to_address).collect())
        .unwrap_or_default()
}

fn to_address(address: &async_imap::imap_proto::types::Address<'_>) -> Address {
    Address {
        name: address
            .name
            .as_ref()
            .map(|name| String::from_utf8_lossy(name).to_string()),
        address: Some(format_mailbox(address)),
    }
}

fn format_mailbox(address: &async_imap::imap_proto::types::Address<'_>) -> String {
    match (&address.mailbox, &address.host) {
        (Some(mailbox), Some(host)) => format!(
            "{}@{}",
            String::from_utf8_lossy(mailbox),
            String::from_utf8_lossy(host)
        ),
        (Some(mailbox), None) => String::from_utf8_lossy(mailbox).to_string(),
        _ => String::new(),
    }
}

fn internal_date(fetch: &async_imap::types::Fetch) -> Option<SystemTime> {
    let date = fetch.internal_date()?;
    Some(UNIX_EPOCH + Duration::from_secs(date.timestamp().max(0) as u64))
}

fn cow_to_string(cow: &Option<std::borrow::Cow<'_, [u8]>>) -> String {
    cow.as_ref()
        .map(|bytes| String::from_utf8_lossy(bytes).to_string())
        .unwrap_or_default()
}

fn optional_cow(cow: &Option<std::borrow::Cow<'_, [u8]>>) -> Option<String> {
    let value = cow_to_string(cow);
    if value.is_empty() { None } else { Some(value) }
}

fn uid_set(uids: &[u32]) -> String {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for uid in sorted {
        match ranges.last_mut() {
            Some((start, end)) if *end == uid.saturating_sub(1) => *end = uid,
            _ => ranges.push((uid, uid)),
        }
    }
    ranges
        .iter()
        .map(|(start, end)| {
            if start == end {
                start.to_string()
            } else {
                format!("{start}:{end}")
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn flag_add_remove(
    additions: &mut Vec<String>,
    removals: &mut Vec<String>,
    change: &Option<bool>,
    flag: &str,
) {
    match change {
        Some(true) => additions.push(flag.to_string()),
        Some(false) => removals.push(flag.to_string()),
        None => {}
    }
}

async fn append_uidplus(
    session: &mut Session<Stream>,
    folder: &str,
    flags: &str,
    raw: &[u8],
) -> Result<u32> {
    let command = append_command(folder, flags, raw.len());
    let id = session.run_command(command).await.map_err(map_err)?;
    let response = session
        .read_response()
        .await?
        .ok_or(MailError::Disconnected)?;
    if !matches!(response.parsed(), Response::Continue { .. }) {
        return Err(MailError::Protocol(
            "expected APPEND continuation".to_string(),
        ));
    }
    let stream = session.get_mut();
    stream.write_all(raw).await?;
    stream.write_all(b"\r\n").await?;
    stream.flush().await?;
    loop {
        let data = session
            .read_response()
            .await?
            .ok_or(MailError::Disconnected)?;
        if data.request_id() != Some(&id) {
            continue;
        }
        let Response::Done {
            status,
            code,
            information,
            ..
        } = data.parsed()
        else {
            continue;
        };
        if status != &imap_proto::Status::Ok {
            let detail = information
                .as_ref()
                .map(|information| information.to_string())
                .unwrap_or_else(|| "APPEND failed".to_string());
            return Err(MailError::Protocol(detail));
        }
        return Ok(append_uid(code.as_ref()));
    }
}

fn append_command(folder: &str, flags: &str, len: usize) -> String {
    let folder = folder.replace('\\', "\\\\").replace('"', "\\\"");
    let flags = if flags.is_empty() {
        String::new()
    } else {
        format!(" ({flags})")
    };
    format!("APPEND \"{folder}\"{flags} {{{len}}}")
}

fn append_uid(code: Option<&ResponseCode<'_>>) -> u32 {
    match code {
        Some(ResponseCode::AppendUid(_, uids)) => uids.first().map(uid_member).unwrap_or(0),
        _ => 0,
    }
}

fn uid_member(member: &UidSetMember) -> u32 {
    match member {
        UidSetMember::Uid(uid) => *uid,
        UidSetMember::UidRange(range) => *range.start(),
    }
}

fn append_flags(flags: MessageFlags) -> String {
    let mut result = Vec::new();
    if flags.seen {
        result.push("\\Seen");
    }
    if flags.answered {
        result.push("\\Answered");
    }
    if flags.flagged {
        result.push("\\Flagged");
    }
    if flags.deleted {
        result.push("\\Deleted");
    }
    if flags.draft {
        result.push("\\Draft");
    }
    result.join(" ")
}

#[cfg(test)]
mod tests {
    use crate::envelope::{FlagChange, MessageFlags};

    use super::{append_flags, delta_query, display_name, folder_role, uid_set};

    #[test]
    fn uid_set_compresses_ranges() {
        assert_eq!(uid_set(&[]), "");
        assert_eq!(uid_set(&[5]), "5");
        assert_eq!(uid_set(&[1, 2, 3, 5, 7, 8]), "1:3,5,7:8");
        assert_eq!(uid_set(&[9, 1, 2]), "1:2,9");
    }

    #[test]
    fn append_flags_formats() {
        assert_eq!(append_flags(MessageFlags::default()), "");
        assert_eq!(
            append_flags(MessageFlags {
                seen: true,
                flagged: true,
                ..MessageFlags::default()
            }),
            "\\Seen \\Flagged"
        );
    }

    #[test]
    fn folder_role_maps_attributes() {
        use async_imap::types::NameAttribute;
        let inbox = folder_role("INBOX", &[NameAttribute::NoInferiors]);
        assert_eq!(inbox, crate::FolderRole::Inbox);
        let drafts = folder_role(
            "[Gmail]/Drafts",
            &[NameAttribute::NoSelect, NameAttribute::Drafts],
        );
        assert_eq!(drafts, crate::FolderRole::Drafts);
        let other = folder_role("Work", &[NameAttribute::NoInferiors]);
        assert_eq!(other, crate::FolderRole::Other);
        let important = folder_role(
            "[Gmail]/Important",
            &[NameAttribute::Extension("\\Important".into())],
        );
        assert_eq!(important, crate::FolderRole::Important);
        let all = folder_role("All Mail", &[NameAttribute::All]);
        assert_eq!(all, crate::FolderRole::All);
        let flagged = folder_role("Starred", &[NameAttribute::Flagged]);
        assert_eq!(flagged, crate::FolderRole::Flagged);
    }

    #[test]
    fn display_name_takes_last_component() {
        assert_eq!(display_name("INBOX"), "INBOX");
        assert_eq!(display_name("[Gmail]/Sent Mail"), "Sent Mail");
        assert_eq!(display_name("INBOX/Archive"), "Archive");
    }

    #[test]
    fn delta_query_requests_vanished_only_with_qresync() {
        assert_eq!(
            delta_query(42, false),
            "(UID ENVELOPE FLAGS RFC822.SIZE INTERNALDATE MODSEQ) (CHANGEDSINCE 42)"
        );
        assert_eq!(
            delta_query(42, true),
            "(UID ENVELOPE FLAGS RFC822.SIZE INTERNALDATE MODSEQ) (CHANGEDSINCE 42 VANISHED)"
        );
    }

    #[test]
    fn flag_add_remove_round_trip() {
        let mut additions = Vec::new();
        let mut removals = Vec::new();
        let change = FlagChange {
            seen: Some(true),
            flagged: Some(false),
            draft: None,
            ..FlagChange::default()
        };
        super::flag_add_remove(&mut additions, &mut removals, &change.seen, "\\Seen");
        super::flag_add_remove(&mut additions, &mut removals, &change.flagged, "\\Flagged");
        super::flag_add_remove(&mut additions, &mut removals, &change.draft, "\\Draft");
        assert_eq!(additions, vec!["\\Seen"]);
        assert_eq!(removals, vec!["\\Flagged"]);
    }

    #[test]
    fn append_command_quotes_folders_and_flags() {
        assert_eq!(
            super::append_command("INBOX", "", 5),
            "APPEND \"INBOX\" {5}"
        );
        assert_eq!(
            super::append_command("INBOX", "\\Seen", 5),
            "APPEND \"INBOX\" (\\Seen) {5}"
        );
        assert_eq!(
            super::append_command("[Gmail]/Sent Mail", "\\Seen", 1024),
            "APPEND \"[Gmail]/Sent Mail\" (\\Seen) {1024}"
        );
        assert_eq!(
            super::append_command("a\"b", "", 1),
            "APPEND \"a\\\"b\" {1}"
        );
    }

    #[test]
    fn append_uid_reads_the_uidplus_response_code() {
        use imap_proto::{ResponseCode, UidSetMember};
        assert_eq!(
            super::append_uid(Some(&ResponseCode::AppendUid(
                38505,
                vec![UidSetMember::Uid(3955)]
            ))),
            3955
        );
        assert_eq!(
            super::append_uid(Some(&ResponseCode::AppendUid(
                38505,
                vec![UidSetMember::UidRange(100..=200)]
            ))),
            100
        );
        assert_eq!(super::append_uid(None), 0);
        assert_eq!(super::append_uid(Some(&ResponseCode::UidNext(9))), 0);
    }
}
