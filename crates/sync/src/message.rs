// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::{SystemTime, UNIX_EPOCH};

use mail_core::envelope::{Address, Envelope};
use store::{BodyState, MessageRecord, flags_to_bits};

pub fn message_record(folder_id: i64, envelope: &Envelope) -> MessageRecord {
    let (from_addr, from_name) = envelope
        .from
        .first()
        .map(|address| (address.address.clone(), address.name.clone()))
        .unwrap_or((None, None));
    MessageRecord {
        id: None,
        folder_id,
        uid: envelope.uid,
        modseq: envelope
            .modseq
            .and_then(|modseq| i64::try_from(modseq).ok()),
        message_id: envelope.message_id.clone(),
        thread_id: None,
        subject: envelope.subject.clone(),
        from_addr,
        from_name,
        to_addrs: addresses_json(&envelope.to),
        cc_addrs: addresses_json(&envelope.cc),
        date_sent: None,
        date_recv: envelope.date.and_then(unix_seconds),
        in_reply_to: envelope.in_reply_to.clone(),
        refs: string_list_json(&envelope.references),
        flags: flags_to_bits(envelope.flags),
        has_attach: false,
        size: Some(i64::from(envelope.size)),
        structure: None,
        raw_path: None,
        body_state: BodyState::None,
    }
}

fn addresses_json(addresses: &[Address]) -> Option<String> {
    let values: Vec<&str> = addresses
        .iter()
        .filter_map(|address| address.address.as_deref())
        .collect();
    string_list_json(&values)
}

fn string_list_json(values: &[impl AsRef<str>]) -> Option<String> {
    if values.is_empty() {
        return None;
    }
    let encoded: Vec<String> = values
        .iter()
        .map(|value| json_string(value.as_ref()))
        .collect();
    Some(format!("[{}]", encoded.join(",")))
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(character),
        }
    }
    out.push('"');
    out
}

fn unix_seconds(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use mail_core::envelope::{Address, Envelope, MessageFlags};

    use super::message_record;

    fn envelope() -> Envelope {
        Envelope {
            uid: 7,
            modseq: Some(99),
            flags: MessageFlags {
                seen: true,
                ..MessageFlags::default()
            },
            size: 1234,
            subject: "hello".to_string(),
            from: vec![Address {
                name: Some("Sender".to_string()),
                address: Some("sender@example.org".to_string()),
            }],
            to: vec![Address {
                name: None,
                address: Some("me@example.org".to_string()),
            }],
            cc: Vec::new(),
            date: Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            message_id: Some("<7@example.org>".to_string()),
            in_reply_to: Some("<6@example.org>".to_string()),
            references: vec!["<5@example.org>".to_string(), "<6@example.org>".to_string()],
        }
    }

    #[test]
    fn maps_envelope_fields() {
        let record = message_record(3, &envelope());
        assert_eq!(record.folder_id, 3);
        assert_eq!(record.uid, 7);
        assert_eq!(record.subject, "hello");
        assert_eq!(record.from_addr.as_deref(), Some("sender@example.org"));
        assert_eq!(record.from_name.as_deref(), Some("Sender"));
        assert_eq!(record.to_addrs.as_deref(), Some(r#"["me@example.org"]"#));
        assert_eq!(record.cc_addrs, None);
        assert_eq!(record.date_recv, Some(1_700_000_000));
        assert_eq!(record.size, Some(1234));
        assert_eq!(record.modseq, Some(99));
        assert_eq!(record.message_id.as_deref(), Some("<7@example.org>"));
        assert_eq!(record.in_reply_to.as_deref(), Some("<6@example.org>"));
        assert_eq!(
            record.refs.as_deref(),
            Some(r#"["<5@example.org>","<6@example.org>"]"#)
        );
        assert_eq!(record.flags, store::FLAG_SEEN);
    }
}
